//! Media subsystem: detection, processing, storage, and prompt integration.
//!
//! Handles inbound and outbound media attached to channel messages:
//!
//! - **[`detection`]** — MIME type detection and content classification for incoming attachments.
//! - **[`processing`]** — Runs STT (speech-to-text), TTS (text-to-speech), and image description
//!   pipelines via [`MediaProcessor`]. Config is provided by [`SttConfig`] / [`TtsConfig`].
//! - **[`storage`]** — Persists media blobs via [`MediaStore`] and resolves attachment URLs.
//! - **[`types`]** — Shared attachment and media descriptor types used across the channel layer.

pub(crate) mod detection;
pub(crate) mod processing;
pub(crate) mod storage;
pub(crate) mod types;

pub(crate) use processing::{MediaProcessor, SttConfig, TtsConfig, describe_media_for_prompt};
pub(crate) use storage::MediaStore;
