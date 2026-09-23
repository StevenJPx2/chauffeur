//! Thin HTTP host for [`chauffeur_core::Runtime`].

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chauffeur_core::{
    DaemonRequest, EventFrame, JsonReminderQueue, Plugin, Runtime, Target, compose, dispatch,
    load_skills,
};
use chauffeur_judge_laya::{LayaJudge, default_socket_path};
use chauffeur_plugin_git::GitPlugin;
use chauffeur_plugin_github::GitHubPlugin;
use chauffeur_plugin_jira::JiraPlugin;
use futures_util::stream;
use tokio::sync::broadcast;

pub const DEFAULT_PORT: u16 = 18_788;

#[derive(Clone)]
struct AppState {
    runtime: Arc<Runtime>,
    token: Option<String>,
}

pub struct DaemonOptions {
    pub address: SocketAddr,
    pub token: Option<String>,
    pub state_dir: PathBuf,
    pub laya_socket: PathBuf,
    pub skills_dir: PathBuf,
}

impl DaemonOptions {
    pub fn from_env(address: SocketAddr) -> Result<Self, String> {
        let home = std::env::var_os("HOME").ok_or("HOME is not set")?;
        let state_dir = std::env::var_os("CHAUFFEUR_STATE_DIR").map_or_else(
            || PathBuf::from(&home).join(".local/state/chauffeur"),
            PathBuf::from,
        );
        let laya_socket = std::env::var_os("LAYA_SOCKET")
            .map_or_else(|| default_socket_path(Path::new(&home)), PathBuf::from);
        let skills_dir =
            std::env::var_os("CHAUFFEUR_SKILLS_DIR").map_or_else(default_skills_dir, PathBuf::from);

        Ok(Self {
            address,
            token: std::env::var("CHAUFFEUR_DAEMON_TOKEN").ok(),
            state_dir,
            laya_socket,
            skills_dir,
        })
    }
}

pub fn build_runtime(options: &DaemonOptions) -> Result<Arc<Runtime>, String> {
    let plugins: Vec<Box<dyn Plugin>> = vec![
        Box::new(GitHubPlugin),
        Box::new(JiraPlugin),
        Box::new(GitPlugin),
    ];
    let rules = compose(&plugins)?;
    let skills = load_skills(&options.skills_dir)?;
    let judge = LayaJudge::connect(&options.laya_socket)?;
    let queue = Arc::new(JsonReminderQueue::new(&options.state_dir));

    let runtime = Runtime::with_skills(rules, skills, Box::new(judge), queue)?;

    Ok(Arc::new(runtime))
}

pub async fn serve(options: DaemonOptions) -> Result<(), String> {
    let address = options.address;
    let state = AppState {
        runtime: build_runtime(&options)?,
        token: options.token,
    };
    let app = Router::new()
        .route("/rpc", post(rpc).layer(DefaultBodyLimit::max(128 * 1024)))
        .route("/events", get(events))
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

    let runtime = Arc::clone(&state.runtime);
    let response = match tokio::task::spawn_blocking(move || dispatch(&runtime, request)).await {
        Ok(response) => response,
        Err(error) => chauffeur_core::DaemonResponse {
            id: None,
            result: None,
            error: Some(format!("dispatch task failed: {error}")),
        },
    };
    let status = if response.error.is_some() {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::OK
    };

    (status, Json(response)).into_response()
}

async fn events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    if !authorized(&headers, state.token.as_deref()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let Some(encoded) = query.get("target") else {
        return (StatusCode::BAD_REQUEST, "missing target").into_response();
    };
    let Ok(target) = serde_json::from_str::<Target>(encoded) else {
        return (StatusCode::BAD_REQUEST, "invalid target").into_response();
    };
    let Ok(initial) = state.runtime.reminders(&target) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "queue unavailable").into_response();
    };

    sse_response(state.runtime, target, initial)
}

fn sse_response(
    runtime: Arc<Runtime>,
    target: Target,
    initial: Vec<chauffeur_core::QueuedReminder>,
) -> Response {
    let mut pending = VecDeque::new();
    pending.push_back(EventFrame::Subscribed {
        target: target.clone(),
    });

    if !initial.is_empty() {
        pending.push_back(EventFrame::Event {
            target: target.clone(),
            reminders: initial,
        });
    }

    let state = StreamState {
        target,
        pending,
        receiver: runtime.subscribe(),
        heartbeat: tokio::time::interval(Duration::from_secs(15)),
    };
    let output = stream::unfold(state, next_frame);

    Response::builder()
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .body(Body::from_stream(output))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

struct StreamState {
    target: Target,
    pending: VecDeque<EventFrame>,
    receiver: broadcast::Receiver<chauffeur_core::QueuedReminder>,
    heartbeat: tokio::time::Interval,
}

async fn next_frame(mut state: StreamState) -> Option<(Result<String, Infallible>, StreamState)> {
    if let Some(frame) = state.pending.pop_front() {
        return Some((Ok(encode_frame(&frame)), state));
    }

    loop {
        tokio::select! {
            received = state.receiver.recv() => match received {
                Ok(reminder) if reminder.target == state.target => {
                    let frame = EventFrame::Event {
                        target: state.target.clone(),
                        reminders: vec![reminder],
                    };

                    return Some((Ok(encode_frame(&frame)), state));
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => return None,
            },
            _ = state.heartbeat.tick() => {
                return Some((Ok(encode_frame(&EventFrame::Heartbeat)), state));
            }
        }
    }
}

fn encode_frame(frame: &EventFrame) -> String {
    serde_json::to_string(frame).map_or_else(
        |_| "event: error\ndata: serialization failed\n\n".to_string(),
        |json| format!("data: {json}\n\n"),
    )
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

#[must_use]
pub fn default_state_dir(home: &Path) -> PathBuf {
    home.join(".local/state/chauffeur")
}

fn default_skills_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| {
            executable
                .parent()?
                .parent()?
                .parent()
                .map(Path::to_path_buf)
        })
        .map(|root| root.join("skills"))
        .unwrap_or_else(|| PathBuf::from("skills"))
}
