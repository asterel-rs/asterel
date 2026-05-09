//! No-op tunnel: direct local access with no external exposure,
//! used when tunnel functionality is not needed.

use std::future::Future;
use std::pin::Pin;

use anyhow::Result;

use super::Tunnel;

/// No-op tunnel — direct local access, no external exposure.
pub struct NoneTunnel;

impl Tunnel for NoneTunnel {
    fn name(&self) -> &'static str {
        "none"
    }

    fn start<'a>(
        &'a self,
        local_host: &'a str,
        local_port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + 'a>> {
        let url = format!("http://{local_host}:{local_port}");
        Box::pin(async move { Ok(url) })
    }

    fn stop(&self) -> Pin<Box<dyn Future<Output = Result<()>> + Send + '_>> {
        Box::pin(async move { Ok(()) })
    }

    fn health_check(&self) -> Pin<Box<dyn Future<Output = bool> + Send + '_>> {
        Box::pin(async move { true })
    }

    fn public_url(&self) -> Option<String> {
        None
    }
}
