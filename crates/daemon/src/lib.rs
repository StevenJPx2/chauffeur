//! Thin HTTP host for the Chauffeur engine.

mod engine;
mod sourcefed;

pub use engine::{EngineHandle, EngineOptions};

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use chauffeur_core::{
    DaemonRequest, DaemonResponse, Effect, METHOD_HEALTH, METHOD_SIGNAL, SignalParams, SignalResult,
};
use chauffeur_judge_jev::JevConfig;
use serde_json::json;

pub const DEFAULT_PORT: u16 = 18_790;

#[derive(Clone)]
struct AppState {
    engine: EngineHandle,
    token: Option<String>,
}

pub struct DaemonOptions {
    pub address: SocketAddr,
    pub token: Option<String>,
    pub config_dir: PathBuf,
    pub skills_dir: PathBuf,
    pub idle_reminders: bool,
    pub state_file: Option<PathBuf>,
}

impl DaemonOptions {
    pub fn from_env(address: SocketAddr) -> Result<Self, String> {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
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
            state_file: Some(state_dir(Path::new(&home))?.join("state.json")),
        })
    }
}

pub async fn serve(options: DaemonOptions) -> Result<(), String> {
    let address = options.address;
    let jev = JevConfig::from_env()
        .ok_or("TYPESAFE_API_KEY is required: Jev is Chauffeur's System One provider")?;

    let engine = EngineHandle::spawn(EngineOptions {
        config_dir: options.config_dir,
        skills_dir: options.skills_dir,
        jev,
        idle_reminders: options.idle_reminders,
        state_file: options.state_file,
    })
    .await?;
    let state = AppState {
        engine,
        token: options.token,
    };
    let app = Router::new()
        .route("/rpc", post(rpc).layer(DefaultBodyLimit::max(128 * 1024)))
        .route(
            "/integrations/sourcefed",
            post(sourcefed_event).layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|error| format!("bind daemon to {address}: {error}"))?;

    axum::serve(listener, app)
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
        METHOD_HEALTH => Ok(json!({ "ok": true })),
        METHOD_SIGNAL => signal(&state.engine, request.params).await,
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
                .any(|effect| matches!(effect, Effect::WithholdEvent { .. }));

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
