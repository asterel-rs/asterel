use anyhow::Context;
use chrono::Utc;
use hmac::{KeyInit, Mac};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::http_providers::{build_embedding_http_client, extract_float_vector, send_json_request};
use super::{EmbeddingFuture, EmbeddingProvider, HmacSha256};

pub(super) struct BedrockEmbedding {
    client: reqwest::Client,
    endpoint: String,
    host: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    model: String,
    dims: usize,
}

impl BedrockEmbedding {
    pub(super) fn new(
        endpoint: &str,
        region: &str,
        access_key_id: &str,
        secret_access_key: &str,
        session_token: Option<String>,
        model: &str,
        dims: usize,
    ) -> anyhow::Result<Self> {
        let parsed = reqwest::Url::parse(endpoint).context("invalid bedrock embedding endpoint")?;
        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("bedrock embedding endpoint missing host"))?
            .to_string();
        Ok(Self {
            client: build_embedding_http_client(),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            host,
            region: region.to_string(),
            access_key_id: access_key_id.to_string(),
            secret_access_key: secret_access_key.to_string(),
            session_token,
            model: model.to_string(),
            dims,
        })
    }

    #[cfg(test)]
    pub(super) fn new_for_tests(
        endpoint: &str,
        region: &str,
        access_key_id: &str,
        secret_access_key: &str,
        model: &str,
        dims: usize,
    ) -> anyhow::Result<Self> {
        Self::new(
            endpoint,
            region,
            access_key_id,
            secret_access_key,
            None,
            model,
            dims,
        )
    }

    fn canonical_uri(&self) -> String {
        let encoded_model =
            url::form_urlencoded::byte_serialize(self.model.as_bytes()).collect::<String>();
        format!("/model/{encoded_model}/invoke")
    }

    fn payload_for_text(&self, text: &str) -> String {
        json!({
            "inputText": text,
            "dimensions": self.dims,
        })
        .to_string()
    }

    fn hex_sha256(input: &[u8]) -> String {
        hex::encode(Sha256::digest(input))
    }

    fn hmac_sign(key: &[u8], data: &str) -> anyhow::Result<Vec<u8>> {
        let mut mac = HmacSha256::new_from_slice(key)
            .map_err(|error| anyhow::anyhow!("invalid HMAC key length: {error}"))?;
        mac.update(data.as_bytes());
        Ok(mac.finalize().into_bytes().to_vec())
    }

    fn signing_key(&self, datestamp: &str) -> anyhow::Result<Vec<u8>> {
        let k_date = Self::hmac_sign(
            format!("AWS4{}", self.secret_access_key).as_bytes(),
            datestamp,
        )?;
        let k_region = Self::hmac_sign(&k_date, &self.region)?;
        let k_service = Self::hmac_sign(&k_region, "bedrock")?;
        Self::hmac_sign(&k_service, "aws4_request")
    }

    fn authorization_header(
        &self,
        payload_hash: &str,
        amz_date: &str,
        datestamp: &str,
    ) -> anyhow::Result<String> {
        let mut canonical_headers = vec![
            format!("host:{}\n", self.host),
            format!("x-amz-content-sha256:{payload_hash}\n"),
            format!("x-amz-date:{amz_date}\n"),
        ];
        let mut signed_headers = vec!["host", "x-amz-content-sha256", "x-amz-date"];
        if let Some(session_token) = &self.session_token {
            canonical_headers.push(format!("x-amz-security-token:{session_token}\n"));
            signed_headers.push("x-amz-security-token");
        }

        canonical_headers.sort_unstable();
        signed_headers.sort_unstable();

        let canonical_request = format!(
            "POST\n{}\n\n{}\
\n{}\n{}",
            self.canonical_uri(),
            canonical_headers.concat(),
            signed_headers.join(";"),
            payload_hash
        );
        let credential_scope = format!("{datestamp}/{}/bedrock/aws4_request", self.region);
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{amz_date}\n{credential_scope}\n{}",
            Self::hex_sha256(canonical_request.as_bytes())
        );
        let signing_key = self.signing_key(datestamp)?;
        let signature = hex::encode(Self::hmac_sign(&signing_key, &string_to_sign)?);

        Ok(format!(
            "AWS4-HMAC-SHA256 Credential={}/{credential_scope}, SignedHeaders={}, Signature={signature}",
            self.access_key_id,
            signed_headers.join(";"),
        ))
    }

    async fn embed_one_text(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let payload = self.payload_for_text(text);
        let payload_hash = Self::hex_sha256(payload.as_bytes());
        let now = Utc::now();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let datestamp = now.format("%Y%m%d").to_string();
        let authorization = self.authorization_header(&payload_hash, &amz_date, &datestamp)?;
        let url = format!("{}{}", self.endpoint, self.canonical_uri());

        let mut request = self
            .client
            .post(url)
            .header("Authorization", authorization)
            .header("Content-Type", "application/json")
            .header("x-amz-content-sha256", payload_hash)
            .header("x-amz-date", amz_date)
            .body(payload);
        if let Some(session_token) = &self.session_token {
            request = request.header("x-amz-security-token", session_token);
        }

        let json = send_json_request("bedrock", request).await?;
        let embedding = json
            .get("embedding")
            .ok_or_else(|| anyhow::anyhow!("bedrock response missing `embedding`"))?;
        extract_float_vector(embedding, "bedrock")
    }
}

impl EmbeddingProvider for BedrockEmbedding {
    fn name(&self) -> &'static str {
        "bedrock"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    fn embed<'a>(&'a self, texts: &'a [&'a str]) -> EmbeddingFuture<'a, Vec<Vec<f32>>> {
        Box::pin(async move {
            let mut embeddings = Vec::with_capacity(texts.len());
            for &text in texts {
                embeddings.push(self.embed_one_text(text).await?);
            }
            Ok(embeddings)
        })
    }
}
