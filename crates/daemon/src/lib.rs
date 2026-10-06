//! Thin HTTP host for the Chauffeur engine.

mod audit;
mod engine;
mod learned;
mod sourcefed;
mod texts;

pub use engine::{EngineHandle, EngineOptions};

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chauffeur_capability_rules::Rulebook;
use chauffeur_core::{
    DaemonRequest, DaemonResponse, Effect, METHOD_HEALTH, METHOD_RULEBOOKS, METHOD_SHUTDOWN,
    METHOD_SIGNAL, METHOD_TEXTS, Reloading, RulebooksResult, SignalParams, SignalResult, Watch,
};
use chauffeur_judge_jev::JevConfig;
pub use chauffeur_plugin_sourcefed::SourcefedConfig;
use serde_json::json;
use texts::HostTexts;

pub const DEFAULT_PORT: u16 = 18_790;

#[derive(Clone)]
struct AppState {
    engine: EngineHandle,
    token: Option<String>,
    /// The shipped rulebooks, reloaded when their folder changes; each
    /// request adds the workspace's own. `None` when rules are disabled.
    rulebooks: Option<Arc<Mutex<Reloading<Vec<Rulebook>>>>>,
    /// The wording each host shows the agent, reloaded when your file changes.
    texts: Arc<Mutex<HostTexts>>,
    /// Raised by the `shutdown` method; the server stops once requests finish.
    shutdown: Arc<tokio::sync::Notify>,
}

/// The daemon's version, which the host compares with its own.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The texts of the host named in `params.host`.
fn host_texts(state: &AppState, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let host = params["host"].as_str().ok_or("texts needs params.host")?;

    state
        .texts
        .lock()
        .map_err(|_| "texts lock poisoned".to_string())?
        .current(host)
}

/// The rulebooks offered in the request's `workspace`, or none when rules
/// are disabled.
fn offered(state: &AppState, params: &serde_json::Value) -> Result<serde_json::Value, String> {
    let Some(rulebooks) = &state.rulebooks else {
        return serde_json::to_value(RulebooksResult::default()).map_err(|error| error.to_string());
    };
    let shipped = rulebooks
        .lock()
        .map_err(|_| "rulebooks lock poisoned".to_string())?
        .current()
        .clone();
    let workspace = params["workspace"].as_str().unwrap_or_default();
    let offered = engine::rulebooks(&shipped, workspace)?;

    serde_json::to_value(offered).map_err(|error| error.to_string())
}

pub struct DaemonOptions {
    pub address: SocketAddr,
    pub token: Option<String>,
    pub config_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub idle_reminders: bool,
    /// sourcefed's daemon, unless `CHAUFFEUR_SOURCEFED=off`.
    pub sourcefed: Option<SourcefedConfig>,
    /// Capability IDs left out, from `CHAUFFEUR_DISABLE=skill-exposure,rules`.
    pub disabled: Vec<String>,
    pub state_file: Option<PathBuf>,
    pub audit_file: Option<PathBuf>,
}

impl DaemonOptions {
    pub fn from_env(address: SocketAddr) -> Result<Self, String> {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        let state = state_dir(Path::new(&home))?;
        let config_dir = std::env::var_os("CHAUFFEUR_CONFIG_DIR").map_or_else(
            || PathBuf::from(&home).join(".config/chauffeur"),
            PathBuf::from,
        );
        // Usually a symlink to this repo's `skills/` folder.
        let skills_dir = std::env::var_os("CHAUFFEUR_SKILLS_DIR")
            .map_or_else(|| config_dir.join("skills"), PathBuf::from);

        Ok(Self {
            address,
            token: std::env::var("CHAUFFEUR_DAEMON_TOKEN").ok(),
            config_dir,
            skills_dir,
            idle_reminders: std::env::var("CHAUFFEUR_IDLE_STEERING")
                .is_ok_and(|value| value == "true"),
            sourcefed: std::env::var("CHAUFFEUR_SOURCEFED")
                .map_or(true, |value| value != "off")
                .then(SourcefedConfig::from_env),
            disabled: std::env::var("CHAUFFEUR_DISABLE")
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string)
                .collect(),
            state_file: Some(state.join("state.json")),
            audit_file: Some(state.join("audit.jsonl")),
        })
    }
}

pub async fn serve(options: DaemonOptions) -> Result<(), String> {
    let address = options.address;
    let jev = JevConfig::from_env()
        .ok_or("TYPESAFE_API_KEY is required: Jev is Chauffeur's System One provider")?;

    // Without rules, no rulebook can run, so none is offered.
    let rulebooks = if options.disabled.iter().any(|id| id == "rules") {
        None
    } else {
        let folder = options.skills_dir.join("rulebooks");
        let shipped = engine::shipped_rulebooks(&options.skills_dir)?;
        let skills_dir = options.skills_dir.clone();

        Some(Arc::new(Mutex::new(Reloading::new(
            "rulebooks",
            shipped,
            Watch::new(vec![folder]),
            Box::new(move |_| engine::shipped_rulebooks(&skills_dir)),
        ))))
    };
    let texts = Arc::new(Mutex::new(HostTexts::load(&options.config_dir)?));
    let engine = EngineHandle::spawn(EngineOptions {
        config_dir: options.config_dir,
        skills_dir: options.skills_dir,
        jev,
        idle_reminders: options.idle_reminders,
        sourcefed: options.sourcefed,
        disabled: options.disabled,
        state_file: options.state_file,
        audit_file: options.audit_file,
    })
    .await?;
    let shutdown = Arc::new(tokio::sync::Notify::new());
    let state = AppState {
        engine,
        token: options.token,
        rulebooks,
        texts,
        shutdown: Arc::clone(&shutdown),
    };
    let app = Router::new()
        // A permission request quotes up to eight whole user messages.
        .route("/rpc", post(rpc).layer(DefaultBodyLimit::max(1024 * 1024)))
        .route(
            "/integrations/sourcefed",
            post(sourcefed_event).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("bind daemon to {address}: {error}"))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(async move { shutdown.notified().await })
        .await
        .map_err(|error| format!("serve daemon: {error}"))
}

async fn rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<DaemonRequest>,
) -> Response {
    if !authorized(&headers, state.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let id = request.id;
    let result = match request.method.as_str() {
        METHOD_HEALTH => Ok(json!({ "ok": true, "version": VERSION })),
        METHOD_SHUTDOWN => {
            // A stored permit: the server stops even if it is not yet waiting.
            state.shutdown.notify_one();

            Ok(json!({ "ok": true }))
        }
        METHOD_SIGNAL => signal(&state.engine, request.params).await,
        METHOD_RULEBOOKS => offered(&state, &request.params),
        METHOD_TEXTS => host_texts(&state, &request.params),
        method => Err(format!("unknown method {method}")),
    };
    let (status, response) = match result {
        Ok(value) => (
            StatusCode::OK,
            DaemonResponse {
                id,
                result: Some(value),
                error: None,
            },
        ),
        Err(error) => (
            StatusCode::BAD_REQUEST,
            DaemonResponse {
                id,
                result: None,
                error: Some(error),
            },
        ),
    };

    (status, Json(response)).into_response()
}

/// sourcefed asks whether a monitor event reaches its session. The event also
/// informs later judgments; a failed engine delivers, as sourcefed does.
async fn sourcefed_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(event): Json<sourcefed::ForwardedEvent>,
) -> Response {
    if !authorized(&headers, state.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());

    match state.engine.ingest(event.into_signal(at)).await {
        Ok(effects) => {
            let withheld = effects
                .iter()
                .any(|effect| matches!(effect, Effect::Gate { deliver: false, .. }));

            (StatusCode::OK, Json(json!({ "deliver": !withheld }))).into_response()
        }
        Err(error) => (
            StatusCode::OK,
            Json(json!({ "deliver": true, "error": error })),
        )
            .into_response(),
    }
}

async fn signal(
    engine: &EngineHandle,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let params: SignalParams =
        serde_json::from_value(params).map_err(|error| format!("invalid params: {error}"))?;
    let effects = engine.ingest(params.signal).await?;

    serde_json::to_value(SignalResult { effects }).map_err(|error| error.to_string())
}

fn authorized(headers: &HeaderMap, token: Option<&str>) -> bool {
    let Some(token) = token else {
        return true;
    };

    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == format!("Bearer {token}"))
}

/// `CHAUFFEUR_STATE_DIR`, else `$XDG_STATE_HOME/chauffeur`, else
/// `~/.local/state/chauffeur`; created if missing.
fn state_dir(home: &Path) -> Result<PathBuf, String> {
    let dir = std::env::var_os("CHAUFFEUR_STATE_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME").map(|base| PathBuf::from(base).join("chauffeur"))
        })
        .unwrap_or_else(|| home.join(".local/state/chauffeur"));

    std::fs::create_dir_all(&dir).map_err(|error| format!("create {}: {error}", dir.display()))?;

    Ok(dir)
}
