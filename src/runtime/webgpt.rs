use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::Serialize;
use serde_json::Value;
use singulari_world::{
    AgentCommitTurnOptions, AgentTurnResponse, CompilePromptContextPacketOptions,
    commit_agent_turn, compile_prompt_context_packet, resolve_store_paths,
};
use singulari_world::{WorldJobStatus, WriteTextTurnJobOptions, write_text_turn_job};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::Duration as StdDuration;

use super::host_worker::HostWorkerOptions;

mod image;
mod json_extract;
mod prompt;

pub(super) use image::{
    WebGptImageDispatchRecord, WebGptImageSessionKind, dispatch_visual_job_via_webgpt,
    visual_dispatch_dir_for_world,
};
use json_extract::extract_json_object_text;
use prompt::build_webgpt_turn_prompt;

#[cfg(test)]
use image::{
    build_webgpt_image_generation_prompt, ensure_image_job_matches_session_kind,
    webgpt_image_reference_paths,
};

pub(crate) const DEFAULT_WEBGPT_TEXT_CDP_PORT: u16 = 9238;
pub(crate) const DEFAULT_WEBGPT_IMAGE_CDP_PORT: u16 = 9239;
pub(crate) const DEFAULT_WEBGPT_REFERENCE_IMAGE_CDP_PORT: u16 = 9240;
pub(crate) const DEFAULT_WEBGPT_TIMEOUT_SECS: u64 = 900;
const WEBGPT_DISPATCH_STALE_CUSHION_SECS: u64 = 60;
const WEBGPT_MCP_CONTROL_TIMEOUT_SECS: u64 = 30;
const WEBGPT_TEXT_JOB_OWNER: &str = "webgpt_host_worker";
const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const MCP_CLIENT_VERSION: &str = "0.1.0";

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
pub(super) struct WebGptLanePrewarmRecord {
    lane: &'static str,
    cdp_port: u16,
    cdp_url: String,
    profile_dir: String,
    status: &'static str,
    exit_code: Option<i32>,
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
    pub(super) prompt_context_path: String,
    pub(super) response_path: String,
    pub(super) repair_attempts: u8,
    pub(super) repair_prompt_path: Option<String>,
    pub(super) repair_response_path: Option<String>,
    pub(super) result_path: Option<String>,
    pub(super) stdout_path: String,
    pub(super) stderr_path: String,
    pub(super) prompt_bytes: u64,
    pub(super) prompt_context_bytes: u64,
    pub(super) dispatched_at: String,
    pub(super) mcp_completed_at: String,
    pub(super) mcp_duration_ms: i64,
    pub(super) total_duration_ms: i64,
    pub(super) exit_code: Option<i32>,
    pub(super) committed_turn_id: Option<String>,
    pub(super) render_packet_path: Option<String>,
    pub(super) commit_record_path: Option<String>,
    pub(super) error: Option<String>,
    pub(super) completed_at: String,
}

#[expect(
    clippy::too_many_lines,
    reason = "text dispatch owns claim, WebGPT call, bounded repair, commit, and durable record"
)]
pub(super) fn dispatch_pending_agent_turn_via_webgpt(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    options: &HostWorkerOptions,
) -> Result<WebGptDispatchOutcome> {
    let dispatch_dir = dispatch_dir_for_pending(store_root, pending)?;
    fs::create_dir_all(&dispatch_dir)
        .with_context(|| format!("failed to create {}", dispatch_dir.display()))?;
    let record_path = dispatch_dir.join(format!("{}-webgpt.json", pending.turn_id));
    let prompt_path = dispatch_dir.join(format!("{}-webgpt-prompt.md", pending.turn_id));
    let prompt_context_path =
        dispatch_dir.join(format!("{}-webgpt-prompt-context.json", pending.turn_id));
    let response_path =
        dispatch_dir.join(format!("{}-webgpt-agent-response.json", pending.turn_id));
    let result_path = dispatch_dir.join(format!("{}-webgpt-result.json", pending.turn_id));
    let stdout_path = dispatch_dir.join(format!("{}-webgpt-stdout.log", pending.turn_id));
    let stderr_path = dispatch_dir.join(format!("{}-webgpt-stderr.log", pending.turn_id));
    let repair_prompt_path =
        dispatch_dir.join(format!("{}-webgpt-repair-1-prompt.md", pending.turn_id));
    let repair_response_path = dispatch_dir.join(format!(
        "{}-webgpt-repair-1-agent-response.json",
        pending.turn_id
    ));
    let repair_result_path =
        dispatch_dir.join(format!("{}-webgpt-repair-1-result.json", pending.turn_id));
    let repair_stdout_path =
        dispatch_dir.join(format!("{}-webgpt-repair-1-stdout.log", pending.turn_id));
    let repair_stderr_path =
        dispatch_dir.join(format!("{}-webgpt-repair-1-stderr.log", pending.turn_id));
    let dispatcher = resolve_webgpt_dispatcher(store_root, pending, options)?;
    let paths = WebGptTurnPaths {
        world_id: pending.world_id.as_str(),
        turn_id: pending.turn_id.as_str(),
        prompt_path: prompt_path.as_path(),
        prompt_context_path: prompt_context_path.as_path(),
        response_path: response_path.as_path(),
        result_path: result_path.as_path(),
        stdout_path: stdout_path.as_path(),
        stderr_path: stderr_path.as_path(),
    };

    if record_path.exists() {
        if let Some(record) = try_commit_existing_webgpt_response(ExistingWebGptResponseInput {
            store_root,
            pending,
            dispatcher: &dispatcher,
            record_path: record_path.as_path(),
            base_paths: paths,
            repair_prompt_path: repair_prompt_path.as_path(),
            repair_response_path: repair_response_path.as_path(),
            repair_result_path: repair_result_path.as_path(),
            repair_stdout_path: repair_stdout_path.as_path(),
            repair_stderr_path: repair_stderr_path.as_path(),
        })? {
            return Ok(WebGptDispatchOutcome::Started(Box::new(record)));
        }
        if !existing_dispatch_is_retryable(record_path.as_path())? {
            return Ok(WebGptDispatchOutcome::AlreadyDispatched(
                record_path.display().to_string(),
            ));
        }
    }

    let prompt_artifacts =
        write_webgpt_prompt_artifacts(store_root, pending, &prompt_context_path, &prompt_path)?;
    let dispatched_at = Utc::now();
    let claim = webgpt_dispatch_claim(
        pending,
        &dispatcher,
        paths,
        dispatched_at,
        prompt_artifacts.prompt_bytes,
        prompt_artifacts.prompt_context_bytes,
    );
    if !write_dispatch_claim(record_path.as_path(), &claim)? {
        return Ok(WebGptDispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }
    write_webgpt_text_job(
        store_root,
        pending,
        WorldJobStatus::Running,
        Some(record_path.display().to_string()),
        Some(format!("webgpt:{}", pending.turn_id)),
        None,
    )?;

    let mut dispatch_result = dispatcher.run(paths)?;
    let mut mcp_completed_at = Utc::now();
    if let Some(raw_conversation_id) = dispatch_result.raw_conversation_id.as_deref() {
        save_webgpt_conversation_binding(
            store_root,
            pending.world_id.as_str(),
            WebGptConversationLane::Text,
            raw_conversation_id,
        )?;
    }

    let mut final_paths = paths;
    let mut commit_result = commit_webgpt_dispatch_if_success(
        store_root,
        pending,
        response_path.as_path(),
        &dispatch_result,
    );
    let mut repair_attempts = 0;
    if let Some(error) = commit_result.error.as_deref()
        && dispatch_result.success
        && is_repairable_webgpt_commit_error(error)
    {
        repair_attempts = 1;
        write_webgpt_repair_prompt(
            prompt_path.as_path(),
            prompt_context_path.as_path(),
            response_path.as_path(),
            error,
            repair_prompt_path.as_path(),
        )?;
        final_paths = WebGptTurnPaths {
            world_id: pending.world_id.as_str(),
            turn_id: pending.turn_id.as_str(),
            prompt_path: repair_prompt_path.as_path(),
            prompt_context_path: prompt_context_path.as_path(),
            response_path: repair_response_path.as_path(),
            result_path: repair_result_path.as_path(),
            stdout_path: repair_stdout_path.as_path(),
            stderr_path: repair_stderr_path.as_path(),
        };
        dispatch_result = dispatcher.run(final_paths)?;
        mcp_completed_at = Utc::now();
        if let Some(raw_conversation_id) = dispatch_result.raw_conversation_id.as_deref() {
            save_webgpt_conversation_binding(
                store_root,
                pending.world_id.as_str(),
                WebGptConversationLane::Text,
                raw_conversation_id,
            )?;
        }
        commit_result = commit_webgpt_dispatch_if_success(
            store_root,
            pending,
            repair_response_path.as_path(),
            &dispatch_result,
        );
    }

    let record = webgpt_dispatch_record(
        pending,
        &dispatcher,
        final_paths,
        record_path.as_path(),
        dispatch_result,
        commit_result,
        WebGptRepairRecord {
            attempts: repair_attempts,
            prompt_path: (repair_attempts > 0).then(|| repair_prompt_path.display().to_string()),
            response_path: (repair_attempts > 0)
                .then(|| repair_response_path.display().to_string()),
        },
        WebGptDispatchTiming {
            dispatched_at,
            mcp_completed_at,
            prompt_bytes: prompt_artifacts.prompt_bytes,
            prompt_context_bytes: prompt_artifacts.prompt_context_bytes,
        },
    );
    fs::write(&record_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", record_path.display()))?;
    write_webgpt_text_job(
        store_root,
        pending,
        text_job_status_from_record(&record),
        Some(text_job_output_ref(&record)),
        Some(format!("webgpt:{}", pending.turn_id)),
        record.error.clone(),
    )?;
    Ok(WebGptDispatchOutcome::Started(Box::new(record)))
}

#[derive(Clone, Copy)]
struct ExistingWebGptResponseInput<'a> {
    store_root: Option<&'a Path>,
    pending: &'a singulari_world::PendingAgentTurn,
    dispatcher: &'a WebGptDispatcher,
    record_path: &'a Path,
    base_paths: WebGptTurnPaths<'a>,
    repair_prompt_path: &'a Path,
    repair_response_path: &'a Path,
    repair_result_path: &'a Path,
    repair_stdout_path: &'a Path,
    repair_stderr_path: &'a Path,
}

fn try_commit_existing_webgpt_response(
    input: ExistingWebGptResponseInput<'_>,
) -> Result<Option<WebGptDispatchRecord>> {
    let Some(existing) = read_json_value_if_present(input.record_path)? else {
        return Ok(None);
    };
    let status = existing.get("status").and_then(Value::as_str);
    let retryable_dispatching =
        matches!(status, Some("dispatching")) && dispatch_record_is_stale(&existing)?;
    let retryable_commit_failed = matches!(status, Some("commit_failed"))
        && existing
            .get("repair_attempts")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            == 0;
    if !retryable_dispatching && !retryable_commit_failed {
        return Ok(None);
    }

    let Some((paths, repair)) = select_existing_webgpt_response_paths(input) else {
        return Ok(None);
    };

    let dispatch_result = existing_webgpt_dispatch_result(&existing);
    let commit_result = commit_webgpt_dispatch_if_success(
        input.store_root,
        input.pending,
        paths.response_path,
        &dispatch_result,
    );
    if repair.attempts == 0
        && commit_result
            .error
            .as_deref()
            .is_some_and(is_repairable_webgpt_commit_error)
    {
        return Ok(None);
    }

    let dispatched_at = existing_record_time(&existing, "dispatched_at").unwrap_or_else(Utc::now);
    let mcp_completed_at =
        existing_record_time(&existing, "mcp_completed_at").unwrap_or_else(Utc::now);
    let prompt_bytes = existing
        .get("prompt_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let prompt_context_bytes = existing
        .get("prompt_context_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let record = webgpt_dispatch_record(
        input.pending,
        input.dispatcher,
        paths,
        input.record_path,
        dispatch_result,
        commit_result,
        repair,
        WebGptDispatchTiming {
            dispatched_at,
            mcp_completed_at,
            prompt_bytes,
            prompt_context_bytes,
        },
    );
    fs::write(input.record_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", input.record_path.display()))?;
    write_webgpt_text_job(
        input.store_root,
        input.pending,
        text_job_status_from_record(&record),
        Some(text_job_output_ref(&record)),
        Some(format!("webgpt:{}", input.pending.turn_id)),
        record.error.clone(),
    )?;
    Ok(Some(record))
}

fn select_existing_webgpt_response_paths(
    input: ExistingWebGptResponseInput<'_>,
) -> Option<(WebGptTurnPaths<'_>, WebGptRepairRecord)> {
    if input.repair_response_path.is_file() {
        return Some((
            WebGptTurnPaths {
                world_id: input.pending.world_id.as_str(),
                turn_id: input.pending.turn_id.as_str(),
                prompt_path: input.repair_prompt_path,
                prompt_context_path: input.base_paths.prompt_context_path,
                response_path: input.repair_response_path,
                result_path: input.repair_result_path,
                stdout_path: input.repair_stdout_path,
                stderr_path: input.repair_stderr_path,
            },
            WebGptRepairRecord {
                attempts: 1,
                prompt_path: Some(input.repair_prompt_path.display().to_string()),
                response_path: Some(input.repair_response_path.display().to_string()),
            },
        ));
    }
    input.base_paths.response_path.is_file().then_some((
        input.base_paths,
        WebGptRepairRecord {
            attempts: 0,
            prompt_path: None,
            response_path: None,
        },
    ))
}

fn existing_webgpt_dispatch_result(existing: &Value) -> WebGptDispatchResult {
    WebGptDispatchResult {
        success: true,
        pid: json_u32_field(existing, "pid"),
        exit_code: existing
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        raw_conversation_id: existing
            .get("raw_conversation_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        current_model: existing
            .get("current_model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        current_reasoning_level: existing
            .get("current_reasoning_level")
            .and_then(Value::as_str)
            .map(str::to_owned),
        error: None,
    }
}

fn json_u32_field(value: &Value, key: &str) -> u32 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|raw| u32::try_from(raw).ok())
        .unwrap_or(0)
}

fn existing_record_time(value: &Value, key: &str) -> Option<DateTime<Utc>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|raw| DateTime::parse_from_rfc3339(raw).ok())
        .map(|value| value.with_timezone(&Utc))
}

struct WebGptPromptArtifacts {
    prompt_bytes: u64,
    prompt_context_bytes: u64,
}

fn write_webgpt_prompt_artifacts(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    prompt_context_path: &Path,
    prompt_path: &Path,
) -> Result<WebGptPromptArtifacts> {
    let prompt_context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root,
        pending,
        engine_session_kind: "webgpt_project_session",
    })?;
    let prompt_context_raw = serde_json::to_vec_pretty(&prompt_context)?;
    let prompt_context_bytes = prompt_context_raw.len() as u64;
    fs::write(prompt_context_path, prompt_context_raw)
        .with_context(|| format!("failed to write {}", prompt_context_path.display()))?;
    let prompt = build_webgpt_turn_prompt(&prompt_context)?;
    let prompt_bytes = prompt.len() as u64;
    fs::write(prompt_path, prompt.as_bytes())
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;
    Ok(WebGptPromptArtifacts {
        prompt_bytes,
        prompt_context_bytes,
    })
}

fn write_webgpt_repair_prompt(
    original_prompt_path: &Path,
    prompt_context_path: &Path,
    rejected_response_path: &Path,
    commit_error: &str,
    repair_prompt_path: &Path,
) -> Result<()> {
    let original_prompt = fs::read_to_string(original_prompt_path)
        .with_context(|| format!("failed to read {}", original_prompt_path.display()))?;
    let rejected_response = fs::read_to_string(rejected_response_path)
        .with_context(|| format!("failed to read {}", rejected_response_path.display()))?;
    let repair_prompt = format!(
        r"{original_prompt}

## Structured Repair Request

The previous AgentTurnResponse was rejected before world mutation.

Commit/audit error:

```text
{commit_error}
```

Rejected AgentTurnResponse:

```json
{rejected_response}
```

Repair scope:

- Return one complete `AgentTurnResponse` JSON object only.
- Keep the same `schema_version`, `world_id`, and `turn_id`.
- Do not ask for more context unless the original prompt context truly cannot
  support a safe turn.
- Change only the invalid proposal, choice grounding, actor agency, or
  evidence fields needed to satisfy the audit.
- Preserve player-visible continuity from the original prompt.
- Hidden/adjudication-only details must not appear in visible_scene,
  next_choices, player-visible summaries, image prompts, or Codex View fields.

Prompt context artifact path for audit reference:

```text
{}
```
",
        prompt_context_path.display()
    );
    fs::write(repair_prompt_path, repair_prompt.as_bytes())
        .with_context(|| format!("failed to write {}", repair_prompt_path.display()))
}

fn is_repairable_webgpt_commit_error(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    [
        "resolution proposal audit failed",
        "scene director proposal audit failed",
        "consequence proposal",
        "consequence mutation",
        "consequence payoff",
        "agent response next_choices",
        "actor agency update",
        "failed to parse",
        "missing field",
        "unknown variant",
        "invalid type",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn write_webgpt_text_job(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    status: WorldJobStatus,
    output_ref: Option<String>,
    attempt_id: Option<String>,
    last_error: Option<String>,
) -> Result<()> {
    write_text_turn_job(&WriteTextTurnJobOptions {
        store_root,
        pending,
        status,
        output_ref,
        claim_owner: Some(WEBGPT_TEXT_JOB_OWNER.to_owned()),
        attempt_id,
        last_error,
    })?;
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "record assembly is clearer with explicit dispatch, commit, repair, and timing inputs"
)]
fn webgpt_dispatch_record(
    pending: &singulari_world::PendingAgentTurn,
    dispatcher: &WebGptDispatcher,
    paths: WebGptTurnPaths<'_>,
    record_path: &Path,
    dispatch_result: WebGptDispatchResult,
    commit_result: WebGptCommitResult,
    repair: WebGptRepairRecord,
    timing: WebGptDispatchTiming,
) -> WebGptDispatchRecord {
    let completed_at = Utc::now();
    WebGptDispatchRecord {
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
        prompt_path: paths.prompt_path.display().to_string(),
        prompt_context_path: paths.prompt_context_path.display().to_string(),
        response_path: paths.response_path.display().to_string(),
        repair_attempts: repair.attempts,
        repair_prompt_path: repair.prompt_path,
        repair_response_path: repair.response_path,
        result_path: Some(paths.result_path.display().to_string()),
        stdout_path: paths.stdout_path.display().to_string(),
        stderr_path: paths.stderr_path.display().to_string(),
        prompt_bytes: timing.prompt_bytes,
        prompt_context_bytes: timing.prompt_context_bytes,
        dispatched_at: timing.dispatched_at.to_rfc3339(),
        mcp_completed_at: timing.mcp_completed_at.to_rfc3339(),
        mcp_duration_ms: duration_ms(timing.dispatched_at, timing.mcp_completed_at),
        total_duration_ms: duration_ms(timing.dispatched_at, completed_at),
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
        completed_at: completed_at.to_rfc3339(),
    }
}

struct WebGptRepairRecord {
    attempts: u8,
    prompt_path: Option<String>,
    response_path: Option<String>,
}

#[derive(Clone, Copy)]
struct WebGptDispatchTiming {
    dispatched_at: DateTime<Utc>,
    mcp_completed_at: DateTime<Utc>,
    prompt_bytes: u64,
    prompt_context_bytes: u64,
}

fn duration_ms(start: DateTime<Utc>, end: DateTime<Utc>) -> i64 {
    end.signed_duration_since(start).num_milliseconds().max(0)
}

fn text_job_status_from_record(record: &WebGptDispatchRecord) -> WorldJobStatus {
    match record.status.as_str() {
        "completed" => WorldJobStatus::Completed,
        "waiting_browser" => WorldJobStatus::Running,
        "commit_failed" => WorldJobStatus::FailedTerminal,
        _ => WorldJobStatus::FailedRetryable,
    }
}

fn text_job_output_ref(record: &WebGptDispatchRecord) -> String {
    record
        .commit_record_path
        .as_deref()
        .or(record.render_packet_path.as_deref())
        .or(record.result_path.as_deref())
        .unwrap_or(record.record_path.as_str())
        .to_owned()
}

fn webgpt_dispatch_claim(
    pending: &singulari_world::PendingAgentTurn,
    dispatcher: &WebGptDispatcher,
    paths: WebGptTurnPaths<'_>,
    dispatched_at: DateTime<Utc>,
    prompt_bytes: u64,
    prompt_context_bytes: u64,
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
        "timeout_secs": dispatcher.timeout_secs(),
        "prompt_path": paths.prompt_path.display().to_string(),
        "prompt_context_path": paths.prompt_context_path.display().to_string(),
        "response_path": paths.response_path.display().to_string(),
        "result_path": paths.result_path.display().to_string(),
        "stdout_path": paths.stdout_path.display().to_string(),
        "stderr_path": paths.stderr_path.display().to_string(),
        "prompt_bytes": prompt_bytes,
        "prompt_context_bytes": prompt_context_bytes,
        "dispatched_at": dispatched_at.to_rfc3339(),
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
        let status = if dispatch_result
            .error
            .as_deref()
            .is_some_and(is_webgpt_timeout_signal)
        {
            "waiting_browser"
        } else {
            "failed"
        };
        return WebGptCommitResult {
            status: status.to_owned(),
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
            error: Some(format!("{error:#}")),
        },
    }
}

#[derive(Clone, Copy)]
struct WebGptTurnPaths<'a> {
    world_id: &'a str,
    turn_id: &'a str,
    prompt_path: &'a Path,
    prompt_context_path: &'a Path,
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

struct PersistentMcpToolResult {
    pid: u32,
    first_text: String,
}

struct PersistentMcpClient {
    child: Child,
    stdin: ChildStdin,
    stdout_rx: Receiver<Result<String, String>>,
    next_id: i64,
    advertised_tools: Option<HashSet<String>>,
}

thread_local! {
    static WEBGPT_MCP_CLIENTS: RefCell<HashMap<String, PersistentMcpClient>> =
        RefCell::new(HashMap::new());
}

impl PersistentMcpClient {
    fn spawn(wrapper: &Path, runtime: &WebGptLaneRuntime, client_name: &str) -> Result<Self> {
        let mut command = Command::new(wrapper);
        runtime.apply_to_command(&mut command);
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to start resident webgpt-mcp server: wrapper={}, lane={}, cdp_url={}, profile_dir={}",
                    wrapper.display(),
                    runtime.lane.as_str(),
                    runtime.cdp_url(),
                    runtime.profile_dir.display()
                )
            })?;
        let stdin = child
            .stdin
            .take()
            .context("resident webgpt-mcp stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("resident webgpt-mcp stdout unavailable")?;
        let mut client = Self {
            child,
            stdin,
            stdout_rx: spawn_resident_mcp_stdout_reader(stdout),
            next_id: 1,
            advertised_tools: None,
        };
        let initialize_id = client.next_jsonrpc_id();
        client.send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": initialize_id,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": client_name, "version": MCP_CLIENT_VERSION},
            },
        }))?;
        client
            .read_response(
                initialize_id,
                StdDuration::from_secs(WEBGPT_MCP_CONTROL_TIMEOUT_SECS),
            )
            .context("resident webgpt-mcp initialize failed")?;
        client.send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        }))?;
        Ok(client)
    }

    fn next_jsonrpc_id(&mut self) -> i64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn child_id(&self) -> u32 {
        self.child.id()
    }

    fn send_json(&mut self, payload: &Value) -> Result<()> {
        if let Some(status) = self.child.try_wait()? {
            anyhow::bail!("resident webgpt-mcp exited before request: status={status}");
        }
        serde_json::to_writer(&mut self.stdin, payload)
            .context("failed to serialize resident MCP payload")?;
        self.stdin
            .write_all(b"\n")
            .context("failed to terminate resident MCP payload line")?;
        self.stdin
            .flush()
            .context("failed to flush resident MCP payload")?;
        Ok(())
    }

    fn read_response(&mut self, expected_id: i64, timeout: StdDuration) -> Result<Value> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                anyhow::bail!(
                    "resident webgpt-mcp response timed out while waiting for id={expected_id}, timeout_secs={}",
                    timeout.as_secs()
                );
            };
            let line = match self.stdout_rx.recv_timeout(remaining) {
                Ok(Ok(line)) => line,
                Ok(Err(error)) => anyhow::bail!("{error}"),
                Err(RecvTimeoutError::Timeout) => {
                    anyhow::bail!(
                        "resident webgpt-mcp response timed out while waiting for id={expected_id}, timeout_secs={}",
                        timeout.as_secs()
                    );
                }
                Err(RecvTimeoutError::Disconnected) => {
                    anyhow::bail!(
                        "resident webgpt-mcp closed stdout while waiting for id={expected_id}"
                    );
                }
            };
            let trimmed = line.trim();
            let payload: Value = serde_json::from_str(trimmed).with_context(|| {
                format!(
                    "invalid resident MCP JSON response while waiting for id={expected_id}: {trimmed}"
                )
            })?;
            if payload.get("id").and_then(Value::as_i64) != Some(expected_id) {
                continue;
            }
            if let Some(error) = payload.get("error") {
                anyhow::bail!("resident MCP response id={expected_id} returned error: {error}");
            }
            return Ok(payload);
        }
    }

    fn ensure_tool_advertised(&mut self, tool_name: &str) -> Result<()> {
        if let Some(tools) = &self.advertised_tools {
            if tools.contains(tool_name) {
                return Ok(());
            }
            anyhow::bail!(
                "expected resident MCP tool {tool_name:?} not found in advertised tools {tools:?}"
            );
        }
        let request_id = self.next_jsonrpc_id();
        self.send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/list",
            "params": {},
        }))?;
        let response = self
            .read_response(
                request_id,
                StdDuration::from_secs(WEBGPT_MCP_CONTROL_TIMEOUT_SECS),
            )
            .context("resident webgpt-mcp tools/list failed")?;
        let tools = response
            .get("result")
            .and_then(|value| value.get("tools"))
            .and_then(Value::as_array)
            .context("resident webgpt-mcp tools/list response missing tools")?
            .iter()
            .filter_map(|item| item.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        if !tools.contains(tool_name) {
            anyhow::bail!(
                "expected resident MCP tool {tool_name:?} not found in advertised tools {tools:?}"
            );
        }
        self.advertised_tools = Some(tools);
        Ok(())
    }

    fn call_tool(
        &mut self,
        tool_name: &str,
        arguments: &Value,
        require_tool: bool,
    ) -> Result<PersistentMcpToolResult> {
        if require_tool {
            self.ensure_tool_advertised(tool_name)?;
        }
        let request_id = self.next_jsonrpc_id();
        self.send_json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments},
        }))?;
        let call_response = self
            .read_response(request_id, resident_mcp_tool_timeout(arguments))
            .context("resident webgpt-mcp tools/call failed")?;
        let first_text = call_response
            .get("result")
            .and_then(|value| value.get("content"))
            .and_then(Value::as_array)
            .and_then(|items| {
                items.iter().find_map(|item| {
                    if item.get("type").and_then(Value::as_str) == Some("text") {
                        item.get("text").and_then(Value::as_str)
                    } else {
                        None
                    }
                })
            })
            .context("resident webgpt-mcp tool call returned no text content")?
            .to_owned();
        Ok(PersistentMcpToolResult {
            pid: self.child_id(),
            first_text,
        })
    }
}

fn spawn_resident_mcp_stdout_reader(stdout: ChildStdout) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(Ok(line)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Err(format!(
                        "failed to read resident MCP response: {error}"
                    )));
                    break;
                }
            }
        }
    });
    rx
}

fn resident_mcp_tool_timeout(arguments: &Value) -> StdDuration {
    let timeout_secs = arguments
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(WEBGPT_MCP_CONTROL_TIMEOUT_SECS)
        .max(5)
        + WEBGPT_DISPATCH_STALE_CUSHION_SECS;
    StdDuration::from_secs(timeout_secs)
}

impl Drop for PersistentMcpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn persistent_mcp_client_key(wrapper: &Path, runtime: &WebGptLaneRuntime) -> String {
    format!(
        "{}|{}|{}|{}",
        wrapper.display(),
        runtime.lane.as_str(),
        runtime.cdp_url(),
        runtime.profile_dir.display()
    )
}

fn call_resident_mcp_tool(
    wrapper: &Path,
    runtime: &WebGptLaneRuntime,
    client_name: &str,
    tool_name: &str,
    arguments: &Value,
    require_tool: bool,
) -> Result<PersistentMcpToolResult> {
    WEBGPT_MCP_CLIENTS.with(|clients| {
        let mut clients = clients.borrow_mut();
        let key = persistent_mcp_client_key(wrapper, runtime);
        if !clients.contains_key(key.as_str()) {
            let client = PersistentMcpClient::spawn(wrapper, runtime, client_name)?;
            clients.insert(key.clone(), client);
        }
        let result = clients
            .get_mut(key.as_str())
            .context("resident webgpt-mcp client missing after insert")?
            .call_tool(tool_name, arguments, require_tool);
        if result.is_err() {
            clients.remove(key.as_str());
        }
        result
    })
}

pub(super) fn prewarm_webgpt_lane_sessions(
    options: &HostWorkerOptions,
    include_text: bool,
    include_visual: bool,
) -> Result<Vec<WebGptLanePrewarmRecord>> {
    let wrapper = resolve_webgpt_mcp_wrapper(options)?;
    let mut lanes = Vec::new();
    if include_text {
        lanes.push((
            "text",
            WebGptLaneRuntime::new(WebGptConversationLane::Text, options)?,
        ));
    }
    if include_visual {
        lanes.push((
            "turn_cg_image",
            WebGptLaneRuntime::new_image(WebGptImageSessionKind::TurnCg, options)?,
        ));
        lanes.push((
            "reference_image",
            WebGptLaneRuntime::new_image(WebGptImageSessionKind::ReferenceAsset, options)?,
        ));
    }

    lanes
        .into_iter()
        .map(|(lane, runtime)| prewarm_webgpt_lane_session(wrapper.as_path(), lane, &runtime))
        .collect()
}

fn prewarm_webgpt_lane_session(
    wrapper: &Path,
    lane: &'static str,
    runtime: &WebGptLaneRuntime,
) -> Result<WebGptLanePrewarmRecord> {
    if lane == "text" {
        call_resident_mcp_tool(
            wrapper,
            runtime,
            "singulari-world-webgpt-prewarm-text",
            "webgpt_health",
            &serde_json::json!({"probe_live": false, "auto_recover": false}),
            true,
        )
        .with_context(|| {
            format!(
                "failed to prewarm resident WebGPT text lane: wrapper={}, cdp_url={}, profile_dir={}",
                wrapper.display(),
                runtime.cdp_url(),
                runtime.profile_dir.display()
            )
        })?;
        return Ok(WebGptLanePrewarmRecord {
            lane,
            cdp_port: runtime.cdp_port,
            cdp_url: runtime.cdp_url(),
            profile_dir: runtime.profile_dir.display().to_string(),
            status: "ready",
            exit_code: None,
        });
    }
    let mut command = Command::new(wrapper);
    runtime.apply_to_command(&mut command);
    let output = command
        .arg("client-call")
        .arg("--wrapper")
        .arg(wrapper)
        .arg("--client-name")
        .arg(format!("singulari-world-webgpt-prewarm-{lane}"))
        .arg("--require-tool")
        .arg("--tool")
        .arg("webgpt_health")
        .arg("--arguments")
        .arg(r#"{"probe_live":false,"auto_recover":false}"#)
        .arg("--output")
        .arg("first-text")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| {
            format!(
                "failed to prewarm WebGPT lane: lane={lane}, wrapper={}, cdp_url={}, profile_dir={}",
                wrapper.display(),
                runtime.cdp_url(),
                runtime.profile_dir.display()
            )
        })?;
    if !output.status.success() {
        anyhow::bail!(
            "WebGPT lane prewarm failed: lane={lane}, cdp_url={}, profile_dir={}, exit_code={:?}, stderr={}, stdout={}",
            runtime.cdp_url(),
            runtime.profile_dir.display(),
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    Ok(WebGptLanePrewarmRecord {
        lane,
        cdp_port: runtime.cdp_port,
        cdp_url: runtime.cdp_url(),
        profile_dir: runtime.profile_dir.display().to_string(),
        status: "ready",
        exit_code: output.status.code(),
    })
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

    const fn timeout_secs(&self) -> u64 {
        match self {
            Self::ExternalCommand { .. } => DEFAULT_WEBGPT_TIMEOUT_SECS,
            Self::McpResearch { timeout_secs, .. } => *timeout_secs,
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
        .env(
            "SINGULARI_WORLD_PROMPT_CONTEXT_PATH",
            paths.prompt_context_path,
        )
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
    let tool_result = match call_resident_mcp_tool(
        wrapper,
        runtime,
        "singulari-world-webgpt-turn",
        "webgpt_research",
        &arguments,
        true,
    ) {
        Ok(result) => result,
        Err(error) => {
            fs::write(paths.stdout_path, b"")
                .with_context(|| format!("failed to write {}", paths.stdout_path.display()))?;
            fs::write(
                paths.stderr_path,
                format!("resident webgpt-mcp call failed: {error:#}").as_bytes(),
            )
            .with_context(|| format!("failed to write {}", paths.stderr_path.display()))?;
            return Ok(WebGptDispatchResult {
                success: false,
                pid: 0,
                exit_code: None,
                raw_conversation_id: None,
                current_model: None,
                current_reasoning_level: None,
                error: Some(format!("{error:#}")),
            });
        }
    };
    let server_pid = tool_result.pid;
    fs::write(paths.stdout_path, tool_result.first_text.as_bytes())
        .with_context(|| format!("failed to write {}", paths.stdout_path.display()))?;
    fs::write(
        paths.stderr_path,
        b"resident webgpt-mcp client reused; server stderr is inherited by host-worker stderr\n",
    )
    .with_context(|| format!("failed to write {}", paths.stderr_path.display()))?;
    let raw_result = tool_result.first_text;
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
        pid: server_pid,
        exit_code: None,
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
        "timeout_secs": timeout_secs.max(5),
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

fn commit_webgpt_agent_response(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    response_path: &Path,
) -> Result<singulari_world::CommittedAgentTurn> {
    let raw_body = fs::read_to_string(response_path)
        .with_context(|| format!("failed to read {}", response_path.display()))?;
    let response = parse_webgpt_agent_turn_response(store_root, pending, &raw_body, response_path)?;
    commit_agent_turn(&AgentCommitTurnOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id: pending.world_id.clone(),
        response,
    })
}

fn parse_webgpt_agent_turn_response(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    raw_body: &str,
    response_path: &Path,
) -> Result<AgentTurnResponse> {
    let mut value = serde_json::from_str::<Value>(raw_body)
        .with_context(|| format!("failed to parse {}", response_path.display()))?;
    let reference_aliases = webgpt_reference_aliases(store_root, pending);
    normalize_webgpt_agent_turn_response(&mut value, &reference_aliases);
    serde_json::from_value::<AgentTurnResponse>(value)
        .with_context(|| format!("failed to parse {}", response_path.display()))
}

fn webgpt_reference_aliases(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
) -> HashMap<String, String> {
    let Ok(prompt_context) = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root,
        pending,
        engine_session_kind: "webgpt_project_session",
    }) else {
        return HashMap::new();
    };
    prompt_context
        .pre_turn_simulation
        .available_affordances
        .into_iter()
        .flat_map(|affordance| {
            let canonical = affordance.affordance_id;
            let aliases = [
                canonical.replace(":___:", "::"),
                canonical.replace(":__:", "::"),
            ];
            aliases
                .into_iter()
                .filter(|alias| alias != &canonical)
                .map(|alias| (alias, canonical.clone()))
                .collect::<Vec<_>>()
        })
        .collect()
}

fn normalize_webgpt_agent_turn_response(
    value: &mut Value,
    reference_aliases: &HashMap<String, String>,
) {
    normalize_reference_values(value, reference_aliases);
    let choice_seeds = collect_webgpt_choice_seeds(value);
    let Some(response) = value.as_object_mut() else {
        return;
    };
    normalize_visible_scene(response);
    let Some(resolution) = response
        .get_mut("resolution_proposal")
        .and_then(Value::as_object_mut)
    else {
        return;
    };

    if let Some(intent) = resolution
        .get_mut("interpreted_intent")
        .and_then(Value::as_object_mut)
    {
        normalize_string_enum_field(intent, "input_kind", normalize_action_input_kind);
        normalize_string_enum_field(intent, "ambiguity", normalize_action_ambiguity);
    }
    if let Some(outcome) = resolution.get_mut("outcome").and_then(Value::as_object_mut) {
        normalize_string_enum_field(outcome, "kind", normalize_resolution_outcome_kind);
    }
    normalize_gate_results(resolution);
    normalize_proposed_effects(resolution);
    normalize_choice_plan(resolution, &choice_seeds);
    normalize_plot_thread_events(response);
    normalize_scene_pressure_events(response);
    normalize_location_events(response);
}

fn normalize_visible_scene(response: &mut serde_json::Map<String, Value>) {
    let Some(scene) = response
        .get_mut("visible_scene")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if let Some(note) = scene.get("tone_notes").and_then(Value::as_str) {
        scene.insert(
            "tone_notes".to_owned(),
            Value::Array(vec![Value::String(note.to_owned())]),
        );
    }
}

fn normalize_reference_values(value: &mut Value, reference_aliases: &HashMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(canonical) = reference_aliases.get(text.as_str()) {
                text.clone_from(canonical);
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_reference_values(item, reference_aliases);
            }
        }
        Value::Object(map) => {
            for child in map.values_mut() {
                normalize_reference_values(child, reference_aliases);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn collect_webgpt_choice_seeds(value: &Value) -> HashMap<u8, (String, String)> {
    value
        .get("next_choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|choice| {
            let slot = u8::try_from(choice.get("slot")?.as_u64()?).ok()?;
            let tag = choice
                .get("tag")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let intent = choice
                .get("intent")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Some((slot, (tag, intent)))
        })
        .collect()
}

fn normalize_gate_results(resolution: &mut serde_json::Map<String, Value>) {
    let Some(gates) = resolution
        .get_mut("gate_results")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for gate in gates {
        let Some(gate) = gate.as_object_mut() else {
            continue;
        };
        if !gate.contains_key("gate_kind") {
            let inferred = gate
                .get("gate_ref")
                .and_then(Value::as_str)
                .map_or("knowledge", infer_gate_kind);
            gate.insert("gate_kind".to_owned(), Value::String(inferred.to_owned()));
        }
        ensure_string_field(gate, "visibility", "player_visible");
        if !gate.contains_key("status") {
            let status = if gate.get("passed").and_then(Value::as_bool) == Some(false) {
                "blocked"
            } else {
                "passed"
            };
            gate.insert("status".to_owned(), Value::String(status.to_owned()));
        }
        if !gate.contains_key("reason") {
            let reason = gate
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or("gate evaluated");
            gate.insert("reason".to_owned(), Value::String(reason.to_owned()));
        }
    }
}

fn normalize_proposed_effects(resolution: &mut serde_json::Map<String, Value>) {
    let Some(effects) = resolution
        .get_mut("proposed_effects")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for effect in effects {
        let Some(effect) = effect.as_object_mut() else {
            continue;
        };
        if !effect.contains_key("effect_kind") {
            let inferred = effect
                .get("kind")
                .and_then(Value::as_str)
                .map_or("scene_pressure_delta", infer_effect_kind);
            effect.insert("effect_kind".to_owned(), Value::String(inferred.to_owned()));
        }
        ensure_string_field(effect, "visibility", "player_visible");
    }
}

fn normalize_choice_plan(
    resolution: &mut serde_json::Map<String, Value>,
    choice_seeds: &HashMap<u8, (String, String)>,
) {
    let Some(choices) = resolution
        .get_mut("next_choice_plan")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for choice in choices {
        let Some(choice) = choice.as_object_mut() else {
            continue;
        };
        let slot = choice
            .get("slot")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(0);
        if !choice.contains_key("plan_kind") {
            let plan_kind = choice.get("kind").and_then(Value::as_str).map_or_else(
                || match slot {
                    6 => "freeform",
                    7 => "delegated_judgment",
                    _ => "ordinary_affordance",
                },
                normalize_choice_plan_kind,
            );
            choice.insert("plan_kind".to_owned(), Value::String(plan_kind.to_owned()));
        }
        if !choice.contains_key("label_seed") {
            let label = choice_seeds
                .get(&slot)
                .map_or("행동", |(tag, _)| tag.as_str());
            choice.insert("label_seed".to_owned(), Value::String(label.to_owned()));
        }
        if !choice.contains_key("intent_seed") {
            let intent = choice
                .get("summary")
                .and_then(Value::as_str)
                .or_else(|| choice_seeds.get(&slot).map(|(_, intent)| intent.as_str()))
                .unwrap_or("다음 행동을 정한다.");
            choice.insert("intent_seed".to_owned(), Value::String(intent.to_owned()));
        }
    }
}

fn normalize_plot_thread_events(response: &mut serde_json::Map<String, Value>) {
    let Some(events) = response
        .get_mut("plot_thread_events")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for event in events {
        let Some(event) = event.as_object_mut() else {
            continue;
        };
        move_string_field(event, "thread_ref", "thread_id");
        if !event.contains_key("change") {
            let change = event
                .get("kind")
                .and_then(Value::as_str)
                .map_or("advanced", normalize_plot_thread_change);
            event.insert("change".to_owned(), Value::String(change.to_owned()));
        }
        ensure_string_field(event, "status_after", "active");
        ensure_string_field(event, "urgency_after", "soon");
    }
}

fn normalize_scene_pressure_events(response: &mut serde_json::Map<String, Value>) {
    let Some(events) = response
        .get_mut("scene_pressure_events")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for event in events {
        let Some(event) = event.as_object_mut() else {
            continue;
        };
        move_string_field(event, "pressure_ref", "pressure_id");
        if !event.contains_key("change") {
            let change = event
                .get("kind")
                .and_then(Value::as_str)
                .map_or("redirected", normalize_scene_pressure_change);
            event.insert("change".to_owned(), Value::String(change.to_owned()));
        }
        if !event.contains_key("intensity_after") {
            event.insert("intensity_after".to_owned(), Value::Number(2.into()));
        }
        ensure_string_field(event, "urgency_after", "soon");
    }
}

fn normalize_location_events(response: &mut serde_json::Map<String, Value>) {
    let Some(events) = response
        .get_mut("location_events")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for event in events {
        let Some(event) = event.as_object_mut() else {
            continue;
        };
        move_string_field(event, "target_ref", "location_id");
        if !event.contains_key("event_kind") {
            let kind = event
                .get("kind")
                .and_then(Value::as_str)
                .map_or("visited", normalize_location_event_kind);
            event.insert("event_kind".to_owned(), Value::String(kind.to_owned()));
        }
        ensure_string_field(event, "name", "미정");
        ensure_string_field(event, "knowledge_state", "visited");
    }
}

fn ensure_string_field(map: &mut serde_json::Map<String, Value>, key: &str, value: &str) {
    map.entry(key.to_owned())
        .or_insert_with(|| Value::String(value.to_owned()));
}

fn move_string_field(map: &mut serde_json::Map<String, Value>, from: &str, to: &str) {
    if map.contains_key(to) {
        return;
    }
    if let Some(value) = map.get(from).and_then(Value::as_str) {
        map.insert(to.to_owned(), Value::String(value.to_owned()));
    }
}

fn normalize_string_enum_field(
    map: &mut serde_json::Map<String, Value>,
    key: &str,
    normalize: fn(&str) -> &'static str,
) {
    let Some(current) = map.get(key).and_then(Value::as_str) else {
        return;
    };
    map.insert(key.to_owned(), Value::String(normalize(current).to_owned()));
}

fn normalize_action_input_kind(value: &str) -> &'static str {
    match value {
        "presented_choice" | "numeric_choice" | "macro_time_flow" | "cc_canvas" => {
            "presented_choice"
        }
        "delegated_judgment" | "guide_choice" => "delegated_judgment",
        "codex_query" => "codex_query",
        _ => "freeform",
    }
}

fn normalize_action_ambiguity(value: &str) -> &'static str {
    match value {
        "clear" => "clear",
        "high" => "high",
        _ => "minor",
    }
}

fn normalize_resolution_outcome_kind(value: &str) -> &'static str {
    match value {
        "success" => "success",
        "partial_success" => "partial_success",
        "blocked" => "blocked",
        "costly_success" => "costly_success",
        "delayed" => "delayed",
        "escalated" => "escalated",
        other if other.contains("blocked") || other.contains("failed") => "blocked",
        other if other.contains("delay") => "delayed",
        other if other.contains("cost") => "costly_success",
        _ => "success",
    }
}

fn infer_gate_kind(value: &str) -> &'static str {
    if value.contains("body") {
        "body"
    } else if value.contains("resource") {
        "resource"
    } else if value.contains("location") || value.contains("place") {
        "location"
    } else if value.contains("social") {
        "social_permission"
    } else if value.contains("time") {
        "time_pressure"
    } else if value.contains("hidden") {
        "hidden_constraint"
    } else if value.contains("affordance") {
        "affordance"
    } else {
        "knowledge"
    }
}

fn infer_effect_kind(value: &str) -> &'static str {
    if value.contains("body") || value.contains("resource") {
        "body_resource_delta"
    } else if value.contains("location") || value.contains("place") {
        "location_delta"
    } else if value.contains("relationship") {
        "relationship_delta"
    } else if value.contains("belief") || value.contains("question") {
        "belief_delta"
    } else if value.contains("world_lore") || value.contains("lore") {
        "world_lore_delta"
    } else if value.contains("pattern") {
        "pattern_debt"
    } else if value.contains("intent") {
        "player_intent_trace"
    } else {
        "scene_pressure_delta"
    }
}

fn normalize_choice_plan_kind(value: &str) -> &'static str {
    match value {
        "freeform" => "freeform",
        "delegated_judgment" | "guide_choice" => "delegated_judgment",
        _ => "ordinary_affordance",
    }
}

fn normalize_plot_thread_change(value: &str) -> &'static str {
    match value {
        "complicated" => "complicated",
        "softened" | "preserve" | "preserved" => "softened",
        "blocked" => "blocked",
        "resolved" => "resolved",
        "failed" => "failed",
        "retired" => "retired",
        _ => "advanced",
    }
}

fn normalize_scene_pressure_change(value: &str) -> &'static str {
    match value {
        "surfaced" => "surfaced",
        "increased" => "increased",
        "softened" => "softened",
        "resolved" => "resolved",
        _ => "redirected",
    }
}

fn normalize_location_event_kind(value: &str) -> &'static str {
    match value {
        "discovered" => "discovered",
        "route_opened" => "route_opened",
        "route_blocked" => "route_blocked",
        "visited" | "establish_visible" | "established" => "visited",
        _ => "updated",
    }
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

fn dispatch_dir_for_pending(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
) -> Result<PathBuf> {
    let paths = singulari_world::resolve_store_paths(store_root)?;
    Ok(paths
        .worlds_dir
        .join(pending.world_id.as_str())
        .join("agent_bridge")
        .join("dispatches"))
}

fn write_dispatch_claim(path: &Path, claim: &serde_json::Value) -> Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(serde_json::to_vec_pretty(claim)?.as_slice())?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            if existing_dispatch_is_retryable(path)? {
                fs::write(path, serde_json::to_vec_pretty(claim)?)
                    .with_context(|| format!("failed to replace retryable {}", path.display()))?;
                return Ok(true);
            }
            Ok(false)
        }
        Err(error) => Err(error).with_context(|| format!("failed to create {}", path.display())),
    }
}

pub(super) fn existing_dispatch_is_retryable(path: &Path) -> Result<bool> {
    let Some(value) = read_json_value_if_present(path)? else {
        return Ok(false);
    };
    let status = value.get("status").and_then(serde_json::Value::as_str);
    let error = value
        .get("error")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let repair_attempts = value
        .get("repair_attempts")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(
        (matches!(status, Some("failed")) && !is_webgpt_timeout_signal(error))
            || (matches!(status, Some("dispatching")) && dispatch_record_is_stale(&value)?)
            || (matches!(status, Some("commit_failed"))
                && repair_attempts == 0
                && is_repairable_webgpt_commit_error(error)),
    )
}

pub(super) fn is_webgpt_timeout_signal(error: &str) -> bool {
    let normalized = error.to_ascii_lowercase();
    normalized.contains("timed out")
        || normalized.contains("timeout")
        || normalized.contains("deadline exceeded")
}

fn dispatch_record_is_stale(value: &serde_json::Value) -> Result<bool> {
    let Some(dispatched_at) = value
        .get("dispatched_at")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(false);
    };
    let dispatched_at = DateTime::parse_from_rfc3339(dispatched_at)
        .with_context(|| format!("invalid dispatch timestamp: {dispatched_at}"))?
        .with_timezone(&Utc);
    let retry_after_secs = dispatch_retry_after_secs(value)?;
    Ok(
        Utc::now().signed_duration_since(dispatched_at)
            >= ChronoDuration::seconds(retry_after_secs),
    )
}

fn dispatch_retry_after_secs(value: &serde_json::Value) -> Result<i64> {
    let timeout_secs = value
        .get("timeout_secs")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(DEFAULT_WEBGPT_TIMEOUT_SECS)
        .max(5)
        .saturating_add(WEBGPT_DISPATCH_STALE_CUSHION_SECS);
    i64::try_from(timeout_secs).context("webgpt dispatch timeout_secs exceeds i64")
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
        AgentSubmitTurnOptions, CompilePromptContextPacketOptions, ImageGenerationJob,
        InitWorldOptions, VisualArtifactKind, compile_prompt_context_packet, enqueue_agent_turn,
        init_world,
    };

    #[test]
    fn dispatch_claim_replaces_failed_record_for_retry() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0001-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "failed",
                "turn_id": "turn_0001",
            }))?,
        )?;

        let claim = serde_json::json!({
            "schema_version": "singulari.webgpt_dispatch_record.v1",
            "status": "dispatching",
            "turn_id": "turn_0001",
            "attempt": "retry",
        });

        assert!(write_dispatch_claim(&path, &claim)?);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(value["status"], serde_json::json!("dispatching"));
        assert_eq!(value["attempt"], serde_json::json!("retry"));
        Ok(())
    }

    #[test]
    fn timeout_dispatch_failures_are_not_reprompted() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0001-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "failed",
                "turn_id": "turn_0001",
                "error": "worker request 'collect_answer' timed out after 900s",
            }))?,
        )?;

        let claim = serde_json::json!({
            "schema_version": "singulari.webgpt_dispatch_record.v1",
            "status": "dispatching",
            "turn_id": "turn_0001",
            "attempt": "retry",
        });

        assert!(!write_dispatch_claim(&path, &claim)?);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(value["status"], serde_json::json!("failed"));
        Ok(())
    }

    #[test]
    fn dispatch_claim_keeps_non_retryable_record() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0001-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "completed",
                "turn_id": "turn_0001",
            }))?,
        )?;

        let claim = serde_json::json!({
            "schema_version": "singulari.webgpt_dispatch_record.v1",
            "status": "dispatching",
            "turn_id": "turn_0001",
        });

        assert!(!write_dispatch_claim(&path, &claim)?);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(value["status"], serde_json::json!("completed"));
        Ok(())
    }

    #[test]
    fn dispatch_claim_replaces_repairable_commit_failed_record() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0001-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "commit_failed",
                "turn_id": "turn_0001",
                "error": "resolution proposal audit failed: failure_kind=TargetRef, message=resolution proposal references an unknown or forbidden ref",
            }))?,
        )?;

        let claim = serde_json::json!({
            "schema_version": "singulari.webgpt_dispatch_record.v1",
            "status": "dispatching",
            "turn_id": "turn_0001",
            "attempt": "retry",
        });

        assert!(write_dispatch_claim(&path, &claim)?);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(value["status"], serde_json::json!("dispatching"));
        assert_eq!(value["attempt"], serde_json::json!("retry"));
        Ok(())
    }

    #[test]
    fn parse_schema_commit_failures_are_repairable() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0002-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "commit_failed",
                "turn_id": "turn_0002",
                "error": "failed to parse /tmp/turn_0002-webgpt-agent-response.json: missing field `cause` at line 1 column 4061",
            }))?,
        )?;

        assert!(existing_dispatch_is_retryable(&path)?);
        Ok(())
    }

    #[test]
    fn repaired_commit_failures_are_not_reprompted() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0002-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "commit_failed",
                "turn_id": "turn_0002",
                "repair_attempts": 1,
                "error": "failed to parse /tmp/turn_0002-webgpt-agent-response.json: unknown variant `freeform_action`",
            }))?,
        )?;

        assert!(!existing_dispatch_is_retryable(&path)?);
        Ok(())
    }

    #[test]
    fn dispatch_claim_replaces_stale_dispatching_record() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0001-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "dispatching",
                "turn_id": "turn_0001",
                "dispatched_at": (Utc::now() - ChronoDuration::minutes(21)).to_rfc3339(),
            }))?,
        )?;

        let claim = serde_json::json!({
            "schema_version": "singulari.webgpt_dispatch_record.v1",
            "status": "dispatching",
            "turn_id": "turn_0001",
            "attempt": "retry",
        });

        assert!(write_dispatch_claim(&path, &claim)?);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(value["status"], serde_json::json!("dispatching"));
        assert_eq!(value["attempt"], serde_json::json!("retry"));
        Ok(())
    }

    #[test]
    fn dispatch_claim_keeps_dispatching_record_inside_timeout_lease() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("turn_0001-webgpt.json");
        fs::write(
            &path,
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": "singulari.webgpt_dispatch_record.v1",
                "status": "dispatching",
                "turn_id": "turn_0001",
                "timeout_secs": 900,
                "dispatched_at": (Utc::now() - ChronoDuration::seconds(120)).to_rfc3339(),
            }))?,
        )?;

        let claim = serde_json::json!({
            "schema_version": "singulari.webgpt_dispatch_record.v1",
            "status": "dispatching",
            "turn_id": "turn_0001",
            "attempt": "retry",
        });

        assert!(!write_dispatch_claim(&path, &claim)?);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
        assert_eq!(value["attempt"], serde_json::Value::Null);
        Ok(())
    }

    #[test]
    fn dispatch_dir_uses_explicit_store_root() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("explicit-store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_dispatch_root
title: "dispatch root test"
premise:
  genre: "fantasy"
  protagonist: "scout"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let mut pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_dispatch_root".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        pending.pending_ref =
            ".world-store/worlds/stw_dispatch_root/agent_bridge/pending_turn.json".to_owned();

        assert_eq!(
            dispatch_dir_for_pending(Some(store.as_path()), &pending)?,
            store
                .join("worlds")
                .join("stw_dispatch_root")
                .join("agent_bridge")
                .join("dispatches")
        );
        Ok(())
    }

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
        assert_eq!(arguments["timeout_secs"], serde_json::json!(12));
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
        let prompt_context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
            store_root: Some(store.as_path()),
            pending: &pending,
            engine_session_kind: "webgpt_project_session",
        })?;
        let prompt = build_webgpt_turn_prompt(&prompt_context)?;

        for required in [
            "너는 Singulari World의 trusted narrative agent다.",
            "출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고",
            "이 계약은 seedless style contract다.",
            "문체/작법 규칙은 소재, 사건, 인물, 장소, 장르 장치, 과거사, 상징을 새로 만들 권한이 없다.",
            "scene_fact_boundaries: 오직 narrative turn packet의 player-visible facts",
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
            "narrative_turn_packet.opening_randomizer가 있으면 사용자의 시드에 덧붙은 player-visible 개막 seed로 취급한다.",
            "opening_randomizer가 없으면 사용자 시드와 visible facts만으로 시작한다.",
            "opening_randomizer는 반복 수렴을 피하기 위한 시작 조건이지, 시드에 없는 장르 장치·숨은 과거사·고정 인물 설정을 만드는 권한이 아니다.",
            "시드나 visible facts에 명시되지 않은 장르 장치, 과거사, 외부 세계 대비, 게임 인터페이스식 능력 구조를 추론해서 주입하지 마라.",
            "protagonist가 현재 정보를 모른다는 사실만으로 장면 밖 배경, 과거사, 시대 대비 독백, 정체성 상실 클리셰를 만들지 마라.",
            "이 WebGPT conversation의 이전 turn들은 말맛, 직전 감정선, 장면 리듬을 잇는 working context다.",
            "conversation/project context가 compact 되었거나 narrative turn packet과 충돌하면 narrative turn packet을 우선한다.",
            "narrative_turn_packet.visible_context.active_character_text_design과 selected_memory_items는 말맛과 가까운 연속성에만 쓴다.",
            "narrative_turn_packet.visible_context.affordance_graph와 pre_turn_simulation.available_affordances는 slot 1..5의 행동 허가표다.",
            "narrative_turn_packet.adjudication_boundary는 판정 전용이다.",
            "웹 검색, 외부 사이트 탐색, repo 탐색, 소스 파일 읽기를 하지 마라.",
            "allowed_reference_atoms JSON:",
            "`target_refs`, `pressure_refs`, `evidence_refs`, `gate_ref`",
            "정확한 ref가 없으면 `current_turn`, `player_input`, 선택된",
            "\"schema_version\":\"singulari.narrative_turn_packet.v1\"",
            "\"pre_turn_simulation\"",
            "\"available_affordances\"",
            "\"pressure_obligations\"",
            "\"hidden_visibility_boundary\"",
            "\"visible_context\"",
            "\"adjudication_boundary\"",
            "\"selected_memory_items\"",
            "\"affordance_graph\"",
            "\"narrative_style_state\"",
            "\"anti_translation_rules\"",
            "\"prohibited_seed_leakage\"",
            "\"active_scene_pressure\"",
            "\"active_plot_threads\"",
            "\"plot_thread_events\"",
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
            "\"conflict_rule\":\"revival_packet_wins\"",
            "AgentTurnResponse JSON만 반환한다.",
            "schema_version=\"singulari.agent_turn_response.v1\"",
            "slot 6 tag=\"자유서술\"",
            "slot 7 tag=\"판단 위임\"",
            "world_id는 \"stw_contract\", turn_id는 \"turn_0001\"와 정확히 같아야 한다.",
        ] {
            assert!(
                prompt.contains(required),
                "webgpt prompt missing realtime contract: {required}"
            );
        }

        let allowed_marker = "allowed_reference_atoms JSON:";
        let allowed_marker_index = prompt
            .find(allowed_marker)
            .context("allowed reference atoms marker missing")?;
        let after_allowed_marker = &prompt[allowed_marker_index + allowed_marker.len()..];
        let allowed_fence_start = after_allowed_marker
            .find("```json")
            .context("allowed reference atoms JSON fence start missing")?;
        let after_allowed_fence = &after_allowed_marker[allowed_fence_start + "```json".len()..];
        let allowed_fence_end = after_allowed_fence
            .find("```")
            .context("allowed reference atoms JSON fence end missing")?;
        let allowed_refs: Vec<String> =
            serde_json::from_str(after_allowed_fence[..allowed_fence_end].trim())?;
        assert!(allowed_refs.iter().any(|value| value == "current_turn"));
        assert!(allowed_refs.iter().any(|value| value == "player_input"));
        assert!(allowed_refs.iter().all(|value| {
            !value.chars().any(char::is_whitespace)
                && !value.starts_with('/')
                && !value.starts_with("singulari.")
        }));

        let marker = "narrative turn packet JSON:";
        let marker_index = prompt
            .find(marker)
            .context("narrative turn packet marker missing")?;
        let after_marker = &prompt[marker_index + marker.len()..];
        let fence_start = after_marker
            .find("```json")
            .context("narrative turn packet JSON fence start missing")?;
        let after_fence = &after_marker[fence_start + "```json".len()..];
        let fence_end = after_fence
            .find("```")
            .context("narrative turn packet JSON fence end missing")?;
        let narrative_packet: serde_json::Value =
            serde_json::from_str(after_fence[..fence_end].trim())?;
        assert_eq!(
            narrative_packet["schema_version"],
            serde_json::json!("singulari.narrative_turn_packet.v1")
        );
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
            "/visible_context/active_change_ledger",
            "/visible_context/active_pattern_debt",
            "/visible_context/active_belief_graph",
            "/visible_context/active_world_process_clock",
            "/visible_context/active_player_intent_trace",
            "/visible_context/active_turn_retrieval_controller",
            "/visible_context/selected_context_capsules",
            "/visible_context/active_autobiographical_index",
        ] {
            assert!(
                narrative_packet.pointer(omitted_path).is_none(),
                "narrative turn packet leaked debug/source path: {omitted_path}"
            );
        }
        Ok(())
    }
}
