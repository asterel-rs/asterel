use std::fmt;
use std::sync::Arc;

use crate::media::MediaStore;
use crate::media::types::MediaFile;

pub(super) struct StoredAttachment {
    file: MediaFile,
    bytes: Vec<u8>,
}

impl StoredAttachment {
    pub(super) fn file(&self) -> &MediaFile {
        &self.file
    }

    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(super) fn size_bytes(&self) -> usize {
        self.bytes.len()
    }
}

#[derive(Debug)]
pub(super) struct AttachmentStoreError {
    source: anyhow::Error,
    size_bytes: usize,
}

impl AttachmentStoreError {
    fn new(source: impl Into<anyhow::Error>, size_bytes: usize) -> Self {
        Self {
            source: source.into(),
            size_bytes,
        }
    }

    pub(super) fn size_bytes(&self) -> usize {
        self.size_bytes
    }
}

impl fmt::Display for AttachmentStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl std::error::Error for AttachmentStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.source.root_cause())
    }
}

pub(super) async fn persist_attachment(
    store: &Arc<MediaStore>,
    bytes: Vec<u8>,
    filename: Option<String>,
) -> Result<StoredAttachment, AttachmentStoreError> {
    let size_bytes = bytes.len();
    let store = Arc::clone(store);

    let task_result = tokio::task::spawn_blocking(move || {
        let stored = store.store(&bytes, filename.as_deref());
        (stored, bytes)
    })
    .await;

    match task_result {
        Ok((Ok(file), bytes)) => Ok(StoredAttachment { file, bytes }),
        Ok((Err(error), _)) => Err(AttachmentStoreError::new(error, size_bytes)),
        Err(error) => Err(AttachmentStoreError::new(error, size_bytes)),
    }
}
