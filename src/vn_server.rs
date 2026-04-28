use crate::agent_bridge::{
    AgentCommitTurnOptions, AgentSubmitTurnOptions, AgentTurnResponse, PendingAgentTurn,
    commit_agent_turn, enqueue_agent_turn, load_pending_agent_turn, normalize_narrative_level,
};
use crate::backend_selection::{
    WorldBackendSelection, WorldTextBackend, WorldVisualBackend, load_world_backend_selection,
    save_world_backend_selection,
};
use crate::host_supervisor::{HostSupervisorPlan, build_host_supervisor_plan};
use crate::job_ledger::{
    ReadWorldJobsOptions, WorldJob, WorldJobKind, WorldJobStatus, read_world_jobs,
};
use crate::models::FREEFORM_CHOICE_SLOT;
use crate::projection_health::{ProjectionHealthReport, build_projection_health_report};
use crate::repair_extra_memory_projection;
use crate::start::{StartWorldOptions, start_world};
use crate::store::{
    WORLD_FILENAME, read_json, resolve_store_paths, resolve_world_id, save_active_world,
    world_file_paths, write_json,
};
use crate::transfer::{ExportWorldOptions, ImportWorldOptions, export_world, import_world};
use crate::turn::{AdvanceTurnOptions, advance_turn};
use crate::visual_assets::{
    BuildWorldVisualAssetsOptions, ImageGenerationJob, build_world_visual_assets,
};
use crate::vn::{BuildVnPacketOptions, build_vn_packet, turn_cg_retry_path};
use crate::world_db::{WorldDbRepairReport, repair_world_db};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
#[cfg(not(test))]
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
#[cfg(not(test))]
use std::process::{Command, Stdio};
use std::sync::Mutex;
#[cfg(not(test))]
use std::thread;

const INDEX_HTML: &str = include_str!("../vn-web/index.html");
const APP_JS: &str = include_str!("../vn-web/app.js");
const STYLES_CSS: &str = include_str!("../vn-web/styles.css");

const DEFAULT_HOST: &str = "127.0.0.1";
const EXPORTS_DIR: &str = "exports";
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_VN_INPUT_CHARS: usize = 2048;
const AGENT_BRIDGE_ENV: &str = "SINGULARI_WORLD_AGENT_BRIDGE";
const WEBGPT_TEXT_CDP_PORT_ENV: &str = "SINGULARI_WORLD_WEBGPT_TEXT_CDP_PORT";
const WEBGPT_IMAGE_CDP_PORT_ENV: &str = "SINGULARI_WORLD_WEBGPT_IMAGE_CDP_PORT";
const DEFAULT_WEBGPT_TEXT_CDP_PORT: u16 = 9238;
const DEFAULT_WEBGPT_IMAGE_CDP_PORT: u16 = 9239;
const INITIAL_AGENT_TURN_INPUT: &str = "세계 개막";
const VN_CHOOSE_RESPONSE_SCHEMA_VERSION: &str = "singulari.vn_choose_response.v1";
const VN_RUNTIME_STATUS_SCHEMA_VERSION: &str = "singulari.vn_runtime_status.v1";
const VN_CG_GALLERY_SCHEMA_VERSION: &str = "singulari.vn_cg_gallery.v1";
const TURN_CG_ASSET_DIR: &str = "assets/vn/turn_cg";
const PROMPT_SUMMARY_MAX_CHARS: usize = 180;
#[cfg(not(test))]
const VN_HOST_WORKER_STDOUT: &str = "agent_bridge/vn-host-worker.log";
#[cfg(not(test))]
const VN_HOST_WORKER_STDERR: &str = "agent_bridge/vn-host-worker.err.log";

#[derive(Debug, Clone)]
pub struct VnServeOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: Option<String>,
    pub host: String,
    pub port: u16,
}

impl VnServeOptions {
    #[must_use]
    pub fn new(world_id: Option<String>, port: u16) -> Self {
        Self {
            store_root: None,
            world_id,
            host: DEFAULT_HOST.to_owned(),
            port,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnChooseRequest {
    pub input: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative_level: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnNewWorldRequest {
    pub seed_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_backend: Option<WorldTextBackend>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visual_backend: Option<WorldVisualBackend>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnSelectWorldRequest {
    pub world_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnSaveWorldRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnLoadWorldRequest {
    pub bundle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnCgRetryRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnCgRetryRecord {
    schema_version: String,
    world_id: String,
    turn_id: String,
    status: String,
    requested_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnWorldListResponse {
    active_world_id: String,
    worlds: Vec<VnWorldSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnWorldSummary {
    world_id: String,
    title: String,
    updated_at: String,
    turn_id: String,
    phase: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnWorldSwitchResponse {
    active_world_id: String,
    packet: crate::vn::VnPacket,
    worlds: Vec<VnWorldSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_pending: Option<VnAgentPendingResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnSaveWorldResponse {
    active_world_id: String,
    world_id: String,
    title: String,
    bundle_dir: String,
    manifest_path: String,
    files_copied: usize,
    worlds: Vec<VnWorldSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnErrorResponse {
    error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnAgentPendingResponse {
    schema_version: String,
    status: String,
    world_id: String,
    turn_id: String,
    pending_ref: String,
    command_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnAgentStatusResponse {
    schema_version: String,
    status: String,
    world_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnRuntimeStatusResponse {
    schema_version: String,
    world_id: String,
    backend_selection: VnBackendSelectionStatus,
    narrative: VnNarrativeRuntimeStatus,
    visual: VnVisualRuntimeStatus,
    details: VnRuntimeDetails,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnBackendSelectionStatus {
    text_backend: String,
    visual_backend: String,
    locked: bool,
    source: String,
    created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnNarrativeRuntimeStatus {
    label: String,
    status: String,
    backend: String,
    online: bool,
    detail: String,
    endpoint: Option<String>,
    agent_bridge_enabled: bool,
    pending_turn_id: Option<String>,
    binding_present: bool,
    latest_dispatch_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnVisualRuntimeStatus {
    label: String,
    status: String,
    backend: String,
    online: bool,
    detail: String,
    endpoint: Option<String>,
    pending_slots: Vec<String>,
    claimed_slots: Vec<String>,
    completed_slots: Vec<String>,
    jobs: Vec<WorldJob>,
    turn_cg_status: String,
    latest_dispatch_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnRuntimeDetails {
    backend_selection: VnBackendSelectionStatus,
    latest_text_dispatch: Option<serde_json::Value>,
    latest_visual_dispatch: Option<serde_json::Value>,
    projection_health: ProjectionHealthReport,
    host_supervisor: HostSupervisorPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnWorldDbRepairResponse {
    schema_version: String,
    world_id: String,
    repair: WorldDbRepairReport,
    projection_health: ProjectionHealthReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnExtraMemoryRepairResponse {
    schema_version: String,
    world_id: String,
    repair: crate::ExtraMemoryRepairReport,
    projection_health: ProjectionHealthReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnCgGalleryResponse {
    schema_version: String,
    world_id: String,
    title: String,
    items: Vec<VnCgGalleryItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VnCgGalleryItem {
    turn_id: String,
    turn_index: u32,
    asset_url: String,
    download_filename: String,
    prompt_summary: String,
    image_prompt: String,
    prompt_policy: String,
    generated_from_packet: bool,
}

#[derive(Debug)]
struct VnServerState {
    store_root: Option<PathBuf>,
    world_id: Mutex<String>,
}

#[derive(Debug)]
struct HttpRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

#[derive(Debug)]
struct HttpResponse {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
}

/// Serve the local visual-novel projection app and turn API.
///
/// # Errors
///
/// Returns an error when the active world cannot be resolved, the host is not
/// loopback/Tailscale-scoped, or the TCP listener cannot bind.
pub fn serve_vn(options: &VnServeOptions) -> Result<()> {
    ensure_allowed_vn_host(options.host.as_str())?;
    let world_id = resolve_world_id(options.store_root.as_deref(), options.world_id.as_deref())?;
    let state = VnServerState {
        store_root: options.store_root.clone(),
        world_id: Mutex::new(world_id),
    };
    let bind_addr = format!("{}:{}", options.host, options.port);
    let listener = TcpListener::bind(bind_addr.as_str())
        .with_context(|| format!("vn server bind failed: {bind_addr}"))?;
    println!("vn server: http://{bind_addr}/");
    println!("world: {}", state.world_id()?);
    wake_existing_pending_work_once(&state);
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = match read_request(&mut stream) {
                    Ok(request) => route_request(&state, &request),
                    Err(error) => error_response("400 Bad Request", error.to_string()),
                };
                if let Err(error) = write_response(&mut stream, &response) {
                    eprintln!("vn server response write failed: {error}");
                }
            }
            Err(error) => eprintln!("vn server connection failed: {error}"),
        }
    }
    Ok(())
}

fn route_request(state: &VnServerState, request: &HttpRequest) -> HttpResponse {
    let path = request.path.split('?').next().unwrap_or("/");
    if let Some(asset_path) = path.strip_prefix("/world-assets/") {
        return world_asset_response(state, asset_path);
    }
    match (request.method.as_str(), path) {
        ("GET", "/" | "/index.html") => html_response(INDEX_HTML),
        ("GET", "/app.js") => static_response("application/javascript; charset=utf-8", APP_JS),
        ("GET", "/styles.css") => static_response("text/css; charset=utf-8", STYLES_CSS),
        ("GET", "/api/health") => json_response(&serde_json::json!({
            "ok": true,
            "world_id": state.world_id().unwrap_or_else(|_| "unknown".to_owned()),
        })),
        ("GET", "/api/vn/worlds") => world_list_response(state),
        ("GET", "/api/vn/visual-assets") => visual_assets_response(state),
        ("GET", "/api/vn/cg/gallery") => cg_gallery_response(state),
        ("GET", "/api/vn/runtime-status") => runtime_status_response(state),
        ("POST", "/api/vn/worlds/select") => select_world_response(state, &request.body),
        ("POST", "/api/vn/worlds/new") => new_world_response(state, &request.body),
        ("POST", "/api/vn/worlds/save") => save_world_response(state, &request.body),
        ("POST", "/api/vn/worlds/load") => load_world_response(state, &request.body),
        ("GET", "/api/vn/current") => current_packet_response(state),
        ("GET", "/api/vn/agent/pending") => pending_agent_turn_response(state),
        ("POST", "/api/vn/agent/commit") => commit_agent_turn_response(state, &request.body),
        ("POST", "/api/vn/cg/retry") => cg_retry_response(state, &request.body),
        ("POST", "/api/vn/repair/world-db") => repair_world_db_response(state),
        ("POST", "/api/vn/repair/extra-memory") => repair_extra_memory_response(state),
        ("POST", "/api/vn/choose") => choose_response(state, &request.body),
        _ => error_response("404 Not Found", format!("unknown route: {}", request.path)),
    }
}

fn world_asset_response(state: &VnServerState, asset_path: &str) -> HttpResponse {
    match read_world_asset(state, asset_path) {
        Ok((content_type, body)) => HttpResponse {
            status: "200 OK",
            content_type,
            body,
        },
        Err(error) => error_response("404 Not Found", error.to_string()),
    }
}

fn world_list_response(state: &VnServerState) -> HttpResponse {
    match world_list(state) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("500 Internal Server Error", error.to_string()),
    }
}

fn visual_assets_response(state: &VnServerState) -> HttpResponse {
    match build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: state.store_root.clone(),
        world_id: match state.world_id() {
            Ok(world_id) => world_id,
            Err(error) => return error_response("500 Internal Server Error", error.to_string()),
        },
    }) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("500 Internal Server Error", error.to_string()),
    }
}

fn cg_gallery_response(state: &VnServerState) -> HttpResponse {
    match cg_gallery(state) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("500 Internal Server Error", error.to_string()),
    }
}

fn runtime_status_response(state: &VnServerState) -> HttpResponse {
    match runtime_status(state) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("500 Internal Server Error", error.to_string()),
    }
}

fn repair_world_db_response(state: &VnServerState) -> HttpResponse {
    match repair_world_db_for_active_world(state) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn repair_extra_memory_response(state: &VnServerState) -> HttpResponse {
    match repair_extra_memory_for_active_world(state) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn select_world_response(state: &VnServerState, body: &[u8]) -> HttpResponse {
    let request = match serde_json::from_slice::<VnSelectWorldRequest>(body) {
        Ok(request) => request,
        Err(error) => return error_response("400 Bad Request", error.to_string()),
    };
    match select_world(state, request.world_id.as_str()) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn new_world_response(state: &VnServerState, body: &[u8]) -> HttpResponse {
    let request = match serde_json::from_slice::<VnNewWorldRequest>(body) {
        Ok(request) => request,
        Err(error) => return error_response("400 Bad Request", error.to_string()),
    };
    match new_world(state, request) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn save_world_response(state: &VnServerState, body: &[u8]) -> HttpResponse {
    let request = if body.is_empty() {
        VnSaveWorldRequest { world_id: None }
    } else {
        match serde_json::from_slice::<VnSaveWorldRequest>(body) {
            Ok(request) => request,
            Err(error) => return error_response("400 Bad Request", error.to_string()),
        }
    };
    match save_world(state, request) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn load_world_response(state: &VnServerState, body: &[u8]) -> HttpResponse {
    let request = match serde_json::from_slice::<VnLoadWorldRequest>(body) {
        Ok(request) => request,
        Err(error) => return error_response("400 Bad Request", error.to_string()),
    };
    match load_world(state, &request) {
        Ok(response) => json_response(&response),
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn current_packet_response(state: &VnServerState) -> HttpResponse {
    match current_packet(state) {
        Ok(packet) => json_response(&packet),
        Err(error) => error_response("500 Internal Server Error", error.to_string()),
    }
}

fn pending_agent_turn_response(state: &VnServerState) -> HttpResponse {
    let world_id = match state.world_id() {
        Ok(world_id) => world_id,
        Err(error) => return error_response("500 Internal Server Error", error.to_string()),
    };
    match load_pending_agent_turn(state.store_root.as_deref(), world_id.as_str()) {
        Ok(pending) => json_response(&VnAgentStatusResponse {
            schema_version: VN_CHOOSE_RESPONSE_SCHEMA_VERSION.to_owned(),
            status: "waiting_agent".to_owned(),
            world_id,
            turn_id: Some(pending.turn_id),
        }),
        Err(_) => json_response(&VnAgentStatusResponse {
            schema_version: VN_CHOOSE_RESPONSE_SCHEMA_VERSION.to_owned(),
            status: "idle".to_owned(),
            world_id,
            turn_id: None,
        }),
    }
}

fn commit_agent_turn_response(state: &VnServerState, body: &[u8]) -> HttpResponse {
    let response = match serde_json::from_slice::<AgentTurnResponse>(body) {
        Ok(response) => response,
        Err(error) => return error_response("400 Bad Request", error.to_string()),
    };
    let world_id = match state.world_id() {
        Ok(world_id) => world_id,
        Err(error) => return error_response("500 Internal Server Error", error.to_string()),
    };
    match commit_agent_turn(&AgentCommitTurnOptions {
        store_root: state.store_root.clone(),
        world_id,
        response,
    }) {
        Ok(committed) => {
            wake_host_worker_once(state, "agent_commit_visual_jobs");
            json_response(&committed.packet)
        }
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn cg_retry_response(state: &VnServerState, body: &[u8]) -> HttpResponse {
    let request = if body.is_empty() {
        VnCgRetryRequest { turn_id: None }
    } else {
        match serde_json::from_slice::<VnCgRetryRequest>(body) {
            Ok(request) => request,
            Err(error) => return error_response("400 Bad Request", error.to_string()),
        }
    };
    match request_turn_cg_retry(state, request) {
        Ok(packet) => {
            wake_host_worker_once(state, "turn_cg_retry");
            json_response(&packet)
        }
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn choose_response(state: &VnServerState, body: &[u8]) -> HttpResponse {
    let request = match serde_json::from_slice::<VnChooseRequest>(body) {
        Ok(request) => request,
        Err(error) => return error_response("400 Bad Request", error.to_string()),
    };
    if let Err(error) = validate_vn_input(request.input.as_str()) {
        return error_response("400 Bad Request", error.to_string());
    }
    if agent_bridge_enabled(state) {
        return choose_agent_pending_response(state, request.input, request.narrative_level);
    }
    let turn_options = AdvanceTurnOptions {
        store_root: state.store_root.clone(),
        world_id: match state.world_id() {
            Ok(world_id) => world_id,
            Err(error) => return error_response("500 Internal Server Error", error.to_string()),
        },
        input: request.input,
    };
    if let Err(error) = advance_turn(&turn_options) {
        return error_response("400 Bad Request", error.to_string());
    }
    current_packet_response(state)
}

fn choose_agent_pending_response(
    state: &VnServerState,
    input: String,
    narrative_level: Option<u8>,
) -> HttpResponse {
    let world_id = match state.world_id() {
        Ok(world_id) => world_id,
        Err(error) => return error_response("500 Internal Server Error", error.to_string()),
    };
    match enqueue_agent_turn(&AgentSubmitTurnOptions {
        store_root: state.store_root.clone(),
        world_id,
        input,
        narrative_level: Some(normalize_narrative_level(narrative_level)),
    }) {
        Ok(pending) => {
            wake_host_worker_once(state, "choose_agent_pending");
            json_response(&vn_agent_pending_response(&pending))
        }
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn vn_agent_pending_response(pending: &PendingAgentTurn) -> VnAgentPendingResponse {
    VnAgentPendingResponse {
        schema_version: VN_CHOOSE_RESPONSE_SCHEMA_VERSION.to_owned(),
        status: "waiting_agent".to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        pending_ref: pending.pending_ref.clone(),
        command_hint: format!(
            "singulari-world agent-next --world-id {} --json",
            pending.world_id
        ),
    }
}

fn wake_host_worker_once(state: &VnServerState, reason: &str) {
    if let Err(error) = spawn_host_worker_once(state, reason) {
        eprintln!("vn server host-worker wake failed: reason={reason}, error={error:#}");
    }
}

fn wake_existing_pending_work_once(state: &VnServerState) {
    let Ok(world_id) = state.world_id() else {
        return;
    };
    if load_pending_agent_turn(state.store_root.as_deref(), world_id.as_str()).is_ok() {
        wake_host_worker_once(state, "vn_serve_start_pending_turn");
    }
}

#[cfg(not(test))]
fn spawn_host_worker_once(state: &VnServerState, reason: &str) -> Result<()> {
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let world_id = state.world_id()?;
    let stdout_path = store_paths.root.join(VN_HOST_WORKER_STDOUT);
    let stderr_path = store_paths.root.join(VN_HOST_WORKER_STDERR);
    if let Some(parent) = stdout_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if let Some(parent) = stderr_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let stdout = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stdout_path)
        .with_context(|| format!("failed to open {}", stdout_path.display()))?;
    let stderr = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&stderr_path)
        .with_context(|| format!("failed to open {}", stderr_path.display()))?;
    let exe = std::env::current_exe().context("failed to resolve current singulari-world exe")?;
    let mut child = Command::new(exe)
        .arg("--store-root")
        .arg(store_paths.root.as_os_str())
        .arg("host-worker")
        .arg("--world-id")
        .arg(world_id.as_str())
        .arg("--once")
        .env("SINGULARI_WORLD_VN_WORKER_WAKE_REASON", reason)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| format!("failed to spawn host-worker --once: reason={reason}"))?;
    thread::spawn(move || {
        if let Err(error) = child.wait() {
            eprintln!("vn server host-worker wait failed: {error}");
        }
    });
    Ok(())
}

#[cfg(test)]
fn spawn_host_worker_once(state: &VnServerState, reason: &str) -> Result<()> {
    let _ = reason;
    let _ = resolve_store_paths(state.store_root.as_deref())?;
    Ok(())
}

fn agent_bridge_enabled(state: &VnServerState) -> bool {
    if let Some(value) = std::env::var_os(AGENT_BRIDGE_ENV) {
        let value = value.to_string_lossy();
        return matches!(value.as_ref(), "1" | "true" | "TRUE" | "yes" | "on");
    }
    let _ = state;
    true
}

fn request_turn_cg_retry(
    state: &VnServerState,
    request: VnCgRetryRequest,
) -> Result<crate::vn::VnPacket> {
    let packet = current_packet(state)?;
    let turn_id = request.turn_id.unwrap_or_else(|| packet.turn_id.clone());
    if turn_id != packet.turn_id {
        bail!(
            "turn CG retry only supports the current turn: current={}, requested={}",
            packet.turn_id,
            turn_id
        );
    }
    if !safe_asset_component(turn_id.as_str()) {
        bail!("unsafe turn id for CG retry: {turn_id}");
    }
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, packet.world_id.as_str());
    let retry_path = turn_cg_retry_path(&files.dir, turn_id.as_str());
    if let Some(parent) = retry_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let record = VnCgRetryRecord {
        schema_version: "singulari.turn_cg_retry.v1".to_owned(),
        world_id: packet.world_id,
        turn_id,
        status: "retry_requested".to_owned(),
        requested_at: Utc::now().to_rfc3339(),
    };
    write_json(&retry_path, &record)?;
    current_packet(state)
}

fn runtime_status(state: &VnServerState) -> Result<VnRuntimeStatusResponse> {
    let world_id = state.world_id()?;
    let selection = backend_selection_status(state, world_id.as_str())?;
    let pending = load_pending_agent_turn(state.store_root.as_deref(), world_id.as_str()).ok();
    let latest_text_dispatch = latest_text_dispatch_record(state, world_id.as_str())?;
    let latest_visual_dispatch = latest_visual_dispatch_record(state, world_id.as_str())?;
    let host_supervisor =
        build_host_supervisor_plan(state.store_root.as_deref(), world_id.as_str())?;
    let projection_health = host_supervisor.projection_health.clone();
    let packet = current_packet(state)?;
    let visual =
        visual_runtime_status(state, &packet, &selection, latest_visual_dispatch.as_ref())?;
    let agent_bridge_enabled = agent_bridge_enabled(state);
    let narrative_backend = selection.text_backend.as_str();
    let label = narrative_runtime_label(
        agent_bridge_enabled,
        narrative_backend,
        pending.as_ref().map(|value| value.turn_id.as_str()),
        None,
        latest_text_dispatch.as_ref(),
    );
    let narrative_endpoint =
        narrative_backend_endpoint(narrative_backend, latest_text_dispatch.as_ref());
    let narrative_online =
        backend_endpoint_online(narrative_backend, narrative_endpoint.as_deref());
    let latest_text_status = effective_text_dispatch_status(
        state,
        world_id.as_str(),
        latest_text_dispatch.as_ref(),
        packet.turn_id.as_str(),
        pending.is_some(),
    )?;
    let narrative_status = narrative_runtime_status_code(
        label,
        narrative_backend,
        narrative_online,
        pending.is_some(),
    );
    let narrative_detail = narrative_runtime_detail(
        narrative_backend,
        narrative_online,
        pending.as_ref().map(|value| value.turn_id.as_str()),
        latest_text_status.as_deref(),
    );
    Ok(VnRuntimeStatusResponse {
        schema_version: VN_RUNTIME_STATUS_SCHEMA_VERSION.to_owned(),
        world_id: world_id.clone(),
        backend_selection: selection.clone(),
        narrative: VnNarrativeRuntimeStatus {
            label: label.to_owned(),
            status: narrative_status,
            backend: narrative_backend.to_owned(),
            online: narrative_online,
            detail: narrative_detail,
            endpoint: narrative_endpoint,
            agent_bridge_enabled,
            pending_turn_id: pending.map(|value| value.turn_id),
            binding_present: false,
            latest_dispatch_status: latest_text_status,
        },
        visual,
        details: VnRuntimeDetails {
            backend_selection: selection,
            latest_text_dispatch,
            latest_visual_dispatch,
            projection_health,
            host_supervisor,
        },
    })
}

fn repair_world_db_for_active_world(state: &VnServerState) -> Result<VnWorldDbRepairResponse> {
    let world_id = state.world_id()?;
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, world_id.as_str());
    let repair = repair_world_db(&files.dir, world_id.as_str())?;
    crate::world_docs::refresh_world_docs(&files.dir)?;
    let projection_health =
        build_projection_health_report(state.store_root.as_deref(), world_id.as_str())?;
    Ok(VnWorldDbRepairResponse {
        schema_version: "singulari.vn_world_db_repair.v1".to_owned(),
        world_id,
        repair,
        projection_health,
    })
}

fn repair_extra_memory_for_active_world(
    state: &VnServerState,
) -> Result<VnExtraMemoryRepairResponse> {
    let world_id = state.world_id()?;
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, world_id.as_str());
    let repair = repair_extra_memory_projection(&files.dir, world_id.as_str())?;
    let projection_health =
        build_projection_health_report(state.store_root.as_deref(), world_id.as_str())?;
    Ok(VnExtraMemoryRepairResponse {
        schema_version: "singulari.vn_extra_memory_repair.v1".to_owned(),
        world_id,
        repair,
        projection_health,
    })
}

fn backend_selection_status(
    state: &VnServerState,
    world_id: &str,
) -> Result<VnBackendSelectionStatus> {
    let selection = load_world_backend_selection(state.store_root.as_deref(), world_id)?;
    Ok(match selection {
        Some(selection) => VnBackendSelectionStatus {
            text_backend: selection.text_backend.as_str().to_owned(),
            visual_backend: selection.visual_backend.as_str().to_owned(),
            locked: selection.locked,
            source: selection.source,
            created_at: Some(selection.created_at),
        },
        None => VnBackendSelectionStatus {
            text_backend: WorldTextBackend::Webgpt.as_str().to_owned(),
            visual_backend: WorldVisualBackend::Webgpt.as_str().to_owned(),
            locked: false,
            source: "vn_server_default".to_owned(),
            created_at: None,
        },
    })
}

fn narrative_runtime_label(
    agent_bridge_enabled: bool,
    backend: &str,
    pending_turn_id: Option<&str>,
    binding_source: Option<&str>,
    latest_dispatch: Option<&serde_json::Value>,
) -> &'static str {
    if pending_turn_id.is_some() {
        return "세계 흐름 복원 중";
    }
    if !agent_bridge_enabled {
        return "서사 연결됨";
    }
    if backend == WorldTextBackend::Webgpt.as_str() {
        return "WebGPT 서사 연결됨";
    }
    if latest_dispatch
        .and_then(|value| value.get("binding_clear_reason"))
        .is_some_and(|value| !value.is_null())
    {
        return "새 서사 맥락 재구성됨";
    }
    if binding_source.is_some() {
        return "서사 연결됨";
    }
    "WebGPT 연결 필요"
}

fn narrative_runtime_status_code(
    label: &str,
    backend: &str,
    online: bool,
    pending: bool,
) -> String {
    if pending {
        return "pending".to_owned();
    }
    if backend == WorldTextBackend::Webgpt.as_str() && !online {
        return "needs_connection".to_owned();
    }
    match label {
        "새 서사 맥락 재구성됨" => "rebuilt",
        "서사 연결됨" | "WebGPT 서사 연결됨" => "connected",
        _ => "needs_connection",
    }
    .to_owned()
}

fn narrative_runtime_detail(
    backend: &str,
    online: bool,
    pending_turn_id: Option<&str>,
    latest_dispatch_status: Option<&str>,
) -> String {
    if let Some(turn_id) = pending_turn_id {
        return format!("pending turn: {turn_id}");
    }
    if backend == WorldTextBackend::Webgpt.as_str() {
        return if online {
            format!(
                "WebGPT text CDP online{}",
                latest_dispatch_suffix(latest_dispatch_status)
            )
        } else {
            "WebGPT text CDP offline; text lane cannot consume pending turns".to_owned()
        };
    }
    format!("unknown narrative backend: {backend}")
}

fn visual_runtime_status(
    state: &VnServerState,
    packet: &crate::vn::VnPacket,
    selection: &VnBackendSelectionStatus,
    latest_visual_dispatch: Option<&serde_json::Value>,
) -> Result<VnVisualRuntimeStatus> {
    let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: state.store_root.clone(),
        world_id: packet.world_id.clone(),
    })?;
    let mut pending_jobs = manifest.image_generation_jobs.clone();
    if let Some(job) = packet.image.image_generation_job.clone() {
        pending_jobs.push(job);
    }
    let world_jobs = read_world_jobs(&ReadWorldJobsOptions {
        store_root: state.store_root.clone(),
        world_id: packet.world_id.clone(),
        extra_visual_jobs: pending_jobs.clone(),
    })?;
    let visual_jobs = world_jobs
        .iter()
        .filter(|job| job.kind != WorldJobKind::TextTurn)
        .collect::<Vec<_>>();
    let pending_slots = visual_jobs
        .iter()
        .filter(|job| job.status == WorldJobStatus::Pending)
        .map(|job| job.slot.clone())
        .collect::<Vec<_>>();
    let claimed_slots = visual_jobs
        .iter()
        .filter(|job| job.status == WorldJobStatus::Claimed)
        .map(|job| job.slot.clone())
        .collect::<Vec<_>>();
    let completed_slots = visual_jobs
        .iter()
        .filter(|job| job.status == WorldJobStatus::Completed)
        .map(|job| job.slot.clone())
        .collect::<Vec<_>>();
    let jobs = visual_jobs.into_iter().cloned().collect::<Vec<WorldJob>>();
    let turn_cg_status = if packet.image.image_generation_job.is_some() {
        "pending"
    } else if packet.image.exists {
        "saved"
    } else {
        "idle"
    }
    .to_owned();
    let visual_backend = selection.visual_backend.as_str();
    let endpoint = visual_backend_endpoint(visual_backend, latest_visual_dispatch);
    let online = backend_endpoint_online(visual_backend, endpoint.as_deref());
    let latest_dispatch_status = dispatch_status(latest_visual_dispatch);
    let label = visual_runtime_label(
        visual_backend,
        online,
        &pending_jobs,
        &claimed_slots,
        &completed_slots,
    );
    let status = if !claimed_slots.is_empty() {
        "claimed"
    } else if !pending_jobs.is_empty() {
        "pending"
    } else if !online {
        "needs_connection"
    } else {
        "ready"
    };
    Ok(VnVisualRuntimeStatus {
        label: label.to_owned(),
        status: status.to_owned(),
        backend: visual_backend.to_owned(),
        online,
        detail: visual_runtime_detail(
            visual_backend,
            online,
            &pending_slots,
            &claimed_slots,
            latest_dispatch_status.as_deref(),
        ),
        endpoint,
        pending_slots,
        claimed_slots,
        completed_slots,
        jobs,
        turn_cg_status,
        latest_dispatch_status,
    })
}

fn visual_runtime_label(
    backend: &str,
    online: bool,
    pending_jobs: &[ImageGenerationJob],
    claimed_slots: &[String],
    completed_slots: &[String],
) -> &'static str {
    if !claimed_slots.is_empty() {
        return "CG 생성 중";
    }
    if !pending_jobs.is_empty() {
        return "CG 생성 대기";
    }
    if !online && backend == WorldVisualBackend::Webgpt.as_str() {
        return "WebGPT 이미지 오프라인";
    }
    if completed_slots.is_empty() {
        return "WebGPT 이미지 연결 필요";
    }
    "CG 준비됨"
}

fn visual_runtime_detail(
    backend: &str,
    online: bool,
    pending_slots: &[String],
    claimed_slots: &[String],
    latest_dispatch_status: Option<&str>,
) -> String {
    if !claimed_slots.is_empty() {
        return format!("claimed slots: {}", claimed_slots.join(", "));
    }
    if !pending_slots.is_empty() {
        return format!("pending slots: {}", pending_slots.join(", "));
    }
    if backend == WorldVisualBackend::Webgpt.as_str() {
        return if online {
            format!(
                "WebGPT image CDP online{}",
                latest_dispatch_suffix(latest_dispatch_status)
            )
        } else {
            "WebGPT image CDP offline; image lane cannot generate new CG".to_owned()
        };
    }
    format!("unknown visual backend: {backend}")
}

fn latest_dispatch_suffix(status: Option<&str>) -> String {
    status
        .map(|value| format!("; latest dispatch: {value}"))
        .unwrap_or_default()
}

fn narrative_backend_endpoint(
    backend: &str,
    latest_dispatch: Option<&serde_json::Value>,
) -> Option<String> {
    let latest_webgpt_url = latest_dispatch
        .and_then(|value| value.get("mcp_cdp_url"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    if latest_webgpt_url.is_some() {
        return latest_webgpt_url;
    }
    if backend == WorldTextBackend::Webgpt.as_str() {
        return Some(webgpt_cdp_url(
            WEBGPT_TEXT_CDP_PORT_ENV,
            DEFAULT_WEBGPT_TEXT_CDP_PORT,
        ));
    }
    None
}

fn visual_backend_endpoint(
    backend: &str,
    latest_dispatch: Option<&serde_json::Value>,
) -> Option<String> {
    if let Some(url) = latest_dispatch
        .and_then(|value| value.get("mcp_cdp_url"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
    {
        return Some(url);
    }
    if backend == WorldVisualBackend::Webgpt.as_str() {
        return Some(webgpt_cdp_url(
            WEBGPT_IMAGE_CDP_PORT_ENV,
            DEFAULT_WEBGPT_IMAGE_CDP_PORT,
        ));
    }
    None
}

fn backend_endpoint_online(backend: &str, endpoint: Option<&str>) -> bool {
    if backend == WorldTextBackend::Webgpt.as_str()
        || backend == WorldVisualBackend::Webgpt.as_str()
    {
        return endpoint
            .and_then(loopback_port_from_endpoint)
            .is_some_and(loopback_port_online);
    }
    endpoint
        .and_then(loopback_port_from_endpoint)
        .is_some_and(loopback_port_online)
}

fn webgpt_cdp_url(env_name: &str, default_port: u16) -> String {
    let port = std::env::var(env_name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default_port);
    format!("http://127.0.0.1:{port}")
}

fn loopback_port_from_endpoint(endpoint: &str) -> Option<u16> {
    let without_scheme = endpoint
        .split_once("://")
        .map_or(endpoint, |(_, rest)| rest);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host_port = host_port.split('@').next_back().unwrap_or(host_port);
    let (host, port) = host_port.rsplit_once(':')?;
    if !matches!(host, "127.0.0.1" | "localhost") {
        return None;
    }
    port.parse().ok()
}

fn loopback_port_online(port: u16) -> bool {
    let Ok(addr) = format!("127.0.0.1:{port}").parse() else {
        return false;
    };
    TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(160)).is_ok()
}

fn dispatch_status(record: Option<&serde_json::Value>) -> Option<String> {
    record
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn effective_text_dispatch_status(
    state: &VnServerState,
    world_id: &str,
    record: Option<&serde_json::Value>,
    current_turn_id: &str,
    pending_exists: bool,
) -> Result<Option<String>> {
    let status = dispatch_status(record);
    if pending_exists || status.as_deref() != Some("failed_uncommitted") {
        return Ok(status);
    }
    let record_turn_id = record
        .and_then(|value| value.get("turn_id"))
        .and_then(serde_json::Value::as_str);
    if record_turn_id != Some(current_turn_id) {
        return Ok(status);
    }
    if committed_agent_turn_record_exists(state, world_id, current_turn_id)? {
        return Ok(Some("committed".to_owned()));
    }
    Ok(status)
}

fn committed_agent_turn_record_exists(
    state: &VnServerState,
    world_id: &str,
    turn_id: &str,
) -> Result<bool> {
    let paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&paths, world_id);
    Ok(files
        .dir
        .join("agent_bridge/committed_turns")
        .join(turn_id)
        .join("commit_record.json")
        .is_file())
}

fn latest_text_dispatch_record(
    state: &VnServerState,
    world_id: &str,
) -> Result<Option<serde_json::Value>> {
    let paths = resolve_store_paths(state.store_root.as_deref())?;
    let dispatch_dir = paths
        .worlds_dir
        .join(world_id)
        .join("agent_bridge/dispatches");
    latest_dispatch_record_in_dir(
        dispatch_dir.as_path(),
        &["singulari.webgpt_dispatch_record.v1"],
    )
}

fn latest_visual_dispatch_record(
    state: &VnServerState,
    world_id: &str,
) -> Result<Option<serde_json::Value>> {
    let paths = resolve_store_paths(state.store_root.as_deref())?;
    let dispatch_dir = paths
        .worlds_dir
        .join(world_id)
        .join("visual_jobs/dispatches");
    latest_dispatch_record_in_dir(
        dispatch_dir.as_path(),
        &["singulari.webgpt_image_dispatch_record.v1"],
    )
}

fn latest_dispatch_record_in_dir(
    dispatch_dir: &Path,
    schema_versions: &[&str],
) -> Result<Option<serde_json::Value>> {
    let Ok(entries) = fs::read_dir(dispatch_dir) else {
        return Ok(None);
    };
    let mut records = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let Ok(value) = read_json::<serde_json::Value>(&path) else {
            continue;
        };
        if value
            .get("schema_version")
            .and_then(serde_json::Value::as_str)
            .is_none_or(|schema| !schema_versions.contains(&schema))
        {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        records.push((modified, value));
    }
    records.sort_by_key(|(modified, _)| *modified);
    Ok(records.pop().map(|(_, value)| value))
}

fn current_packet(state: &VnServerState) -> Result<crate::vn::VnPacket> {
    let mut options = BuildVnPacketOptions::new(state.world_id()?);
    options.store_root.clone_from(&state.store_root);
    build_vn_packet(&options)
}

fn cg_gallery(state: &VnServerState) -> Result<VnCgGalleryResponse> {
    let world_id = state.world_id()?;
    let world = crate::store::load_world_record(state.store_root.as_deref(), world_id.as_str())?;
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, world_id.as_str());
    let cg_dir = files.dir.join(TURN_CG_ASSET_DIR);
    let mut turn_ids = Vec::new();
    if cg_dir.exists() {
        for entry in fs::read_dir(cg_dir.as_path())
            .with_context(|| format!("failed to read {}", cg_dir.display()))?
        {
            let entry = entry.with_context(|| format!("failed to read {}", cg_dir.display()))?;
            let path = entry.path();
            if path.extension().and_then(std::ffi::OsStr::to_str) != Some("png") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(std::ffi::OsStr::to_str) else {
                continue;
            };
            if safe_asset_component(stem) {
                turn_ids.push(stem.to_owned());
            }
        }
    }
    turn_ids.sort_by_key(|turn_id| turn_index(turn_id.as_str()));
    let items = turn_ids
        .into_iter()
        .map(|turn_id| cg_gallery_item(state, world_id.as_str(), turn_id))
        .collect::<Result<Vec<_>>>()?;
    Ok(VnCgGalleryResponse {
        schema_version: VN_CG_GALLERY_SCHEMA_VERSION.to_owned(),
        world_id,
        title: world.title,
        items,
    })
}

fn cg_gallery_item(
    state: &VnServerState,
    world_id: &str,
    turn_id: String,
) -> Result<VnCgGalleryItem> {
    let mut options = BuildVnPacketOptions::new(world_id.to_owned());
    options.store_root.clone_from(&state.store_root);
    options.turn_id = Some(turn_id.clone());
    let packet = build_vn_packet(&options)?;
    let image_prompt = packet.image.image_prompt;
    Ok(VnCgGalleryItem {
        turn_index: turn_index(turn_id.as_str()),
        asset_url: format!("/world-assets/{world_id}/{TURN_CG_ASSET_DIR}/{turn_id}.png"),
        download_filename: format!("{world_id}-{turn_id}.png"),
        prompt_summary: summarize_prompt(image_prompt.as_str()),
        image_prompt,
        prompt_policy: packet.image.prompt_policy,
        generated_from_packet: true,
        turn_id,
    })
}

fn turn_index(turn_id: &str) -> u32 {
    turn_id
        .strip_prefix("turn_")
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or_default()
}

fn summarize_prompt(prompt: &str) -> String {
    let normalized = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= PROMPT_SUMMARY_MAX_CHARS {
        return normalized;
    }
    let mut summary = normalized
        .chars()
        .take(PROMPT_SUMMARY_MAX_CHARS)
        .collect::<String>();
    summary.push('…');
    summary
}

fn world_list(state: &VnServerState) -> Result<VnWorldListResponse> {
    let active_world_id = state.world_id()?;
    Ok(VnWorldListResponse {
        active_world_id,
        worlds: list_worlds(state)?,
    })
}

fn select_world(state: &VnServerState, world_id: &str) -> Result<VnWorldSwitchResponse> {
    let world_id = world_id.trim();
    if world_id.is_empty() {
        bail!("world_id must not be empty");
    }
    ensure_safe_world_id(world_id)?;
    let paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&paths, world_id);
    if !files.world.exists() {
        bail!("world not found: {world_id}");
    }
    let snapshot: crate::models::TurnSnapshot = read_json(&files.latest_snapshot)?;
    save_active_world(
        state.store_root.as_deref(),
        world_id,
        snapshot.session_id.as_str(),
    )?;
    state.set_world_id(world_id)?;
    Ok(VnWorldSwitchResponse {
        active_world_id: world_id.to_owned(),
        packet: current_packet(state)?,
        worlds: list_worlds(state)?,
        agent_pending: None,
    })
}

fn new_world(state: &VnServerState, request: VnNewWorldRequest) -> Result<VnWorldSwitchResponse> {
    let text_backend = request.text_backend.unwrap_or(WorldTextBackend::Webgpt);
    let visual_backend = request.visual_backend.unwrap_or(WorldVisualBackend::Webgpt);
    let started = start_world(&StartWorldOptions {
        seed_text: request.seed_text,
        world_id: None,
        title: request.title,
        store_root: state.store_root.clone(),
        session_id: None,
    })?;
    let world_id = started.initialized.world.world_id;
    save_world_backend_selection(
        state.store_root.as_deref(),
        &WorldBackendSelection::new(
            world_id.clone(),
            text_backend,
            visual_backend,
            "vn_world_create",
        ),
    )?;
    state.set_world_id(world_id.as_str())?;
    let agent_pending = if agent_bridge_enabled(state) {
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: state.store_root.clone(),
            world_id: world_id.clone(),
            input: INITIAL_AGENT_TURN_INPUT.to_owned(),
            narrative_level: None,
        })?;
        wake_host_worker_once(state, "new_world_initial_turn");
        Some(vn_agent_pending_response(&pending))
    } else {
        None
    };
    Ok(VnWorldSwitchResponse {
        active_world_id: world_id,
        packet: current_packet(state)?,
        worlds: list_worlds(state)?,
        agent_pending,
    })
}

fn save_world(state: &VnServerState, request: VnSaveWorldRequest) -> Result<VnSaveWorldResponse> {
    let world_id = match request.world_id {
        Some(world_id) => world_id.trim().to_owned(),
        None => state.world_id()?,
    };
    ensure_safe_world_id(world_id.as_str())?;
    let world = crate::store::load_world_record(state.store_root.as_deref(), world_id.as_str())?;
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let bundle_dir = store_paths
        .root
        .join(EXPORTS_DIR)
        .join(export_bundle_name(world_id.as_str()));
    let report = export_world(&ExportWorldOptions {
        store_root: state.store_root.clone(),
        world_id: world_id.clone(),
        output: bundle_dir,
    })?;
    Ok(VnSaveWorldResponse {
        active_world_id: state.world_id()?,
        world_id,
        title: world.title,
        bundle_dir: report.bundle_dir.display().to_string(),
        manifest_path: report.manifest_path.display().to_string(),
        files_copied: report.files_copied,
        worlds: list_worlds(state)?,
    })
}

fn load_world(
    state: &VnServerState,
    request: &VnLoadWorldRequest,
) -> Result<VnWorldSwitchResponse> {
    let bundle_text = request.bundle.trim();
    if bundle_text.is_empty() {
        bail!("world bundle path must not be empty");
    }
    let bundle = PathBuf::from(bundle_text);
    let imported_world = read_bundle_world_record(bundle.as_path())?;
    ensure_safe_world_id(imported_world.world_id.as_str())?;
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, imported_world.world_id.as_str());
    if files.world.exists() {
        return select_world(state, imported_world.world_id.as_str());
    }
    let report = import_world(&ImportWorldOptions {
        store_root: state.store_root.clone(),
        bundle,
        activate: true,
    })?;
    state.set_world_id(report.world_id.as_str())?;
    Ok(VnWorldSwitchResponse {
        active_world_id: report.world_id,
        packet: current_packet(state)?,
        worlds: list_worlds(state)?,
        agent_pending: None,
    })
}

fn read_world_asset(state: &VnServerState, asset_path: &str) -> Result<(&'static str, Vec<u8>)> {
    let mut parts = asset_path.split('/');
    let world_id = parts.next().unwrap_or_default();
    ensure_safe_world_id(world_id)?;
    let mut relative = PathBuf::new();
    for part in parts {
        if !safe_asset_component(part) {
            bail!("unsafe world asset path: {asset_path}");
        }
        relative.push(part);
    }
    if relative.as_os_str().is_empty() {
        bail!("world asset path missing relative file: {asset_path}");
    }
    let store_paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, world_id);
    let path = files.dir.join(&relative);
    if !path.starts_with(&files.dir) || !path.is_file() {
        bail!("world asset not found: {asset_path}");
    }
    let metadata = fs::symlink_metadata(&path)
        .with_context(|| format!("failed to stat {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("world asset rejected symlink: {asset_path}");
    }
    let content_type = asset_content_type(&path);
    let body = fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
    Ok((content_type, body))
}

fn list_worlds(state: &VnServerState) -> Result<Vec<VnWorldSummary>> {
    let paths = resolve_store_paths(state.store_root.as_deref())?;
    if !paths.worlds_dir.exists() {
        return Ok(Vec::new());
    }
    let mut worlds = Vec::new();
    for entry in fs::read_dir(&paths.worlds_dir)
        .with_context(|| format!("failed to read {}", paths.worlds_dir.display()))?
    {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let world_id = entry.file_name().to_string_lossy().into_owned();
        if ensure_safe_world_id(world_id.as_str()).is_err() {
            continue;
        }
        if let Ok(summary) = world_summary(state, world_id.as_str()) {
            worlds.push(summary);
        }
    }
    worlds.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(worlds)
}

fn world_summary(state: &VnServerState, world_id: &str) -> Result<VnWorldSummary> {
    let world = crate::store::load_world_record(state.store_root.as_deref(), world_id)?;
    let paths = resolve_store_paths(state.store_root.as_deref())?;
    let files = world_file_paths(&paths, world_id);
    let snapshot: crate::models::TurnSnapshot = read_json(&files.latest_snapshot)?;
    Ok(VnWorldSummary {
        world_id: world.world_id,
        title: world.title,
        updated_at: world.updated_at,
        turn_id: snapshot.turn_id,
        phase: snapshot.phase,
    })
}

impl VnServerState {
    fn world_id(&self) -> Result<String> {
        self.world_id
            .lock()
            .map(|world_id| world_id.clone())
            .map_err(|_| anyhow::anyhow!("vn server world lock poisoned"))
    }

    fn set_world_id(&self, next_world_id: &str) -> Result<()> {
        let mut world_id = self
            .world_id
            .lock()
            .map_err(|_| anyhow::anyhow!("vn server world lock poisoned"))?;
        next_world_id.clone_into(&mut *world_id);
        Ok(())
    }
}

fn validate_vn_input(input: &str) -> Result<()> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("VN input must not be empty");
    }
    if trimmed.chars().count() > MAX_VN_INPUT_CHARS {
        bail!("VN input too long: max_chars={MAX_VN_INPUT_CHARS}");
    }
    if trimmed
        .parse::<u8>()
        .is_ok_and(|slot| (1..=7).contains(&slot) && slot != FREEFORM_CHOICE_SLOT)
    {
        return Ok(());
    }
    if is_inline_freeform(trimmed) {
        return Ok(());
    }
    bail!("VN input must be 1..7 or inline 6 <action>");
}

fn ensure_safe_world_id(world_id: &str) -> Result<()> {
    if !world_id.is_empty()
        && world_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return Ok(());
    }
    bail!("unsafe world_id: {world_id}");
}

fn safe_asset_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn asset_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    }
}

fn export_bundle_name(world_id: &str) -> String {
    let now = Utc::now();
    format!(
        "{}_{}_{:09}",
        world_id,
        now.format("%Y%m%dT%H%M%SZ"),
        now.timestamp_subsec_nanos()
    )
}

#[cfg(test)]
fn save_request_body(world_id: Option<&str>) -> Vec<u8> {
    serde_json::to_vec(&VnSaveWorldRequest {
        world_id: world_id.map(str::to_owned),
    })
    .unwrap_or_default()
}

#[cfg(test)]
fn load_request_body(bundle: &Path) -> Vec<u8> {
    serde_json::to_vec(&VnLoadWorldRequest {
        bundle: bundle.display().to_string(),
    })
    .unwrap_or_default()
}

fn read_bundle_world_record(bundle: &Path) -> Result<crate::models::WorldRecord> {
    let root = if bundle.join(WORLD_FILENAME).is_file() {
        bundle.to_path_buf()
    } else {
        let mut candidates = Vec::new();
        for entry in
            fs::read_dir(bundle).with_context(|| format!("failed to read {}", bundle.display()))?
        {
            let entry = entry.with_context(|| format!("failed to read {}", bundle.display()))?;
            if entry.path().join(WORLD_FILENAME).is_file() {
                candidates.push(entry.path());
            }
        }
        match candidates.len() {
            1 => candidates.remove(0),
            0 => bail!(
                "world bundle missing {WORLD_FILENAME}: {}",
                bundle.display()
            ),
            _ => bail!(
                "world bundle is ambiguous; multiple child world roots under {}",
                bundle.display()
            ),
        }
    };
    read_json(&root.join(WORLD_FILENAME))
}

fn is_inline_freeform(input: &str) -> bool {
    let Some(slot_digit) = char::from_digit(u32::from(FREEFORM_CHOICE_SLOT), 10) else {
        return false;
    };
    let Some(after_slot) = input.strip_prefix(slot_digit) else {
        return false;
    };
    if after_slot.is_empty() {
        return false;
    }
    let rest = if let Some(rest) = after_slot.strip_prefix("번") {
        rest
    } else if after_slot.starts_with(char::is_whitespace) {
        after_slot
    } else if after_slot
        .chars()
        .next()
        .is_some_and(|ch| matches!(ch, '.' | ')' | ':' | '-' | '—'))
    {
        &after_slot[after_slot.chars().next().map_or(0, char::len_utf8)..]
    } else {
        return false;
    };
    let action = rest
        .trim_start()
        .trim_start_matches(['.', ')', ':', '-', '—'])
        .trim_start();
    !action.is_empty()
}

fn ensure_allowed_vn_host(host: &str) -> Result<()> {
    if matches!(host, "127.0.0.1" | "localhost") {
        return Ok(());
    }
    if host.ends_with(".ts.net") {
        return Ok(());
    }
    if host
        .parse::<IpAddr>()
        .is_ok_and(|addr| addr.is_loopback() || is_tailscale_ipv4(addr))
    {
        return Ok(());
    }
    bail!("vn server host must be loopback or Tailscale-scoped: got {host}");
}

fn is_tailscale_ipv4(addr: IpAddr) -> bool {
    let IpAddr::V4(addr) = addr else {
        return false;
    };
    let octets = addr.octets();
    let lower = Ipv4Addr::new(100, 64, 0, 0).octets();
    let upper = Ipv4Addr::new(100, 127, 255, 255).octets();
    octets >= lower && octets <= upper
}

fn read_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    let mut buffer = Vec::new();
    let mut temp = [0_u8; 4096];
    loop {
        let read = stream
            .read(&mut temp)
            .context("vn server request read failed")?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..read]);
        if buffer.len() > MAX_REQUEST_BYTES {
            bail!("request too large");
        }
        if header_end(&buffer).is_some() {
            break;
        }
    }
    let header_end = header_end(&buffer).context("missing HTTP header terminator")?;
    let headers = std::str::from_utf8(&buffer[..header_end]).context("request header not utf-8")?;
    let mut lines = headers.lines();
    let request_line = lines.next().context("missing HTTP request line")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().context("missing HTTP method")?.to_owned();
    let path = parts.next().context("missing HTTP path")?.to_owned();
    let content_length = content_length(headers)?;
    let body_start = header_end + 4;
    while buffer.len() < body_start + content_length {
        let read = stream
            .read(&mut temp)
            .context("vn server body read failed")?;
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&temp[..read]);
        if buffer.len() > MAX_REQUEST_BYTES {
            bail!("request too large");
        }
    }
    let body_end = (body_start + content_length).min(buffer.len());
    Ok(HttpRequest {
        method,
        path,
        body: buffer[body_start..body_end].to_vec(),
    })
}

fn header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn content_length(headers: &str) -> Result<usize> {
    for line in headers.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value
                .trim()
                .parse::<usize>()
                .context("invalid content-length");
        }
    }
    Ok(0)
}

fn write_response(stream: &mut TcpStream, response: &HttpResponse) -> Result<()> {
    let header = format!(
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        response.status,
        response.content_type,
        response.body.len()
    );
    stream
        .write_all(header.as_bytes())
        .context("vn server response header write failed")?;
    stream
        .write_all(&response.body)
        .context("vn server response body write failed")?;
    Ok(())
}

fn html_response(body: &str) -> HttpResponse {
    static_response("text/html; charset=utf-8", body)
}

fn static_response(content_type: &'static str, body: &str) -> HttpResponse {
    HttpResponse {
        status: "200 OK",
        content_type,
        body: body.as_bytes().to_vec(),
    }
}

fn json_response<T: Serialize>(value: &T) -> HttpResponse {
    match serde_json::to_vec_pretty(value) {
        Ok(body) => HttpResponse {
            status: "200 OK",
            content_type: "application/json; charset=utf-8",
            body,
        },
        Err(error) => error_response("500 Internal Server Error", error.to_string()),
    }
}

fn error_response(status: &'static str, error: String) -> HttpResponse {
    json_response(&VnErrorResponse { error }).with_status(status)
}

impl HttpResponse {
    fn with_status(mut self, status: &'static str) -> Self {
        self.status = status;
        self
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "test helper keeps request bodies owned like real HTTP reads"
)]
#[cfg(test)]
fn choose_request_body(input: &str) -> Vec<u8> {
    serde_json::to_vec(&VnChooseRequest {
        input: input.to_owned(),
        narrative_level: None,
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        VnAgentPendingResponse, VnCgRetryRequest, VnNewWorldRequest, VnSaveWorldResponse,
        VnServerState, VnWorldSwitchResponse, cg_gallery, cg_retry_response, choose_request_body,
        choose_response, ensure_allowed_vn_host, load_request_body, load_world_response, new_world,
        read_world_asset, repair_world_db_response, runtime_status, save_request_body,
        save_world_response, validate_vn_input, world_list,
    };
    use crate::backend_selection::{WorldTextBackend, WorldVisualBackend};
    use crate::job_ledger::{WorldJobKind, WorldJobStatus};
    use crate::store::{InitWorldOptions, init_world};
    use crate::transfer::{ExportWorldOptions, export_world};
    use crate::turn::{AdvanceTurnOptions, advance_turn};
    use crate::visual_assets::{STAGE_BACKGROUND_FILENAME, VN_ASSETS_DIR};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use tempfile::tempdir;

    #[test]
    fn validate_vn_input_accepts_direct_choices_and_inline_freeform() {
        assert!(validate_vn_input("1").is_ok());
        assert!(validate_vn_input("5").is_ok());
        assert!(validate_vn_input("6 세아에게 낮게 묻는다").is_ok());
        assert!(validate_vn_input("6번 문서관을 본다").is_ok());
        assert!(validate_vn_input("6").is_err());
        assert!(validate_vn_input("7").is_ok());
        assert!(validate_vn_input("8").is_err());
        assert!(validate_vn_input("문서관을 본다").is_err());
    }

    #[test]
    fn vn_host_allows_loopback_and_tailscale_only() {
        assert!(ensure_allowed_vn_host("127.0.0.1").is_ok());
        assert!(ensure_allowed_vn_host("localhost").is_ok());
        assert!(ensure_allowed_vn_host("100.64.0.1").is_ok());
        assert!(ensure_allowed_vn_host("100.127.255.255").is_ok());
        assert!(ensure_allowed_vn_host("macbookair.tailnet.ts.net").is_ok());
        assert!(ensure_allowed_vn_host("0.0.0.0").is_err());
        assert!(ensure_allowed_vn_host("192.168.0.10").is_err());
    }

    #[test]
    fn choose_response_queues_pending_turn_when_agent_bridge_is_enabled() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_server
title: "VN 서버 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_vn_server".to_owned(),
            input: "1".to_owned(),
        })?;
        let state = VnServerState {
            store_root: Some(store),
            world_id: Mutex::new("stw_vn_server".to_owned()),
        };
        let response = choose_response(&state, &choose_request_body("6 세아에게 낮게 묻는다"));
        assert_eq!(response.status, "200 OK");
        let pending: VnAgentPendingResponse = serde_json::from_slice(&response.body)?;
        assert_eq!(pending.status, "waiting_agent");
        assert_eq!(pending.turn_id, "turn_0002");
        Ok(())
    }

    #[test]
    fn cg_retry_response_marks_background_turn_cg_job() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_cg_retry
title: "CG 재시도 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_vn_cg_retry".to_owned(),
            input: "1".to_owned(),
        })?;
        let state = VnServerState {
            store_root: Some(store),
            world_id: Mutex::new("stw_vn_cg_retry".to_owned()),
        };
        let body = serde_json::to_vec(&VnCgRetryRequest { turn_id: None })?;
        let response = cg_retry_response(&state, &body);
        assert_eq!(response.status, "200 OK");
        let packet: crate::vn::VnPacket = serde_json::from_slice(&response.body)?;
        assert!(packet.image.auto_decision.retry_requested);
        assert!(packet.image.image_generation_job.is_some());
        assert!(
            packet
                .image
                .image_generation_job
                .as_ref()
                .is_some_and(|job| job.register_policy.contains("background retry"))
        );
        Ok(())
    }

    #[test]
    fn world_launcher_lists_and_starts_new_world() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_launcher_base
title: "런처 기본 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let state = VnServerState {
            store_root: Some(store),
            world_id: Mutex::new("stw_launcher_base".to_owned()),
        };

        let initial_list = world_list(&state)?;
        assert_eq!(initial_list.active_world_id, "stw_launcher_base");
        assert_eq!(initial_list.worlds.len(), 1);

        let response = new_world(
            &state,
            VnNewWorldRequest {
                seed_text: "중세 변경 마을, 남자 순찰자, 봉인된 길표식".to_owned(),
                title: Some("런처 새 세계".to_owned()),
                text_backend: Some(WorldTextBackend::Webgpt),
                visual_backend: Some(WorldVisualBackend::Webgpt),
            },
        )?;
        assert_eq!(response.packet.title, "런처 새 세계");
        assert_eq!(response.packet.world_id, response.active_world_id);
        assert!(response.worlds.len() >= 2);
        assert_eq!(state.world_id()?, response.active_world_id);
        let Some(backend_selection) = crate::backend_selection::load_world_backend_selection(
            state.store_root.as_deref(),
            response.active_world_id.as_str(),
        )?
        else {
            anyhow::bail!("new world should lock backend selection");
        };
        assert_eq!(backend_selection.text_backend, WorldTextBackend::Webgpt);
        assert_eq!(backend_selection.visual_backend, WorldVisualBackend::Webgpt);
        Ok(())
    }

    #[test]
    fn runtime_status_surfaces_friendly_visual_job_state() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_runtime_status
title: "런타임 상태 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let state = VnServerState {
            store_root: Some(store),
            world_id: Mutex::new("stw_vn_runtime_status".to_owned()),
        };

        let status = runtime_status(&state)?;

        assert_eq!(status.schema_version, "singulari.vn_runtime_status.v1");
        assert_eq!(status.backend_selection.text_backend, "webgpt");
        assert_eq!(status.backend_selection.visual_backend, "webgpt");
        assert_eq!(status.narrative.backend, "webgpt");
        assert_eq!(status.visual.backend, "webgpt");
        assert_eq!(
            status.details.projection_health.status.to_string(),
            "healthy"
        );
        assert_eq!(status.details.host_supervisor.status.to_string(), "ready");
        assert_eq!(
            status.details.host_supervisor.recommended_action,
            "dispatch_lanes:image"
        );
        assert_eq!(status.visual.label, "CG 생성 대기");
        assert!(
            status
                .visual
                .pending_slots
                .contains(&"menu_background".to_owned())
        );
        assert!(
            status
                .visual
                .pending_slots
                .contains(&"stage_background".to_owned())
        );
        assert!(status.visual.jobs.iter().any(|job| {
            job.slot == "menu_background"
                && job.kind == WorldJobKind::UiAsset
                && job.status == WorldJobStatus::Pending
        }));
        assert!(status.visual.jobs.iter().any(|job| {
            job.slot == "stage_background"
                && job.kind == WorldJobKind::UiAsset
                && job.status == WorldJobStatus::Pending
        }));
        assert!(matches!(
            status.narrative.label.as_str(),
            "서사 연결됨" | "WebGPT 서사 연결됨"
        ));
        Ok(())
    }

    #[test]
    fn repair_world_db_response_rebuilds_projection_and_reports_health() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_repair_db
title: "DB 복구 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        std::fs::remove_file(initialized.world_dir.join(crate::WORLD_DB_FILENAME))?;
        let state = VnServerState {
            store_root: Some(store),
            world_id: Mutex::new("stw_vn_repair_db".to_owned()),
        };

        let response = repair_world_db_response(&state);

        assert_eq!(response.status, "200 OK");
        let body: serde_json::Value = serde_json::from_slice(&response.body)?;
        assert_eq!(body["repair"]["rebuilt"], true);
        assert_eq!(body["projection_health"]["status"], "healthy");
        Ok(())
    }

    #[test]
    fn cg_gallery_lists_saved_turn_images_with_prompt_metadata() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_gallery
title: "갤러리 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_vn_gallery".to_owned(),
            input: "1".to_owned(),
        })?;
        let turn_cg_dir = store
            .join("worlds/stw_vn_gallery")
            .join(VN_ASSETS_DIR)
            .join("turn_cg");
        std::fs::create_dir_all(turn_cg_dir.as_path())?;
        std::fs::write(turn_cg_dir.join("turn_0001.png"), b"png fixture")?;
        let state = VnServerState {
            store_root: Some(store),
            world_id: Mutex::new("stw_vn_gallery".to_owned()),
        };

        let gallery = cg_gallery(&state)?;

        assert_eq!(gallery.schema_version, "singulari.vn_cg_gallery.v1");
        assert_eq!(gallery.items.len(), 1);
        let item = &gallery.items[0];
        assert_eq!(item.turn_id, "turn_0001");
        assert_eq!(item.turn_index, 1);
        assert!(item.asset_url.ends_with("/assets/vn/turn_cg/turn_0001.png"));
        assert!(item.image_prompt.contains("Scene narrative"));
        assert!(!item.prompt_summary.is_empty());
        Ok(())
    }

    #[test]
    fn world_save_and_load_operate_on_whole_world_bundles() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let target_store = temp.path().join("target-store");
        let source_store = temp.path().join("source-store");
        let current_seed = temp.path().join("current-seed.yaml");
        let source_seed = temp.path().join("source-seed.yaml");
        std::fs::write(
            &current_seed,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_current
title: "현재 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        std::fs::write(
            &source_seed,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_imported
title: "불러온 세계"
premise:
  genre: "서정 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path: current_seed,
            store_root: Some(target_store.clone()),
            session_id: None,
        })?;
        init_world(&InitWorldOptions {
            seed_path: source_seed,
            store_root: Some(source_store.clone()),
            session_id: None,
        })?;

        let state = VnServerState {
            store_root: Some(target_store.clone()),
            world_id: Mutex::new("stw_vn_current".to_owned()),
        };
        let save_response = save_world_response(&state, &save_request_body(None));
        assert_eq!(save_response.status, "200 OK");
        let saved: VnSaveWorldResponse = serde_json::from_slice(&save_response.body)?;
        assert_eq!(saved.world_id, "stw_vn_current");
        assert!(PathBuf::from(saved.bundle_dir).join("world.json").is_file());

        let bundle = temp.path().join("import-bundle");
        export_world(&ExportWorldOptions {
            store_root: Some(source_store),
            world_id: "stw_vn_imported".to_owned(),
            output: bundle.clone(),
        })?;
        let load_response = load_world_response(&state, &load_request_body(bundle.as_path()));
        assert_eq!(load_response.status, "200 OK");
        let loaded: VnWorldSwitchResponse = serde_json::from_slice(&load_response.body)?;
        assert_eq!(loaded.active_world_id, "stw_vn_imported");
        assert_eq!(loaded.packet.world_id, "stw_vn_imported");
        assert_eq!(state.world_id()?, "stw_vn_imported");
        assert_eq!(world_list(&state)?.worlds.len(), 2);
        Ok(())
    }

    #[test]
    fn world_asset_route_serves_same_origin_visual_asset_files() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_asset
title: "에셋 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let asset_dir = initialized.world_dir.join(VN_ASSETS_DIR);
        std::fs::create_dir_all(&asset_dir)?;
        std::fs::write(asset_dir.join(STAGE_BACKGROUND_FILENAME), b"png-bytes")?;
        let state = VnServerState {
            store_root: Some(store),
            world_id: Mutex::new("stw_vn_asset".to_owned()),
        };
        let (content_type, body) =
            read_world_asset(&state, "stw_vn_asset/assets/vn/stage_background.png")?;
        assert_eq!(content_type, "image/png");
        assert_eq!(body, b"png-bytes");
        Ok(())
    }
}
