//! Telegram Bot API low-level methods: multipart media upload, message
//! sending, typing indicators, and file download URL resolution.
use std::path::Path;

use anyhow::Context;
use reqwest::multipart::{Form, Part};

use super::TelegramChannel;
use crate::transport::channels::api_request::{
    CHANNEL_API_MAX_RATE_LIMIT_RETRIES, channel_api_error_message, wait_for_rate_limit,
};

impl TelegramChannel {
    // ── Private helpers ──────────────────────────────────────────────

    async fn send_media_multipart(
        &self,
        endpoint: &str,
        chat_id: &str,
        media_field: &str,
        file_bytes: Vec<u8>,
        file_name: String,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        for attempt in 0..=CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
            let part = Part::bytes(file_bytes.clone()).file_name(file_name.clone());
            let mut form = Form::new()
                .text("chat_id", chat_id.to_string())
                .part(media_field.to_string(), part);
            if let Some(cap) = caption {
                form = form.text("caption", cap.to_string());
            }

            let resp = self
                .client
                .post(self.api_url(endpoint))
                .multipart(form)
                .send()
                .await
                .with_context(|| format!("send Telegram {endpoint}"))?;

            if resp.status().as_u16() == 429 && attempt < CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
                wait_for_rate_limit(resp.headers()).await;
                continue;
            }

            if !resp.status().is_success() {
                let err = channel_api_error_message("Telegram", endpoint, resp).await;
                anyhow::bail!(err);
            }

            return Ok(());
        }

        anyhow::bail!("Telegram {endpoint} failed due to rate limiting")
    }

    async fn send_media_by_url_json(
        &self,
        endpoint: &str,
        chat_id: &str,
        media_field: &str,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let mut body = serde_json::Map::new();
        body.insert(
            "chat_id".to_string(),
            serde_json::Value::String(chat_id.to_string()),
        );
        body.insert(
            media_field.to_string(),
            serde_json::Value::String(url.to_string()),
        );
        if let Some(cap) = caption {
            body.insert(
                "caption".to_string(),
                serde_json::Value::String(cap.to_string()),
            );
        }
        let body = serde_json::Value::Object(body);
        let operation = format!("{endpoint} by URL");
        for attempt in 0..=CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
            let resp = self
                .client
                .post(self.api_url(endpoint))
                .json(&body)
                .send()
                .await
                .with_context(|| format!("send Telegram {operation}"))?;

            if resp.status().as_u16() == 429 && attempt < CHANNEL_API_MAX_RATE_LIMIT_RETRIES {
                wait_for_rate_limit(resp.headers()).await;
                continue;
            }

            if !resp.status().is_success() {
                let err = channel_api_error_message("Telegram", &operation, resp).await;
                anyhow::bail!(err);
            }

            return Ok(());
        }

        anyhow::bail!("Telegram {operation} failed due to rate limiting")
    }

    async fn read_file_bytes(
        file_path: &Path,
        default_name: &str,
    ) -> anyhow::Result<(Vec<u8>, String)> {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(default_name)
            .to_string();
        let file_bytes = tokio::fs::read(file_path)
            .await
            .with_context(|| format!("read file for Telegram {default_name}"))?;
        Ok((file_bytes, file_name))
    }

    // ── Document ─────────────────────────────────────────────────────

    /// Send a document/file to a Telegram chat.
    /// # Errors
    /// Returns an error if reading the file, the HTTP request, or the Telegram
    /// API response fails.
    pub async fn send_document(
        &self,
        chat_id: &str,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let (file_bytes, name) = Self::read_file_bytes(file_path, "file").await?;
        self.send_media_multipart(
            "sendDocument",
            chat_id,
            "document",
            file_bytes,
            name.clone(),
            caption,
        )
        .await?;
        tracing::info!("Telegram document sent to {chat_id}: {name}");
        Ok(())
    }

    /// Send a document from bytes (in-memory) to a Telegram chat.
    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_document_bytes(
        &self,
        chat_id: &str,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_multipart(
            "sendDocument",
            chat_id,
            "document",
            file_bytes,
            file_name.to_string(),
            caption,
        )
        .await?;
        tracing::info!("Telegram document sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a file by URL (Telegram will download it).
    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_document_by_url(
        &self,
        chat_id: &str,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url_json("sendDocument", chat_id, "document", url, caption)
            .await?;
        tracing::info!("Telegram document (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    // ── Photo ────────────────────────────────────────────────────────

    /// Send a photo to a Telegram chat.
    /// # Errors
    /// Returns an error if reading the file, the HTTP request, or the Telegram
    /// API response fails.
    pub async fn send_photo(
        &self,
        chat_id: &str,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let (file_bytes, name) = Self::read_file_bytes(file_path, "photo.jpg").await?;
        self.send_media_multipart(
            "sendPhoto",
            chat_id,
            "photo",
            file_bytes,
            name.clone(),
            caption,
        )
        .await?;
        tracing::info!("Telegram photo sent to {chat_id}: {name}");
        Ok(())
    }

    /// Send a photo from bytes (in-memory) to a Telegram chat.
    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_photo_bytes(
        &self,
        chat_id: &str,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_multipart(
            "sendPhoto",
            chat_id,
            "photo",
            file_bytes,
            file_name.to_string(),
            caption,
        )
        .await?;
        tracing::info!("Telegram photo sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// Send a photo by URL (Telegram will download it).
    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_photo_by_url(
        &self,
        chat_id: &str,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url_json("sendPhoto", chat_id, "photo", url, caption)
            .await?;
        tracing::info!("Telegram photo (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    // ── Video ────────────────────────────────────────────────────────

    /// Send a video to a Telegram chat.
    /// # Errors
    /// Returns an error if reading the file, the HTTP request, or the Telegram
    /// API response fails.
    pub async fn send_video(
        &self,
        chat_id: &str,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let (file_bytes, name) = Self::read_file_bytes(file_path, "video.mp4").await?;
        self.send_media_multipart(
            "sendVideo",
            chat_id,
            "video",
            file_bytes,
            name.clone(),
            caption,
        )
        .await?;
        tracing::info!("Telegram video sent to {chat_id}: {name}");
        Ok(())
    }

    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_video_bytes(
        &self,
        chat_id: &str,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_multipart(
            "sendVideo",
            chat_id,
            "video",
            file_bytes,
            file_name.to_string(),
            caption,
        )
        .await?;
        tracing::info!("Telegram video sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_video_by_url(
        &self,
        chat_id: &str,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url_json("sendVideo", chat_id, "video", url, caption)
            .await?;
        tracing::info!("Telegram video (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    // ── Audio ────────────────────────────────────────────────────────

    /// Send an audio file to a Telegram chat.
    /// # Errors
    /// Returns an error if reading the file, the HTTP request, or the Telegram
    /// API response fails.
    pub async fn send_audio(
        &self,
        chat_id: &str,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let (file_bytes, name) = Self::read_file_bytes(file_path, "audio.mp3").await?;
        self.send_media_multipart(
            "sendAudio",
            chat_id,
            "audio",
            file_bytes,
            name.clone(),
            caption,
        )
        .await?;
        tracing::info!("Telegram audio sent to {chat_id}: {name}");
        Ok(())
    }

    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_audio_bytes(
        &self,
        chat_id: &str,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_multipart(
            "sendAudio",
            chat_id,
            "audio",
            file_bytes,
            file_name.to_string(),
            caption,
        )
        .await?;
        tracing::info!("Telegram audio sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_audio_by_url(
        &self,
        chat_id: &str,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url_json("sendAudio", chat_id, "audio", url, caption)
            .await?;
        tracing::info!("Telegram audio (URL) sent to {chat_id}: {url}");
        Ok(())
    }

    // ── Voice ────────────────────────────────────────────────────────

    /// Send a voice message to a Telegram chat.
    /// # Errors
    /// Returns an error if reading the file, the HTTP request, or the Telegram
    /// API response fails.
    pub async fn send_voice(
        &self,
        chat_id: &str,
        file_path: &Path,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        let (file_bytes, name) = Self::read_file_bytes(file_path, "voice.ogg").await?;
        self.send_media_multipart(
            "sendVoice",
            chat_id,
            "voice",
            file_bytes,
            name.clone(),
            caption,
        )
        .await?;
        tracing::info!("Telegram voice sent to {chat_id}: {name}");
        Ok(())
    }

    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_voice_bytes(
        &self,
        chat_id: &str,
        file_bytes: Vec<u8>,
        file_name: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_multipart(
            "sendVoice",
            chat_id,
            "voice",
            file_bytes,
            file_name.to_string(),
            caption,
        )
        .await?;
        tracing::info!("Telegram voice sent to {chat_id}: {file_name}");
        Ok(())
    }

    /// # Errors
    /// Returns an error if the HTTP request or the Telegram API response fails.
    pub async fn send_voice_by_url(
        &self,
        chat_id: &str,
        url: &str,
        caption: Option<&str>,
    ) -> anyhow::Result<()> {
        self.send_media_by_url_json("sendVoice", chat_id, "voice", url, caption)
            .await?;
        tracing::info!("Telegram voice (URL) sent to {chat_id}: {url}");
        Ok(())
    }
}
