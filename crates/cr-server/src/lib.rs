//! HTTP/WebSocket control plane shared by the browser and future surfaces.

pub mod rack_acp;
pub mod rack_cli;
pub mod rack_copilot;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use cr_core::{CompiledGraph, Graph, GraphCommand, ModuleDescriptor, NodeId, apply_commands};
use cr_midi::BudgetedLimiterStats;
use crossbeam_channel::Sender;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::sync::{Mutex, broadcast, oneshot};

use rack_copilot::{
    ApplyRequest, CopilotError, ProposeRequest, RackCopilot, RackCopilotConfig,
    RackMutationResponse, RackProposal, RackSnapshot,
};

const INDEX_HTML: &str = include_str!("../assets/index.html");
const APP_CSS: &str = include_str!("../assets/app.css");
const APP_JS: &str = include_str!("../assets/app.js");

#[derive(Debug, Clone, Default)]
pub struct ServerConfig {
    pub persistence_path: Option<PathBuf>,
    pub runtime_waker: Option<std::thread::Thread>,
    pub catalog: Catalog,
    pub rack_copilot: RackCopilotConfig,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub modules: Vec<ModuleDescriptor>,
    pub devices: Vec<DeviceCatalogEntry>,
    pub external_inputs: Vec<BindingCatalogEntry>,
    pub external_outputs: Vec<BindingCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceCatalogEntry {
    pub id: String,
    pub display_name: String,
    pub pack: String,
    pub port: String,
    pub channel: u8,
    pub parameters: Vec<ParameterCatalogEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParameterCatalogEntry {
    pub id: String,
    pub min: f32,
    pub max: f32,
    pub encoding: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BindingCatalogEntry {
    pub id: String,
    pub transport: String,
    pub min: f32,
    pub max: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IoConnectionState {
    Connected,
    Disconnected,
    DryRun,
    Disabled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoTelemetry {
    pub id: String,
    pub label: String,
    pub transport: String,
    pub endpoint: String,
    pub state: IoConnectionState,
    pub connect_attempts: u64,
    pub reconnects: u64,
    pub received: u64,
    pub sent: u64,
    pub errors: u64,
    pub dropped: u64,
}

/// A validated graph waiting to be applied by the real-time owner at a tick
/// boundary. The server does not mutate engine state directly.
pub struct RuntimeGraphRequest {
    pub compiled: CompiledGraph,
    pub reply: oneshot::Sender<Result<(), String>>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimiterTelemetry {
    pub sent_updates: u64,
    pub sent_messages: u64,
    pub deduped: u64,
    pub parameter_rate_limited: u64,
    pub bus_rate_limited: u64,
    pub bus_rate_limited_messages: u64,
    pub unknown_slot: u64,
}

impl From<BudgetedLimiterStats> for LimiterTelemetry {
    fn from(stats: BudgetedLimiterStats) -> Self {
        Self {
            sent_updates: stats.sent,
            sent_messages: stats.sent_messages,
            deduped: stats.deduped,
            parameter_rate_limited: stats.parameter_rate_limited,
            bus_rate_limited: stats.bus_rate_limited,
            bus_rate_limited_messages: stats.bus_rate_limited_messages,
            unknown_slot: stats.unknown_slot,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeMeter {
    pub id: NodeId,
    pub value: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub protocol_version: u16,
    pub revision: u64,
    pub graph: Graph,
    pub meters: Vec<NodeMeter>,
    pub limiter: LimiterTelemetry,
    pub runtime_tick: u64,
    pub runtime_running: bool,
    pub persistence_warning: Option<String>,
    #[serde(default)]
    pub io: Vec<IoTelemetry>,
}

impl Snapshot {
    fn new(graph: Graph) -> Self {
        Self {
            protocol_version: 1,
            revision: 0,
            graph,
            meters: Vec::new(),
            limiter: LimiterTelemetry::default(),
            runtime_tick: 0,
            runtime_running: false,
            persistence_warning: None,
            io: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerEvent {
    Snapshot(Snapshot),
    CommandRejected { message: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandBatch {
    pub base_revision: u64,
    pub commands: Vec<GraphCommand>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandResponse {
    pub revision: u64,
    pub persisted: bool,
    pub warning: Option<String>,
}

struct Shared {
    snapshot: RwLock<Snapshot>,
    command_gate: Mutex<()>,
    runtime_tx: Sender<RuntimeGraphRequest>,
    events: broadcast::Sender<ServerEvent>,
    persistence_path: Option<PathBuf>,
    runtime_waker: Option<std::thread::Thread>,
    catalog: Catalog,
    rack_copilot: RackCopilot,
}

#[derive(Clone)]
struct AppState(Arc<Shared>);

#[derive(Clone)]
pub struct ServerHandle(Arc<Shared>);

impl ServerHandle {
    /// Replace the latest adapter status. The next coalesced runtime publish
    /// carries it to every client in the same snapshot as meters and counters.
    pub fn update_io(&self, io: Vec<IoTelemetry>) {
        self.0
            .snapshot
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .io = io;
    }

    /// Publish coalesced runtime state. Safe to call from the pacing thread.
    pub fn publish_runtime(
        &self,
        tick: u64,
        values: &[f32],
        limiter: BudgetedLimiterStats,
        running: bool,
    ) {
        let event = {
            let mut snapshot = self.0.snapshot.write().unwrap_or_else(|e| e.into_inner());
            snapshot.runtime_tick = tick;
            snapshot.runtime_running = running;
            snapshot.limiter = limiter.into();
            snapshot.meters = snapshot
                .graph
                .nodes
                .iter()
                .zip(values.iter().copied())
                .map(|(node, value)| NodeMeter {
                    id: node.id.clone(),
                    value,
                })
                .collect();
            ServerEvent::Snapshot(snapshot.clone())
        };
        let _ = self.0.events.send(event);
    }

    pub fn snapshot(&self) -> Snapshot {
        self.0
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

/// Build the control plane and return both its router and a runtime publisher.
pub fn build(
    graph: Graph,
    runtime_tx: Sender<RuntimeGraphRequest>,
    config: ServerConfig,
) -> (Router, ServerHandle) {
    let (events, _) = broadcast::channel(64);
    let shared = Arc::new(Shared {
        snapshot: RwLock::new(Snapshot::new(graph)),
        command_gate: Mutex::new(()),
        runtime_tx,
        events,
        persistence_path: config.persistence_path,
        runtime_waker: config.runtime_waker,
        catalog: config.catalog,
        rack_copilot: RackCopilot::new(config.rack_copilot),
    });
    let state = AppState(Arc::clone(&shared));
    let router = Router::new()
        .route("/", get(index))
        .route("/assets/app.css", get(css))
        .route("/assets/app.js", get(js))
        .route("/api/v1/health", get(health))
        .route("/api/v1/state", get(get_state))
        .route("/api/v1/catalog", get(get_catalog))
        .route("/api/v1/commands", post(post_commands))
        .route("/api/v1/rack/snapshot", get(get_rack_snapshot))
        .route("/api/v1/rack/copilot/propose", post(propose_rack_patch))
        .route("/api/v1/rack/copilot/apply", post(apply_rack_proposal))
        .route("/api/v1/rack/copilot/undo", post(undo_rack_proposal))
        .route("/api/v1/events", get(websocket))
        .with_state(state);
    (router, ServerHandle(shared))
}

/// Run a router on a dedicated Tokio thread until `running` becomes false.
pub fn spawn(
    router: Router,
    listener: std::net::TcpListener,
    running: Arc<AtomicBool>,
) -> std::thread::JoinHandle<anyhow::Result<()>> {
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Runtime::new()?;
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    while running.load(Ordering::Acquire) {
                        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                    }
                })
                .await?;
            Ok(())
        })
    })
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn css() -> impl IntoResponse {
    ([("content-type", "text/css; charset=utf-8")], APP_CSS)
}

async fn js() -> impl IntoResponse {
    ([("content-type", "text/javascript; charset=utf-8")], APP_JS)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let snapshot = state.0.snapshot.read().unwrap_or_else(|e| e.into_inner());
    Json(serde_json::json!({
        "status": if snapshot.runtime_running { "ok" } else { "starting" },
        "protocol_version": snapshot.protocol_version,
        "revision": snapshot.revision,
        "runtime_tick": snapshot.runtime_tick,
    }))
}

async fn get_state(State(state): State<AppState>) -> Json<Snapshot> {
    Json(
        state
            .0
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    )
}

async fn get_catalog(State(state): State<AppState>) -> Json<Catalog> {
    Json(state.0.catalog.clone())
}

async fn get_rack_snapshot(State(state): State<AppState>) -> Result<Json<RackSnapshot>, ApiError> {
    state
        .0
        .rack_copilot
        .snapshot()
        .await
        .map(Json)
        .map_err(ApiError::from_copilot)
}

async fn propose_rack_patch(
    State(state): State<AppState>,
    Json(request): Json<ProposeRequest>,
) -> Result<Json<RackProposal>, ApiError> {
    state
        .0
        .rack_copilot
        .propose(request.prompt, request.provider)
        .await
        .map(Json)
        .map_err(ApiError::from_copilot)
}

async fn apply_rack_proposal(
    State(state): State<AppState>,
    Json(request): Json<ApplyRequest>,
) -> Result<Json<RackMutationResponse>, ApiError> {
    state
        .0
        .rack_copilot
        .apply(request.proposal_id)
        .await
        .map(Json)
        .map_err(ApiError::from_copilot)
}

async fn undo_rack_proposal(
    State(state): State<AppState>,
) -> Result<Json<RackMutationResponse>, ApiError> {
    state
        .0
        .rack_copilot
        .undo()
        .await
        .map(Json)
        .map_err(ApiError::from_copilot)
}

async fn post_commands(
    State(state): State<AppState>,
    Json(batch): Json<CommandBatch>,
) -> Result<Json<CommandResponse>, ApiError> {
    let _gate = state.0.command_gate.lock().await;
    let current = state
        .0
        .snapshot
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    if batch.base_revision != current.revision {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!(
                "stale graph revision {}; current revision is {}",
                batch.base_revision, current.revision
            ),
        ));
    }
    if batch.commands.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "command batch must not be empty",
        ));
    }

    let mut staged = current.graph.clone();
    let compiled = apply_commands(&mut staged, batch.commands)
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let (reply, applied) = oneshot::channel();
    state
        .0
        .runtime_tx
        .try_send(RuntimeGraphRequest { compiled, reply })
        .map_err(|e| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("runtime busy: {e}"),
            )
        })?;
    if let Some(thread) = &state.0.runtime_waker {
        thread.unpark();
    }
    applied
        .await
        .map_err(|_| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "runtime stopped"))?
        .map_err(|e| ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, e))?;

    let persistence = match &state.0.persistence_path {
        Some(path) => persist_graph_atomic(path, &staged)
            .map(|()| (true, None))
            .unwrap_or_else(|e| (false, Some(e.to_string()))),
        None => (false, None),
    };

    let event = {
        let mut snapshot = state.0.snapshot.write().unwrap_or_else(|e| e.into_inner());
        snapshot.graph = staged;
        snapshot.revision = snapshot.revision.saturating_add(1);
        snapshot.persistence_warning = persistence.1.clone();
        ServerEvent::Snapshot(snapshot.clone())
    };
    let revision = match &event {
        ServerEvent::Snapshot(snapshot) => snapshot.revision,
        ServerEvent::CommandRejected { .. } => current.revision,
    };
    let _ = state.0.events.send(event);

    Ok(Json(CommandResponse {
        revision,
        persisted: persistence.0,
        warning: persistence.1,
    }))
}

async fn websocket(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| websocket_session(socket, state))
}

async fn websocket_session(mut socket: WebSocket, state: AppState) {
    let initial = ServerEvent::Snapshot(
        state
            .0
            .snapshot
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    );
    if send_event(&mut socket, &initial).await.is_err() {
        return;
    }
    let mut events = state.0.events.subscribe();
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) if send_event(&mut socket, &event).await.is_err() => break,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = socket.next() => match incoming {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                _ => {}
            }
        }
    }
}

async fn send_event(socket: &mut WebSocket, event: &ServerEvent) -> Result<(), axum::Error> {
    let json = serde_json::to_string(event).unwrap_or_else(|_| {
        r#"{"type":"command_rejected","payload":{"message":"serialization failed"}}"#.to_owned()
    });
    socket.send(Message::Text(json.into())).await
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    fn from_copilot(error: CopilotError) -> Self {
        let status = match error {
            CopilotError::Disabled | CopilotError::MissingCredential(_) => {
                StatusCode::SERVICE_UNAVAILABLE
            }
            CopilotError::RackUnavailable(_) | CopilotError::Provider(_) => StatusCode::BAD_GATEWAY,
            CopilotError::InvalidProposal(_) => StatusCode::UNPROCESSABLE_ENTITY,
            CopilotError::ProposalNotFound(_) => StatusCode::NOT_FOUND,
            CopilotError::StaleProposal => StatusCode::CONFLICT,
        };
        Self::new(status, error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

pub fn persist_graph_atomic(path: &Path, graph: &Graph) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let yaml = serde_norway::to_string(graph)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("graph.yaml");
    let temp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    let result = (|| -> anyhow::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(yaml.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

pub fn load_persisted_graph(path: &Path) -> anyhow::Result<Option<Graph>> {
    match std::fs::read_to_string(path) {
        Ok(yaml) => Ok(Some(serde_norway::from_str(&yaml)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rack_acp::RackAcpConfig;
    use crate::rack_cli::{RackClaudeConfig, RackGrokConfig};
    use crate::rack_copilot::{
        RackCable, RackCommand, RackCopilotProvider, RackModel, RackParam, RackPatch,
        RackPatchModule, RackPlan, RackPort, RackSnapshot,
    };
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use cr_core::{BindingId, GraphNode, LfoShape, MathOperation, NodeSpec};
    use crossbeam_channel::bounded;
    use tower::ServiceExt;

    fn mock_rack_snapshot() -> RackSnapshot {
        RackSnapshot {
            fingerprint: "base-fingerprint".into(),
            catalog: vec![RackModel {
                plugin: "Fundamental".into(),
                model: "VCO".into(),
                name: "VCO".into(),
                description: "Oscillator".into(),
                tags: vec!["Oscillator".into()],
                params: vec![RackParam {
                    id: 0,
                    name: "Frequency".into(),
                    min: -4.0,
                    max: 4.0,
                    default: 0.0,
                }],
                inputs: vec![RackPort {
                    id: 0,
                    name: "V/OCT".into(),
                }],
                outputs: vec![RackPort {
                    id: 0,
                    name: "Sine".into(),
                }],
            }],
            patch: RackPatch {
                modules: Vec::<RackPatchModule>::new(),
                cables: Vec::<RackCable>::new(),
            },
        }
    }

    fn spawn_mock_rack(
        responses: Vec<serde_json::Value>,
    ) -> (
        std::net::SocketAddr,
        std::sync::Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
        std::thread::JoinHandle<()>,
    ) {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let received = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = std::sync::Arc::clone(&received);
        let worker = std::thread::spawn(move || {
            for response in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                captured
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(request.trim()).unwrap());
                writeln!(stream, "{}", serde_json::to_string(&response).unwrap()).unwrap();
            }
        });
        (address, received, worker)
    }

    fn spawn_mock_openai(plan: &RackPlan) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let plan = serde_json::to_string(plan).unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        request.extend_from_slice(&chunk[..count]);
                        if request.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                    Err(error) if error.kind() == std::io::ErrorKind::TimedOut => break,
                    Err(error) => panic!("mock provider read failed: {error}"),
                }
            }
            assert!(
                String::from_utf8_lossy(&request)
                    .to_ascii_lowercase()
                    .contains("authorization: bearer ")
            );
            let body = serde_json::json!({
                "output": [{"type": "message", "content": [{"type": "output_text", "text": plan}]}]
            })
            .to_string();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        (format!("http://{address}/v1/responses"), worker)
    }

    #[cfg(unix)]
    fn mock_acp_agent(plan: &RackPlan) -> (RackAcpConfig, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "controlroom-server-acp-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let responses = directory.join("responses.ndjson");
        let plan_text = serde_json::to_string(plan).unwrap();
        let lines = [
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1,
                "result": {"protocolVersion": 1, "agentCapabilities": {}, "authMethods": []}
            }),
            serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "result": {"sessionId": "rack-plan"}
            }),
            serde_json::json!({
                "jsonrpc": "2.0", "method": "session/update",
                "params": {"sessionId": "rack-plan", "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": plan_text}
                }}
            }),
            serde_json::json!({
                "jsonrpc": "2.0", "id": 3, "result": {"stopReason": "end_turn"}
            }),
        ]
        .into_iter()
        .map(|line| serde_json::to_string(&line).unwrap())
        .collect::<Vec<_>>()
        .join("\n");
        std::fs::write(&responses, format!("{lines}\n")).unwrap();

        let script = directory.join("mock-acp-agent");
        std::fs::write(
            &script,
            "#!/bin/sh\nexec 3<responses.ndjson\nIFS= read -r request\nIFS= read -r response <&3\nprintf '%s\\n' \"$response\"\nIFS= read -r request\nIFS= read -r response <&3\nprintf '%s\\n' \"$response\"\nIFS= read -r request\nIFS= read -r response <&3\nprintf '%s\\n' \"$response\"\nIFS= read -r response <&3\nprintf '%s\\n' \"$response\"\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions).unwrap();
        (
            RackAcpConfig {
                command: script.to_string_lossy().into_owned(),
                args: vec![],
                cwd: Some(directory.clone()),
            },
            directory,
        )
    }

    #[cfg(unix)]
    fn mock_structured_cli(
        plan: &RackPlan,
        name: &str,
        output_key: &str,
    ) -> (String, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;

        let directory = std::env::temp_dir().join(format!(
            "controlroom-server-cli-test-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let response = directory.join("response.json");
        std::fs::write(&response, serde_json::json!({output_key: plan}).to_string()).unwrap();
        let script = directory.join(name);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\ncat >/dev/null || true\ncat '{}'\n",
                response.display()
            ),
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o700);
        std::fs::set_permissions(&script, permissions).unwrap();
        (script.to_string_lossy().into_owned(), directory)
    }

    fn lfo_graph() -> Graph {
        Graph {
            nodes: vec![GraphNode {
                id: NodeId::new("lfo"),
                spec: NodeSpec::Lfo {
                    shape: LfoShape::Triangle,
                    freq_hz: 1.0,
                },
                input: None,
            }],
        }
    }

    fn toolbox_nodes() -> Vec<GraphNode> {
        vec![
            GraphNode {
                id: NodeId::new("random"),
                spec: NodeSpec::SampleHold {
                    rate_hz: 2.0,
                    seed: 42,
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("steps"),
                spec: NodeSpec::StepSequence {
                    rate_hz: 4.0,
                    values: vec![0.0, 0.5, 1.0],
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("slew"),
                spec: NodeSpec::Slew {
                    rise_per_second: 1.0,
                    fall_per_second: 2.0,
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("clamp"),
                spec: NodeSpec::Clamp { min: 0.0, max: 1.0 },
                input: None,
            },
            GraphNode {
                id: NodeId::new("curve"),
                spec: NodeSpec::Curve { exponent: 2.0 },
                input: None,
            },
            GraphNode {
                id: NodeId::new("quantize"),
                spec: NodeSpec::Quantize { steps: 8 },
                input: None,
            },
            GraphNode {
                id: NodeId::new("math"),
                spec: NodeSpec::Math {
                    operation: MathOperation::Multiply,
                    operand: 2.0,
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("macro"),
                spec: NodeSpec::Macro {
                    binding: BindingId::new("s88.knob.1"),
                    default: 0.0,
                },
                input: None,
            },
            GraphNode {
                id: NodeId::new("envelope"),
                spec: NodeSpec::Envelope {
                    gate: NodeId::new("macro"),
                    attack_seconds: 0.1,
                    decay_seconds: 0.2,
                    sustain: 0.6,
                    release_seconds: 0.3,
                },
                input: None,
            },
        ]
    }

    fn command_request(batch: &CommandBatch) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/commands")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(batch).unwrap()))
            .unwrap()
    }

    #[tokio::test]
    async fn state_endpoint_exposes_protocol_and_graph() {
        let (tx, _rx) = bounded(1);
        let (router, _) = build(lfo_graph(), tx, ServerConfig::default());
        let response = router
            .oneshot(Request::get("/api/v1/state").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot: Snapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot.protocol_version, 1);
        assert_eq!(snapshot.graph, lfo_graph());
        assert!(snapshot.io.is_empty());
    }

    #[tokio::test]
    async fn catalog_and_unified_io_status_are_exposed() {
        let (tx, _rx) = bounded(1);
        let catalog = Catalog {
            modules: cr_core::builtin_module_catalog(),
            devices: vec![DeviceCatalogEntry {
                id: "peak".into(),
                display_name: "Novation Peak".into(),
                pack: "novation-peak".into(),
                port: "Peak".into(),
                channel: 1,
                parameters: vec![ParameterCatalogEntry {
                    id: "filter.cutoff".into(),
                    min: 0.0,
                    max: 127.0,
                    encoding: "cc7".into(),
                }],
            }],
            external_inputs: Vec::new(),
            external_outputs: Vec::new(),
        };
        let (router, handle) = build(
            lfo_graph(),
            tx,
            ServerConfig {
                catalog: catalog.clone(),
                ..ServerConfig::default()
            },
        );
        handle.update_io(vec![IoTelemetry {
            id: "peak".into(),
            label: "peak".into(),
            transport: "midi_out".into(),
            endpoint: "Peak MIDI".into(),
            state: IoConnectionState::Connected,
            connect_attempts: 1,
            reconnects: 0,
            received: 0,
            sent: 12,
            errors: 0,
            dropped: 0,
        }]);

        let response = router
            .clone()
            .oneshot(Request::get("/api/v1/catalog").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let decoded = serde_json::from_slice::<Catalog>(&body).unwrap();
        assert_eq!(decoded, catalog);
        assert!(decoded.modules.len() >= 50);
        assert!(
            decoded
                .modules
                .iter()
                .all(|module| !module.ports.is_empty())
        );

        let response = router
            .oneshot(Request::get("/api/v1/state").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let snapshot: Snapshot = serde_json::from_slice(&body).unwrap();
        assert_eq!(snapshot.io[0].state, IoConnectionState::Connected);
        assert_eq!(snapshot.io[0].sent, 12);
    }

    #[tokio::test]
    async fn command_batch_waits_for_runtime_then_commits() {
        let (tx, rx) = bounded(1);
        let (router, handle) = build(lfo_graph(), tx, ServerConfig::default());
        let runtime = std::thread::spawn(move || {
            let request = rx.recv().unwrap();
            assert_eq!(request.compiled.len(), 2);
            request.reply.send(Ok(())).unwrap();
        });
        let batch = CommandBatch {
            base_revision: 0,
            commands: vec![
                GraphCommand::AddNode {
                    id: NodeId::new("scale"),
                    spec: NodeSpec::Scale { min: 0.0, max: 1.0 },
                },
                GraphCommand::Connect {
                    source: NodeId::new("lfo"),
                    target: NodeId::new("scale"),
                },
            ],
        };
        let response = router.oneshot(command_request(&batch)).await.unwrap();
        runtime.join().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(handle.snapshot().revision, 1);
        assert_eq!(handle.snapshot().graph.nodes.len(), 2);
    }

    #[tokio::test]
    async fn modulation_toolbox_round_trips_through_command_api() {
        let (tx, rx) = bounded(1);
        let (router, handle) = build(lfo_graph(), tx, ServerConfig::default());
        let expected = toolbox_nodes();
        let expected_count = expected.len() + 1;
        let runtime = std::thread::spawn(move || {
            let request = rx.recv().unwrap();
            assert_eq!(request.compiled.len(), expected_count);
            request.reply.send(Ok(())).unwrap();
        });
        let batch = CommandBatch {
            base_revision: 0,
            commands: expected
                .into_iter()
                .map(|node| GraphCommand::AddNode {
                    id: node.id,
                    spec: node.spec,
                })
                .collect(),
        };
        let response = router.oneshot(command_request(&batch)).await.unwrap();
        runtime.join().unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(handle.snapshot().graph.nodes.len(), expected_count);
    }

    #[tokio::test]
    async fn runtime_rejection_leaves_server_graph_unchanged() {
        let (tx, rx) = bounded(1);
        let (router, handle) = build(lfo_graph(), tx, ServerConfig::default());
        let runtime = std::thread::spawn(move || {
            let request = rx.recv().unwrap();
            request
                .reply
                .send(Err("unknown device parameter".into()))
                .unwrap();
        });
        let batch = CommandBatch {
            base_revision: 0,
            commands: vec![GraphCommand::AddNode {
                id: NodeId::new("scale"),
                spec: NodeSpec::Scale { min: 0.0, max: 1.0 },
            }],
        };
        let response = router.oneshot(command_request(&batch)).await.unwrap();
        runtime.join().unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(handle.snapshot().revision, 0);
        assert_eq!(handle.snapshot().graph, lfo_graph());
    }

    #[tokio::test]
    async fn stale_revision_is_rejected_without_touching_runtime() {
        let (tx, rx) = bounded(1);
        let (router, _) = build(lfo_graph(), tx, ServerConfig::default());
        let batch = CommandBatch {
            base_revision: 4,
            commands: vec![GraphCommand::Disconnect {
                target: NodeId::new("lfo"),
            }],
        };
        let response = router.oneshot(command_request(&batch)).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn rack_copilot_stages_then_applies_only_the_stored_validated_plan() {
        let snapshot = mock_rack_snapshot();
        let plan = RackPlan {
            summary: "Add a playable oscillator".into(),
            warnings: vec![],
            commands: vec![RackCommand::AddModule {
                reference: "ai:vco".into(),
                plugin: "Fundamental".into(),
                model: "VCO".into(),
                x: 8.0,
                y: 0.0,
            }],
            library_suggestions: vec![],
        };
        let (provider_endpoint, provider) = spawn_mock_openai(&plan);
        let (rack_address, rack_requests, rack) = spawn_mock_rack(vec![
            serde_json::json!({"ok": true, "snapshot": snapshot}),
            serde_json::json!({"ok": true, "snapshot": mock_rack_snapshot()}),
            serde_json::json!({"ok": true, "message": "applied", "fingerprint": "changed"}),
        ]);
        let (tx, _rx) = bounded(1);
        let (router, _) = build(
            lfo_graph(),
            tx,
            ServerConfig {
                rack_copilot: RackCopilotConfig {
                    enabled: true,
                    provider: crate::rack_copilot::RackCopilotProvider::Openai,
                    endpoint: provider_endpoint,
                    model: "test-model".into(),
                    api_key_env: "PATH".into(),
                    rack_agent: rack_address.to_string(),
                    timeout_seconds: 5,
                    claude: crate::rack_cli::RackClaudeConfig::default(),
                    grok: crate::rack_cli::RackGrokConfig::default(),
                    acp: crate::rack_acp::RackAcpConfig::default(),
                },
                ..ServerConfig::default()
            },
        );

        let response = router
            .clone()
            .oneshot(
                Request::post("/api/v1/rack/copilot/propose")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"prompt":"give me a tone"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let proposal: RackProposal =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(proposal.plan, plan);

        let response = router
            .oneshot(
                Request::post("/api/v1/rack/copilot/apply")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"proposal_id": proposal.id}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let applied: RackMutationResponse =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(applied.fingerprint, "changed");

        provider.join().unwrap();
        rack.join().unwrap();
        let requests = rack_requests.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|value| value["op"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["snapshot", "snapshot", "apply"]
        );
        assert_eq!(requests[2]["commands"][0]["model"], "VCO");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rack_copilot_accepts_acp_plans_through_the_same_validation_boundary() {
        let plan = RackPlan {
            summary: "Add an ACP-planned oscillator".into(),
            warnings: vec![],
            commands: vec![RackCommand::AddModule {
                reference: "ai:acp-vco".into(),
                plugin: "Fundamental".into(),
                model: "VCO".into(),
                x: 10.0,
                y: 0.0,
            }],
            library_suggestions: vec![],
        };
        let (acp, acp_directory) = mock_acp_agent(&plan);
        let (rack_address, rack_requests, rack) = spawn_mock_rack(vec![
            serde_json::json!({"ok": true, "snapshot": mock_rack_snapshot()}),
        ]);
        let (tx, _rx) = bounded(1);
        let (router, _) = build(
            lfo_graph(),
            tx,
            ServerConfig {
                rack_copilot: RackCopilotConfig {
                    enabled: true,
                    provider: RackCopilotProvider::Acp,
                    rack_agent: rack_address.to_string(),
                    timeout_seconds: 5,
                    acp,
                    ..RackCopilotConfig::default()
                },
                ..ServerConfig::default()
            },
        );

        let response = router
            .oneshot(
                Request::post("/api/v1/rack/copilot/propose")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"prompt":"make a tone through ACP"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let proposal: RackProposal =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(proposal.provider, RackCopilotProvider::Acp);
        assert_eq!(proposal.plan, plan);
        assert!(proposal.model.contains("mock-acp-agent"));

        rack.join().unwrap();
        assert_eq!(rack_requests.lock().unwrap()[0]["op"], "snapshot");
        std::fs::remove_dir_all(acp_directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rack_copilot_accepts_claude_structured_output_through_shared_validation() {
        let plan = RackPlan {
            summary: "Claude adds an oscillator".into(),
            warnings: vec![],
            commands: vec![RackCommand::AddModule {
                reference: "ai:claude-vco".into(),
                plugin: "Fundamental".into(),
                model: "VCO".into(),
                x: 12.0,
                y: 0.0,
            }],
            library_suggestions: vec![],
        };
        let (command, directory) = mock_structured_cli(&plan, "mock-claude", "structured_output");
        let (rack_address, _, rack) = spawn_mock_rack(vec![
            serde_json::json!({"ok": true, "snapshot": mock_rack_snapshot()}),
        ]);
        let (tx, _rx) = bounded(1);
        let (router, _) = build(
            lfo_graph(),
            tx,
            ServerConfig {
                rack_copilot: RackCopilotConfig {
                    enabled: true,
                    provider: RackCopilotProvider::Openai,
                    rack_agent: rack_address.to_string(),
                    timeout_seconds: 5,
                    claude: RackClaudeConfig {
                        command,
                        model: Some("test-claude".into()),
                    },
                    ..RackCopilotConfig::default()
                },
                ..ServerConfig::default()
            },
        );
        let response = router
            .oneshot(
                Request::post("/api/v1/rack/copilot/propose")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"prompt":"make a Claude tone","provider":"claude"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let proposal: RackProposal =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(proposal.provider, RackCopilotProvider::Claude);
        assert_eq!(proposal.model, "test-claude");
        assert_eq!(proposal.plan, plan);
        rack.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rack_copilot_accepts_grok_structured_output_through_shared_validation() {
        let plan = RackPlan {
            summary: "Grok adds an oscillator".into(),
            warnings: vec![],
            commands: vec![RackCommand::AddModule {
                reference: "ai:grok-vco".into(),
                plugin: "Fundamental".into(),
                model: "VCO".into(),
                x: 14.0,
                y: 0.0,
            }],
            library_suggestions: vec![],
        };
        let (command, directory) = mock_structured_cli(&plan, "mock-grok", "structuredOutput");
        let (rack_address, _, rack) = spawn_mock_rack(vec![
            serde_json::json!({"ok": true, "snapshot": mock_rack_snapshot()}),
        ]);
        let (tx, _rx) = bounded(1);
        let (router, _) = build(
            lfo_graph(),
            tx,
            ServerConfig {
                rack_copilot: RackCopilotConfig {
                    enabled: true,
                    provider: RackCopilotProvider::Openai,
                    rack_agent: rack_address.to_string(),
                    timeout_seconds: 5,
                    grok: RackGrokConfig {
                        command,
                        model: Some("test-grok".into()),
                    },
                    ..RackCopilotConfig::default()
                },
                ..ServerConfig::default()
            },
        );
        let response = router
            .oneshot(
                Request::post("/api/v1/rack/copilot/propose")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"prompt":"make a Grok tone","provider":"grok"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let proposal: RackProposal =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(proposal.provider, RackCopilotProvider::Grok);
        assert_eq!(proposal.model, "test-grok");
        assert_eq!(proposal.plan, plan);
        rack.join().unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn persistence_round_trips_and_missing_file_is_not_an_error() {
        let path = std::env::temp_dir().join(format!(
            "controlroom-cr-server-test-{}-{}.yaml",
            std::process::id(),
            std::thread::current().name().unwrap_or("unnamed")
        ));
        let _ = std::fs::remove_file(&path);
        assert_eq!(load_persisted_graph(&path).unwrap(), None);
        persist_graph_atomic(&path, &lfo_graph()).unwrap();
        assert_eq!(load_persisted_graph(&path).unwrap(), Some(lfo_graph()));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn modulation_toolbox_persists_without_schema_loss() {
        let path = std::env::temp_dir().join(format!(
            "controlroom-toolbox-persistence-{}.yaml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let graph = Graph {
            nodes: toolbox_nodes(),
        };
        persist_graph_atomic(&path, &graph).unwrap();
        assert_eq!(load_persisted_graph(&path).unwrap(), Some(graph));
        std::fs::remove_file(path).unwrap();
    }
}
