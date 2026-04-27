use crate::agent_bridge::{
    AgentCommitTurnOptions, AgentSubmitTurnOptions, AgentTurnResponse, commit_agent_turn,
    enqueue_agent_turn, load_pending_agent_turn,
};
use crate::start::{StartWorldOptions, start_world};
use crate::store::{
    WORLD_FILENAME, read_json, resolve_store_paths, resolve_world_id, save_active_world,
    world_file_paths, write_json,
};
use crate::transfer::{ExportWorldOptions, ImportWorldOptions, export_world, import_world};
use crate::turn::{AdvanceTurnOptions, advance_turn};
use crate::visual_assets::{BuildWorldVisualAssetsOptions, build_world_visual_assets};
use crate::vn::{BuildVnPacketOptions, build_vn_packet, turn_cg_retry_path};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const INDEX_HTML: &str = include_str!("../vn-web/index.html");
const APP_JS: &str = include_str!("../vn-web/app.js");
const STYLES_CSS: &str = include_str!("../vn-web/styles.css");

const DEFAULT_HOST: &str = "127.0.0.1";
const EXPORTS_DIR: &str = "exports";
const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_VN_INPUT_CHARS: usize = 2048;
const AGENT_BRIDGE_ENV: &str = "SINGULARI_WORLD_AGENT_BRIDGE";
const VN_CHOOSE_RESPONSE_SCHEMA_VERSION: &str = "singulari.vn_choose_response.v1";

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnNewWorldRequest {
    pub seed_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
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
/// local-only, or the TCP listener cannot bind.
pub fn serve_vn(options: &VnServeOptions) -> Result<()> {
    ensure_local_host(options.host.as_str())?;
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
    for stream in listener.incoming() {
        match stream {
            Ok(mut stream) => {
                let response = match read_request(&mut stream) {
                    Ok(request) => route_request(&state, &request),
                    Err(error) => error_response("400 Bad Request", error.to_string()),
                };
                write_response(&mut stream, &response)?;
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
        ("POST", "/api/vn/worlds/select") => select_world_response(state, &request.body),
        ("POST", "/api/vn/worlds/new") => new_world_response(state, &request.body),
        ("POST", "/api/vn/worlds/save") => save_world_response(state, &request.body),
        ("POST", "/api/vn/worlds/load") => load_world_response(state, &request.body),
        ("GET", "/api/vn/current") => current_packet_response(state),
        ("GET", "/api/vn/agent/pending") => pending_agent_turn_response(state),
        ("POST", "/api/vn/agent/commit") => commit_agent_turn_response(state, &request.body),
        ("POST", "/api/vn/cg/retry") => cg_retry_response(state, &request.body),
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
        Ok(committed) => json_response(&committed.packet),
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
        Ok(packet) => json_response(&packet),
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
    if agent_bridge_enabled() {
        return choose_agent_pending_response(state, request.input);
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

fn choose_agent_pending_response(state: &VnServerState, input: String) -> HttpResponse {
    let world_id = match state.world_id() {
        Ok(world_id) => world_id,
        Err(error) => return error_response("500 Internal Server Error", error.to_string()),
    };
    match enqueue_agent_turn(&AgentSubmitTurnOptions {
        store_root: state.store_root.clone(),
        world_id,
        input,
    }) {
        Ok(pending) => json_response(&VnAgentPendingResponse {
            schema_version: VN_CHOOSE_RESPONSE_SCHEMA_VERSION.to_owned(),
            status: "waiting_agent".to_owned(),
            world_id: pending.world_id.clone(),
            turn_id: pending.turn_id.clone(),
            pending_ref: pending.pending_ref,
            command_hint: format!(
                "singulari-world agent-next --world-id {} --json",
                pending.world_id
            ),
        }),
        Err(error) => error_response("400 Bad Request", error.to_string()),
    }
}

fn agent_bridge_enabled() -> bool {
    std::env::var_os(AGENT_BRIDGE_ENV).is_some_and(|value| {
        let value = value.to_string_lossy();
        matches!(value.as_ref(), "1" | "true" | "TRUE" | "yes" | "on")
    })
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

fn current_packet(state: &VnServerState) -> Result<crate::vn::VnPacket> {
    let mut options = BuildVnPacketOptions::new(state.world_id()?);
    options.store_root.clone_from(&state.store_root);
    build_vn_packet(&options)
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
    })
}

fn new_world(state: &VnServerState, request: VnNewWorldRequest) -> Result<VnWorldSwitchResponse> {
    let started = start_world(&StartWorldOptions {
        seed_text: request.seed_text,
        world_id: None,
        title: request.title,
        store_root: state.store_root.clone(),
        session_id: None,
    })?;
    let world_id = started.initialized.world.world_id;
    state.set_world_id(world_id.as_str())?;
    Ok(VnWorldSwitchResponse {
        active_world_id: world_id,
        packet: current_packet(state)?,
        worlds: list_worlds(state)?,
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
        .is_ok_and(|slot| (1..=6).contains(&slot))
    {
        return Ok(());
    }
    if is_inline_freeform(trimmed) {
        return Ok(());
    }
    bail!("VN input must be 1..6 or inline 7 <action>");
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
    let Some(after_slot) = input.strip_prefix('7') else {
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

fn ensure_local_host(host: &str) -> Result<()> {
    if matches!(host, "127.0.0.1" | "localhost") {
        return Ok(());
    }
    bail!("vn server host must be local-only: got {host}");
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
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{
        VnCgRetryRequest, VnNewWorldRequest, VnSaveWorldResponse, VnServerState,
        VnWorldSwitchResponse, cg_retry_response, choose_request_body, choose_response,
        load_request_body, load_world_response, new_world, read_world_asset, save_request_body,
        save_world_response, validate_vn_input, world_list,
    };
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
        assert!(validate_vn_input("6").is_ok());
        assert!(validate_vn_input("7 세아에게 낮게 묻는다").is_ok());
        assert!(validate_vn_input("7번 문서관을 본다").is_ok());
        assert!(validate_vn_input("7").is_err());
        assert!(validate_vn_input("8").is_err());
        assert!(validate_vn_input("문서관을 본다").is_err());
    }

    #[test]
    fn choose_response_advances_turn_and_returns_next_packet() -> anyhow::Result<()> {
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
  protagonist: "현대인의 전생, 남자 주인공"
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
        let response = choose_response(&state, &choose_request_body("7 세아에게 낮게 묻는다"));
        assert_eq!(response.status, "200 OK");
        let packet: crate::vn::VnPacket = serde_json::from_slice(&response.body)?;
        assert_eq!(packet.turn_id, "turn_0002");
        assert!(packet.scene.status.contains("7번 [자유서술]"));
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
  protagonist: "현대인의 전생, 남자 주인공"
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
  protagonist: "현대인의 전생, 남자 주인공"
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
                seed_text: "판타지, 현대인 전생, 남주".to_owned(),
                title: Some("런처 새 세계".to_owned()),
            },
        )?;
        assert_eq!(response.packet.title, "런처 새 세계");
        assert_eq!(response.packet.world_id, response.active_world_id);
        assert!(response.worlds.len() >= 2);
        assert_eq!(state.world_id()?, response.active_world_id);
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
  protagonist: "현대인의 전생, 남자 주인공"
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
  protagonist: "현대인의 전생, 남자 주인공"
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
  protagonist: "현대인의 전생, 남자 주인공"
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
