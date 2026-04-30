use anyhow::{Context, Result};
use chrono::Utc;
use serde::Serialize;
use singulari_world::{
    CompleteVisualJobOptions, ImageGenerationJob, VisualArtifactKind, complete_visual_job,
    resolve_store_paths, validate_visual_canon_policy_for_job, visual_canon_policy_prompt,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use crate::runtime::host_worker::HostWorkerOptions;

use super::{
    WebGptLaneRuntime, is_webgpt_timeout_signal, load_webgpt_image_conversation_binding,
    resolve_webgpt_mcp_wrapper, safe_file_component, save_webgpt_image_conversation_binding,
    webgpt_conversation_url, write_dispatch_claim,
};

#[derive(Debug, Serialize)]
pub(in crate::runtime) struct WebGptImageDispatchRecord {
    pub(in crate::runtime) schema_version: &'static str,
    pub(in crate::runtime) status: String,
    pub(in crate::runtime) world_id: String,
    pub(in crate::runtime) slot: String,
    pub(in crate::runtime) claim_id: Option<String>,
    pub(in crate::runtime) mcp_wrapper: String,
    pub(in crate::runtime) mcp_profile_dir: String,
    pub(in crate::runtime) mcp_cdp_port: u16,
    pub(in crate::runtime) mcp_cdp_url: String,
    pub(in crate::runtime) image_session_kind: String,
    pub(in crate::runtime) reference_paths: Vec<String>,
    pub(in crate::runtime) conversation_id: Option<String>,
    pub(in crate::runtime) raw_conversation_id: Option<String>,
    pub(in crate::runtime) pid: u32,
    pub(in crate::runtime) record_path: String,
    pub(in crate::runtime) prompt_path: String,
    pub(in crate::runtime) result_path: String,
    pub(in crate::runtime) stdout_path: String,
    pub(in crate::runtime) stderr_path: String,
    pub(in crate::runtime) generated_path: Option<String>,
    pub(in crate::runtime) generated_sha256: Option<String>,
    pub(in crate::runtime) generated_bytes: Option<usize>,
    pub(in crate::runtime) destination_path: String,
    pub(in crate::runtime) completion_path: Option<String>,
    pub(in crate::runtime) dispatched_at: String,
    pub(in crate::runtime) exit_code: Option<i32>,
    pub(in crate::runtime) error: Option<String>,
    pub(in crate::runtime) completed_at: String,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Hash)]
pub(in crate::runtime) enum WebGptImageSessionKind {
    TurnCg,
    ReferenceAsset,
}

impl WebGptImageSessionKind {
    pub(in crate::runtime) fn from_slot(slot: &str) -> Self {
        if slot.starts_with("turn_cg:") {
            Self::TurnCg
        } else {
            Self::ReferenceAsset
        }
    }

    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::TurnCg => "turn_cg",
            Self::ReferenceAsset => "reference_asset",
        }
    }

    pub(super) const fn binding_filename(self) -> &'static str {
        match self {
            Self::TurnCg => "webgpt_image_conversation_binding.json",
            Self::ReferenceAsset => "webgpt_reference_asset_conversation_binding.json",
        }
    }

    pub(super) const fn source(self) -> &'static str {
        match self {
            Self::TurnCg => "webgpt_mcp_image_generation_turn_cg",
            Self::ReferenceAsset => "webgpt_mcp_image_generation_reference_asset",
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "WebGPT image dispatch keeps MCP image generation, extraction, and visual-job completion in one durable record"
)]
pub(in crate::runtime) fn dispatch_visual_job_via_webgpt(
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
        "timeout_secs": options.webgpt_timeout_secs,
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

    let waiting_browser = error.as_deref().is_some_and(is_webgpt_timeout_signal);
    let status = if error.is_none() {
        "completed"
    } else if waiting_browser {
        "waiting_browser"
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
    if let Some(error) = &record.error
        && record.status != "waiting_browser"
    {
        anyhow::bail!("{error}");
    }
    Ok(record)
}

pub(super) fn ensure_image_job_matches_session_kind(
    job: &ImageGenerationJob,
    image_session_kind: WebGptImageSessionKind,
) -> Result<()> {
    validate_visual_canon_policy_for_job(job)?;
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

pub(super) fn webgpt_image_reference_paths(job: &ImageGenerationJob) -> Result<Vec<String>> {
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

pub(super) fn build_webgpt_image_generation_prompt(
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
    prompt.push_str("\n\n");
    prompt.push_str(visual_canon_policy_prompt(&job.visual_canon_policy).as_str());
    if !job.reference_paths.is_empty() {
        prompt.push_str("\n\nReference continuity notes: ");
        prompt.push_str(job.reference_paths.join(", ").as_str());
    }
    prompt
}

pub(in crate::runtime) fn visual_dispatch_dir_for_world(
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
