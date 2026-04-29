use anyhow::{Context, Result};
use clap::ValueEnum;
use singulari_world::{
    ACTIVE_WORLD_FILENAME, BuildVnPacketOptions, ClaimVisualJobOptions, ImageGenerationJob,
    ReleaseVisualJobClaimOptions, WorldTextBackend, WorldVisualBackend, build_vn_packet,
    claim_visual_job, load_active_world, load_pending_agent_turn, load_visual_job_claim,
    load_world_backend_selection, release_visual_job_claim, resolve_store_paths, resolve_world_id,
};
use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use super::webgpt::{
    WebGptDispatchOutcome, WebGptDispatchRecord, WebGptImageDispatchRecord, WebGptImageSessionKind,
    dispatch_pending_agent_turn_via_webgpt, dispatch_visual_job_via_webgpt,
    ensure_webgpt_lane_runtime_isolated, prewarm_webgpt_lane_sessions, safe_file_component,
    visual_dispatch_dir_for_world,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum HostWorkerTextBackend {
    /// Use `WebGPT` as the narrative engine.
    Webgpt,
}

impl HostWorkerTextBackend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Webgpt => "webgpt",
        }
    }
}

impl fmt::Display for HostWorkerTextBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<WorldTextBackend> for HostWorkerTextBackend {
    fn from(value: WorldTextBackend) -> Self {
        match value {
            WorldTextBackend::Webgpt => Self::Webgpt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum HostWorkerVisualBackend {
    /// Use `ChatGPT` Web image generation through `WebGPT` MCP.
    Webgpt,
    /// Do not claim or generate visual jobs from this worker.
    None,
}

impl HostWorkerVisualBackend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Webgpt => "webgpt",
            Self::None => "none",
        }
    }
}

impl fmt::Display for HostWorkerVisualBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<WorldVisualBackend> for HostWorkerVisualBackend {
    fn from(value: WorldVisualBackend) -> Self {
        match value {
            WorldVisualBackend::Webgpt => Self::Webgpt,
        }
    }
}

const HOST_WORKER_EVENT_SCHEMA_VERSION: &str = "singulari.host_worker_event.v1";
const HOST_WORKER_CONSUMER: &str = "webgpt_host_worker";
#[derive(Debug, Clone)]
pub(crate) struct HostWorkerOptions {
    pub(crate) interval_ms: u64,
    pub(crate) once: bool,
    pub(crate) text_backend: HostWorkerTextBackend,
    pub(crate) visual_backend: HostWorkerVisualBackend,
    pub(crate) webgpt_turn_command: Option<PathBuf>,
    pub(crate) webgpt_mcp_wrapper: Option<PathBuf>,
    pub(crate) webgpt_model: Option<String>,
    pub(crate) webgpt_reasoning_level: Option<String>,
    pub(crate) webgpt_text_profile_dir: Option<PathBuf>,
    pub(crate) webgpt_image_profile_dir: Option<PathBuf>,
    pub(crate) webgpt_reference_image_profile_dir: Option<PathBuf>,
    pub(crate) webgpt_text_cdp_port: u16,
    pub(crate) webgpt_image_cdp_port: u16,
    pub(crate) webgpt_reference_image_cdp_port: u16,
    pub(crate) webgpt_timeout_secs: u64,
}

#[allow(
    clippy::too_many_lines,
    reason = "Host worker loop keeps backend resolution, startup events, and idle behavior visible at the CLI boundary"
)]
pub(crate) fn handle_host_worker(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    options: &HostWorkerOptions,
) -> Result<()> {
    let interval = Duration::from_millis(options.interval_ms.max(250));
    ensure_webgpt_lane_runtime_isolated(options)?;
    let mut emitted = HashSet::new();
    let initial_world_id = resolve_host_worker_world_id(store_root, world_id)?;
    let (initial_text_backend, initial_visual_backend) =
        if let Some(initial_world_id) = initial_world_id.as_deref() {
            effective_host_worker_backends(store_root, initial_world_id, options)?
        } else {
            (options.text_backend, options.visual_backend)
        };
    emit_host_event(&serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "worker_started",
        "world_id": initial_world_id.as_deref(),
        "text_backend": initial_text_backend.as_str(),
        "visual_backend": initial_visual_backend.as_str(),
        "requested_text_backend": options.text_backend.as_str(),
        "requested_visual_backend": options.visual_backend.as_str(),
        "visual_jobs": host_worker_visual_jobs_label(initial_visual_backend),
        "consumer": HOST_WORKER_CONSUMER,
    }))?;

    loop {
        let Some(world_id) = resolve_host_worker_world_id(store_root, world_id)? else {
            if emitted.insert("worker-waiting-for-active-world".to_owned()) {
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "worker_waiting_for_active_world",
                    "world_id": null,
                    "text_backend": options.text_backend.as_str(),
                    "visual_backend": options.visual_backend.as_str(),
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
            }
            if options.once {
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "worker_idle",
                    "world_id": null,
                    "text_backend": options.text_backend.as_str(),
                    "visual_backend": options.visual_backend.as_str(),
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
                break;
            }
            thread::sleep(interval);
            continue;
        };
        let (text_backend, visual_backend) =
            effective_host_worker_backends(store_root, world_id.as_str(), options)?;
        prewarm_effective_webgpt_lanes_once(&mut emitted, text_backend, visual_backend, options)?;
        let mut emitted_this_tick = false;
        if visual_backend == HostWorkerVisualBackend::None {
            if emit_host_pending_agent_turn_event(
                store_root,
                world_id.as_str(),
                &mut emitted,
                text_backend,
                options,
            )? {
                emitted_this_tick = true;
            }
        } else if emit_host_text_and_visual_events_parallel(
            store_root,
            world_id.as_str(),
            &mut emitted,
            text_backend,
            visual_backend,
            options,
        )? {
            emitted_this_tick = true;
        }
        if options.once {
            if emitted_this_tick {
                continue;
            }
            emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                "event": "worker_idle",
                "world_id": world_id,
                    "text_backend": text_backend.as_str(),
                    "visual_backend": visual_backend.as_str(),
                "consumer": HOST_WORKER_CONSUMER,
            }))?;
            break;
        }
        thread::sleep(interval);
    }
    Ok(())
}

fn prewarm_effective_webgpt_lanes_once(
    emitted: &mut HashSet<String>,
    text_backend: HostWorkerTextBackend,
    visual_backend: HostWorkerVisualBackend,
    options: &HostWorkerOptions,
) -> Result<()> {
    let include_text = matches!(text_backend, HostWorkerTextBackend::Webgpt)
        && options.webgpt_turn_command.is_none();
    let include_visual = matches!(visual_backend, HostWorkerVisualBackend::Webgpt);
    if !include_text && !include_visual {
        return Ok(());
    }
    if !emitted.insert("webgpt-lane-prewarm".to_owned()) {
        return Ok(());
    }
    let lanes = prewarm_webgpt_lane_sessions(options, include_text, include_visual)?;
    emit_host_event(&serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "webgpt_lanes_prewarmed",
        "lanes": lanes,
        "consumer": HOST_WORKER_CONSUMER,
    }))?;
    Ok(())
}

fn effective_host_worker_backends(
    store_root: Option<&Path>,
    world_id: &str,
    options: &HostWorkerOptions,
) -> Result<(HostWorkerTextBackend, HostWorkerVisualBackend)> {
    let Some(selection) = load_world_backend_selection(store_root, world_id)? else {
        return Ok((options.text_backend, options.visual_backend));
    };
    if !selection.locked {
        anyhow::bail!("backend selection is not locked for world_id={world_id}");
    }
    Ok((
        HostWorkerTextBackend::from(selection.text_backend),
        HostWorkerVisualBackend::from(selection.visual_backend),
    ))
}

const fn host_worker_visual_jobs_label(backend: HostWorkerVisualBackend) -> &'static str {
    match backend {
        HostWorkerVisualBackend::Webgpt => "claim_and_generate_webgpt",
        HostWorkerVisualBackend::None => "disabled",
    }
}

fn resolve_host_worker_world_id(
    store_root: Option<&Path>,
    world_id: Option<&str>,
) -> Result<Option<String>> {
    if world_id.is_some() {
        return resolve_world_id(store_root, world_id).map(Some);
    }
    let store_paths = resolve_store_paths(store_root)?;
    let active_path = store_paths.root.join(ACTIVE_WORLD_FILENAME);
    if !active_path.exists() {
        return Ok(None);
    }
    Ok(Some(load_active_world(store_root)?.world_id))
}

fn emit_host_pending_agent_turn_event(
    store_root: Option<&Path>,
    world_id: &str,
    emitted: &mut HashSet<String>,
    text_backend: HostWorkerTextBackend,
    options: &HostWorkerOptions,
) -> Result<bool> {
    let Ok(pending) = load_pending_agent_turn(store_root, world_id) else {
        return Ok(false);
    };
    match text_backend {
        HostWorkerTextBackend::Webgpt => {
            emit_webgpt_pending_agent_turn_event(store_root, emitted, &pending, options)
        }
    }
}

fn emit_webgpt_pending_agent_turn_event(
    store_root: Option<&Path>,
    emitted: &mut HashSet<String>,
    pending: &singulari_world::PendingAgentTurn,
    options: &HostWorkerOptions,
) -> Result<bool> {
    match dispatch_pending_agent_turn_via_webgpt(store_root, pending, options)? {
        WebGptDispatchOutcome::Started(record) => {
            let event_key = format!("webgpt-started:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&webgpt_dispatch_started_event(pending, &record))?;
            Ok(true)
        }
        WebGptDispatchOutcome::AlreadyDispatched(record_path) => {
            let event_key = format!("webgpt-skipped:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&serde_json::json!({
                "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                "event": "webgpt_dispatch_skipped",
                "reason": "already_dispatched",
                "world_id": pending.world_id,
                "turn_id": pending.turn_id,
                "record_path": record_path,
                "consumer": HOST_WORKER_CONSUMER,
            }))?;
            Ok(true)
        }
    }
}

enum HostTextDispatchResult {
    Webgpt { outcome: WebGptDispatchOutcome },
}

enum HostVisualDispatchResult {
    Webgpt(WebGptImageDispatchRecord),
}

fn emit_host_text_and_visual_events_parallel(
    store_root: Option<&Path>,
    world_id: &str,
    emitted: &mut HashSet<String>,
    text_backend: HostWorkerTextBackend,
    visual_backend: HostWorkerVisualBackend,
    options: &HostWorkerOptions,
) -> Result<bool> {
    let pending = load_pending_agent_turn(store_root, world_id).ok();
    let visual_claims = match visual_backend {
        HostWorkerVisualBackend::Webgpt => {
            claim_next_host_visual_jobs(store_root, world_id, "singulari_webgpt_image_worker")?
        }
        HostWorkerVisualBackend::None => Vec::new(),
    };
    if pending.is_none() && visual_claims.is_empty() {
        return Ok(false);
    }

    let (text_result, image_results) = thread::scope(|scope| {
        let text_handle = pending.as_ref().map(|pending| {
            scope.spawn(move || match text_backend {
                HostWorkerTextBackend::Webgpt => {
                    let outcome =
                        dispatch_pending_agent_turn_via_webgpt(store_root, pending, options)?;
                    Ok(HostTextDispatchResult::Webgpt { outcome })
                }
            })
        });
        let image_handles = visual_claims
            .iter()
            .map(|claim| {
                scope.spawn(move || match visual_backend {
                    HostWorkerVisualBackend::Webgpt => {
                        dispatch_visual_job_via_webgpt_with_claim_release(
                            store_root, claim, options,
                        )
                        .map(HostVisualDispatchResult::Webgpt)
                    }
                    HostWorkerVisualBackend::None => {
                        anyhow::bail!("visual backend none cannot dispatch visual claim")
                    }
                })
            })
            .collect::<Vec<_>>();

        let text_result = text_handle.map(|handle| {
            handle
                .join()
                .unwrap_or_else(|panic| Err(thread_panic_error("text dispatch", panic.as_ref())))
        });
        let image_results = image_handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|panic| {
                    Err(thread_panic_error("image dispatch", panic.as_ref()))
                })
            })
            .collect::<Vec<_>>();
        (text_result, image_results)
    });

    let mut emitted_any = false;
    if let Some((pending, result)) = pending.as_ref().zip(text_result)
        && emit_text_dispatch_result(pending, result?, emitted)?
    {
        emitted_any = true;
    }

    for result in image_results {
        if emit_visual_dispatch_result(result?)? {
            emitted_any = true;
        }
    }
    Ok(emitted_any)
}

fn dispatch_visual_job_via_webgpt_with_claim_release(
    store_root: Option<&Path>,
    claim: &singulari_world::VisualJobClaim,
    options: &HostWorkerOptions,
) -> Result<WebGptImageDispatchRecord> {
    match dispatch_visual_job_via_webgpt(store_root, claim, options) {
        Ok(record) => Ok(record),
        Err(dispatch_error) => {
            let release = release_visual_job_claim(&ReleaseVisualJobClaimOptions {
                store_root: store_root.map(Path::to_path_buf),
                world_id: claim.world_id.clone(),
                slot: claim.slot.clone(),
            })
            .with_context(|| {
                format!(
                    "failed to release visual job claim after WebGPT image dispatch failed: world_id={}, slot={}, claim_id={}",
                    claim.world_id, claim.slot, claim.claim_id
                )
            })?;
            let released_claim_id = release
                .claim
                .as_ref()
                .map_or("<none>", |released| released.claim_id.as_str());
            Err(dispatch_error).with_context(|| {
                format!(
                    "released visual job claim after WebGPT image dispatch failed: world_id={}, slot={}, claim_id={}, released_claim_id={released_claim_id}",
                    claim.world_id, claim.slot, claim.claim_id
                )
            })
        }
    }
}

fn emit_text_dispatch_result(
    pending: &singulari_world::PendingAgentTurn,
    result: HostTextDispatchResult,
    emitted: &mut HashSet<String>,
) -> Result<bool> {
    match result {
        HostTextDispatchResult::Webgpt { outcome } => {
            emit_webgpt_text_dispatch_result(pending, outcome, emitted)
        }
    }
}

fn emit_webgpt_text_dispatch_result(
    pending: &singulari_world::PendingAgentTurn,
    outcome: WebGptDispatchOutcome,
    emitted: &mut HashSet<String>,
) -> Result<bool> {
    match outcome {
        WebGptDispatchOutcome::Started(record) => {
            let event_key = format!("webgpt-started:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&webgpt_dispatch_started_event(pending, &record))?;
        }
        WebGptDispatchOutcome::AlreadyDispatched(record_path) => {
            let event_key = format!("webgpt-skipped:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&webgpt_dispatch_skipped_event(
                pending,
                record_path.as_str(),
            ))?;
        }
    }
    Ok(true)
}

fn emit_visual_dispatch_result(result: HostVisualDispatchResult) -> Result<bool> {
    match result {
        HostVisualDispatchResult::Webgpt(record) => {
            emit_host_event(&webgpt_image_completed_event(&record))?;
        }
    }
    Ok(true)
}

fn thread_panic_error(context: &str, panic: &(dyn std::any::Any + Send)) -> anyhow::Error {
    let reason = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic payload");
    anyhow::anyhow!("{context} panicked: {reason}")
}

fn webgpt_dispatch_started_event(
    pending: &singulari_world::PendingAgentTurn,
    record: &WebGptDispatchRecord,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "webgpt_dispatch_started",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "turn_status": record.status,
        "adapter_command": record.adapter_command,
        "mcp_wrapper": record.mcp_wrapper,
        "conversation_id": record.raw_conversation_id,
        "pid": record.pid,
        "record_path": record.record_path,
        "prompt_path": record.prompt_path,
        "response_path": record.response_path,
        "result_path": record.result_path,
        "stdout_path": record.stdout_path,
        "stderr_path": record.stderr_path,
        "committed_turn_id": record.committed_turn_id,
        "consumer": HOST_WORKER_CONSUMER,
    })
}

fn webgpt_dispatch_skipped_event(
    pending: &singulari_world::PendingAgentTurn,
    record_path: &str,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "webgpt_dispatch_skipped",
        "reason": "already_dispatched",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "record_path": record_path,
        "consumer": HOST_WORKER_CONSUMER,
    })
}

fn claim_next_host_visual_jobs(
    store_root: Option<&Path>,
    world_id: &str,
    claimed_by: &str,
) -> Result<Vec<singulari_world::VisualJobClaim>> {
    let mut claimed = undispatched_owned_visual_claims(store_root, world_id, claimed_by)?;
    let jobs = current_host_visual_jobs(store_root, world_id)?;
    let mut claimed_session_kinds = claimed
        .iter()
        .map(|claim| WebGptImageSessionKind::from_slot(claim.slot.as_str()))
        .collect::<HashSet<_>>();
    for job in jobs {
        let session_kind = WebGptImageSessionKind::from_slot(job.slot.as_str());
        if !claimed_session_kinds.insert(session_kind) {
            continue;
        }
        if let Some(existing_claim) =
            load_visual_job_claim(store_root, world_id, job.slot.as_str())?
        {
            if existing_claim.claimed_by == claimed_by
                && !visual_dispatch_record_exists_for_claim(store_root, &existing_claim)?
            {
                claimed.push(existing_claim);
            }
            continue;
        }
        let outcome = claim_visual_job(&ClaimVisualJobOptions {
            store_root: store_root.map(Path::to_path_buf),
            world_id: world_id.to_owned(),
            slot: Some(job.slot.clone()),
            claimed_by: claimed_by.to_owned(),
            force: false,
            extra_jobs: current_turn_visual_jobs(store_root, world_id)?,
        })?;
        let singulari_world::VisualJobClaimOutcome::Claimed { claim } = outcome else {
            anyhow::bail!(
                "visual job vanished before claim: world_id={world_id}, slot={}",
                job.slot
            );
        };
        claimed.push(*claim);
    }
    Ok(claimed)
}

fn undispatched_owned_visual_claims(
    store_root: Option<&Path>,
    world_id: &str,
    claimed_by: &str,
) -> Result<Vec<singulari_world::VisualJobClaim>> {
    let paths = resolve_store_paths(store_root)?;
    let claims_dir = paths
        .worlds_dir
        .join(world_id)
        .join("visual_jobs")
        .join("claims");
    if !claims_dir.exists() {
        return Ok(Vec::new());
    }
    let mut claims = Vec::new();
    for entry in fs::read_dir(&claims_dir)
        .with_context(|| format!("failed to read {}", claims_dir.display()))?
    {
        let entry =
            entry.with_context(|| format!("failed to read entry in {}", claims_dir.display()))?;
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(path.as_path())
            .with_context(|| format!("failed to read visual job claim {}", path.display()))?;
        let claim = serde_json::from_str::<singulari_world::VisualJobClaim>(raw.as_str())
            .with_context(|| format!("failed to parse visual job claim {}", path.display()))?;
        if claim.world_id == world_id
            && claim.claimed_by == claimed_by
            && !visual_dispatch_record_exists_for_claim(store_root, &claim)?
        {
            claims.push(claim);
        }
    }
    claims.sort_by(|left, right| left.claimed_at.cmp(&right.claimed_at));
    Ok(claims)
}

fn visual_dispatch_record_exists_for_claim(
    store_root: Option<&Path>,
    claim: &singulari_world::VisualJobClaim,
) -> Result<bool> {
    let dispatch_dir = visual_dispatch_dir_for_world(store_root, claim.world_id.as_str())?;
    let slot_component = safe_file_component(claim.slot.as_str());
    let claim_component = safe_file_component(claim.claim_id.as_str());
    let record_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image.json"
    ));
    Ok(record_path.exists())
}

fn webgpt_image_completed_event(record: &WebGptImageDispatchRecord) -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "webgpt_image_generate_completed",
        "world_id": record.world_id.as_str(),
        "slot": record.slot.as_str(),
        "claim_id": record.claim_id.as_deref(),
        "generated_path": record.generated_path.as_deref(),
        "destination_path": record.destination_path.as_str(),
        "record_path": record.record_path.as_str(),
        "status": record.status.as_str(),
        "consumer": HOST_WORKER_CONSUMER,
    })
}

fn current_host_visual_jobs(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<Vec<ImageGenerationJob>> {
    let mut packet_options = BuildVnPacketOptions::new(world_id.to_owned());
    packet_options.store_root = store_root.map(Path::to_path_buf);
    let packet = build_vn_packet(&packet_options)?;
    if let Some(job) = packet.image.image_generation_job {
        let mut jobs = vec![job];
        jobs.extend(packet.visual_assets.image_generation_jobs);
        return Ok(dedupe_visual_jobs_by_slot(jobs));
    }
    let jobs = packet.visual_assets.image_generation_jobs;
    Ok(dedupe_visual_jobs_by_slot(jobs))
}

pub(crate) fn current_turn_visual_jobs(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<Vec<ImageGenerationJob>> {
    Ok(current_host_visual_jobs(store_root, world_id)?
        .into_iter()
        .filter(|job| job.slot.starts_with("turn_cg:"))
        .collect())
}

fn dedupe_visual_jobs_by_slot(jobs: Vec<ImageGenerationJob>) -> Vec<ImageGenerationJob> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for job in jobs {
        if seen.insert(job.slot.clone()) {
            deduped.push(job);
        }
    }
    deduped
}

fn emit_host_event(event: &serde_json::Value) -> Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string(event)?)?;
    stdout.flush()?;
    Ok(())
}
