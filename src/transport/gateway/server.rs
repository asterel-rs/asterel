//! HTTP/WS gateway server bootstrap: builds the Axum router, binds the
//! listener, and wires handler endpoints to shared application state.
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum::Router;
use axum::http::{HeaderName, StatusCode};
use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::services::ServeDir;
use tower_http::timeout::TimeoutLayer;

#[cfg(feature = "whatsapp")]
use super::GatewayWhatsAppState;
use super::handlers;
use super::replay_guard::ReplayGuard;
use super::{
    AppState, GatewayAccessState, GatewayCompanionState, GatewayConnectionState,
    GatewayRuntimeState, MEDIA_BODY_SIZE, REQUEST_TIMEOUT_SECS,
};
use crate::config::Config;
use crate::core::memory;
use crate::runtime::services::{
    GatewayReadinessProfile, RuntimeSurfaceResources, load_companion_admin_settings,
    new_a2a_task_store, new_tenant_binding_store, prepare_gateway_surface_plan,
};
use crate::security::pairing::{PairingGuard, is_public_bind};
#[cfg(feature = "whatsapp")]
use crate::transport::channels::WhatsAppChannel;

/// Run the HTTP gateway using axum with proper HTTP/1.1 compliance.
///
/// # Errors
///
/// Returns an error when public bind is refused by policy, when the bind
/// address cannot be parsed or bound, or when gateway startup fails.
pub async fn run_gateway(host: &str, port: u16, config: Arc<Config>) -> Result<()> {
    run_gateway_with_profile(host, port, config, GatewayReadinessProfile::Standalone).await
}

/// # Errors
/// Returns an error when gateway startup or serving fails.
pub async fn run_gateway_with_profile(
    host: &str,
    port: u16,
    config: Arc<Config>,
    readiness_profile: GatewayReadinessProfile,
) -> Result<()> {
    // ── Security: refuse public bind without tunnel or explicit opt-in ──
    if is_public_bind(host)
        && config.tunnel.provider == crate::config::TunnelProvider::None
        && !config.gateway.allow_public_bind
    {
        anyhow::bail!(
            "🛑 Refusing to bind to {host} — gateway would be exposed to the internet.\n\
             Fix: use --host 127.0.0.1 (default), configure a tunnel, or set\n\
             [gateway] allow_public_bind = true in config.toml (NOT recommended)."
        );
    }

    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .context("parse gateway bind address")?;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .context("bind gateway socket")?;

    run_gateway_with_listener(host, listener, config, readiness_profile).await
}

fn resolve_webhook_secret(config: &Config) -> Option<Arc<str>> {
    config
        .channels_config
        .webhook
        .as_ref()
        .and_then(|webhook| webhook.secret.as_deref())
        .map(Arc::from)
}

#[cfg(feature = "whatsapp")]
fn build_whatsapp_channel(config: &Config) -> Option<Arc<WhatsAppChannel>> {
    config.channels_config.whatsapp.as_ref().map(|whatsapp| {
        Arc::new(WhatsAppChannel::new(
            whatsapp.access_token.clone(),
            whatsapp.phone_number_id.clone(),
            whatsapp.verify_token.clone(),
            whatsapp.allowed_numbers.clone(),
        ))
    })
}

fn build_gateway_state(
    config: &Arc<Config>,
    resources: RuntimeSurfaceResources,
    pairing: Arc<PairingGuard>,
    webhook_secret: Option<Arc<str>>,
    readiness_profile: GatewayReadinessProfile,
) -> AppState {
    let companion_context_ingestion = Arc::new(memory::DefaultIngestPipeline::new(Arc::clone(
        &resources.memory,
    )));
    let companion_context_gates = super::CompanionContextGateStore::new();
    let companion_caption_logs = super::CompanionCaptionLogStore::new();
    let companion_widget_runtimes = super::CompanionWidgetRuntimeStore::new();
    let companion_request_windows = super::CompanionRequestWindowStore::new();
    let (gateway_events, _gateway_events_rx) = tokio::sync::broadcast::channel(256);
    let companion_settings = load_companion_admin_settings(config).unwrap_or_else(|error| {
        tracing::warn!(
            %error,
            "gateway: failed to load persisted companion settings; using defaults"
        );
        super::GatewayCompanionSettings::default()
    });

    AppState {
        runtime: GatewayRuntimeState {
            provider: resources.provider,
            registry: resources.registry,
            subagent_manager: resources.subagents,
            rate_limiter: resources.rate_limiter,
            max_tool_loop_iterations: config.autonomy.max_tool_loop_iterations,
            loop_detection: config.tools.loop_detection.clone(),
            permission_store: resources.permission_store,
            model: resources.model_name,
            temperature: resources.temperature,
            session_history_max_tokens: usize::try_from(config.session.parent_fork_max_tokens)
                .unwrap_or(100_000),
            mem: resources.memory,
            observer: resources.observer,
            auto_save: config.memory.auto_save,
            security: resources.security,
            external_knowledge_trust: config.security.external_knowledge_trust.clone(),
            session_manager: resources.sessions,
            self_amendment_candidate_review: resources.self_amendment_candidate_review,
            readiness_profile,
            config: Arc::clone(config),
        },
        access: GatewayAccessState {
            webhook_secret,
            pairing,
            defense_mode: config.gateway.defense_mode,
            defense_kill_switch: config.gateway.defense_kill_switch,
        },
        companion: GatewayCompanionState {
            replay_guard: Arc::new(ReplayGuard::new_with_storage(
                config
                    .workspace_dir
                    .join(".asterel")
                    .join("gateway")
                    .join("replay_guard.json"),
            )),
            settings: Arc::new(RwLock::new(companion_settings)),
            companion_context_gates,
            companion_context_ingestion,
            companion_caption_logs,
            companion_widget_runtimes,
            companion_request_windows,
            gateway_events,
        },
        connections: GatewayConnectionState {
            a2a_tasks: new_a2a_task_store(),
            tenant_bindings: new_tenant_binding_store(config),
            active_ws_connections: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        },
        #[cfg(feature = "whatsapp")]
        whatsapp: GatewayWhatsAppState {
            channel: build_whatsapp_channel(config),
            app_secret: resolve_whatsapp_app_secret(config),
        },
    }
}

/// Run the HTTP gateway from a pre-bound listener.
///
/// # Errors
///
/// Returns an error when gateway dependencies cannot be initialized, tunnel
/// startup fails, or HTTP serving exits with an error.
pub async fn run_gateway_with_listener(
    host: &str,
    listener: tokio::net::TcpListener,
    config: Arc<Config>,
    readiness_profile: GatewayReadinessProfile,
) -> Result<()> {
    let actual_port = listener
        .local_addr()
        .context("get gateway listener local address")?
        .port();
    let display_addr = format!("{host}:{actual_port}");

    let gateway_plan = prepare_gateway_surface_plan(config.as_ref()).await?;
    let resources = gateway_plan.compose(config.as_ref()).await?;
    let webhook_secret = resolve_webhook_secret(&config);

    #[cfg(feature = "whatsapp")]
    let whatsapp_enabled = config.channels_config.whatsapp.is_some();
    #[cfg(feature = "whatsapp")]
    let whatsapp_signature_ready = resolve_whatsapp_app_secret(&config).is_some();
    #[cfg(not(feature = "whatsapp"))]
    let whatsapp_enabled = false;
    #[cfg(not(feature = "whatsapp"))]
    let whatsapp_signature_ready = false;

    let pairing_store_path = config
        .workspace_dir
        .join(".asterel")
        .join("gateway")
        .join("pairing_tokens.json");
    let pairing = Arc::new(PairingGuard::new_with_storage(
        config.gateway.require_pairing,
        &config.gateway.paired_tokens,
        Some(config.gateway.token_ttl_secs),
        Some(pairing_store_path),
    ));

    let tunnel_url = start_tunnel(&config, host, actual_port)
        .await
        .context("start gateway tunnel")?;

    print_gateway_banner(
        &display_addr,
        tunnel_url.as_deref(),
        whatsapp_enabled,
        whatsapp_signature_ready,
        &pairing,
        webhook_secret.is_some(),
    );

    crate::runtime::diagnostics::health::mark_component_ok("gateway");

    let state = build_gateway_state(
        &config,
        resources,
        pairing,
        webhook_secret,
        readiness_profile,
    );

    let app = build_app(
        state,
        &config.gateway.cors_origins,
        config.gateway.static_dir.as_deref(),
        config.gateway.max_body_size_bytes,
    );
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .context("serve HTTP gateway")?;

    Ok(())
}

// Priority: environment variable > config file.
#[cfg(feature = "whatsapp")]
fn resolve_whatsapp_app_secret(config: &Config) -> Option<Arc<str>> {
    std::env::var("ASTEREL_WHATSAPP_APP_SECRET")
        .ok()
        .and_then(|secret| {
            let secret = secret.trim();
            (!secret.is_empty()).then(|| secret.to_owned())
        })
        .or_else(|| {
            config.channels_config.whatsapp.as_ref().and_then(|wa| {
                wa.app_secret
                    .as_deref()
                    .map(str::trim)
                    .filter(|secret| !secret.is_empty())
                    .map(ToOwned::to_owned)
            })
        })
        .map(Arc::from)
}

async fn start_tunnel(config: &Config, host: &str, port: u16) -> Result<Option<String>> {
    let security = crate::security::SecurityPolicy::from_config_runtime(
        &config.autonomy,
        &config.runtime,
        &config.workspace_dir,
    );
    let tunnel = crate::runtime::tunnel::create_tunnel(&config.tunnel, &security)
        .context("create tunnel for gateway")?;

    let Some(ref tun) = tunnel else {
        return Ok(None);
    };

    println!("› {}", t!("gateway.tunnel_starting", name = tun.name()));
    match tun.start(host, port).await {
        Ok(url) => {
            println!("✓ {}", t!("gateway.tunnel_active", url = url));
            Ok(Some(url))
        }
        Err(e) => {
            println!("! {}", t!("gateway.tunnel_failed", error = e));
            println!("   {}", t!("gateway.tunnel_fallback"));
            Ok(None)
        }
    }
}

#[allow(clippy::fn_params_excessive_bools)] // Banner display flags are naturally boolean
fn print_gateway_banner(
    display_addr: &str,
    tunnel_url: Option<&str>,
    whatsapp_enabled: bool,
    whatsapp_signature_ready: bool,
    pairing: &PairingGuard,
    webhook_secret_enabled: bool,
) {
    println!("◆ {}", t!("gateway.listening", addr = display_addr));
    if let Some(url) = tunnel_url {
        println!("  › {}", t!("gateway.public_url", url = url));
    }
    println!("  {}", t!("gateway.route_pair"));
    println!("  {}", t!("gateway.route_webhook"));
    println!("  POST /companion/context/ingest → Companion context ingress");
    println!("  POST /companion/multimodal/ingest → Companion multimodal memory ingress");
    println!("  POST /companion/surface/caption → Companion caption event");
    println!("  POST /companion/surface/widget → Companion widget command");
    println!("  POST /companion/surface/request-window/open → Companion request-window open");
    println!("  GET /ws → WebSocket");
    println!("  GET /.well-known/agent.json → A2A agent card");
    println!("  POST /a2a/v1/messages → A2A message ingress");
    println!("  GET /a2a/v1/tasks → A2A task listing endpoint");
    println!("  GET /a2a/v1/tasks/<id> → A2A task lookup endpoint");
    println!("  POST /a2a/v1/tasks/<id>/cancel → A2A task cancel endpoint");
    if whatsapp_enabled {
        println!("  {}", t!("gateway.route_whatsapp_get"));
        println!("  {}", t!("gateway.route_whatsapp_post"));
        if !whatsapp_signature_ready {
            println!(
                "  ! WhatsApp app_secret missing: POST /whatsapp will reject until configured"
            );
        }
    }
    println!("  {}", t!("gateway.route_health"));
    if let Some(code) = pairing.pairing_code() {
        println!();
        println!("  ✓ {}", t!("gateway.pairing_required"));
        println!("     ┌──────────────┐");
        println!("     │  {code}  │");
        println!("     └──────────────┘");
        println!("     {}", t!("gateway.pairing_send", code = code));
    } else if pairing.require_pairing() {
        println!("  ✓ {}", t!("gateway.pairing_active"));
    } else {
        println!("  ! {}", t!("gateway.pairing_disabled"));
    }
    if webhook_secret_enabled {
        println!("  ✓ {}", t!("gateway.webhook_secret_enabled"));
    }
    println!("  {}\n", t!("gateway.stop_hint"));
}

fn build_app(
    state: AppState,
    cors_origins: &[String],
    static_dir: Option<&str>,
    max_body_size_bytes: usize,
) -> Router {
    let max_body_size_bytes = if max_body_size_bytes == 0 {
        tracing::warn!(
            fallback_limit = 65_536,
            "invalid gateway max_body_size_bytes value; using default limit"
        );
        65_536
    } else {
        max_body_size_bytes
    };

    let app = handlers::build_public_routes()
        .merge(handlers::build_admin_routes())
        .layer(RequestBodyLimitLayer::new(max_body_size_bytes));

    #[cfg(feature = "whatsapp")]
    let app = handlers::build_whatsapp_routes(app);

    let app = app.merge(
        handlers::build_admin_upload_routes().layer(RequestBodyLimitLayer::new(MEDIA_BODY_SIZE)),
    );

    apply_gateway_layers(app, state, cors_origins, static_dir)
}

fn apply_gateway_layers(
    app: Router<AppState>,
    state: AppState,
    cors_origins: &[String],
    static_dir: Option<&str>,
) -> Router {
    let mut app = app.with_state(state).layer(TimeoutLayer::with_status_code(
        StatusCode::REQUEST_TIMEOUT,
        Duration::from_secs(REQUEST_TIMEOUT_SECS),
    ));

    if !cors_origins.is_empty() {
        let origins: Vec<_> = cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
        app = app.layer(
            CorsLayer::new()
                .allow_origin(origins)
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                    HeaderName::from_static("x-webhook-secret"),
                    HeaderName::from_static("x-asterel-source"),
                    HeaderName::from_static("x-asterel-tenant"),
                ]),
        );
    }

    // Serve Star Office frontend from a static directory (fallback — does not shadow API routes).
    if let Some(dir) = static_dir {
        app = app.fallback_service(ServeDir::new(dir).append_index_html_on_directories(true));
        tracing::info!(static_dir = dir, "Star Office static file serving enabled");
    }

    app
}
