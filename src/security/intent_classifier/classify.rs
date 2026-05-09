//! ONNX-based intent classifier: tokenize → embed → classify.
//!
//! Only compiled when the `intent-classifier` feature is enabled.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use ort::value::Tensor;
use tokio::sync::oneshot;

use super::session::OnnxSessions;
use super::tokenizer::TokenizerHandle;
use super::traits::IntentClassifier;
use super::types::{ClassificationLabel, ClassificationResult};

/// The ONNX classifier currently supports five labels.
const MAX_CLASS_COUNT: usize = 5;
/// Maximum number of pending classify requests buffered for the worker.
const INFERENCE_QUEUE_CAPACITY: usize = 64;
/// Maximum time to wait for warmup acknowledgement from the worker.
const WARMUP_RESPONSE_TIMEOUT: Duration = Duration::from_secs(10);
/// Maximum time to wait for one inference response from the worker.
const CLASSIFY_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

const fn fail_closed_result() -> ClassificationResult {
    ClassificationResult {
        label: ClassificationLabel::InjectionToolJailbreak,
        confidence: 1.0,
        inference_time_us: 0,
    }
}

struct EmbeddingOutput {
    token_embeddings: Vec<f32>,
    model_seq_len: usize,
    embedding_dim: usize,
}

/// Inner inference state owned by a dedicated worker thread.
struct ClassifierInner {
    sessions: OnnxSessions,
    tokenizer: TokenizerHandle,
}

impl ClassifierInner {
    fn new(models_dir: &Path) -> Self {
        Self {
            sessions: OnnxSessions::new(models_dir),
            tokenizer: TokenizerHandle::new(models_dir),
        }
    }

    fn warmup(&self) -> Result<()> {
        self.sessions
            .embedding_session()
            .context("failed to warm up embedding session")?;
        self.sessions
            .classifier_session()
            .context("failed to warm up classifier session")?;
        Ok(())
    }

    fn run_inference(&self, text: &str) -> Option<ClassificationResult> {
        match self.run_inference_result(text) {
            Ok(result) => Some(result),
            Err(error) => {
                tracing::debug!(error = %error, "intent classification failed");
                None
            }
        }
    }

    fn run_inference_result(&self, text: &str) -> Result<ClassificationResult> {
        let start = Instant::now();

        let (input_ids, attention_mask) = self
            .tokenizer
            .encode_for_embedding(text)
            .context("tokenization failed")?;

        let embedding = self.run_embedding_model(&input_ids, &attention_mask)?;
        let pooled = mean_pool_with_attention(
            &embedding.token_embeddings,
            embedding.model_seq_len,
            embedding.embedding_dim,
            &attention_mask,
        )?;
        let logits = self.run_classifier_model(pooled)?;
        let (best_idx, best_prob) = softmax_argmax(&logits).context("softmax argmax failed")?;

        let label = ClassificationLabel::ALL
            .get(best_idx)
            .copied()
            .context("classifier produced out-of-range class index")?;

        let elapsed = start.elapsed();
        let inference_time_us = u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX);

        Ok(ClassificationResult {
            label,
            confidence: best_prob,
            inference_time_us,
        })
    }

    fn run_embedding_model(
        &self,
        input_ids: &[i64],
        attention_mask: &[i64],
    ) -> Result<EmbeddingOutput> {
        if input_ids.is_empty() {
            bail!("tokenizer produced an empty token sequence");
        }
        let seq_len_i64 = i64::try_from(input_ids.len()).context("token sequence too long")?;

        let embedding_session = self
            .sessions
            .embedding_session()
            .context("embedding session unavailable")?;
        let mut embedding_session = embedding_session
            .lock()
            .map_err(|_| anyhow::anyhow!("embedding session lock poisoned"))?;

        let input_ids_tensor =
            Tensor::<i64>::from_array(([1_i64, seq_len_i64], input_ids.to_vec()))
                .context("failed to build embedding input_ids tensor")?;
        let attention_mask_tensor =
            Tensor::<i64>::from_array(([1_i64, seq_len_i64], attention_mask.to_vec()))
                .context("failed to build embedding attention_mask tensor")?;

        let outputs = embedding_session
            .run(ort::inputs![
                "input_ids" => input_ids_tensor,
                "attention_mask" => attention_mask_tensor,
            ])
            .context("embedding model execution failed")?;

        let (token_shape, token_embeddings) = outputs[0]
            .try_extract_tensor::<f32>()
            .context("embedding output extraction failed")?;

        let embedding_dim = token_shape
            .get(2)
            .copied()
            .and_then(|d| usize::try_from(d).ok())
            .context("embedding output missing/invalid dimension 2")?;
        let model_seq_len = token_shape
            .get(1)
            .copied()
            .and_then(|d| usize::try_from(d).ok())
            .context("embedding output missing/invalid dimension 1")?;

        Ok(EmbeddingOutput {
            token_embeddings: token_embeddings.to_vec(),
            model_seq_len,
            embedding_dim,
        })
    }

    fn run_classifier_model(&self, pooled_embedding: Vec<f32>) -> Result<Vec<f32>> {
        let embedding_dim = i64::try_from(pooled_embedding.len())
            .context("pooled embedding too large for tensor shape")?;

        let classifier_session = self
            .sessions
            .classifier_session()
            .context("classifier session unavailable")?;
        let mut classifier_session = classifier_session
            .lock()
            .map_err(|_| anyhow::anyhow!("classifier session lock poisoned"))?;

        let input_tensor = Tensor::<f32>::from_array(([1_i64, embedding_dim], pooled_embedding))
            .context("failed to build classifier input tensor")?;

        let outputs = classifier_session
            .run(ort::inputs!["input" => input_tensor])
            .context("classifier model execution failed")?;

        let (logit_shape, raw_logits) = outputs[0]
            .try_extract_tensor::<f32>()
            .context("classifier output extraction failed")?;

        let logits: Vec<f32> = raw_logits.to_vec();
        let declared_classes = logit_shape
            .get(1)
            .or_else(|| logit_shape.first())
            .copied()
            .and_then(|d| usize::try_from(d).ok())
            .context("classifier output missing/invalid class dimension")?;

        let class_count = declared_classes.min(logits.len()).min(MAX_CLASS_COUNT);
        if class_count == 0 {
            bail!("classifier returned zero classes");
        }

        Ok(logits.into_iter().take(class_count).collect())
    }
}

enum WorkerMessage {
    Warmup {
        response: mpsc::Sender<bool>,
    },
    Classify {
        text: String,
        response: oneshot::Sender<Option<ClassificationResult>>,
    },
}

struct InferenceDispatcher {
    command_tx: Mutex<mpsc::SyncSender<WorkerMessage>>,
    ready: Arc<AtomicBool>,
}

enum DispatchError {
    QueueFull,
    WorkerUnavailable,
}

impl InferenceDispatcher {
    fn send(&self, message: WorkerMessage) -> std::result::Result<(), DispatchError> {
        match self.command_tx.lock() {
            Ok(command_tx) => match command_tx.try_send(message) {
                Ok(()) => Ok(()),
                Err(mpsc::TrySendError::Full(_)) => Err(DispatchError::QueueFull),
                Err(mpsc::TrySendError::Disconnected(_)) => Err(DispatchError::WorkerUnavailable),
            },
            Err(_) => Err(DispatchError::WorkerUnavailable),
        }
    }
}

fn mean_pool_with_attention(
    token_embeddings: &[f32],
    model_seq_len: usize,
    embedding_dim: usize,
    attention_mask: &[i64],
) -> Result<Vec<f32>> {
    if embedding_dim == 0 {
        bail!("embedding dimension is zero");
    }

    let available_seq_len = token_embeddings.len() / embedding_dim;
    let effective_seq_len = attention_mask
        .len()
        .min(model_seq_len)
        .min(available_seq_len);
    if effective_seq_len == 0 {
        bail!("no tokens available for pooling");
    }

    let mut pooled = vec![0.0f32; embedding_dim];
    let mut mask_sum = 0.0f32;

    for (token_idx, &mask) in attention_mask.iter().take(effective_seq_len).enumerate() {
        if mask <= 0 {
            continue;
        }
        let mask_val = 1.0_f32;

        let base = token_idx
            .checked_mul(embedding_dim)
            .context("token embedding index overflow")?;
        let end = base
            .checked_add(embedding_dim)
            .context("token embedding index overflow")?;
        let row = token_embeddings
            .get(base..end)
            .context("token embedding row out of bounds")?;
        for (pooled_dim, &value) in pooled.iter_mut().zip(row.iter()) {
            *pooled_dim += value * mask_val;
        }
        mask_sum += mask_val;
    }

    if mask_sum == 0.0 {
        bail!("attention mask contained no active tokens");
    }

    for value in &mut pooled {
        *value /= mask_sum;
    }

    Ok(pooled)
}

fn softmax_argmax(logits: &[f32]) -> Option<(usize, f32)> {
    if logits.is_empty() {
        return None;
    }

    let max_logit = logits.iter().copied().reduce(f32::max)?;

    let mut probs = Vec::with_capacity(logits.len());
    let mut exp_sum = 0.0f32;
    for &logit in logits {
        let prob = (logit - max_logit).exp();
        probs.push(prob);
        exp_sum += prob;
    }

    if exp_sum <= 0.0 || !exp_sum.is_finite() {
        return None;
    }

    for prob in &mut probs {
        *prob /= exp_sum;
    }

    probs
        .iter()
        .copied()
        .enumerate()
        .max_by(|left, right| left.1.total_cmp(&right.1))
}

/// ONNX Runtime-backed intent classifier.
///
/// Uses all-MiniLM-L6-v2 for 384-dim embeddings and an `XGBoost` 5-class
/// classifier for intent prediction.
///
/// Inference runs on a single dedicated worker thread. This removes the need
/// for manual `Send/Sync` implementations around ONNX session wrappers while preserving
/// asynchronous call sites.
pub(super) struct OrtIntentClassifier {
    dispatcher: Option<Arc<InferenceDispatcher>>,
}

impl OrtIntentClassifier {
    /// Create a classifier from the model files in the given directory.
    pub fn new(models_dir: &Path) -> Self {
        let sessions = OnnxSessions::new(models_dir);
        let tokenizer = TokenizerHandle::new(models_dir);
        if !sessions.models_exist() || !tokenizer.exists() {
            return Self { dispatcher: None };
        }

        let (command_tx, command_rx) =
            mpsc::sync_channel::<WorkerMessage>(INFERENCE_QUEUE_CAPACITY);
        let ready = Arc::new(AtomicBool::new(true));
        let dispatcher = Arc::new(InferenceDispatcher {
            command_tx: Mutex::new(command_tx),
            ready: Arc::clone(&ready),
        });
        Self::spawn_worker_thread(models_dir.to_path_buf(), &ready, command_rx);

        Self {
            dispatcher: Some(dispatcher),
        }
    }

    fn spawn_worker_thread(
        models_dir: PathBuf,
        ready: &Arc<AtomicBool>,
        command_rx: mpsc::Receiver<WorkerMessage>,
    ) {
        let worker_ready = Arc::clone(ready);
        let spawn_result = std::thread::Builder::new()
            .name("intent-classifier-worker".to_string())
            .spawn(move || {
                let inner = ClassifierInner::new(&models_dir);

                for message in command_rx {
                    match message {
                        WorkerMessage::Warmup { response } => {
                            let warmup_ok = inner.warmup().is_ok();
                            worker_ready.store(warmup_ok, Ordering::Relaxed);
                            if response.send(warmup_ok).is_err() {
                                tracing::debug!(
                                    "intent classifier warmup response receiver dropped"
                                );
                            }
                        }
                        WorkerMessage::Classify { text, response } => {
                            if !worker_ready.load(Ordering::Relaxed) {
                                if response.send(None).is_err() {
                                    tracing::debug!(
                                        "intent classifier fallback response receiver dropped"
                                    );
                                }
                                continue;
                            }
                            if response.send(inner.run_inference(&text)).is_err() {
                                tracing::debug!(
                                    "intent classifier inference response receiver dropped"
                                );
                            }
                        }
                    }
                }
                worker_ready.store(false, Ordering::Relaxed);
            });

        if let Err(error) = spawn_result {
            tracing::warn!(error = %error, "failed to spawn intent classifier worker thread");
            ready.store(false, Ordering::Relaxed);
        }
    }

    /// Try to initialize sessions eagerly. Non-fatal on failure.
    pub fn try_warmup(&self) {
        let Some(dispatcher) = &self.dispatcher else {
            return;
        };
        if !dispatcher.ready.load(Ordering::Relaxed) {
            return;
        }

        let (response_tx, response_rx) = mpsc::channel();
        if let Err(error) = dispatcher.send(WorkerMessage::Warmup {
            response: response_tx,
        }) {
            if matches!(error, DispatchError::WorkerUnavailable) {
                dispatcher.ready.store(false, Ordering::Relaxed);
            }
            return;
        }

        match response_rx.recv_timeout(WARMUP_RESPONSE_TIMEOUT) {
            Ok(warmup_ok) => dispatcher.ready.store(warmup_ok, Ordering::Relaxed),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                tracing::warn!("intent classifier warmup timed out; forcing fail-closed readiness");
                dispatcher.ready.store(false, Ordering::Relaxed);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                dispatcher.ready.store(false, Ordering::Relaxed);
            }
        }
    }
}

impl IntentClassifier for OrtIntentClassifier {
    fn name(&self) -> &'static str {
        "ort-intent-classifier"
    }

    fn is_ready(&self) -> bool {
        self.dispatcher
            .as_ref()
            .is_some_and(|dispatcher| dispatcher.ready.load(Ordering::Relaxed))
    }

    fn classify<'a>(
        &'a self,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Option<ClassificationResult>> + Send + 'a>> {
        Box::pin(async move {
            let Some(dispatcher) = &self.dispatcher else {
                return None;
            };
            if !dispatcher.ready.load(Ordering::Relaxed) {
                return Some(fail_closed_result());
            }

            let (response_tx, response_rx) = oneshot::channel();
            if let Err(error) = dispatcher.send(WorkerMessage::Classify {
                text: text.to_string(),
                response: response_tx,
            }) {
                match error {
                    DispatchError::QueueFull => return Some(fail_closed_result()),
                    DispatchError::WorkerUnavailable => {
                        dispatcher.ready.store(false, Ordering::Relaxed);
                        return Some(fail_closed_result());
                    }
                }
            }

            if let Ok(Ok(result)) =
                tokio::time::timeout(CLASSIFY_RESPONSE_TIMEOUT, response_rx).await
            {
                result
            } else {
                dispatcher.ready.store(false, Ordering::Relaxed);
                Some(fail_closed_result())
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{mean_pool_with_attention, softmax_argmax};

    #[test]
    fn mean_pool_uses_only_unmasked_tokens() {
        // Two tokens with 2D embeddings: [1, 2] and [3, 4].
        let token_embeddings = vec![1.0f32, 2.0, 3.0, 4.0];
        let pooled = mean_pool_with_attention(&token_embeddings, 2, 2, &[1, 0]).expect("pool");
        assert_eq!(pooled, vec![1.0, 2.0]);
    }

    #[test]
    fn mean_pool_rejects_all_zero_attention() {
        let token_embeddings = vec![1.0f32, 2.0, 3.0, 4.0];
        let err = mean_pool_with_attention(&token_embeddings, 2, 2, &[0, 0]).unwrap_err();
        assert!(
            err.to_string()
                .contains("attention mask contained no active tokens"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn softmax_argmax_returns_best_class_probability() {
        let (idx, prob) = softmax_argmax(&[-1.0, 0.0, 2.0]).expect("argmax");
        assert_eq!(idx, 2);
        assert!(
            prob > 0.8,
            "probability should be high for dominant logit: {prob}"
        );
    }
}
