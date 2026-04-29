use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use singulari_world::{
    AgentCommitTurnOptions, AgentRevivalCompileOptions, AgentTurnResponse,
    CompleteVisualJobOptions, ImageGenerationJob, VisualArtifactKind,
    assemble_prompt_context_packet, assemble_turn_context_packet, build_agent_revival_packet,
    commit_agent_turn, complete_visual_job, resolve_store_paths,
};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use super::host_worker::HostWorkerOptions;

pub(crate) const DEFAULT_WEBGPT_TEXT_CDP_PORT: u16 = 9238;
pub(crate) const DEFAULT_WEBGPT_IMAGE_CDP_PORT: u16 = 9239;
pub(crate) const DEFAULT_WEBGPT_REFERENCE_IMAGE_CDP_PORT: u16 = 9240;

fn ensure_control_safe_runtime_value(field: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        anyhow::bail!("{field} contains control characters");
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct WebGptLaneRuntime {
    lane: WebGptConversationLane,
    profile_dir: PathBuf,
    cdp_port: u16,
}

impl WebGptLaneRuntime {
    fn new(lane: WebGptConversationLane, options: &HostWorkerOptions) -> Result<Self> {
        Ok(Self {
            lane,
            profile_dir: webgpt_lane_profile_dir(lane, options)?,
            cdp_port: match lane {
                WebGptConversationLane::Text => options.webgpt_text_cdp_port,
                WebGptConversationLane::Image => options.webgpt_image_cdp_port,
            },
        })
    }

    fn new_image(
        session_kind: WebGptImageSessionKind,
        options: &HostWorkerOptions,
    ) -> Result<Self> {
        Ok(Self {
            lane: WebGptConversationLane::Image,
            profile_dir: webgpt_image_lane_profile_dir(session_kind, options)?,
            cdp_port: match session_kind {
                WebGptImageSessionKind::TurnCg => options.webgpt_image_cdp_port,
                WebGptImageSessionKind::ReferenceAsset => options.webgpt_reference_image_cdp_port,
            },
        })
    }

    fn cdp_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.cdp_port)
    }

    fn apply_to_command(&self, command: &mut Command) {
        command
            .env("WEBGPT_MCP_CDP_PORT", self.cdp_port.to_string())
            .env("WEBGPT_MCP_PROFILE_DIR", self.profile_dir.as_os_str())
            .env(
                "WEBGPT_MCP_MANUAL_PROFILE_DIR",
                self.profile_dir.as_os_str(),
            )
            .env(
                "WEBGPT_MCP_BOOTSTRAP_SNAPSHOT_DIR",
                self.profile_dir.with_extension("snapshot").as_os_str(),
            );
    }
}

pub(super) fn ensure_webgpt_lane_runtime_isolated(options: &HostWorkerOptions) -> Result<()> {
    let text = WebGptLaneRuntime::new(WebGptConversationLane::Text, options)?;
    let turn_cg = WebGptLaneRuntime::new_image(WebGptImageSessionKind::TurnCg, options)?;
    let reference = WebGptLaneRuntime::new_image(WebGptImageSessionKind::ReferenceAsset, options)?;
    for (left_name, left, right_name, right) in [
        ("text", &text, "turn_cg_image", &turn_cg),
        ("text", &text, "reference_image", &reference),
        ("turn_cg_image", &turn_cg, "reference_image", &reference),
    ] {
        if left.cdp_port == right.cdp_port {
            anyhow::bail!(
                "webgpt lanes must use distinct CDP ports: left={left_name}, right={right_name}, port={}",
                left.cdp_port
            );
        }
        if left.profile_dir == right.profile_dir {
            anyhow::bail!(
                "webgpt lanes must use distinct profile dirs: left={left_name}, right={right_name}, profile_dir={}",
                left.profile_dir.display()
            );
        }
    }
    Ok(())
}

fn webgpt_lane_profile_dir(
    lane: WebGptConversationLane,
    options: &HostWorkerOptions,
) -> Result<PathBuf> {
    let configured = match lane {
        WebGptConversationLane::Text => options.webgpt_text_profile_dir.clone(),
        WebGptConversationLane::Image => options.webgpt_image_profile_dir.clone(),
    };
    if let Some(path) = configured {
        return Ok(path);
    }
    let root = if let Some(path) = std::env::var_os("SINGULARI_WORLD_WEBGPT_PROFILE_ROOT") {
        PathBuf::from(path)
    } else {
        webgpt_default_profile_root()?
    };
    Ok(root.join(lane.profile_dir_name()))
}

fn webgpt_image_lane_profile_dir(
    session_kind: WebGptImageSessionKind,
    options: &HostWorkerOptions,
) -> Result<PathBuf> {
    match session_kind {
        WebGptImageSessionKind::TurnCg => {
            webgpt_lane_profile_dir(WebGptConversationLane::Image, options)
        }
        WebGptImageSessionKind::ReferenceAsset => {
            if let Some(path) = options.webgpt_reference_image_profile_dir.clone() {
                return Ok(path);
            }
            let root = if let Some(path) = std::env::var_os("SINGULARI_WORLD_WEBGPT_PROFILE_ROOT") {
                PathBuf::from(path)
            } else {
                webgpt_default_profile_root()?
            };
            Ok(root.join("reference-image-profile"))
        }
    }
}

fn webgpt_default_profile_root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is required for WebGPT profile defaults")?;
    Ok(PathBuf::from(home)
        .join(".hesperides")
        .join("singulari-world")
        .join("webgpt"))
}

pub(super) enum WebGptDispatchOutcome {
    Started(Box<WebGptDispatchRecord>),
    AlreadyDispatched(String),
}

#[derive(Debug, Serialize)]
pub(super) struct WebGptDispatchRecord {
    pub(super) schema_version: &'static str,
    pub(super) status: String,
    pub(super) world_id: String,
    pub(super) turn_id: String,
    pub(super) adapter_command: Option<String>,
    pub(super) mcp_wrapper: Option<String>,
    pub(super) mcp_profile_dir: Option<String>,
    pub(super) mcp_cdp_port: Option<u16>,
    pub(super) mcp_cdp_url: Option<String>,
    pub(super) conversation_id: Option<String>,
    pub(super) raw_conversation_id: Option<String>,
    pub(super) current_model: Option<String>,
    pub(super) current_reasoning_level: Option<String>,
    pub(super) pid: u32,
    pub(super) record_path: String,
    pub(super) prompt_path: String,
    pub(super) response_path: String,
    pub(super) result_path: Option<String>,
    pub(super) stdout_path: String,
    pub(super) stderr_path: String,
    pub(super) dispatched_at: String,
    pub(super) exit_code: Option<i32>,
    pub(super) committed_turn_id: Option<String>,
    pub(super) render_packet_path: Option<String>,
    pub(super) commit_record_path: Option<String>,
    pub(super) error: Option<String>,
    pub(super) completed_at: String,
}

#[derive(Debug, Serialize)]
pub(super) struct WebGptImageDispatchRecord {
    pub(super) schema_version: &'static str,
    pub(super) status: String,
    pub(super) world_id: String,
    pub(super) slot: String,
    pub(super) claim_id: Option<String>,
    pub(super) mcp_wrapper: String,
    pub(super) mcp_profile_dir: String,
    pub(super) mcp_cdp_port: u16,
    pub(super) mcp_cdp_url: String,
    pub(super) image_session_kind: String,
    pub(super) reference_paths: Vec<String>,
    pub(super) conversation_id: Option<String>,
    pub(super) raw_conversation_id: Option<String>,
    pub(super) pid: u32,
    pub(super) record_path: String,
    pub(super) prompt_path: String,
    pub(super) result_path: String,
    pub(super) stdout_path: String,
    pub(super) stderr_path: String,
    pub(super) generated_path: Option<String>,
    pub(super) generated_sha256: Option<String>,
    pub(super) generated_bytes: Option<usize>,
    pub(super) destination_path: String,
    pub(super) completion_path: Option<String>,
    pub(super) dispatched_at: String,
    pub(super) exit_code: Option<i32>,
    pub(super) error: Option<String>,
    pub(super) completed_at: String,
}

pub(super) fn dispatch_pending_agent_turn_via_webgpt(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    options: &HostWorkerOptions,
) -> Result<WebGptDispatchOutcome> {
    let dispatch_dir = dispatch_dir_for_pending(pending)?;
    fs::create_dir_all(&dispatch_dir)
        .with_context(|| format!("failed to create {}", dispatch_dir.display()))?;
    let record_path = dispatch_dir.join(format!("{}-webgpt.json", pending.turn_id));
    if record_path.exists() {
        return Ok(WebGptDispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let prompt_path = dispatch_dir.join(format!("{}-webgpt-prompt.md", pending.turn_id));
    let response_path =
        dispatch_dir.join(format!("{}-webgpt-agent-response.json", pending.turn_id));
    let result_path = dispatch_dir.join(format!("{}-webgpt-result.json", pending.turn_id));
    let stdout_path = dispatch_dir.join(format!("{}-webgpt-stdout.log", pending.turn_id));
    let stderr_path = dispatch_dir.join(format!("{}-webgpt-stderr.log", pending.turn_id));
    let prompt = build_webgpt_turn_prompt(store_root, pending)?;
    fs::write(&prompt_path, prompt.as_bytes())
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;
    let dispatcher = resolve_webgpt_dispatcher(store_root, pending, options)?;

    let paths = WebGptTurnPaths {
        world_id: pending.world_id.as_str(),
        turn_id: pending.turn_id.as_str(),
        prompt_path: prompt_path.as_path(),
        response_path: response_path.as_path(),
        result_path: result_path.as_path(),
        stdout_path: stdout_path.as_path(),
        stderr_path: stderr_path.as_path(),
    };
    let claim = webgpt_dispatch_claim(pending, &dispatcher, paths);
    if !write_dispatch_claim(record_path.as_path(), &claim)? {
        return Ok(WebGptDispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let dispatch_result = dispatcher.run(paths)?;
    if let Some(raw_conversation_id) = dispatch_result.raw_conversation_id.as_deref() {
        save_webgpt_conversation_binding(
            store_root,
            pending.world_id.as_str(),
            WebGptConversationLane::Text,
            raw_conversation_id,
        )?;
    }

    let commit_result = commit_webgpt_dispatch_if_success(
        store_root,
        pending,
        response_path.as_path(),
        &dispatch_result,
    );

    let record = WebGptDispatchRecord {
        schema_version: "singulari.webgpt_dispatch_record.v1",
        status: commit_result.status,
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        adapter_command: dispatcher.adapter_command_display(),
        mcp_wrapper: dispatcher.mcp_wrapper_display(),
        mcp_profile_dir: dispatcher.mcp_profile_dir_display(),
        mcp_cdp_port: dispatcher.mcp_cdp_port(),
        mcp_cdp_url: dispatcher.mcp_cdp_url(),
        conversation_id: dispatcher.conversation_id().map(str::to_owned),
        raw_conversation_id: dispatch_result.raw_conversation_id,
        current_model: dispatch_result.current_model,
        current_reasoning_level: dispatch_result.current_reasoning_level,
        pid: dispatch_result.pid,
        record_path: record_path.display().to_string(),
        prompt_path: prompt_path.display().to_string(),
        response_path: response_path.display().to_string(),
        result_path: Some(result_path.display().to_string()),
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        dispatched_at: Utc::now().to_rfc3339(),
        exit_code: dispatch_result.exit_code,
        committed_turn_id: commit_result
            .committed
            .as_ref()
            .map(|value| value.turn_id.clone()),
        render_packet_path: commit_result
            .committed
            .as_ref()
            .map(|value| value.render_packet_path.clone()),
        commit_record_path: commit_result
            .committed
            .as_ref()
            .map(|value| value.commit_record_path.clone()),
        error: dispatch_result.error.or(commit_result.error),
        completed_at: Utc::now().to_rfc3339(),
    };
    fs::write(&record_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", record_path.display()))?;
    Ok(WebGptDispatchOutcome::Started(Box::new(record)))
}

fn webgpt_dispatch_claim(
    pending: &singulari_world::PendingAgentTurn,
    dispatcher: &WebGptDispatcher,
    paths: WebGptTurnPaths<'_>,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "singulari.webgpt_dispatch_record.v1",
        "status": "dispatching",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "adapter_command": dispatcher.adapter_command_display(),
        "mcp_wrapper": dispatcher.mcp_wrapper_display(),
        "mcp_profile_dir": dispatcher.mcp_profile_dir_display(),
        "mcp_cdp_port": dispatcher.mcp_cdp_port(),
        "mcp_cdp_url": dispatcher.mcp_cdp_url(),
        "conversation_id": dispatcher.conversation_id(),
        "prompt_path": paths.prompt_path.display().to_string(),
        "response_path": paths.response_path.display().to_string(),
        "result_path": paths.result_path.display().to_string(),
        "stdout_path": paths.stdout_path.display().to_string(),
        "stderr_path": paths.stderr_path.display().to_string(),
        "dispatched_at": Utc::now().to_rfc3339(),
    })
}

struct WebGptCommitResult {
    status: String,
    committed: Option<singulari_world::CommittedAgentTurn>,
    error: Option<String>,
}

fn commit_webgpt_dispatch_if_success(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    response_path: &Path,
    dispatch_result: &WebGptDispatchResult,
) -> WebGptCommitResult {
    if !dispatch_result.success {
        return WebGptCommitResult {
            status: "failed".to_owned(),
            committed: None,
            error: None,
        };
    }
    match commit_webgpt_agent_response(store_root, pending, response_path) {
        Ok(committed) => WebGptCommitResult {
            status: "completed".to_owned(),
            committed: Some(committed),
            error: None,
        },
        Err(error) => WebGptCommitResult {
            status: "commit_failed".to_owned(),
            committed: None,
            error: Some(error.to_string()),
        },
    }
}

#[derive(Clone, Copy)]
struct WebGptTurnPaths<'a> {
    world_id: &'a str,
    turn_id: &'a str,
    prompt_path: &'a Path,
    response_path: &'a Path,
    result_path: &'a Path,
    stdout_path: &'a Path,
    stderr_path: &'a Path,
}

struct WebGptDispatchResult {
    success: bool,
    pid: u32,
    exit_code: Option<i32>,
    raw_conversation_id: Option<String>,
    current_model: Option<String>,
    current_reasoning_level: Option<String>,
    error: Option<String>,
}

enum WebGptDispatcher {
    ExternalCommand {
        command: PathBuf,
    },
    McpResearch {
        wrapper: PathBuf,
        runtime: WebGptLaneRuntime,
        conversation_id: Option<String>,
        model: Option<String>,
        reasoning_level: Option<String>,
        timeout_secs: u64,
    },
}

impl WebGptDispatcher {
    fn adapter_command_display(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { command } => Some(command.display().to_string()),
            Self::McpResearch { .. } => None,
        }
    }

    fn mcp_wrapper_display(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { wrapper, .. } => Some(wrapper.display().to_string()),
        }
    }

    fn mcp_profile_dir_display(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { runtime, .. } => Some(runtime.profile_dir.display().to_string()),
        }
    }

    fn mcp_cdp_port(&self) -> Option<u16> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { runtime, .. } => Some(runtime.cdp_port),
        }
    }

    fn mcp_cdp_url(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { runtime, .. } => Some(runtime.cdp_url()),
        }
    }

    fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch {
                conversation_id, ..
            } => conversation_id.as_deref(),
        }
    }

    fn run(&self, paths: WebGptTurnPaths<'_>) -> Result<WebGptDispatchResult> {
        match self {
            Self::ExternalCommand { command } => run_external_webgpt_turn_command(command, paths),
            Self::McpResearch {
                wrapper,
                runtime,
                conversation_id,
                model,
                reasoning_level,
                timeout_secs,
            } => run_webgpt_mcp_research_turn(
                wrapper,
                conversation_id.as_deref(),
                model.as_deref(),
                reasoning_level.as_deref(),
                *timeout_secs,
                runtime,
                paths,
            ),
        }
    }
}

fn resolve_webgpt_dispatcher(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    options: &HostWorkerOptions,
) -> Result<WebGptDispatcher> {
    if let Some(command) = &options.webgpt_turn_command {
        return Ok(WebGptDispatcher::ExternalCommand {
            command: command.clone(),
        });
    }
    Ok(WebGptDispatcher::McpResearch {
        wrapper: resolve_webgpt_mcp_wrapper(options)?,
        runtime: WebGptLaneRuntime::new(WebGptConversationLane::Text, options)?,
        conversation_id: load_webgpt_conversation_binding(
            store_root,
            pending.world_id.as_str(),
            WebGptConversationLane::Text,
        )?,
        model: options.webgpt_model.clone(),
        reasoning_level: options.webgpt_reasoning_level.clone(),
        timeout_secs: options.webgpt_timeout_secs,
    })
}

fn run_external_webgpt_turn_command(
    command: &Path,
    paths: WebGptTurnPaths<'_>,
) -> Result<WebGptDispatchResult> {
    let child = Command::new(command)
        .arg("--prompt-path")
        .arg(paths.prompt_path)
        .arg("--response-path")
        .arg(paths.response_path)
        .arg("--world-id")
        .arg(paths.world_id)
        .arg("--turn-id")
        .arg(paths.turn_id)
        .env("SINGULARI_WORLD_ENGINE", "webgpt")
        .env("SINGULARI_WORLD_PROMPT_PATH", paths.prompt_path)
        .env("SINGULARI_WORLD_RESPONSE_PATH", paths.response_path)
        .env("SINGULARI_WORLD_WORLD_ID", paths.world_id)
        .env("SINGULARI_WORLD_TURN_ID", paths.turn_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn webgpt turn adapter: command={}",
                command.display()
            )
        })?;
    let pid = child.id();
    let output = child
        .wait_with_output()
        .context("failed to wait for webgpt turn adapter")?;
    fs::write(paths.stdout_path, &output.stdout)
        .with_context(|| format!("failed to write {}", paths.stdout_path.display()))?;
    fs::write(paths.stderr_path, &output.stderr)
        .with_context(|| format!("failed to write {}", paths.stderr_path.display()))?;
    Ok(WebGptDispatchResult {
        success: output.status.success(),
        pid,
        exit_code: output.status.code(),
        raw_conversation_id: None,
        current_model: None,
        current_reasoning_level: None,
        error: if output.status.success() {
            None
        } else {
            Some(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                .filter(|value| !value.is_empty())
        },
    })
}

fn run_webgpt_mcp_research_turn(
    wrapper: &Path,
    conversation_id: Option<&str>,
    model: Option<&str>,
    reasoning_level: Option<&str>,
    timeout_secs: u64,
    runtime: &WebGptLaneRuntime,
    paths: WebGptTurnPaths<'_>,
) -> Result<WebGptDispatchResult> {
    let prompt = fs::read_to_string(paths.prompt_path)
        .with_context(|| format!("failed to read {}", paths.prompt_path.display()))?;
    let arguments = build_webgpt_research_arguments(
        prompt.as_str(),
        conversation_id,
        model,
        reasoning_level,
        timeout_secs,
    );
    let arguments_raw = serde_json::to_string(&arguments)?;
    let mut command = Command::new(wrapper);
    runtime.apply_to_command(&mut command);
    let child = command
            .arg("client-call")
            .arg("--wrapper")
            .arg(wrapper)
            .arg("--client-name")
            .arg("singulari-world-webgpt-turn")
            .arg("--require-tool")
            .arg("--tool")
            .arg("webgpt_research")
            .arg("--arguments")
            .arg(arguments_raw)
            .arg("--output")
            .arg("first-text")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to spawn webgpt-mcp client-call: wrapper={}, lane={}, cdp_url={}, profile_dir={}",
                    wrapper.display(),
                    runtime.lane.as_str(),
                    runtime.cdp_url(),
                    runtime.profile_dir.display()
                )
            })?;
    let pid = child.id();
    let output = child
        .wait_with_output()
        .context("failed to wait for webgpt-mcp client-call")?;
    fs::write(paths.stdout_path, &output.stdout)
        .with_context(|| format!("failed to write {}", paths.stdout_path.display()))?;
    fs::write(paths.stderr_path, &output.stderr)
        .with_context(|| format!("failed to write {}", paths.stderr_path.display()))?;
    if !output.status.success() {
        return Ok(WebGptDispatchResult {
            success: false,
            pid,
            exit_code: output.status.code(),
            raw_conversation_id: None,
            current_model: None,
            current_reasoning_level: None,
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                .filter(|value| !value.is_empty()),
        });
    }
    let raw_result = String::from_utf8(output.stdout)
        .context("webgpt-mcp client-call stdout was not valid UTF-8")?;
    fs::write(paths.result_path, raw_result.as_bytes())
        .with_context(|| format!("failed to write {}", paths.result_path.display()))?;
    let result = serde_json::from_str::<serde_json::Value>(&raw_result)
        .context("failed to parse webgpt_research result JSON")?;
    let answer_markdown = result
        .get("answer_markdown")
        .and_then(serde_json::Value::as_str)
        .context("webgpt_research result missing answer_markdown")?;
    let agent_response_json = extract_json_object_text(answer_markdown)
        .context("webgpt answer did not contain an AgentTurnResponse JSON object")?;
    fs::write(paths.response_path, agent_response_json.as_bytes())
        .with_context(|| format!("failed to write {}", paths.response_path.display()))?;
    Ok(WebGptDispatchResult {
        success: true,
        pid,
        exit_code: output.status.code(),
        raw_conversation_id: result
            .get("raw_conversation_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        current_model: result
            .get("current_model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        current_reasoning_level: result
            .get("current_reasoning_level")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        error: None,
    })
}

fn build_webgpt_research_arguments(
    prompt: &str,
    conversation_id: Option<&str>,
    model: Option<&str>,
    reasoning_level: Option<&str>,
    timeout_secs: u64,
) -> serde_json::Value {
    let mut arguments = serde_json::json!({
        "prompt": prompt,
        "timeout_secs": timeout_secs.max(60),
        "auto_recover": true,
        "recovery_attempts": 1,
    });
    if let Some(object) = arguments.as_object_mut() {
        if let Some(conversation_id) = conversation_id.filter(|value| !value.trim().is_empty()) {
            object.insert(
                "conversation_id".to_owned(),
                serde_json::json!(conversation_id),
            );
        } else {
            object.insert("new_conversation".to_owned(), serde_json::json!(true));
        }
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            object.insert("model".to_owned(), serde_json::json!(model));
        }
        if let Some(reasoning_level) = reasoning_level.filter(|value| !value.trim().is_empty()) {
            object.insert(
                "reasoning_level".to_owned(),
                serde_json::json!(reasoning_level),
            );
        }
    }
    arguments
}

fn resolve_webgpt_mcp_wrapper(options: &HostWorkerOptions) -> Result<PathBuf> {
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    let project_root = find_project_root_from(cwd.as_path())?.unwrap_or(cwd.clone());
    if let Some(wrapper) = &options.webgpt_mcp_wrapper {
        return validate_standalone_webgpt_mcp_wrapper(
            wrapper,
            cwd.as_path(),
            project_root.as_path(),
        );
    }
    if let Some(wrapper) = std::env::var_os("SINGULARI_WORLD_WEBGPT_MCP_WRAPPER").map(PathBuf::from)
    {
        return validate_standalone_webgpt_mcp_wrapper(
            wrapper.as_path(),
            cwd.as_path(),
            project_root.as_path(),
        );
    }
    if let Some(wrapper) = local_env_value("SINGULARI_WORLD_WEBGPT_MCP_WRAPPER")?.map(PathBuf::from)
    {
        return validate_standalone_webgpt_mcp_wrapper(
            wrapper.as_path(),
            cwd.as_path(),
            project_root.as_path(),
        );
    }
    if let Some(wrapper) = find_bundled_webgpt_mcp_wrapper_from(cwd.as_path())? {
        return Ok(wrapper);
    }
    anyhow::bail!(
        "webgpt backend requires this repository's bundled webgpt-mcp-checkout/scripts/webgpt-mcp.sh, --webgpt-mcp-wrapper, or SINGULARI_WORLD_WEBGPT_MCP_WRAPPER in env/.env; run scripts/setup-webgpt-runtime.sh on a fresh clone"
    );
}

fn local_env_value(name: &str) -> Result<Option<String>> {
    let path = std::env::current_dir()
        .context("failed to resolve current working directory")?
        .join(".env");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path.as_path())
        .with_context(|| format!("failed to read {}", path.display()))?;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != name {
            continue;
        }
        let value = unquote_local_env_value(value.trim());
        ensure_control_safe_runtime_value(name, value.as_str())?;
        if !value.is_empty() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn unquote_local_env_value(value: &str) -> String {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn validate_standalone_webgpt_mcp_wrapper(
    wrapper: &Path,
    cwd: &Path,
    project_root: &Path,
) -> Result<PathBuf> {
    let wrapper = if wrapper.is_absolute() {
        wrapper.to_path_buf()
    } else {
        cwd.join(wrapper)
    };
    if !wrapper.is_file() {
        anyhow::bail!("WebGPT MCP wrapper does not exist: {}", wrapper.display());
    }
    let wrapper = wrapper
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", wrapper.display()))?;
    let project_root = project_root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", project_root.display()))?;
    if !wrapper.starts_with(project_root.as_path()) {
        anyhow::bail!(
            "WebGPT MCP wrapper must live inside this singulari-world repository: wrapper={}, repo={}",
            wrapper.display(),
            project_root.display()
        );
    }
    Ok(wrapper)
}

fn find_project_root_from(start: &Path) -> Result<Option<PathBuf>> {
    let mut dir = start.to_path_buf();
    loop {
        if dir.join("Cargo.toml").is_file() && dir.join("src/main.rs").is_file() {
            return Ok(Some(dir));
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

fn find_bundled_webgpt_mcp_wrapper_from(start: &Path) -> Result<Option<PathBuf>> {
    let Some(project_root) = find_project_root_from(start)? else {
        return Ok(None);
    };
    let direct = project_root.join("webgpt-mcp-checkout/scripts/webgpt-mcp.sh");
    if direct.is_file() {
        return Ok(Some(direct));
    }
    Ok(None)
}

#[derive(Debug, Clone, Copy)]
enum WebGptConversationLane {
    Text,
    Image,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(super) enum WebGptImageSessionKind {
    TurnCg,
    ReferenceAsset,
}

impl WebGptImageSessionKind {
    pub(super) fn from_slot(slot: &str) -> Self {
        if slot.starts_with("turn_cg:") {
            Self::TurnCg
        } else {
            Self::ReferenceAsset
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::TurnCg => "turn_cg",
            Self::ReferenceAsset => "reference_asset",
        }
    }

    const fn binding_filename(self) -> &'static str {
        match self {
            Self::TurnCg => "webgpt_image_conversation_binding.json",
            Self::ReferenceAsset => "webgpt_reference_asset_conversation_binding.json",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::TurnCg => "webgpt_mcp_image_generation_turn_cg",
            Self::ReferenceAsset => "webgpt_mcp_image_generation_reference_asset",
        }
    }
}

impl WebGptConversationLane {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    const fn binding_filename(self) -> &'static str {
        match self {
            Self::Text => "webgpt_conversation_binding.json",
            Self::Image => "webgpt_image_conversation_binding.json",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Text => "webgpt_mcp_research",
            Self::Image => "webgpt_mcp_image_generation",
        }
    }

    const fn profile_dir_name(self) -> &'static str {
        match self {
            Self::Text => "text-profile",
            Self::Image => "image-profile",
        }
    }
}

fn webgpt_conversation_url(conversation_id: &str) -> String {
    format!("https://chatgpt.com/c/{conversation_id}")
}

fn webgpt_conversation_binding_path(
    store_root: Option<&Path>,
    world_id: &str,
    lane: WebGptConversationLane,
) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths
        .root
        .join("worlds")
        .join(world_id)
        .join("agent_bridge")
        .join(lane.binding_filename()))
}

fn load_webgpt_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    lane: WebGptConversationLane,
) -> Result<Option<String>> {
    let path = webgpt_conversation_binding_path(store_root, world_id, lane)?;
    if !path.exists() {
        return Ok(None);
    }
    let value = read_json_value_if_present(path.as_path())?
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(value
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty()))
}

fn webgpt_image_conversation_binding_path(
    store_root: Option<&Path>,
    world_id: &str,
    session_kind: WebGptImageSessionKind,
) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths
        .root
        .join("worlds")
        .join(world_id)
        .join("agent_bridge")
        .join(session_kind.binding_filename()))
}

fn load_webgpt_image_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    session_kind: WebGptImageSessionKind,
) -> Result<Option<String>> {
    let path = webgpt_image_conversation_binding_path(store_root, world_id, session_kind)?;
    if !path.exists() {
        return Ok(None);
    }
    let value = read_json_value_if_present(path.as_path())?
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(value
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty()))
}

fn save_webgpt_image_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    session_kind: WebGptImageSessionKind,
    conversation_id: &str,
) -> Result<()> {
    let path = webgpt_image_conversation_binding_path(store_root, world_id, session_kind)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "singulari.webgpt_image_conversation_binding.v1",
            "world_id": world_id,
            "lane": WebGptConversationLane::Image.as_str(),
            "image_session_kind": session_kind.as_str(),
            "conversation_id": conversation_id,
            "conversation_url": webgpt_conversation_url(conversation_id),
            "source": session_kind.source(),
            "updated_at": Utc::now().to_rfc3339(),
        }))?,
    )
    .with_context(|| format!("failed to write {}", path.display()))
}

fn save_webgpt_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    lane: WebGptConversationLane,
    conversation_id: &str,
) -> Result<()> {
    let path = webgpt_conversation_binding_path(store_root, world_id, lane)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "singulari.webgpt_conversation_binding.v1",
            "world_id": world_id,
            "lane": lane.as_str(),
            "conversation_id": conversation_id,
            "conversation_url": webgpt_conversation_url(conversation_id),
            "source": lane.source(),
            "updated_at": Utc::now().to_rfc3339(),
        }))?,
    )
    .with_context(|| format!("failed to write {}", path.display()))
}

fn extract_json_object_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok_and(|value| value.is_object()) {
        return Some(trimmed.to_owned());
    }
    if let Some(fenced) = extract_fenced_json_text(trimmed) {
        return Some(fenced);
    }
    extract_first_balanced_json_object(trimmed)
}

fn extract_fenced_json_text(raw: &str) -> Option<String> {
    let fence_start = raw.find("```")?;
    let after_start = &raw[fence_start + 3..];
    let after_header = after_start
        .find('\n')
        .map_or(after_start, |index| &after_start[index + 1..]);
    let fence_end = after_header.find("```")?;
    let candidate = after_header[..fence_end].trim();
    if serde_json::from_str::<serde_json::Value>(candidate).is_ok_and(|value| value.is_object()) {
        Some(candidate.to_owned())
    } else {
        None
    }
}

fn extract_first_balanced_json_object(raw: &str) -> Option<String> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let candidate = raw[start?..=index].trim();
                    if serde_json::from_str::<serde_json::Value>(candidate)
                        .is_ok_and(|value| value.is_object())
                    {
                        return Some(candidate.to_owned());
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}

fn commit_webgpt_agent_response(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    response_path: &Path,
) -> Result<singulari_world::CommittedAgentTurn> {
    let raw_body = fs::read_to_string(response_path)
        .with_context(|| format!("failed to read {}", response_path.display()))?;
    let response = serde_json::from_str::<AgentTurnResponse>(&raw_body)
        .with_context(|| format!("failed to parse {}", response_path.display()))?;
    commit_agent_turn(&AgentCommitTurnOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id: pending.world_id.clone(),
        response,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "WebGPT image dispatch keeps MCP image generation, extraction, and visual-job completion in one durable record"
)]
pub(super) fn dispatch_visual_job_via_webgpt(
    store_root: Option<&Path>,
    claim: &singulari_world::VisualJobClaim,
    options: &HostWorkerOptions,
) -> Result<WebGptImageDispatchRecord> {
    let dispatch_dir = visual_dispatch_dir_for_world(store_root, claim.world_id.as_str())?;
    fs::create_dir_all(&dispatch_dir)
        .with_context(|| format!("failed to create {}", dispatch_dir.display()))?;
    let slot_component = safe_file_component(claim.slot.as_str());
    let claim_component = safe_file_component(claim.claim_id.as_str());
    let record_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image.json"
    ));
    let prompt_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-prompt.md"
    ));
    let result_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-result.json"
    ));
    let stdout_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-stdout.log"
    ));
    let stderr_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-stderr.log"
    ));
    let image_session_kind = WebGptImageSessionKind::from_slot(claim.slot.as_str());
    ensure_image_job_matches_session_kind(&claim.job, image_session_kind)?;
    let conversation_id = load_webgpt_image_conversation_binding(
        store_root,
        claim.world_id.as_str(),
        image_session_kind,
    )?;
    let prompt = build_webgpt_image_generation_prompt(
        claim.world_id.as_str(),
        &claim.job,
        conversation_id.as_deref(),
        image_session_kind,
    );
    let reference_paths = webgpt_image_reference_paths(&claim.job)?;
    fs::write(&prompt_path, prompt.as_bytes())
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;
    let wrapper = resolve_webgpt_mcp_wrapper(options)?;
    let runtime = WebGptLaneRuntime::new_image(image_session_kind, options)?;

    let dispatched_at = Utc::now().to_rfc3339();
    let claim_record = serde_json::json!({
        "schema_version": "singulari.webgpt_image_dispatch_record.v1",
        "status": "dispatching",
        "world_id": claim.world_id.as_str(),
        "slot": claim.slot.as_str(),
        "claim_id": claim.claim_id.as_str(),
        "mcp_wrapper": wrapper.display().to_string(),
        "mcp_profile_dir": runtime.profile_dir.display().to_string(),
        "mcp_cdp_port": runtime.cdp_port,
        "mcp_cdp_url": runtime.cdp_url(),
        "image_session_kind": image_session_kind.as_str(),
        "reference_paths": reference_paths.as_slice(),
        "conversation_id": conversation_id.as_deref(),
        "prompt_path": prompt_path.display().to_string(),
        "result_path": result_path.display().to_string(),
        "stdout_path": stdout_path.display().to_string(),
        "stderr_path": stderr_path.display().to_string(),
        "destination_path": claim.job.destination_path.as_str(),
        "dispatched_at": dispatched_at.as_str(),
    });
    if !write_dispatch_claim(record_path.as_path(), &claim_record)? {
        anyhow::bail!(
            "webgpt image dispatch already exists: record_path={}",
            record_path.display()
        );
    }

    let child = spawn_webgpt_image_generation(
        wrapper.as_path(),
        &runtime,
        conversation_id.as_deref(),
        reference_paths.as_slice(),
        prompt.as_str(),
        options.webgpt_timeout_secs,
    )?;
    let pid = child.id();
    let output = child
        .wait_with_output()
        .context("failed to wait for webgpt image generation")?;
    fs::write(stdout_path.as_path(), &output.stdout)
        .with_context(|| format!("failed to write {}", stdout_path.display()))?;
    fs::write(stderr_path.as_path(), &output.stderr)
        .with_context(|| format!("failed to write {}", stderr_path.display()))?;
    let mut destination_path = claim.job.destination_path.clone();
    let mut completion_path = None;
    let mut generated_path = None;
    let mut generated_sha256 = None;
    let mut generated_bytes = None;
    let mut raw_conversation_id = None;
    let mut error = if output.status.success() {
        None
    } else {
        Some(String::from_utf8_lossy(&output.stderr).trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                Some(format!(
                    "webgpt image generation exited with {}",
                    output.status
                ))
            })
    };

    if error.is_none() {
        let raw_result = String::from_utf8(output.stdout.clone())
            .context("webgpt image generation stdout was not valid UTF-8")?;
        fs::write(result_path.as_path(), raw_result.as_bytes())
            .with_context(|| format!("failed to write {}", result_path.display()))?;
        let result = serde_json::from_str::<serde_json::Value>(&raw_result)
            .context("failed to parse webgpt_generate_image result JSON")?;
        raw_conversation_id = result
            .get("conversation_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        if let Some(raw_conversation_id) = raw_conversation_id.as_deref() {
            save_webgpt_image_conversation_binding(
                store_root,
                claim.world_id.as_str(),
                image_session_kind,
                raw_conversation_id,
            )?;
        }
        match first_webgpt_generated_image(&result) {
            Some(image) => {
                generated_path = image
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                generated_sha256 = image
                    .get("sha256")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                generated_bytes = image
                    .get("byte_len")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok());
            }
            None => {
                error = Some("webgpt_generate_image returned no images".to_owned());
            }
        }
    }

    if error.is_none() {
        match generated_path.as_deref().map(PathBuf::from) {
            Some(path) => match complete_visual_job(&CompleteVisualJobOptions {
                store_root: store_root.map(Path::to_path_buf),
                world_id: claim.world_id.clone(),
                slot: claim.slot.clone(),
                claim_id: Some(claim.claim_id.clone()),
                generated_path: Some(path),
            }) {
                Ok(completion) => {
                    destination_path = completion.destination_path;
                    completion_path = Some(completion.completion_path);
                }
                Err(complete_error) => {
                    error = Some(complete_error.to_string());
                }
            },
            None => {
                error = Some("webgpt_generate_image returned image without path".to_owned());
            }
        }
    }

    let status = if error.is_none() {
        "completed"
    } else {
        "failed"
    }
    .to_owned();
    let record = WebGptImageDispatchRecord {
        schema_version: "singulari.webgpt_image_dispatch_record.v1",
        status,
        world_id: claim.world_id.clone(),
        slot: claim.slot.clone(),
        claim_id: Some(claim.claim_id.clone()),
        mcp_wrapper: wrapper.display().to_string(),
        mcp_profile_dir: runtime.profile_dir.display().to_string(),
        mcp_cdp_port: runtime.cdp_port,
        mcp_cdp_url: runtime.cdp_url(),
        image_session_kind: image_session_kind.as_str().to_owned(),
        reference_paths,
        conversation_id,
        raw_conversation_id,
        pid,
        record_path: record_path.display().to_string(),
        prompt_path: prompt_path.display().to_string(),
        result_path: result_path.display().to_string(),
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        generated_path,
        generated_sha256,
        generated_bytes,
        destination_path,
        completion_path,
        dispatched_at,
        exit_code: output.status.code(),
        error,
        completed_at: Utc::now().to_rfc3339(),
    };
    fs::write(record_path.as_path(), serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", record_path.display()))?;
    if let Some(error) = &record.error {
        anyhow::bail!("{error}");
    }
    Ok(record)
}

fn ensure_image_job_matches_session_kind(
    job: &ImageGenerationJob,
    image_session_kind: WebGptImageSessionKind,
) -> Result<()> {
    let valid = match image_session_kind {
        WebGptImageSessionKind::TurnCg => {
            job.artifact_kind == VisualArtifactKind::SceneCg
                && job.display_allowed
                && !job.reference_allowed
                && job.canonical_use == job.artifact_kind.canonical_use()
        }
        WebGptImageSessionKind::ReferenceAsset => {
            let design_reference = matches!(
                job.artifact_kind,
                VisualArtifactKind::CharacterDesignSheet | VisualArtifactKind::LocationDesignSheet
            ) && !job.display_allowed
                && job.reference_allowed
                && job.canonical_use == job.artifact_kind.canonical_use();
            let ui_background = job.artifact_kind == VisualArtifactKind::UiBackground
                && job.display_allowed
                && !job.reference_allowed
                && job.canonical_use == job.artifact_kind.canonical_use();
            design_reference || ui_background
        }
    };
    if valid {
        return Ok(());
    }
    anyhow::bail!(
        "webgpt image job kind/session mismatch: slot={}, artifact_kind={:?}, canonical_use={}, display_allowed={}, reference_allowed={}, image_session_kind={}",
        job.slot,
        job.artifact_kind,
        job.canonical_use,
        job.display_allowed,
        job.reference_allowed,
        image_session_kind.as_str()
    )
}

fn webgpt_image_reference_paths(job: &ImageGenerationJob) -> Result<Vec<String>> {
    job.reference_paths
        .iter()
        .map(|raw_path| {
            let path = PathBuf::from(raw_path);
            if !path.is_file() {
                anyhow::bail!(
                    "webgpt image reference asset missing: slot={}, path={}",
                    job.slot,
                    path.display()
                );
            }
            path.canonicalize()
                .with_context(|| {
                    format!(
                        "failed to canonicalize webgpt image reference asset: slot={}, path={}",
                        job.slot,
                        path.display()
                    )
                })
                .map(|path| path.display().to_string())
        })
        .collect()
}

fn spawn_webgpt_image_generation(
    wrapper: &Path,
    runtime: &WebGptLaneRuntime,
    conversation_id: Option<&str>,
    reference_paths: &[String],
    prompt: &str,
    timeout_secs: u64,
) -> Result<Child> {
    let mut arguments = serde_json::json!({
        "prompt": prompt,
        "max_images": 1,
        "timeout_secs": timeout_secs.max(60),
        "auto_recover": true,
        "recovery_attempts": 1,
    });
    if let Some(object) = arguments.as_object_mut()
        && let Some(conversation_id) = conversation_id.filter(|value| !value.trim().is_empty())
    {
        object.insert(
            "conversation_id".to_owned(),
            serde_json::json!(conversation_id),
        );
    }
    if let Some(object) = arguments.as_object_mut()
        && !reference_paths.is_empty()
    {
        object.insert(
            "reference_paths".to_owned(),
            serde_json::json!(reference_paths),
        );
    }
    let mut command = Command::new(wrapper);
    runtime.apply_to_command(&mut command);
    command
        .arg("client-call")
        .arg("--wrapper")
        .arg(wrapper)
        .arg("--client-name")
        .arg("singulari-world-webgpt-image")
        .arg("--require-tool")
        .arg("--tool")
        .arg("webgpt_generate_image")
        .arg("--arguments")
        .arg(serde_json::to_string(&arguments)?)
        .arg("--output")
        .arg("first-text")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn webgpt image generation: wrapper={}, lane={}, cdp_url={}, profile_dir={}",
                wrapper.display(),
                runtime.lane.as_str(),
                runtime.cdp_url(),
                runtime.profile_dir.display()
            )
        })
}

fn first_webgpt_generated_image(result: &serde_json::Value) -> Option<&serde_json::Value> {
    result
        .get("images")
        .and_then(serde_json::Value::as_array)
        .and_then(|images| images.first())
}

fn build_webgpt_image_generation_prompt(
    world_id: &str,
    job: &ImageGenerationJob,
    conversation_id: Option<&str>,
    session_kind: WebGptImageSessionKind,
) -> String {
    let mut prompt = String::new();
    match session_kind {
        WebGptImageSessionKind::TurnCg => {
            prompt.push_str(
                "Generate exactly one full-screen visual novel scene CG for Singulari World.\n",
            );
            prompt.push_str(
                "This ChatGPT conversation is the dedicated world-scoped turn-CG session for ",
            );
            prompt.push_str(world_id);
            prompt.push_str(
                ". Reuse this same session URL only for scene-CG continuity across this world.\n",
            );
        }
        WebGptImageSessionKind::ReferenceAsset => {
            prompt.push_str("Generate exactly one reference asset image for Singulari World.\n");
            prompt.push_str("This ChatGPT conversation is the dedicated world-scoped reference-asset session for ");
            prompt.push_str(world_id);
            prompt.push_str(". Reuse this same session URL only for source-material continuity, not scene-CG continuity.\n");
        }
    }
    if let Some(conversation_id) = conversation_id {
        prompt.push_str("Current image session URL: ");
        prompt.push_str(webgpt_conversation_url(conversation_id).as_str());
        prompt.push('\n');
    }
    match session_kind {
        WebGptImageSessionKind::TurnCg => {
            prompt.push_str("Treat previous turn-CG images in this same conversation as continuity references for palette, line quality, camera language, and recurring setting motifs.\n");
            prompt.push_str("Reference assets named below are source material only; use them as continuity references, but never render a character design sheet, contact sheet, asset board, or UI resource as the scene itself.\n");
        }
        WebGptImageSessionKind::ReferenceAsset => {
            prompt.push_str("Reference assets are source material only. The resulting image must be saved to its requested asset path and must never be treated as or displayed as a turn scene CG.\n");
            prompt.push_str("Do not use turn-CG conversation history or previous scene CGs as source instructions unless they are explicitly listed below.\n");
        }
    }
    prompt.push_str("Return no prose unless ChatGPT requires a short title. Do not make a collage, grid, contact sheet, or variants.\n");
    prompt.push_str("Image job slot: ");
    prompt.push_str(job.slot.as_str());
    prompt.push_str("\nArtifact kind: ");
    prompt.push_str(job.artifact_kind.as_str());
    prompt.push_str("\nCanonical use: ");
    prompt.push_str(job.canonical_use.as_str());
    prompt.push_str("\nDestination path: ");
    prompt.push_str(job.destination_path.as_str());
    prompt.push('\n');
    prompt.push_str("Use the image prompt below as the sole visual brief.\n\n");
    prompt.push_str(job.prompt.as_str());
    if !job.reference_paths.is_empty() {
        prompt.push_str("\n\nReference continuity notes: ");
        prompt.push_str(job.reference_paths.join(", ").as_str());
    }
    prompt
}

pub(super) fn visual_dispatch_dir_for_world(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths
        .worlds_dir
        .join(world_id)
        .join("visual_jobs")
        .join("dispatches"))
}

fn read_json_value_if_present(path: &Path) -> Result<Option<serde_json::Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value = serde_json::from_str(raw.as_str())
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(value))
}

fn dispatch_dir_for_pending(pending: &singulari_world::PendingAgentTurn) -> Result<PathBuf> {
    let pending_ref = Path::new(pending.pending_ref.as_str());
    let Some(agent_bridge_dir) = pending_ref.parent() else {
        anyhow::bail!(
            "pending turn has invalid pending_ref: world_id={}, turn_id={}, pending_ref={}",
            pending.world_id,
            pending.turn_id,
            pending.pending_ref
        );
    };
    Ok(agent_bridge_dir.join("dispatches"))
}

fn write_dispatch_claim(path: &Path, claim: &serde_json::Value) -> Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(serde_json::to_vec_pretty(claim)?.as_slice())?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).with_context(|| format!("failed to create {}", path.display())),
    }
}

const AGENT_TURN_RESPONSE_SCHEMA_GUIDE: &str = r#"AgentTurnResponse 스키마:
```json
{
  "schema_version": "singulari.agent_turn_response.v1",
  "world_id": "<world_id>",
  "turn_id": "<turn_id>",
  "visible_scene": {
    "schema_version": "singulari.narrative_scene.v1",
    "text_blocks": ["위 서사 출력 지시와 pending.output_contract.narrative_budget에 맞춘 한국어 VN 본문"],
    "tone_notes": ["짧은 톤 메모"]
  },
  "adjudication": {
    "outcome": "accepted",
    "summary": "플레이어-visible 한 줄 요약",
    "gates": [
      {"gate":"body","status":"pass","reason":"..."},
      {"gate":"resource","status":"pass","reason":"..."},
      {"gate":"time","status":"pass","reason":"..."},
      {"gate":"social_permission","status":"pass","reason":"..."},
      {"gate":"knowledge","status":"pass","reason":"..."}
    ],
    "visible_constraints": ["아직 확인되지 않은 플레이어-visible 제약"],
    "consequences": ["이번 턴의 플레이어-visible 결과"]
  },
  "canon_event": {
    "visibility": "player_visible",
    "kind": "guided_choice",
    "summary": "플레이어-visible 사건 요약"
  },
  "entity_updates": [
    {
      "entity_id": "char_or_place_or_item_id",
      "update_kind": "seen_action",
      "visibility": "player_visible",
      "summary": "이번 턴에서 player-visible entity state가 어떻게 변했는지",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "relationship_updates": [
    {
      "source_entity_id": "char:a",
      "target_entity_id": "char:b",
      "relation_kind": "suspicion|trust|debt|fear|distance",
      "visibility": "player_visible",
      "summary": "이번 턴에서 대사 거리감/협조/의심에 영향을 주는 관계 변화",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "plot_thread_events": [
    {
      "thread_id": "prompt_context.visible_context.active_plot_threads 중 이번 턴에서 실제로 변한 thread_id",
      "change": "advanced",
      "status_after": "active",
      "urgency_after": "soon",
      "summary": "이번 visible_scene에서 이 thread가 어떻게 변했는지 한 문장",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "scene_pressure_events": [
    {
      "pressure_id": "prompt_context.visible_context.active_scene_pressure 중 이번 턴에서 실제로 변한 pressure_id",
      "change": "increased",
      "intensity_after": 3,
      "urgency_after": "soon",
      "summary": "이번 visible_scene에서 압력이 어떻게 변했는지",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "world_lore_updates": [
    {
      "subject": "player-visible subject",
      "predicate": "is|has|requires|forbids",
      "object": "이번 턴에서 확정된 세계 규칙/관습/장소 사실",
      "category": "customs|geography|social_order|danger_model|language_register",
      "visibility": "player_visible",
      "summary": "세계관에 남길 player-visible 사실",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "character_text_design_updates": [
    {
      "character_id": "char:id",
      "speech_pattern": "화법/어미/말버릇",
      "gesture_pattern": "습관적 제스처",
      "drift_note": "이번 턴 이후 조정할 말맛",
      "visibility": "player_visible",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "body_resource_events": [
    {
      "event_kind": "resource_gained",
      "target_id": "resource:scene:item",
      "visibility": "player_visible",
      "summary": "이번 턴에서 얻거나 잃은 visible body/resource 변화",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "location_events": [
    {
      "event_kind": "discovered",
      "location_id": "place:id",
      "name": "플레이어-visible 장소 이름",
      "knowledge_state": "known",
      "summary": "이번 턴에서 열린 장소/동선 변화",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "extra_contacts": [],
  "hidden_state_delta": [
    {
      "delta_kind": "secret_status",
      "target_id": "secret_or_timer_id",
      "summary": "판정 전용 hidden delta. visible text에 복사 금지",
      "evidence_refs": ["private_adjudication_context"]
    }
  ],
  "needs_context": [],
  "next_choices": [
    {"slot":1,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"현재 장면 단서와 player_input에서 이어지는 구체 행동"},
    {"slot":2,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"몸, 장소, 물건, 흔적 중 이번 장면에 실제로 나온 단서를 살핀다"},
    {"slot":3,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"이번 장면에 실제로 있는 인물, 기척, 관계 신호에 반응한다"},
    {"slot":4,"tag":"현재 장면에 맞춘 기록 선택명","intent":"이번 장면에서 드러난 기록/단서/세계 지식을 확인한다"},
    {"slot":5,"tag":"현재 장면에 맞춘 흐름 선택명","intent":"이번 사건의 변화 압력이 다음 행동에 어떤 영향을 주는지 본다"},
    {"slot":6,"tag":"자유서술","intent":"플레이어가 원하는 행동과 말, 내면 독백을 직접 서술한다"},
    {"slot":7,"tag":"판단 위임","intent":"맡긴다. 세부 내용은 선택 후 드러난다."}
  ]
}
```
- next_choices는 서사 생성과 같은 응답에서 반드시 함께 작성한다. 별도 선택지 재생성 턴을 만들지 않는다.
- slot 1,2,3,4,5의 tag/intent는 템플릿 문구가 아니라 이번 visible_scene에서 바로 이어지는 구체 선택지여야 한다.
- next_choices 안에는 label/preview/choices 필드를 쓰지 않는다. 오직 slot/tag/intent만 쓴다.
- plot_thread_events는 이번 턴에서 실제로 진행/복잡화/차단/해결/실패/퇴장한 active_visible thread만 적는다. 변화가 없으면 빈 배열이다.
- plot_thread_events.thread_id는 prompt_context.visible_context.active_plot_threads에 있는 thread_id만 쓴다. 새 thread를 임의로 만들거나 hidden/dormant 상태로 바꾸지 않는다.
- scene_pressure_events는 이번 턴에서 실제로 강해지거나 약해진 visible_active pressure만 적는다. hidden_adjudication_only pressure_id는 절대 쓰지 않는다.
- entity_updates/relationship_updates/world_lore_updates/character_text_design_updates는 전부 typed schema다. 변화가 없으면 빈 배열이며, 임의 key/value JSON을 넣지 않는다.
- body_resource_events/location_events도 typed schema다. 장면에 실제 증거가 없으면 빈 배열로 둔다.
- needs_context는 기본적으로 빈 배열이다. 현재 prompt_context.selected_context_capsules에 없는 맥락 없이는 응답을 닫을 수 없을 때만 request_id/capsule_kinds/query/reason/evidence_refs를 넣는다. needs_context가 비어 있지 않으면 host는 그 응답을 커밋하지 않는다.
- slot 번호가 기능 계약이다. tag는 UI 문구이므로 장면에 맞게 짧게 바꿔도 된다. 단 slot 7 tag는 "판단 위임"으로 유지한다.
- extra_contacts는 주변 인물이 플레이어와 직접 상호작용했거나, 의미 있는 목격/거래/도움/위협/감정 흔적을 남겼을 때만 쓴다.
- extra_contacts 항목을 쓸 때는 surface_label, contact_summary를 반드시 실제 장면 내용으로 채운다. 스키마 설명 문구나 예시 문구를 값으로 복사하지 않는다.
- 단순 배경 군중은 extra_contacts에 넣지 않는다. 한 번 스쳐간 인물은 memory_action "trace", 다시 떠올릴 이유가 분명하면 "remember"를 쓴다."#;

const SEEDLESS_PROSE_CONTRACT: &str = r"- 이 계약은 seedless style contract다. 여기 있는 문체/작법 규칙은 소재, 사건, 인물, 장소, 장르 장치, 과거사, 상징을 새로 만들 권한이 없다.
- scene_fact_boundaries: 오직 prompt context packet의 player-visible facts, current player_input, visible canon, selected memory items에서 허용된 사실만 쓴다. style contract, schema examples, previous WebGPT phrasing, UI labels는 장면 사실이 아니다.
- 캐릭터 voice_anchors는 캐릭터 텍스트 디자인이다. speech는 화법, endings는 어미/말끝, tone은 어투/거리감/어휘, gestures는 반복 제스처, habits는 행동 습관, drift는 변화 방향으로 적용한다.
- 문체와 서사 작법은 캐릭터에 귀속하지 말고 visible_scene의 전역 서사에만 적용한다. 문단 순서는 장면 압력과 player_input에 맞춰 달라져야 하며, 고정된 전개 템플릿을 반복하지 않는다.
- paragraph_grammar: 각 문단은 감각 변화, 몸의 반응, 외부 압력, 해석을 유보한 단서, 다음 행동을 압박하는 변화 중 최소 둘을 포함한다.
- 시작 문단은 배경 설명이 아니라 현재 장면에서 감각적으로 바뀐 것과 visible constraint를 연다.
- 상호작용 문단은 말 한 줄, 작은 몸짓, 끊긴 반응, 침묵, 거리 변화 중 하나를 중심으로 둔다. 대사 뒤에는 설명 대신 행동이나 사물 반응을 붙인다.
- 마감 문단은 요약이나 교훈으로 닫지 말고, 바로 다음 선택을 압박하는 미해결 상태로 닫는다.
- dialogue_contract: 대사는 설명문이 아니다. 인물이 자기 상태, 세계 규칙, 선택지 의도를 길게 해설하지 않게 하고, 말은 짧은 호흡·망설임·생략·상대와의 거리감으로 구분한다.
- style_vector: sentence_pressure=high, exposition_directness=low, sensory_density=medium_high, dialogue_explanation=low, paragraph_closure=unresolved, metaphor_density=low_medium, interior_monologue=restrained, scene_continuity=strict.
- anti_translation_rules: 한국어 문체는 자연스러운 구어 기반 서사다. 번역체/보고서체/만연체를 피하고, 긴 인과문은 감각·반응·판단으로 쪼갠다.
- 문장은 보통 25~55자 안팎으로 끊고, 90자를 넘는 문장은 드물게만 쓴다. 한 문장에 원인, 감각, 판단, 결과를 모두 넣지 마라.
- `해당`, `진행`, `확인`, `수행`, `위치하다`, `존재하다`, `~하는 것이 필요했다`, `~하는 것으로 보였다`, `~할 수 있었다` 같은 번역체/보고서체 표현을 남발하지 마라.
- prohibited_seed_leakage: Style source는 리듬, 생략, 문단 배열, 대사 압력, 금지 표현만 제어한다. Style source를 content source로 해석하지 마라.
- 유명 작품명, 작가명, 장르 관습, 예시 문장, 문체 설명에서 소재를 빌려오지 마라. seed/canon에 없는 사물·인물·사건·상징·시대감은 품질 향상 명목으로도 추가하지 않는다.
- 추상 감정 설명보다 몸, 시선, 호흡, 손, 거리, 소리, 냄새, 온도 같은 관찰 가능한 흔적으로 보여준다.
- 문단 끝에는 다음 행동을 압박하는 작은 불편함이나 미해결 감각을 남긴다. 다만 선택지 의도나 내부 판정을 본문에서 해설하지 않는다.
- 문체를 설명문으로 노출하지 마라. 대사 말끝, 행동 습관, 문단 박자, 장면 압력으로만 체감되게 써라.
- tone_notes에는 이번 턴에서 실제로 반영한 캐릭터 화법/어미/어투와 전역 서사 문체를 짧게 기록한다.";

fn build_webgpt_turn_prompt(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
) -> Result<String> {
    let source_revival_packet = build_agent_revival_packet(&AgentRevivalCompileOptions {
        store_root,
        pending,
        engine_session_kind: "webgpt_project_session",
    })?;
    let turn_context_packet = assemble_turn_context_packet(pending, source_revival_packet);
    let prompt_context_packet = assemble_prompt_context_packet(&turn_context_packet)?;
    let prompt_context_packet = serde_json::to_string_pretty(&prompt_context_packet)
        .context("failed to serialize webgpt prompt context packet")?;
    let narrative_budget = &pending.output_contract.narrative_budget;
    Ok(format!(
        r#"Singulari World web frontend에서 pending turn 하나가 들어왔어. 너는 WebGPT narrative engine adapter다.

서사 출력 지시:
- 이번 턴 서사 목표: {level_label}. 기본 선택 턴이면 {standard_blocks}문단 / 약 {target_chars}자까지 충분히 써라. 큰 사건이면 {major_blocks}문단 / 약 {major_target_chars}자까지 확장해라.
- text_blocks는 한 항목을 너무 길게 뭉치지 말고, 장면 박자마다 별도 문단으로 나눠라.
- 짧은 로그나 요약이 아니라 한국어 VN prose로 써라. 장면, 감각, 행동, 반응, 여운을 각각 분리해서 쌓아라.

역할:
- 너는 Singulari World의 trusted narrative agent다.
- 플레이어에게 다시 묻지 말고, 아래 prompt context packet만 보고 바로 서사 턴을 작성한다.
- hidden/private context는 판정에만 쓰고, visible_scene/canon_event/choice text에는 절대 누출하지 않는다.
- 출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고, 게임식 수치 계산처럼 보이게 쓰지 않는다.
{text_design_directive}
- 출력량은 prompt context packet의 output_contract.narrative_level과 narrative_budget을 따른다. 레벨 간 차이는 확연해야 한다.
- 레벨 1은 표준 VN 밀도, 레벨 2는 장면 확장 밀도, 레벨 3은 장편 연재 밀도다. 레벨 2/3에서는 같은 사건도 감각, 행동, 반응, 여운, 압박을 더 길게 쌓는다.
- player_input이 "세계 개막"이면 그것은 선택지가 아니라 시드에서 첫 서사를 여는 bootstrap turn이다.
- prompt_context.opening_randomizer가 있으면 사용자의 시드에 덧붙은 player-visible 개막 seed로 취급한다. 그 안의 location_frame, protagonist_frame, immediate_pressure, first_visible_object, social_weather, opening_question을 첫 장면의 시작 조건으로 반영한다.
- opening_randomizer가 없으면 사용자 시드와 visible facts만으로 시작한다. 이전 conversation 문구나 일반적인 bootstrap 기본값을 재사용하지 마라.
- opening_randomizer는 반복 수렴을 피하기 위한 시작 조건이지, 시드에 없는 장르 장치·숨은 과거사·고정 인물 설정을 만드는 권한이 아니다.
- 시드나 visible facts에 명시되지 않은 장르 장치, 과거사, 외부 세계 대비, 게임 인터페이스식 능력 구조를 추론해서 주입하지 마라. 이런 장치는 explicit positive evidence가 있을 때만 쓴다.
- protagonist가 현재 정보를 모른다는 사실만으로 장면 밖 배경, 과거사, 시대 대비 독백, 정체성 상실 클리셰를 만들지 마라.
- 매 턴 survival/social/material/threat/mystery/desire/moral_cost/time_pressure 중 최소 하나의 장면 압력을 visible_scene과 next_choices에 반영한다. 편향을 지우더라도 무미건조한 로그로 쓰지 마라.
- `anchor_character` 저장 필드는 호환용이다. 시드나 visible canon이 명시하지 않으면 구체 인물, 배후 구조, 정해진 역할로 해석하지 마라. 장면 초점은 visible evidence가 만든다.
- slot 7은 항상 판단 위임이고 preview는 숨긴다: "맡긴다. 세부 내용은 선택 후 드러난다."
- slot 6은 항상 자유서술이며 inline prose를 요구하는 선택지로 둔다.
- 이 WebGPT conversation의 이전 turn들은 말맛, 직전 감정선, 장면 리듬을 잇는 working context다.
- ChatGPT Project의 새 세션이나 기존 conversation history는 세계 상태 저장소가 아니다. 세계 연속성은 prompt_context_packet으로만 복원한다.
- prompt_context.visible_context.active_scene_pressure는 이번 턴 선택지와 문단 박자를 누르는 압력 계약이다. visible_scene/next_choices에 반영한다.
- prompt_context.visible_context.active_plot_threads는 현재 열린 문제와 미해결 질문이다. quest-log처럼 설명하지 말고, 이번 장면이 자연스럽게 건드리는 thread만 선택지와 장면 압력으로 이어라.
- prompt_context.visible_context.active_body_resource_state는 주인공의 몸 상태와 실제 보유 자원이다. 없는 물건을 선택지 해결책으로 만들지 말고, 몸 제약은 행동/감각/타인 반응에 필요한 만큼만 반영해라.
- prompt_context.visible_context.active_location_graph는 현재 장소와 알려진 주변 장소다. 이동 선택지는 이 표면의 known/visited 장소 또는 visible_scene에서 정당화된 탐색 행동으로만 열어라.
- prompt_context.visible_context.affordance_graph는 slot 1..5의 행동 허가표다. next_choices 1..5는 각 slot의 affordance_kind/action_contract/source_refs/forbidden_shortcuts를 지켜 장면별 문구로 다시 써라. affordance_id나 source_refs 자체를 선택지에 노출하지 마라.
- prompt_context.visible_context.belief_graph는 주인공과 player-visible narrator가 확정적으로 아는 것의 경계다. belief node가 없는 원인, 정체, 배후, 과거사, 세계 규칙은 확정 서술하지 말고 단서나 불확실성으로만 남겨라.
- prompt_context.visible_context.world_process_clock는 보이는 세계 진행 압력이다. 다음 턴으로 넘기면 악화, 완화, 전환, 해소 중 하나가 일어날 수 있음을 문단 압력과 선택지 비용에 반영해라.
- prompt_context.visible_context.narrative_style_state는 서사 문체와 문단 박자 계약이다. 소재나 설정을 만들지 말고 밀도, 문장 압력, 대사 호흡, 번역체 방지에만 적용해라.
- prompt_context.visible_context.active_character_text_design은 캐릭터별 화법/어미/어투/제스처/습관/drift 계약이다. 전역 문체와 섞지 말고, 인물이 말하거나 행동할 때만 자연스럽게 반영해라.
- prompt_context.visible_context.active_change_ledger는 플레이어 행동으로 변한 세계/관계/압력의 요약 장부다. 오래된 원시 사건보다 active_changes의 before/after/cause_turns를 우선해서 현재 장면의 여파로 반영해라.
- prompt_context.visible_context.active_pattern_debt는 반복 방지 압력이다. canon 사실로 쓰지 말고 replacement_pressure를 선택지 모양, 장면 박자, 문단 마감의 변화로만 반영해라.
- prompt_context.visible_context.active_belief_graph는 장기 누적된 믿음/오해/추론 경계다. 세계의 객관 진실이 아니라 holder/confidence가 붙은 인식 상태로만 써라.
- prompt_context.visible_context.active_world_process_clock는 장기 진행 압력이다. 매 턴 자동 진행하지 말고 player action, time passage, pressure evidence가 닿을 때만 결과에 반영해라.
- prompt_context.visible_context.active_player_intent_trace는 최근 플레이어 행동 모양이다. 플레이어 성격으로 고정하지 말고 다음 선택지 affordance를 미세 조정하는 scene-scoped 압력으로만 써라.
- prompt_context.visible_context.active_turn_retrieval_controller는 이번 턴의 검색 목표와 cue다. canon이나 인물 심리가 아니라 selected_context_capsules를 왜 읽었는지 설명하는 selector brain으로만 써라.
- prompt_context.visible_context.selected_context_capsules는 이번 턴 compiler가 실제로 로드한 context capsule 본문이다. rejected_capsules는 이번 턴에 의도적으로 배제된 맥락이므로 복원하거나 우회하지 마라. broad projection보다 selected capsule을 우선하되, capsule은 source of truth가 아니라 prompt transport unit임을 유지해라.
- prompt_context.visible_context.selected_memory_items는 이번 턴에 물리적으로 선택된 장기기억이다. 각 item의 reason/evidence_refs/source_id를 따라 필요한 항목만 쓰고, selected_memory_items 전체를 다시 요약하지 마라.
- prompt_context.adjudication_context는 판정 전용이다. hidden_world_process_clock를 포함해 visible_scene, next_choices, canon_event, image prompt에 복사하지 마라.
- prompt_context.prompt_policy.omitted_debug_sections는 의도적으로 프롬프트에서 뺀 debug/source 섹션이다. 비어 있는 사실을 임의로 복원하지 마라.
- prompt_context.budget_report는 이번 프롬프트에 포함/제외된 장기맥락의 감사표다. included에 없는 source를 기억으로 취급하지 말고, excluded는 필요하면 다음 턴 compiler가 다시 선별할 수 있는 후보일 뿐 현재 사실로 쓰지 마라.
- 세계의 사실/상태/source of truth는 아래 prompt context packet과 world store다. 웹 채팅 UI나 이전 MCP tool 결과를 source of truth로 쓰지 마라.
- conversation/project context가 compact 되었거나 prompt context packet과 충돌하면 prompt context packet을 우선한다.
- 웹 검색, 외부 사이트 탐색, repo 탐색, 소스 파일 읽기를 하지 마라. 필요한 스키마와 revival packet은 이 프롬프트 안에 있다.

{agent_schema}

prompt context packet JSON:
```json
{prompt_context_packet}
```

출력:
- AgentTurnResponse JSON 하나만 반환한다.
- Markdown fence, 설명문, 도입문 없이 JSON 본문만 반환한다.
- world_id는 "{world_id}", turn_id는 "{turn_id}"와 정확히 같아야 한다.
"#,
        level_label = narrative_budget.level_label,
        standard_blocks = narrative_budget.standard_choice_turn_blocks,
        target_chars = narrative_budget.target_chars,
        major_blocks = narrative_budget.major_turn_blocks,
        major_target_chars = narrative_budget.major_target_chars,
        text_design_directive = SEEDLESS_PROSE_CONTRACT,
        agent_schema = AGENT_TURN_RESPONSE_SCHEMA_GUIDE,
        prompt_context_packet = prompt_context_packet,
        world_id = pending.world_id,
        turn_id = pending.turn_id,
    ))
}

pub(super) fn safe_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::host_worker::{
        HostWorkerOptions, HostWorkerTextBackend, HostWorkerVisualBackend,
    };
    use super::*;
    use singulari_world::{
        AgentSubmitTurnOptions, InitWorldOptions, enqueue_agent_turn, init_world,
    };

    #[test]
    fn webgpt_answer_json_extractor_accepts_fenced_response() -> anyhow::Result<()> {
        let raw = r#"좋아.

```json
{"schema_version":"singulari.agent_turn_response.v1","world_id":"stw","turn_id":"turn_0001"}
```
"#;
        let Some(extracted) = extract_json_object_text(raw) else {
            anyhow::bail!("json should be extracted");
        };
        let value: serde_json::Value = serde_json::from_str(extracted.as_str())?;
        assert_eq!(value["world_id"], serde_json::json!("stw"));
        Ok(())
    }

    #[test]
    fn webgpt_research_arguments_reuse_bound_conversation() {
        let arguments = build_webgpt_research_arguments(
            "prompt",
            Some("conv-123"),
            Some("gpt-5.5"),
            Some("high"),
            12,
        );
        assert_eq!(arguments["conversation_id"], serde_json::json!("conv-123"));
        assert!(arguments.get("new_conversation").is_none());
        assert_eq!(arguments["model"], serde_json::json!("gpt-5.5"));
        assert_eq!(arguments["reasoning_level"], serde_json::json!("high"));
        assert_eq!(arguments["timeout_secs"], serde_json::json!(60));
    }

    #[test]
    fn webgpt_conversation_bindings_are_world_scoped_per_lane() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        save_webgpt_conversation_binding(
            Some(store.as_path()),
            "stw_lane",
            WebGptConversationLane::Text,
            "text-conv",
        )?;
        save_webgpt_conversation_binding(
            Some(store.as_path()),
            "stw_lane",
            WebGptConversationLane::Image,
            "image-conv",
        )?;

        assert_eq!(
            load_webgpt_conversation_binding(
                Some(store.as_path()),
                "stw_lane",
                WebGptConversationLane::Text
            )?,
            Some("text-conv".to_owned())
        );
        assert_eq!(
            load_webgpt_conversation_binding(
                Some(store.as_path()),
                "stw_lane",
                WebGptConversationLane::Image
            )?,
            Some("image-conv".to_owned())
        );

        let text_binding = std::fs::read_to_string(
            store
                .join("worlds/stw_lane/agent_bridge")
                .join(WebGptConversationLane::Text.binding_filename()),
        )?;
        let image_binding = std::fs::read_to_string(
            store
                .join("worlds/stw_lane/agent_bridge")
                .join(WebGptConversationLane::Image.binding_filename()),
        )?;
        assert!(text_binding.contains("\"lane\": \"text\""));
        assert!(text_binding.contains("https://chatgpt.com/c/text-conv"));
        assert!(image_binding.contains("\"lane\": \"image\""));
        assert!(image_binding.contains("https://chatgpt.com/c/image-conv"));
        Ok(())
    }

    #[test]
    fn bundled_webgpt_wrapper_search_does_not_cross_into_sibling_checkout() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path().join("singulari-world");
        let sibling = temp.path().join("webgpt-mcp-checkout/scripts");
        fs::create_dir_all(repo.join("src"))?;
        fs::write(repo.join("Cargo.toml"), "[package]\n")?;
        fs::write(repo.join("src/main.rs"), "fn main() {}\n")?;
        fs::create_dir_all(sibling.as_path())?;
        fs::write(sibling.join("webgpt-mcp.sh"), "#!/usr/bin/env bash\n")?;

        assert!(find_bundled_webgpt_mcp_wrapper_from(repo.join("src").as_path())?.is_none());

        let bundled = repo.join("webgpt-mcp-checkout/scripts");
        fs::create_dir_all(bundled.as_path())?;
        fs::write(bundled.join("webgpt-mcp.sh"), "#!/usr/bin/env bash\n")?;

        assert_eq!(
            find_bundled_webgpt_mcp_wrapper_from(repo.join("src").as_path())?,
            Some(bundled.join("webgpt-mcp.sh"))
        );
        Ok(())
    }

    #[test]
    fn explicit_webgpt_wrapper_must_stay_inside_project_root() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let repo = temp.path().join("singulari-world");
        let repo_wrapper = repo.join("webgpt-mcp-checkout/scripts/webgpt-mcp.sh");
        fs::create_dir_all(repo.join("src"))?;
        fs::create_dir_all(repo_wrapper.parent().context("repo wrapper parent")?)?;
        fs::write(repo.join("Cargo.toml"), "[package]\n")?;
        fs::write(repo.join("src/main.rs"), "fn main() {}\n")?;
        fs::write(repo_wrapper.as_path(), "#!/usr/bin/env bash\n")?;

        let outside_wrapper = temp.path().join("other-webgpt-mcp/scripts/webgpt-mcp.sh");
        fs::create_dir_all(outside_wrapper.parent().context("outside wrapper parent")?)?;
        fs::write(outside_wrapper.as_path(), "#!/usr/bin/env bash\n")?;

        assert!(
            validate_standalone_webgpt_mcp_wrapper(
                repo_wrapper.as_path(),
                repo.as_path(),
                repo.as_path(),
            )?
            .ends_with("webgpt-mcp-checkout/scripts/webgpt-mcp.sh")
        );
        assert!(
            validate_standalone_webgpt_mcp_wrapper(
                outside_wrapper.as_path(),
                repo.as_path(),
                repo.as_path(),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn webgpt_image_conversation_bindings_separate_turn_cg_from_reference_assets()
    -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        save_webgpt_image_conversation_binding(
            Some(store.as_path()),
            "stw_visual_sessions",
            WebGptImageSessionKind::TurnCg,
            "turn-cg-conv",
        )?;
        save_webgpt_image_conversation_binding(
            Some(store.as_path()),
            "stw_visual_sessions",
            WebGptImageSessionKind::ReferenceAsset,
            "asset-conv",
        )?;

        assert_eq!(
            load_webgpt_image_conversation_binding(
                Some(store.as_path()),
                "stw_visual_sessions",
                WebGptImageSessionKind::TurnCg
            )?,
            Some("turn-cg-conv".to_owned())
        );
        assert_eq!(
            load_webgpt_image_conversation_binding(
                Some(store.as_path()),
                "stw_visual_sessions",
                WebGptImageSessionKind::ReferenceAsset
            )?,
            Some("asset-conv".to_owned())
        );

        let bridge_dir = store.join("worlds/stw_visual_sessions/agent_bridge");
        let turn_cg_binding =
            std::fs::read_to_string(bridge_dir.join("webgpt_image_conversation_binding.json"))?;
        let asset_binding = std::fs::read_to_string(
            bridge_dir.join("webgpt_reference_asset_conversation_binding.json"),
        )?;
        assert!(turn_cg_binding.contains("\"image_session_kind\": \"turn_cg\""));
        assert!(turn_cg_binding.contains("https://chatgpt.com/c/turn-cg-conv"));
        assert!(asset_binding.contains("\"image_session_kind\": \"reference_asset\""));
        assert!(asset_binding.contains("https://chatgpt.com/c/asset-conv"));
        Ok(())
    }

    #[test]
    fn webgpt_image_session_accepts_ui_background_jobs_as_asset_lane() -> anyhow::Result<()> {
        let job = test_image_job(
            "menu_background",
            VisualArtifactKind::UiBackground,
            true,
            false,
        );

        ensure_image_job_matches_session_kind(&job, WebGptImageSessionKind::ReferenceAsset)?;
        Ok(())
    }

    #[test]
    fn webgpt_image_session_rejects_ui_background_on_turn_cg_lane() {
        let job = test_image_job(
            "menu_background",
            VisualArtifactKind::UiBackground,
            true,
            false,
        );

        assert!(
            ensure_image_job_matches_session_kind(&job, WebGptImageSessionKind::TurnCg).is_err()
        );
    }

    fn test_image_job(
        slot: &str,
        artifact_kind: VisualArtifactKind,
        display_allowed: bool,
        reference_allowed: bool,
    ) -> ImageGenerationJob {
        ImageGenerationJob {
            tool: "worldsim.image.generate".to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: slot.to_owned(),
                prompt: "prompt".to_owned(),
                destination_path: format!("assets/{slot}.png"),
                reference_paths: Vec::new(),
                overwrite: false,
            },
            slot: slot.to_owned(),
            artifact_kind,
            canonical_use: artifact_kind.canonical_use().to_owned(),
            display_allowed,
            reference_allowed,
            prompt: "prompt".to_owned(),
            destination_path: format!("assets/{slot}.png"),
            reference_asset_urls: Vec::new(),
            reference_paths: Vec::new(),
            overwrite: false,
            register_policy: "test".to_owned(),
        }
    }

    #[test]
    fn webgpt_lane_runtime_defaults_are_isolated() -> anyhow::Result<()> {
        let options = HostWorkerOptions {
            interval_ms: 750,
            once: true,
            text_backend: HostWorkerTextBackend::Webgpt,
            visual_backend: HostWorkerVisualBackend::Webgpt,
            webgpt_turn_command: None,
            webgpt_mcp_wrapper: None,
            webgpt_model: None,
            webgpt_reasoning_level: None,
            webgpt_text_profile_dir: Some("/tmp/singulari-webgpt-text".into()),
            webgpt_image_profile_dir: Some("/tmp/singulari-webgpt-image".into()),
            webgpt_reference_image_profile_dir: Some(
                "/tmp/singulari-webgpt-reference-image".into(),
            ),
            webgpt_text_cdp_port: DEFAULT_WEBGPT_TEXT_CDP_PORT,
            webgpt_image_cdp_port: DEFAULT_WEBGPT_IMAGE_CDP_PORT,
            webgpt_reference_image_cdp_port: DEFAULT_WEBGPT_REFERENCE_IMAGE_CDP_PORT,
            webgpt_timeout_secs: 900,
        };

        let text = WebGptLaneRuntime::new(WebGptConversationLane::Text, &options)?;
        let image = WebGptLaneRuntime::new_image(WebGptImageSessionKind::TurnCg, &options)?;
        let reference =
            WebGptLaneRuntime::new_image(WebGptImageSessionKind::ReferenceAsset, &options)?;

        assert_eq!(text.cdp_port, 9238);
        assert_eq!(image.cdp_port, 9239);
        assert_eq!(reference.cdp_port, 9240);
        assert_ne!(text.cdp_url(), image.cdp_url());
        assert_ne!(image.cdp_url(), reference.cdp_url());
        assert_ne!(text.profile_dir, image.profile_dir);
        assert_ne!(image.profile_dir, reference.profile_dir);
        ensure_webgpt_lane_runtime_isolated(&options)?;
        Ok(())
    }

    #[test]
    fn webgpt_lane_runtime_rejects_shared_port() -> anyhow::Result<()> {
        let options = HostWorkerOptions {
            interval_ms: 750,
            once: true,
            text_backend: HostWorkerTextBackend::Webgpt,
            visual_backend: HostWorkerVisualBackend::Webgpt,
            webgpt_turn_command: None,
            webgpt_mcp_wrapper: None,
            webgpt_model: None,
            webgpt_reasoning_level: None,
            webgpt_text_profile_dir: Some("/tmp/singulari-webgpt-text".into()),
            webgpt_image_profile_dir: Some("/tmp/singulari-webgpt-image".into()),
            webgpt_reference_image_profile_dir: Some(
                "/tmp/singulari-webgpt-reference-image".into(),
            ),
            webgpt_text_cdp_port: 9238,
            webgpt_image_cdp_port: 9238,
            webgpt_reference_image_cdp_port: 9240,
            webgpt_timeout_secs: 900,
        };

        let Err(error) = ensure_webgpt_lane_runtime_isolated(&options) else {
            anyhow::bail!("shared WebGPT CDP ports reached dispatch");
        };
        assert!(error.to_string().contains("distinct CDP ports"));
        Ok(())
    }

    #[test]
    fn webgpt_turn_cg_prompt_reuses_turn_cg_session_url() {
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "turn_cg:turn_0002".to_owned(),
                prompt: "draw the scene".to_owned(),
                destination_path: "/tmp/turn_0002.png".to_owned(),
                reference_paths: vec!["/tmp/char.png".to_owned()],
                overwrite: false,
            },
            slot: "turn_cg:turn_0002".to_owned(),
            artifact_kind: VisualArtifactKind::SceneCg,
            canonical_use: "display_scene".to_owned(),
            display_allowed: true,
            reference_allowed: false,
            prompt: "draw the scene".to_owned(),
            destination_path: "/tmp/turn_0002.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: vec!["/tmp/char.png".to_owned()],
            overwrite: false,
            register_policy: "test".to_owned(),
        };
        let prompt = build_webgpt_image_generation_prompt(
            "stw_visual",
            &job,
            Some("image-conv"),
            WebGptImageSessionKind::TurnCg,
        );

        assert!(prompt.contains("dedicated world-scoped turn-CG session for stw_visual"));
        assert!(prompt.contains("https://chatgpt.com/c/image-conv"));
        assert!(prompt.contains("previous turn-CG images in this same conversation"));
        assert!(prompt.contains("never render a character design sheet"));
        assert!(prompt.contains("Image job slot: turn_cg:turn_0002"));
        assert!(prompt.contains("Reference continuity notes: /tmp/char.png"));
    }

    #[test]
    fn webgpt_reference_asset_prompt_is_not_a_scene_cg_prompt() {
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "character_sheet:char:protagonist".to_owned(),
                prompt: "draw the character sheet".to_owned(),
                destination_path: "/tmp/char_protagonist.png".to_owned(),
                reference_paths: Vec::new(),
                overwrite: false,
            },
            slot: "character_sheet:char:protagonist".to_owned(),
            artifact_kind: VisualArtifactKind::CharacterDesignSheet,
            canonical_use: "reference_generation".to_owned(),
            display_allowed: false,
            reference_allowed: true,
            prompt: "draw the character sheet".to_owned(),
            destination_path: "/tmp/char_protagonist.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: Vec::new(),
            overwrite: false,
            register_policy: "test".to_owned(),
        };
        let prompt = build_webgpt_image_generation_prompt(
            "stw_visual",
            &job,
            Some("asset-conv"),
            WebGptImageSessionKind::ReferenceAsset,
        );

        assert!(prompt.contains("reference asset image"));
        assert!(prompt.contains("dedicated world-scoped reference-asset session"));
        assert!(prompt.contains("must never be treated as or displayed as a turn scene CG"));
        assert!(prompt.contains("Do not use turn-CG conversation history"));
        assert!(prompt.contains("Image job slot: character_sheet:char:protagonist"));
        assert!(!prompt.contains("full-screen visual novel scene CG"));
    }

    #[test]
    fn webgpt_image_reference_paths_canonicalize_assets() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let reference = temp.path().join("char.png");
        std::fs::write(&reference, b"png fixture")?;
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "turn_cg:turn_0002".to_owned(),
                prompt: "draw the scene".to_owned(),
                destination_path: "/tmp/turn_0002.png".to_owned(),
                reference_paths: vec![reference.display().to_string()],
                overwrite: false,
            },
            slot: "turn_cg:turn_0002".to_owned(),
            artifact_kind: VisualArtifactKind::SceneCg,
            canonical_use: "display_scene".to_owned(),
            display_allowed: true,
            reference_allowed: false,
            prompt: "draw the scene".to_owned(),
            destination_path: "/tmp/turn_0002.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: vec![reference.display().to_string()],
            overwrite: false,
            register_policy: "test".to_owned(),
        };

        assert_eq!(
            webgpt_image_reference_paths(&job)?,
            vec![reference.canonicalize()?.display().to_string()]
        );
        Ok(())
    }

    #[test]
    fn webgpt_image_reference_paths_fail_loud_when_asset_is_missing() -> anyhow::Result<()> {
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "turn_cg:turn_0002".to_owned(),
                prompt: "draw the scene".to_owned(),
                destination_path: "/tmp/turn_0002.png".to_owned(),
                reference_paths: vec!["/tmp/singulari-missing-reference.png".to_owned()],
                overwrite: false,
            },
            slot: "turn_cg:turn_0002".to_owned(),
            artifact_kind: VisualArtifactKind::SceneCg,
            canonical_use: "display_scene".to_owned(),
            display_allowed: true,
            reference_allowed: false,
            prompt: "draw the scene".to_owned(),
            destination_path: "/tmp/turn_0002.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: vec!["/tmp/singulari-missing-reference.png".to_owned()],
            overwrite: false,
            register_policy: "test".to_owned(),
        };

        let Err(error) = webgpt_image_reference_paths(&job) else {
            anyhow::bail!("missing reference asset reached image dispatch");
        };
        assert!(
            error
                .to_string()
                .contains("webgpt image reference asset missing")
        );
        Ok(())
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "prompt contract regression test intentionally keeps the full expectation list together"
    )]
    fn webgpt_prompt_carries_realtime_agent_contract() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_contract
title: "webgpt contract test"
premise:
  genre: "fantasy"
  protagonist: "modern reincarnated protagonist"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_contract".to_owned(),
            input: "2".to_owned(),
            narrative_level: None,
        })?;
        let prompt = build_webgpt_turn_prompt(Some(store.as_path()), &pending)?;

        for required in [
            "너는 Singulari World의 trusted narrative agent다.",
            "출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고",
            "이 계약은 seedless style contract다.",
            "문체/작법 규칙은 소재, 사건, 인물, 장소, 장르 장치, 과거사, 상징을 새로 만들 권한이 없다.",
            "scene_fact_boundaries: 오직 prompt context packet의 player-visible facts",
            "speech는 화법, endings는 어미/말끝, tone은 어투/거리감/어휘",
            "문체와 서사 작법은 캐릭터에 귀속하지 말고 visible_scene의 전역 서사에만 적용한다.",
            "paragraph_grammar: 각 문단은 감각 변화, 몸의 반응, 외부 압력, 해석을 유보한 단서, 다음 행동을 압박하는 변화 중 최소 둘을 포함한다.",
            "시작 문단은 배경 설명이 아니라 현재 장면에서 감각적으로 바뀐 것과 visible constraint를 연다.",
            "상호작용 문단은 말 한 줄, 작은 몸짓, 끊긴 반응, 침묵, 거리 변화 중 하나를 중심으로 둔다.",
            "마감 문단은 요약이나 교훈으로 닫지 말고",
            "dialogue_contract: 대사는 설명문이 아니다.",
            "style_vector: sentence_pressure=high",
            "anti_translation_rules: 한국어 문체는 자연스러운 구어 기반 서사다.",
            "번역체/보고서체/만연체를 피하고, 긴 인과문은 감각·반응·판단으로 쪼갠다.",
            "문장은 보통 25~55자 안팎으로 끊고, 90자를 넘는 문장은 드물게만 쓴다.",
            "`해당`, `진행`, `확인`, `수행`, `위치하다`, `존재하다`",
            "prohibited_seed_leakage: Style source는 리듬, 생략, 문단 배열, 대사 압력, 금지 표현만 제어한다.",
            "유명 작품명, 작가명, 장르 관습, 예시 문장, 문체 설명에서 소재를 빌려오지 마라.",
            "추상 감정 설명보다 몸, 시선, 호흡, 손, 거리, 소리, 냄새, 온도 같은 관찰 가능한 흔적으로 보여준다.",
            "선택지 의도나 내부 판정을 본문에서 해설하지 않는다.",
            "레벨 1은 표준 VN 밀도, 레벨 2는 장면 확장 밀도, 레벨 3은 장편 연재 밀도다.",
            "prompt_context.opening_randomizer가 있으면 사용자의 시드에 덧붙은 player-visible 개막 seed로 취급한다.",
            "opening_randomizer가 없으면 사용자 시드와 visible facts만으로 시작한다.",
            "opening_randomizer는 반복 수렴을 피하기 위한 시작 조건이지, 시드에 없는 장르 장치·숨은 과거사·고정 인물 설정을 만드는 권한이 아니다.",
            "시드나 visible facts에 명시되지 않은 장르 장치, 과거사, 외부 세계 대비, 게임 인터페이스식 능력 구조를 추론해서 주입하지 마라.",
            "protagonist가 현재 정보를 모른다는 사실만으로 장면 밖 배경, 과거사, 시대 대비 독백, 정체성 상실 클리셰를 만들지 마라.",
            "이 WebGPT conversation의 이전 turn들은 말맛, 직전 감정선, 장면 리듬을 잇는 working context다.",
            "conversation/project context가 compact 되었거나 prompt context packet과 충돌하면 prompt context packet을 우선한다.",
            "prompt_context.visible_context.selected_memory_items는 이번 턴에 물리적으로 선택된 장기기억이다.",
            "prompt_context.visible_context.affordance_graph는 slot 1..5의 행동 허가표다.",
            "prompt_context.visible_context.belief_graph는 주인공과 player-visible narrator가 확정적으로 아는 것의 경계다.",
            "prompt_context.visible_context.world_process_clock는 보이는 세계 진행 압력이다.",
            "prompt_context.visible_context.narrative_style_state는 서사 문체와 문단 박자 계약이다.",
            "prompt_context.visible_context.active_change_ledger는 플레이어 행동으로 변한 세계/관계/압력의 요약 장부다.",
            "prompt_context.visible_context.active_pattern_debt는 반복 방지 압력이다.",
            "prompt_context.visible_context.active_belief_graph는 장기 누적된 믿음/오해/추론 경계다.",
            "prompt_context.visible_context.active_world_process_clock는 장기 진행 압력이다.",
            "prompt_context.visible_context.active_player_intent_trace는 최근 플레이어 행동 모양이다.",
            "prompt_context.adjudication_context는 판정 전용이다.",
            "prompt_context.prompt_policy.omitted_debug_sections",
            "prompt_context.budget_report",
            "웹 검색, 외부 사이트 탐색, repo 탐색, 소스 파일 읽기를 하지 마라.",
            "\"schema_version\": \"singulari.prompt_context_packet.v1\"",
            "\"schema_version\": \"singulari.prompt_context_budget_report.v1\"",
            "\"visible_context\"",
            "\"adjudication_context\"",
            "\"selected_memory_items\"",
            "\"active_change_ledger\"",
            "\"active_pattern_debt\"",
            "\"active_belief_graph\"",
            "\"active_world_process_clock\"",
            "\"active_player_intent_trace\"",
            "\"active_turn_retrieval_controller\"",
            "\"selected_context_capsules\"",
            "\"affordance_graph\"",
            "\"ordinary_choice_slots\"",
            "\"forbidden_shortcuts\"",
            "\"belief_graph\"",
            "\"protagonist_visible_beliefs\"",
            "\"narrator_knowledge_limits\"",
            "\"world_process_clock\"",
            "\"hidden_world_process_clock\"",
            "\"narrative_style_state\"",
            "\"anti_translation_rules\"",
            "\"prohibited_seed_leakage\"",
            "\"prompt_policy\"",
            "\"omitted_debug_sections\"",
            "\"active_scene_pressure\"",
            "\"active_plot_threads\"",
            "\"plot_thread_events\"",
            "\"change\": \"advanced\"",
            "\"scene_pressure_events\"",
            "\"world_lore_updates\"",
            "\"character_text_design_updates\"",
            "\"body_resource_events\"",
            "\"location_events\"",
            "\"hidden_state_delta\"",
            "\"active_body_resource_state\"",
            "\"active_location_graph\"",
            "\"active_character_text_design\"",
            "\"source_of_truth_policy\"",
            "\"conflict_rule\": \"revival_packet_wins\"",
            "AgentTurnResponse 스키마:",
            "\"schema_version\": \"singulari.agent_turn_response.v1\"",
            "{\"slot\":6,\"tag\":\"자유서술\"",
            "{\"slot\":7,\"tag\":\"판단 위임\"",
            "world_id는 \"stw_contract\", turn_id는 \"turn_0001\"와 정확히 같아야 한다.",
        ] {
            assert!(
                prompt.contains(required),
                "webgpt prompt missing realtime contract: {required}"
            );
        }

        let prompt_context = serde_json::to_value(
            singulari_world::extract_prompt_context_from_prompt(&prompt)?,
        )?;
        for omitted_path in [
            "/source_revival",
            "/retrieval_profile",
            "/anti_repetition_rules",
            "/memory_revival",
            "/resume_pack",
            "/active_memory_revival",
            "/visible_context/player_visible_archive_view",
            "/visible_context/query_recall",
            "/visible_context/recent_entity_updates",
            "/visible_context/recent_relationship_updates",
            "/visible_context/active_relationship_graph",
            "/visible_context/active_world_lore",
            "/visible_context/agent_context_projection",
        ] {
            assert!(
                prompt_context.pointer(omitted_path).is_none(),
                "webgpt prompt context leaked debug/source path: {omitted_path}"
            );
        }
        Ok(())
    }
}
