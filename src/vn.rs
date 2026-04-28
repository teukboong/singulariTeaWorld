use crate::backend_selection::{WorldVisualBackend, load_world_backend_selection};
use crate::models::{
    CodexView, DashboardSummary, EntityRecords, FREEFORM_CHOICE_SLOT, GUIDE_CHOICE_SLOT,
    GUIDE_CHOICE_TAG, INITIAL_TURN_ID, PROTAGONIST_CHARACTER_ID, RenderPacket, ScanTarget,
    TurnChoice, TurnLogEntry, TurnSnapshot, WorldRecord, is_guide_choice_tag,
    normalize_turn_choices, redact_guide_choice_public_hints,
};
use crate::render::{RenderPacketLoadOptions, load_render_packet, render_packet_markdown};
use crate::store::{TURN_LOG_FILENAME, load_world_record, resolve_store_paths, world_file_paths};
use crate::visual_assets::{
    BuildWorldVisualAssetsOptions, ImageGenerationJob, VN_ASSETS_DIR, VisualBudgetPolicy,
    WorldVisualAssets, build_world_visual_assets, compile_turn_visual_prompt,
    visual_generation_job,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::{env, fs};

pub const VN_PACKET_SCHEMA_VERSION: &str = "singulari.vn_packet.v1";
pub const VN_PROTAGONIST_STATUS_SCHEMA_VERSION: &str = "singulari.vn_protagonist_status.v1";
const HOST_WORKER_VISUAL_GENERATOR: &str = "host_worker.visual.generate";
const VISUAL_BACKEND_ENV: &str = "SINGULARI_WORLD_VISUAL_BACKEND";
const WEBGPT_VISUAL_BACKEND: &str = "webgpt";
const WEBGPT_TURN_CG_CADENCE_ENV: &str = "SINGULARI_WORLD_WEBGPT_TURN_CG_CADENCE_MIN";
const DEFAULT_WEBGPT_TURN_CG_CADENCE_MIN: u32 = 2;
const RECENT_TURN_LOG_LIMIT: usize = 24;
const TURN_CG_DIR: &str = "turn_cg";
const TURN_CG_JOBS_DIR: &str = "cg_jobs";

#[derive(Debug, Clone)]
pub struct BuildVnPacketOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub turn_id: Option<String>,
    pub scene_image_url: Option<String>,
}

impl BuildVnPacketOptions {
    #[must_use]
    pub fn new(world_id: String) -> Self {
        Self {
            store_root: None,
            world_id,
            turn_id: None,
            scene_image_url: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub title: String,
    pub mode: String,
    pub scene: VnScene,
    pub image: VnSceneImage,
    pub visual_assets: WorldVisualAssets,
    pub choices: Vec<VnChoice>,
    pub codex_surface: VnCodexSurface,
    pub hidden_filter: VnHiddenFilter,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnScene {
    pub location: String,
    pub current_event: String,
    pub status: String,
    pub text_blocks: Vec<String>,
    pub scan_lines: Vec<String>,
    pub adjudication: Option<VnAdjudication>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnAdjudication {
    pub outcome: String,
    pub summary: String,
    pub visible_constraints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnSceneImage {
    pub generator: String,
    pub image_prompt: String,
    pub recommended_path: String,
    pub asset_url: String,
    pub exists: bool,
    pub auto_decision: VnTurnCgDecision,
    pub budget_policy: VisualBudgetPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_generation_job: Option<ImageGenerationJob>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub existing_image_url: Option<String>,
    pub prompt_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnTurnCgDecision {
    pub source: String,
    pub action: String,
    pub execution_mode: String,
    pub requested: bool,
    pub retry_requested: bool,
    pub cadence_turns_remaining: u32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnChoice {
    pub slot: u8,
    pub tag: String,
    pub label: String,
    pub intent: String,
    pub requires_inline_text: bool,
    pub input_template: String,
    pub command_template: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnCodexSurface {
    pub full_markdown: String,
    pub dashboard: DashboardSummary,
    pub scan_targets: Vec<ScanTarget>,
    pub adjudication: Option<VnAdjudication>,
    pub codex_view: Option<CodexView>,
    pub current_turn: VnCurrentTurnStatus,
    pub protagonist: VnProtagonistStatus,
    pub turn_log: Vec<VnTurnLogSummary>,
    pub redaction_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnCurrentTurnStatus {
    pub turn_id: String,
    pub input: Option<String>,
    pub input_kind: Option<String>,
    pub selected_choice: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnProtagonistStatus {
    pub status_schema: String,
    pub phase: String,
    pub location: String,
    pub current_event: Option<String>,
    pub current_event_progress: Option<String>,
    pub inventory: Vec<String>,
    pub body: Vec<String>,
    pub mind: Vec<String>,
    pub open_questions: Vec<String>,
    pub dashboard_rows: Vec<VnStatusRow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnStatusRow {
    pub row_id: String,
    pub cells: Vec<VnStatusCell>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnStatusCell {
    pub emoji: String,
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnTurnLogSummary {
    pub turn_id: String,
    pub input: String,
    pub input_kind: String,
    pub selected_choice: Option<String>,
    pub canon_event_id: String,
    pub render_packet_ref: String,
    pub render_markdown: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VnHiddenFilter {
    pub forbidden_reveal_count: usize,
    pub policy: String,
}

/// Build a player-visible visual-novel projection packet from a render packet.
///
/// # Errors
///
/// Returns an error when the world record, render packet, or store paths cannot
/// be loaded.
pub fn build_vn_packet(options: &BuildVnPacketOptions) -> Result<VnPacket> {
    let world = load_world_record(options.store_root.as_deref(), options.world_id.as_str())?;
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let render_packet = load_render_packet(&RenderPacketLoadOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
        turn_id: options.turn_id.clone(),
    })?;
    let entities: EntityRecords = crate::store::read_json(&files.entities)?;
    let place_record = entities
        .places
        .iter()
        .find(|place| place.id == render_packet.visible_state.dashboard.location);
    let codex_surface = vn_codex_surface(&world, &files, &render_packet, &entities)?;
    let mut visual_assets = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: options.store_root.clone(),
        world_id: world.world_id.clone(),
    })?;
    apply_host_visual_backend_budget_policy(
        options.store_root.as_deref(),
        world.world_id.as_str(),
        &mut visual_assets.budget_policy,
    )?;
    let turn_cg_path = turn_cg_path(&files.dir, render_packet.turn_id.as_str());
    let turn_cg_url = turn_cg_asset_url(world.world_id.as_str(), render_packet.turn_id.as_str());
    let turn_cg_exists = turn_cg_path.is_file();
    let retry_requested = turn_cg_retry_requested(&files.dir, render_packet.turn_id.as_str());
    let turn_cg_decision = budgeted_turn_cg_decision(
        &render_packet,
        turn_cg_exists,
        retry_requested,
        &visual_assets.budget_policy,
    );
    let turn_visual_prompt = compile_turn_visual_prompt(&world, &render_packet, &visual_assets);
    let turn_cg_job = if turn_cg_decision.requested && (!turn_cg_exists || retry_requested) {
        Some(turn_cg_image_generation_job(
            render_packet.turn_id.as_str(),
            &turn_visual_prompt,
            &turn_cg_path,
            retry_requested,
        ))
    } else {
        None
    };
    let existing_image_url = options
        .scene_image_url
        .clone()
        .or_else(|| turn_cg_exists.then_some(turn_cg_url.clone()))
        .or_else(|| {
            latest_existing_turn_cg_url(
                world.world_id.as_str(),
                &files.dir,
                render_packet.turn_id.as_str(),
            )
        });
    Ok(VnPacket {
        schema_version: VN_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id.clone(),
        turn_id: render_packet.turn_id.clone(),
        title: world.title.clone(),
        mode: render_packet.mode.clone(),
        scene: vn_scene(&world, &render_packet, place_record),
        image: VnSceneImage {
            generator: HOST_WORKER_VISUAL_GENERATOR.to_owned(),
            image_prompt: turn_visual_prompt.prompt,
            recommended_path: turn_cg_path.display().to_string(),
            asset_url: turn_cg_url,
            exists: turn_cg_exists,
            auto_decision: turn_cg_decision,
            budget_policy: visual_assets.budget_policy.clone(),
            image_generation_job: turn_cg_job,
            existing_image_url,
            prompt_policy: turn_visual_prompt.prompt_policy,
        },
        visual_assets,
        choices: vn_choices(&world, &render_packet.visible_state.choices),
        codex_surface,
        hidden_filter: VnHiddenFilter {
            forbidden_reveal_count: render_packet.forbidden_reveals.len(),
            policy: "hidden truth is counted but never projected into VN text or image prompt"
                .to_owned(),
        },
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn apply_host_visual_backend_budget_policy(
    store_root: Option<&Path>,
    world_id: &str,
    policy: &mut VisualBudgetPolicy,
) -> Result<()> {
    if world_uses_webgpt_visual_backend(store_root, world_id)? {
        policy.turn_cg_cadence_min = configured_webgpt_turn_cg_cadence_min();
    }
    Ok(())
}

fn world_uses_webgpt_visual_backend(store_root: Option<&Path>, world_id: &str) -> Result<bool> {
    let Some(selection) = load_world_backend_selection(store_root, world_id)? else {
        return Ok(
            env::var(VISUAL_BACKEND_ENV).is_ok_and(|value| value.trim() == WEBGPT_VISUAL_BACKEND)
        );
    };
    if !selection.locked {
        anyhow::bail!("backend selection is not locked for world_id={world_id}");
    }
    Ok(selection.visual_backend == WorldVisualBackend::Webgpt)
}

fn configured_webgpt_turn_cg_cadence_min() -> u32 {
    env::var(WEBGPT_TURN_CG_CADENCE_ENV)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_WEBGPT_TURN_CG_CADENCE_MIN)
}

fn turn_cg_path(world_dir: &std::path::Path, turn_id: &str) -> std::path::PathBuf {
    world_dir
        .join(VN_ASSETS_DIR)
        .join(TURN_CG_DIR)
        .join(format!("{turn_id}.png"))
}

fn turn_cg_asset_url(world_id: &str, turn_id: &str) -> String {
    format!("/world-assets/{world_id}/{VN_ASSETS_DIR}/{TURN_CG_DIR}/{turn_id}.png")
}

fn turn_cg_retry_requested(world_dir: &std::path::Path, turn_id: &str) -> bool {
    turn_cg_retry_path(world_dir, turn_id).is_file()
}

#[must_use]
pub fn turn_cg_retry_path(world_dir: &std::path::Path, turn_id: &str) -> std::path::PathBuf {
    world_dir
        .join(VN_ASSETS_DIR)
        .join(TURN_CG_JOBS_DIR)
        .join(format!("{turn_id}_retry.json"))
}

fn turn_cg_image_generation_job(
    turn_id: &str,
    compiled_prompt: &crate::visual_assets::CompiledVisualPrompt,
    destination_path: &std::path::Path,
    retry_requested: bool,
) -> ImageGenerationJob {
    visual_generation_job(
        format!("turn_cg:{turn_id}"),
        compiled_prompt.prompt.clone(),
        destination_path.display().to_string(),
        compiled_prompt.reference_asset_urls.clone(),
        compiled_prompt.reference_paths.clone(),
        if retry_requested {
            "background retry requested; save exactly to destination_path without blocking VN flow"
        } else {
            "background auto job; save exactly to destination_path without blocking VN flow"
        },
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "Turn CG budget decisions keep all cadence, retry, and force-generation reasons in one auditable policy function"
)]
fn budgeted_turn_cg_decision(
    packet: &RenderPacket,
    asset_exists: bool,
    retry_requested: bool,
    policy: &VisualBudgetPolicy,
) -> VnTurnCgDecision {
    let cadence = policy.turn_cg_cadence_min.max(1);
    let turn_index = turn_index(packet.turn_id.as_str()).unwrap_or_default();
    let cadence_turns_remaining = cadence_turns_remaining(turn_index, cadence);
    if retry_requested {
        return turn_cg_decision(
            "user_retry_request",
            "generate_scene",
            true,
            true,
            0,
            "사용자가 백그라운드 재시도를 요청했다.",
        );
    }
    if asset_exists {
        return turn_cg_decision(
            "visual_budget_policy",
            "reuse_existing",
            false,
            false,
            cadence_turns_remaining,
            "이미 저장된 turn CG가 있어서 새 작업을 만들지 않는다.",
        );
    }
    if policy.mode == "off" {
        return turn_cg_decision(
            "visual_budget_policy",
            "off",
            false,
            false,
            cadence_turns_remaining,
            "visual budget mode가 off라 자동 CG 생성을 건너뛴다.",
        );
    }
    if packet.turn_id == INITIAL_TURN_ID {
        return turn_cg_decision(
            "visual_budget_policy",
            "background_only",
            false,
            false,
            cadence_turns_remaining,
            "초기 접속 장면은 세계 배경 레이어를 우선 사용한다.",
        );
    }
    if turn_index == 1 {
        return turn_cg_decision(
            "visual_budget_policy",
            "generate_scene",
            true,
            false,
            0,
            "첫 서사 턴은 시드에서 건져 올린 장면이므로 초기 turn CG 후보를 만든다.",
        );
    }
    if packet.mode == "codex" {
        return turn_cg_decision(
            "visual_budget_policy",
            "off",
            false,
            false,
            cadence_turns_remaining,
            "기록/조회 턴은 서사 CG 예산을 쓰지 않는다.",
        );
    }
    if let Some(reason) = force_visual_signal(packet) {
        return turn_cg_decision(
            "visual_budget_policy",
            "generate_scene",
            true,
            false,
            0,
            reason,
        );
    }
    if policy.mode == "eager" {
        return turn_cg_decision(
            "visual_budget_policy",
            "generate_scene",
            true,
            false,
            0,
            "visual budget mode가 eager라 서사 턴마다 CG 후보를 만든다.",
        );
    }
    if turn_index > 0 && is_turn_on_cadence(turn_index, cadence) {
        return turn_cg_decision(
            "visual_budget_policy",
            "generate_scene",
            true,
            false,
            0,
            format!("balanced cadence가 {cadence}턴에 도달해 장면 CG 후보를 만든다."),
        );
    }
    turn_cg_decision(
        "visual_budget_policy",
        "reuse_last",
        false,
        false,
        cadence_turns_remaining,
        "balanced budget에서는 평범한 진행 턴의 새 CG 생성을 건너뛰고 기존 배경/이전 장면을 유지한다.",
    )
}

fn turn_cg_decision(
    source: &str,
    action: &str,
    requested: bool,
    retry_requested: bool,
    cadence_turns_remaining: u32,
    reason: impl Into<String>,
) -> VnTurnCgDecision {
    VnTurnCgDecision {
        source: source.to_owned(),
        action: action.to_owned(),
        execution_mode: "background".to_owned(),
        requested,
        retry_requested,
        cadence_turns_remaining,
        reason: reason.into(),
    }
}

fn force_visual_signal(packet: &RenderPacket) -> Option<String> {
    if packet.mode == "cc" {
        return Some("렌더 전환/캔버스 모드라 장면 CG 후보를 만든다.".to_owned());
    }
    let dashboard = &packet.visible_state.dashboard;
    let visible_text = format!(
        "{} {} {}",
        dashboard.current_event, dashboard.status, dashboard.phase
    );
    if contains_any(
        visible_text.as_str(),
        &[
            "전투",
            "싸움",
            "공격",
            "combat",
            "battle",
            "발견",
            "드러",
            "reveal",
            "revelation",
            "climax",
            "클라이맥스",
            "새 장소",
            "new location",
        ],
    ) {
        return Some("장면 전환/전투/발견 계열 신호가 있어 CG 후보를 만든다.".to_owned());
    }
    packet.adjudication.as_ref().and_then(|report| {
        let consequence_text = report.consequences.join(" ");
        contains_any(
            consequence_text.as_str(),
            &[
                "canvas_transform_requested",
                "combat",
                "battle",
                "revelation",
                "new_major_location",
                "first_major_character",
            ],
        )
        .then(|| "판정 결과에 강제 CG 생성 신호가 있다.".to_owned())
    })
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn turn_index(turn_id: &str) -> Option<u32> {
    turn_id.strip_prefix("turn_")?.parse().ok()
}

fn cadence_turns_remaining(turn_index: u32, cadence: u32) -> u32 {
    if turn_index == 0 {
        return cadence;
    }
    let remainder = turn_index % cadence;
    if remainder == 0 {
        0
    } else {
        cadence - remainder
    }
}

fn latest_existing_turn_cg_url(
    world_id: &str,
    world_dir: &std::path::Path,
    current_turn_id: &str,
) -> Option<String> {
    let current_turn_index = turn_index(current_turn_id)?;
    let cg_dir = world_dir.join(VN_ASSETS_DIR).join(TURN_CG_DIR);
    let latest_turn_index = fs::read_dir(cg_dir)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let file_name = entry.file_name();
            let file_name = file_name.to_str()?;
            let turn_id = file_name.strip_suffix(".png")?;
            let index = turn_index(turn_id)?;
            (index < current_turn_index).then_some(index)
        })
        .max()?;
    Some(turn_cg_asset_url(
        world_id,
        &format!("turn_{latest_turn_index:04}"),
    ))
}

fn vn_codex_surface(
    world: &WorldRecord,
    files: &crate::store::WorldFilePaths,
    packet: &RenderPacket,
    entities: &EntityRecords,
) -> Result<VnCodexSurface> {
    let latest_snapshot: TurnSnapshot = crate::store::read_json(&files.latest_snapshot)?;
    let protagonist_record = entities
        .characters
        .iter()
        .find(|character| character.id == PROTAGONIST_CHARACTER_ID);
    let place_record = entities
        .places
        .iter()
        .find(|place| place.id == packet.visible_state.dashboard.location);
    let turn_log = recent_turn_log(world, files, latest_snapshot.session_id.as_str())?;
    let scan_targets = player_visible_scan_targets(&packet.visible_state.scan_targets);
    Ok(VnCodexSurface {
        full_markdown: redact_guide_choice_public_hints(&render_packet_markdown(packet)),
        dashboard: player_visible_dashboard(world, &packet.visible_state.dashboard, place_record),
        scan_targets,
        adjudication: packet.adjudication.as_ref().map(vn_adjudication),
        codex_view: packet.codex_view.clone(),
        current_turn: current_turn_status(packet, &turn_log),
        protagonist: protagonist_status(
            world,
            &latest_snapshot,
            &packet.visible_state.dashboard,
            place_record,
            protagonist_record,
        ),
        turn_log,
        redaction_policy:
            "player-visible Codex surface only; deprecated Guide-choice hints and hidden truth stay filtered"
                .to_owned(),
    })
}

fn vn_scene(
    world: &WorldRecord,
    packet: &RenderPacket,
    place: Option<&crate::models::PlaceRecord>,
) -> VnScene {
    let dashboard = &packet.visible_state.dashboard;
    let scene_location = display_location(world, dashboard.location.as_str(), place);
    let text_blocks = packet
        .narrative_scene
        .as_ref()
        .filter(|scene| !scene.text_blocks.is_empty())
        .map_or_else(
            || {
                if packet.turn_id == INITIAL_TURN_ID && dashboard.status == "흐름 수렴 중" {
                    return Vec::new();
                }
                vec![
                    format!(
                        "`{}`의 표면에서 장면이 열린다. 지금 확정된 변화는 세계의 장부에 남았다.",
                        dashboard.location
                    ),
                    format!(
                        "{} 다음 장면은 아직 플레이어가 확인한 단서 안에 머문다.",
                        dashboard.status
                    ),
                ]
            },
            |scene| {
                scene
                    .text_blocks
                    .iter()
                    .map(|block| redact_guide_choice_public_hints(block))
                    .collect()
            },
        );
    let scan_targets = player_visible_scan_targets(&packet.visible_state.scan_targets);
    VnScene {
        location: scene_location,
        current_event: display_event(dashboard.current_event.as_str()),
        status: redact_guide_choice_public_hints(dashboard.status.as_str()),
        text_blocks,
        scan_lines: scan_targets
            .iter()
            .map(|target| {
                redact_guide_choice_public_hints(&format!(
                    "{} / {} / {} / {}",
                    target.target, target.class, target.distance, target.thought
                ))
            })
            .collect(),
        adjudication: packet.adjudication.as_ref().map(vn_adjudication),
    }
}

fn player_visible_dashboard(
    world: &WorldRecord,
    dashboard: &DashboardSummary,
    place: Option<&crate::models::PlaceRecord>,
) -> DashboardSummary {
    DashboardSummary {
        phase: player_visible_phase(dashboard.phase.as_str()),
        location: display_location(world, dashboard.location.as_str(), place),
        anchor_invariant: "플레이어-visible".to_owned(),
        current_event: display_event(dashboard.current_event.as_str()),
        status: redact_guide_choice_public_hints(dashboard.status.as_str()),
    }
}

fn player_visible_scan_targets(targets: &[ScanTarget]) -> Vec<ScanTarget> {
    targets.iter().map(player_visible_scan_target).collect()
}

fn player_visible_scan_target(target: &ScanTarget) -> ScanTarget {
    if leaks_internal_anchor_or_hidden_text([
        target.target.as_str(),
        target.class.as_str(),
        target.distance.as_str(),
        target.thought.as_str(),
    ]) {
        return ScanTarget {
            target: "미확정 단서".to_owned(),
            class: "unknown".to_owned(),
            distance: "주변".to_owned(),
            thought: "아직 이름 붙일 증거가 부족하다".to_owned(),
        };
    }
    target.clone()
}

fn leaks_internal_anchor_or_hidden_text<'a>(parts: impl IntoIterator<Item = &'a str>) -> bool {
    parts.into_iter().any(|part| {
        [
            "숨겨진 진실",
            "숨겨져",
            "hidden",
            "secret",
            "anchor_character",
            "앵커 인물",
            "시드가 정한",
            "정체와 역할",
            "seed-defined",
        ]
        .iter()
        .any(|needle| part.contains(needle))
    })
}

fn vn_adjudication(report: &crate::models::AdjudicationReport) -> VnAdjudication {
    VnAdjudication {
        outcome: report.outcome.clone(),
        summary: redact_guide_choice_public_hints(report.summary.as_str()),
        visible_constraints: report
            .visible_constraints
            .iter()
            .map(|constraint| redact_guide_choice_public_hints(constraint))
            .collect(),
    }
}

fn current_turn_status(
    packet: &RenderPacket,
    turn_log: &[VnTurnLogSummary],
) -> VnCurrentTurnStatus {
    let current_log = turn_log
        .iter()
        .rev()
        .find(|entry| entry.turn_id == packet.turn_id)
        .or_else(|| turn_log.last());
    VnCurrentTurnStatus {
        turn_id: packet.turn_id.clone(),
        input: current_log.map(|entry| entry.input.clone()),
        input_kind: current_log.map(|entry| entry.input_kind.clone()),
        selected_choice: current_log
            .and_then(|entry| entry.selected_choice.clone())
            .map(|choice| redact_guide_choice_public_hints(choice.as_str())),
        summary: redact_guide_choice_public_hints(packet.visible_state.dashboard.status.as_str()),
    }
}

fn protagonist_status(
    world: &WorldRecord,
    snapshot: &TurnSnapshot,
    dashboard: &DashboardSummary,
    place: Option<&crate::models::PlaceRecord>,
    protagonist: Option<&crate::models::CharacterRecord>,
) -> VnProtagonistStatus {
    let protagonist_name = protagonist
        .map(|character| character.name.visible.as_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("미정");
    let protagonist_role = protagonist
        .map(|character| character.role.as_str())
        .filter(|role| !role.trim().is_empty());
    let body_needs = protagonist.map(|character| &character.body.needs);
    let current_event = snapshot
        .current_event
        .as_ref()
        .map(|event| event.event_id.clone());
    let current_event_progress = snapshot
        .current_event
        .as_ref()
        .map(|event| event.progress.clone());
    let inventory = snapshot
        .protagonist_state
        .inventory
        .iter()
        .map(|value| redact_guide_choice_public_hints(value))
        .collect::<Vec<_>>();
    let body = snapshot
        .protagonist_state
        .body
        .iter()
        .map(|value| redact_guide_choice_public_hints(value))
        .collect::<Vec<_>>();
    let mind = snapshot
        .protagonist_state
        .mind
        .iter()
        .map(|value| redact_guide_choice_public_hints(value))
        .collect::<Vec<_>>();
    let open_questions = snapshot
        .open_questions
        .iter()
        .map(|value| redact_guide_choice_public_hints(value))
        .collect::<Vec<_>>();
    let status_source = VnStatusRowSource {
        snapshot,
        dashboard,
        world,
        place,
        protagonist_name,
        protagonist_role,
        body_needs,
        current_event: current_event.as_deref(),
        current_event_progress: current_event_progress.as_deref(),
        inventory: &inventory,
        body: &body,
        mind: &mind,
        open_questions: &open_questions,
    };
    let dashboard_rows = protagonist_status_rows(&status_source);
    VnProtagonistStatus {
        status_schema: VN_PROTAGONIST_STATUS_SCHEMA_VERSION.to_owned(),
        phase: player_visible_phase(snapshot.phase.as_str()),
        location: display_location(world, snapshot.protagonist_state.location.as_str(), place),
        current_event: current_event
            .as_deref()
            .map(display_event)
            .filter(|event| event != "아직 이름 붙은 사건 없음"),
        current_event_progress: current_event_progress
            .as_deref()
            .map(display_progress)
            .filter(|progress| !progress.trim().is_empty()),
        inventory,
        body,
        mind,
        open_questions,
        dashboard_rows,
    }
}

struct VnStatusRowSource<'a> {
    snapshot: &'a TurnSnapshot,
    dashboard: &'a DashboardSummary,
    world: &'a WorldRecord,
    place: Option<&'a crate::models::PlaceRecord>,
    protagonist_name: &'a str,
    protagonist_role: Option<&'a str>,
    body_needs: Option<&'a crate::models::BodyNeeds>,
    current_event: Option<&'a str>,
    current_event_progress: Option<&'a str>,
    inventory: &'a [String],
    body: &'a [String],
    mind: &'a [String],
    open_questions: &'a [String],
}

fn protagonist_status_rows(source: &VnStatusRowSource<'_>) -> Vec<VnStatusRow> {
    vec![
        vitality_mind_row(source),
        needs_row(source),
        temperature_row(),
        weather_moon_row(),
        location_row(source),
        event_row(source),
        progress_row(source),
        condition_row(source),
        inventory_row(source),
        identity_row(source),
        time_senses_row(source),
    ]
}

fn vitality_mind_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "vitality_mind",
        vec![
            status_cell("🫀", "건강", human_body_status(source.body)),
            status_cell("🧠", "정신", human_mind_status(source.mind)),
        ],
    )
}

fn needs_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "needs",
        vec![
            status_cell(
                "🍞",
                "허기",
                need_value(source.body_needs.map(|needs| needs.hunger.as_str())),
            ),
            status_cell(
                "💧",
                "갈증",
                need_value(source.body_needs.map(|needs| needs.thirst.as_str())),
            ),
            status_cell(
                "🛌",
                "피로",
                need_value(source.body_needs.map(|needs| needs.fatigue.as_str())),
            ),
        ],
    )
}

fn temperature_row() -> VnStatusRow {
    status_row(
        "temperature",
        vec![
            status_cell("🌡️", "체온", "스스로 느끼기엔 크게 무너지지 않음"),
            status_cell("🏕️", "기온", "피부로 확정할 단서가 아직 부족함"),
        ],
    )
}

fn weather_moon_row() -> VnStatusRow {
    status_row(
        "weather_moon",
        vec![
            status_cell("🌦️", "날씨", "장면 안에서 아직 뚜렷하게 감지되지 않음"),
            status_cell("🌙", "달의 위상", "아직 올려다볼 근거가 없음"),
        ],
    )
}

fn location_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "location",
        vec![status_cell(
            "📍",
            "위치",
            display_location(
                source.world,
                source.dashboard.location.as_str(),
                source.place,
            ),
        )],
    )
}

fn event_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "event",
        vec![status_cell(
            "🧭",
            "진행 중 사건",
            source.current_event.map_or_else(
                || display_event(source.dashboard.current_event.as_str()),
                display_event,
            ),
        )],
    )
}

fn progress_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "progress",
        vec![status_cell(
            "📖",
            "진행도",
            source.current_event_progress.map_or_else(
                || display_progress(source.snapshot.phase.as_str()),
                display_progress,
            ),
        )],
    )
}

fn condition_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "condition",
        vec![
            status_cell("🧩", "상태", source.dashboard.status.as_str()),
            status_cell("🕯️", "희망", hope_status(source.open_questions)),
            status_cell("⚠️", "치명 상태", critical_status(source.body)),
        ],
    )
}

fn inventory_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "inventory",
        vec![status_cell(
            "🎒",
            "소지품",
            list_or(source.inventory, "손에 든 것은 아직 없음"),
        )],
    )
}

fn identity_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "identity",
        vec![
            status_cell(
                "👤",
                "이름",
                identity_value(source.protagonist_name, source.protagonist_role),
            ),
            status_cell("🎂", "나이", "아직 명시되지 않음"),
            status_cell("🔁", "턴", source.snapshot.turn_id.as_str()),
        ],
    )
}

fn time_senses_row(source: &VnStatusRowSource<'_>) -> VnStatusRow {
    status_row(
        "time_senses",
        vec![
            status_cell("🕰️", "시간", time_status(source.snapshot.phase.as_str())),
            status_cell(
                "👁️",
                "감각",
                senses_status(
                    source.dashboard,
                    source.open_questions,
                    source.snapshot.phase.as_str(),
                ),
            ),
            status_cell("🍃", "공기", air_status(source.world)),
            status_cell("⏳", "경과", elapsed_status(source.snapshot.phase.as_str())),
        ],
    )
}

fn status_row(row_id: &str, cells: Vec<VnStatusCell>) -> VnStatusRow {
    VnStatusRow {
        row_id: row_id.to_owned(),
        cells,
    }
}

fn status_cell(emoji: &str, label: &str, value: impl Into<String>) -> VnStatusCell {
    let raw_value = value.into();
    VnStatusCell {
        emoji: emoji.to_owned(),
        label: label.to_owned(),
        value: redact_guide_choice_public_hints(raw_value.as_str()),
    }
}

fn human_body_status(body: &[String]) -> String {
    list_or(body, "겉으로 드러난 큰 손상은 아직 없음")
}

fn human_mind_status(mind: &[String]) -> String {
    let visible = mind
        .iter()
        .filter_map(|entry| match entry.as_str() {
            "pre-event calm" => Some("낯선 상황에서도 침착함".to_owned()),
            "선택한 방향으로 몸이 움직이기 시작한다" => {
                Some("방금 고른 행동의 단서를 정리하는 중".to_owned())
            }
            value if value.trim().is_empty() => None,
            value if value.contains('_') || value.contains("event") => None,
            value => Some(value.to_owned()),
        })
        .collect::<Vec<_>>();
    list_or(&visible, "낯선 세계를 붙잡아 해석하는 중")
}

fn need_value(value: Option<&str>) -> String {
    match value.filter(|value| !value.trim().is_empty()) {
        Some("humanly sensed" | "world-dependent") | None => {
            "몸의 요구는 아직 뚜렷하게 올라오지 않음".to_owned()
        }
        Some(value) => value.to_owned(),
    }
}

fn hope_status(open_questions: &[String]) -> String {
    if open_questions.is_empty() {
        "아직 붙잡을 수 있는 여지가 남아 있음".to_owned()
    } else {
        "아직 확인해야 할 단서가 남아 있음".to_owned()
    }
}

fn critical_status(body: &[String]) -> String {
    if body
        .iter()
        .any(|entry| entry.contains("치명") || entry.contains("죽음") || entry.contains("출혈"))
    {
        "즉시 살펴야 할 위험 신호가 있음".to_owned()
    } else {
        "치명 징후는 아직 보이지 않음".to_owned()
    }
}

fn identity_value(name: &str, role: Option<&str>) -> String {
    if name == "미정" {
        return "아직 이름을 밝히지 않음".to_owned();
    }
    match role.filter(|role| concise_player_visible_value(role)) {
        Some(role) => format!("{name} ({role})"),
        None => name.to_owned(),
    }
}

fn senses_status(dashboard: &DashboardSummary, open_questions: &[String], phase: &str) -> String {
    if open_questions.is_empty() {
        format!("{}의 표면 단서를 읽는 중", dashboard.location)
    } else if phase != "interlude"
        && open_questions
            .first()
            .is_some_and(|question| question.contains("첫 사건은 아직 시작되지 않았다"))
    {
        "방금 관찰한 단서를 정리하는 중".to_owned()
    } else {
        open_questions
            .first()
            .cloned()
            .unwrap_or_else(|| "주변 단서를 다시 살피는 중".to_owned())
    }
}

fn time_status(phase: &str) -> String {
    match phase {
        "interlude" => "첫 선택 직전".to_owned(),
        "event" => "첫 관찰 직후".to_owned(),
        _ => "장면 안에서 흐르는 중".to_owned(),
    }
}

fn air_status(world: &WorldRecord) -> String {
    let premise = world_visible_premise_text(world);
    if premise.contains("비가 그친") {
        "비가 그친 직후의 젖은 공기".to_owned()
    } else if premise.contains("항구") {
        "물기와 소금기가 섞인 공기".to_owned()
    } else {
        "아직 방향을 단정하기 어려움".to_owned()
    }
}

fn elapsed_status(phase: &str) -> String {
    match phase {
        "interlude" => "첫 선택 전".to_owned(),
        "event" => "한 번의 행동이 반영됨".to_owned(),
        other => format!("{other} 국면"),
    }
}

fn player_visible_phase(phase: &str) -> String {
    match phase {
        "interlude" => "도입".to_owned(),
        "event" => "진행 중".to_owned(),
        other => other.to_owned(),
    }
}

fn display_event(event: &str) -> String {
    match event {
        "" | "none" => "아직 이름 붙은 사건 없음".to_owned(),
        "interlude" => "첫 사건 전조".to_owned(),
        other => other.to_owned(),
    }
}

fn display_progress(progress: &str) -> String {
    match progress {
        "" | "interlude" => "첫 선택을 기다리는 중".to_owned(),
        "event" => "관찰을 마치고 다음 행동을 고르는 중".to_owned(),
        "사건의 초입에서 다음 박자를 기다림" => {
            "관찰을 마치고 다음 행동을 고르는 중".to_owned()
        }
        other => other.to_owned(),
    }
}

fn display_location(
    world: &WorldRecord,
    location: &str,
    place: Option<&crate::models::PlaceRecord>,
) -> String {
    if let Some(place) = place
        && place.name != "미정"
    {
        return place.name.clone();
    }
    if location == crate::models::OPENING_LOCATION_ID {
        return inferred_opening_place(world);
    }
    location.to_owned()
}

fn inferred_opening_place(world: &WorldRecord) -> String {
    let premise = world_visible_premise_text(world);
    if premise.contains("비가 그친 항구") || premise.contains("항구 도시") {
        return "비가 그친 항구".to_owned();
    }
    if premise.contains("항구") {
        return "항구".to_owned();
    }
    "첫 장소".to_owned()
}

fn world_visible_premise_text(world: &WorldRecord) -> String {
    format!(
        "{} {} {}",
        world.premise.genre, world.premise.protagonist, world.premise.opening_state
    )
}

fn concise_player_visible_value(value: &str) -> bool {
    let chars = value.chars().count();
    chars <= 24 && !value.contains('(') && !value.contains("주인공 (")
}

fn list_or(values: &[String], empty_text: &str) -> String {
    if values.is_empty() {
        empty_text.to_owned()
    } else {
        values.join(" / ")
    }
}

fn is_turn_on_cadence(turn_index: u32, cadence: u32) -> bool {
    turn_index == (turn_index / cadence) * cadence
}

fn recent_turn_log(
    world: &WorldRecord,
    files: &crate::store::WorldFilePaths,
    session_id: &str,
) -> Result<Vec<VnTurnLogSummary>> {
    let path = files
        .dir
        .join("sessions")
        .join(session_id)
        .join(TURN_LOG_FILENAME);
    let raw = if path.exists() {
        fs::read_to_string(&path)
            .with_context(|| format!("failed to read turn log {}", path.display()))?
    } else {
        String::new()
    };
    let mut entries = Vec::new();
    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let entry: TurnLogEntry = serde_json::from_str(line)
            .with_context(|| format!("failed to parse turn log {}", path.display()))?;
        entries.push(vn_turn_log_summary(entry));
    }
    prepend_initial_turn_log_if_missing(world, files, session_id, &mut entries)?;
    let start = entries.len().saturating_sub(RECENT_TURN_LOG_LIMIT);
    Ok(entries.split_off(start))
}

fn prepend_initial_turn_log_if_missing(
    world: &WorldRecord,
    files: &crate::store::WorldFilePaths,
    session_id: &str,
    entries: &mut Vec<VnTurnLogSummary>,
) -> Result<()> {
    if entries.iter().any(|entry| entry.turn_id == INITIAL_TURN_ID) {
        return Ok(());
    }
    let render_packet_path = files
        .dir
        .join("sessions")
        .join(session_id)
        .join("render_packets")
        .join(format!("{INITIAL_TURN_ID}.json"));
    if !render_packet_path.exists() {
        return Ok(());
    }
    let packet: RenderPacket = crate::store::read_json(&render_packet_path)?;
    entries.insert(
        0,
        VnTurnLogSummary {
            turn_id: INITIAL_TURN_ID.to_owned(),
            input: "세계 생성".to_owned(),
            input_kind: "world_start".to_owned(),
            selected_choice: None,
            canon_event_id: "world_init".to_owned(),
            render_packet_ref: render_packet_path.display().to_string(),
            render_markdown: Some(redact_guide_choice_public_hints(&render_packet_markdown(
                &packet,
            ))),
            created_at: world.created_at.clone(),
        },
    );
    Ok(())
}

fn vn_turn_log_summary(entry: TurnLogEntry) -> VnTurnLogSummary {
    let render_markdown = load_turn_markdown(entry.render_packet_ref.as_str());
    VnTurnLogSummary {
        turn_id: entry.turn_id,
        input: entry.input,
        input_kind: entry.input_kind.to_string(),
        selected_choice: entry.selected_choice.map(|choice| {
            let slot = if is_guide_choice_tag(choice.tag.as_str()) {
                GUIDE_CHOICE_SLOT
            } else {
                choice.slot
            };
            redact_guide_choice_public_hints(&format!("{}. {}", slot, choice.tag))
        }),
        canon_event_id: entry.canon_event_id,
        render_packet_ref: entry.render_packet_ref,
        render_markdown,
        created_at: entry.created_at,
    }
}

fn load_turn_markdown(render_packet_ref: &str) -> Option<String> {
    let packet: RenderPacket =
        crate::store::read_json(PathBuf::from(render_packet_ref).as_path()).ok()?;
    Some(redact_guide_choice_public_hints(&render_packet_markdown(
        &packet,
    )))
}

fn vn_choices(world: &WorldRecord, choices: &[TurnChoice]) -> Vec<VnChoice> {
    normalize_turn_choices(choices)
        .into_iter()
        .map(|choice| {
            let requires_inline_text = choice.slot == FREEFORM_CHOICE_SLOT;
            let tag = if is_guide_choice_tag(choice.tag.as_str()) {
                GUIDE_CHOICE_TAG.to_owned()
            } else {
                redact_guide_choice_public_hints(choice.tag.as_str())
            };
            let intent = redact_guide_choice_public_hints(choice.player_visible_intent());
            let input_template = if requires_inline_text {
                format!("{} <action>", choice.slot)
            } else {
                choice.slot.to_string()
            };
            let command_template = if requires_inline_text {
                format!(
                    "singulari-world turn --world-id {} --input '{} <action>' --render",
                    world.world_id, choice.slot
                )
            } else {
                format!(
                    "singulari-world turn --world-id {} --input '{}' --render",
                    world.world_id, choice.slot
                )
            };
            VnChoice {
                slot: choice.slot,
                label: format!("{}. {}", choice.slot, tag),
                tag,
                intent,
                requires_inline_text,
                input_template,
                command_template,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{BuildVnPacketOptions, HOST_WORKER_VISUAL_GENERATOR, build_vn_packet};
    use crate::backend_selection::{
        WorldBackendSelection, WorldTextBackend, WorldVisualBackend, save_world_backend_selection,
    };
    use crate::models::{
        FREEFORM_CHOICE_SLOT, GUIDE_CHOICE_REDACTED_INTENT, GUIDE_CHOICE_SLOT, GUIDE_CHOICE_TAG,
        LEGACY_GUIDE_CHOICE_TAG, TurnChoice,
    };
    use crate::store::{InitWorldOptions, init_world};
    use crate::turn::{AdvanceTurnOptions, advance_turn};
    use tempfile::tempdir;

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "integration-style VN packet fixture is clearer as one executable example"
    )]
    fn vn_packet_projects_visible_turn_and_freeform_choice() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_packet
title: "VN 세계"
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
            world_id: "stw_vn_packet".to_owned(),
            input: "6 세아에게 낮게 묻는다".to_owned(),
        })?;
        let mut options = BuildVnPacketOptions::new("stw_vn_packet".to_owned());
        options.store_root = Some(store);
        let packet = build_vn_packet(&options)?;
        assert_eq!(packet.schema_version, super::VN_PACKET_SCHEMA_VERSION);
        assert!(
            packet
                .choices
                .iter()
                .any(|choice| choice.slot == FREEFORM_CHOICE_SLOT && choice.requires_inline_text)
        );
        assert!(
            packet
                .image
                .image_prompt
                .contains("Use only player-visible information")
        );
        assert_eq!(packet.image.generator, HOST_WORKER_VISUAL_GENERATOR);
        assert_eq!(packet.image.budget_policy.mode, "balanced");
        assert_eq!(packet.image.auto_decision.action, "generate_scene");
        assert!(packet.image.image_generation_job.is_some());
        assert_eq!(packet.visual_assets.image_generation_jobs.len(), 3);
        assert!(packet.image.image_prompt.contains("Scene narrative"));
        assert!(
            packet
                .visual_assets
                .menu_background
                .asset_url
                .contains("/world-assets/stw_vn_packet/")
        );
        assert!(
            !packet
                .image
                .image_prompt
                .contains("manifestation rules are world-local")
        );
        assert!(packet.codex_surface.full_markdown.contains("### 선택"));
        for hidden_marker in [
            "숨겨진 진실",
            "앵커 인물",
            "anchor_character",
            "시드가 정한",
            "정체와 역할",
            "안내자의 선택",
        ] {
            assert!(
                !packet.scene.text_blocks.join("\n").contains(hidden_marker),
                "scene leaked marker: {hidden_marker}"
            );
            assert!(
                !packet.scene.scan_lines.join("\n").contains(hidden_marker),
                "scene scan leaked marker: {hidden_marker}"
            );
            assert!(
                !packet.codex_surface.full_markdown.contains(hidden_marker),
                "codex markdown leaked marker: {hidden_marker}"
            );
            assert!(
                !packet.image.image_prompt.contains(hidden_marker),
                "image prompt leaked marker: {hidden_marker}"
            );
            assert!(
                !packet
                    .codex_surface
                    .turn_log
                    .iter()
                    .filter_map(|entry| entry.render_markdown.as_deref())
                    .any(|markdown| markdown.contains(hidden_marker)),
                "turn log markdown leaked marker: {hidden_marker}"
            );
        }
        assert_eq!(packet.codex_surface.turn_log.len(), 2);
        assert_eq!(packet.codex_surface.turn_log[0].input_kind, "world_start");
        assert_eq!(
            packet.codex_surface.turn_log[1].input_kind,
            "freeform_action"
        );
        assert_eq!(
            packet.codex_surface.current_turn.selected_choice.as_deref(),
            Some("6. 자유서술")
        );
        assert_eq!(packet.codex_surface.protagonist.location, "첫 장소");
        assert_eq!(
            packet.codex_surface.protagonist.status_schema,
            super::VN_PROTAGONIST_STATUS_SCHEMA_VERSION
        );
        assert!(
            packet
                .codex_surface
                .protagonist
                .dashboard_rows
                .iter()
                .flat_map(|row| row.cells.iter())
                .any(|cell| cell.label == "건강" && !cell.emoji.is_empty())
        );
        Ok(())
    }

    #[test]
    fn balanced_budget_generates_turn_cg_on_sparse_cadence() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_budget
title: "예산 세계"
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
        for _ in 0..5 {
            advance_turn(&AdvanceTurnOptions {
                store_root: Some(store.clone()),
                world_id: "stw_vn_budget".to_owned(),
                input: "1".to_owned(),
            })?;
        }
        let mut options = BuildVnPacketOptions::new("stw_vn_budget".to_owned());
        options.store_root = Some(store.clone());
        let packet = build_vn_packet(&options)?;
        assert_eq!(packet.turn_id, "turn_0005");
        assert_eq!(packet.image.auto_decision.action, "generate_scene");
        assert!(packet.image.image_generation_job.is_some());
        assert_eq!(packet.image.auto_decision.cadence_turns_remaining, 0);
        let Some(job) = packet.image.image_generation_job.as_ref() else {
            anyhow::bail!("turn_0005 should have a scene CG job");
        };
        assert!(job.reference_paths.is_empty());

        let sheet_dir = initialized
            .world_dir
            .join(crate::visual_assets::VN_ASSETS_DIR)
            .join(crate::visual_assets::CHARACTER_SHEETS_DIR);
        std::fs::create_dir_all(&sheet_dir)?;
        std::fs::write(sheet_dir.join("char_protagonist.png"), b"fake")?;
        std::fs::write(sheet_dir.join("char_anchor.png"), b"fake")?;
        let mut options = BuildVnPacketOptions::new("stw_vn_budget".to_owned());
        options.store_root = Some(store);
        let packet = build_vn_packet(&options)?;
        let Some(job) = packet.image.image_generation_job.as_ref() else {
            anyhow::bail!("turn_0005 should still have a scene CG job");
        };
        assert_eq!(job.reference_paths.len(), 1);
        assert!(job.reference_paths[0].ends_with("char_protagonist.png"));
        assert!(
            !job.reference_paths
                .iter()
                .any(|path| path.ends_with("char_anchor.png"))
        );
        Ok(())
    }

    #[test]
    fn vn_choices_redact_legacy_guide_choice_label() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_legacy_guide
title: "레거시 선택지 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store),
            session_id: None,
        })?;
        let projected = super::vn_choices(
            &initialized.world,
            &[TurnChoice {
                slot: 4,
                tag: LEGACY_GUIDE_CHOICE_TAG.to_owned(),
                intent: "안내자가 보기에 가장 덜 무모하고 가장 의미 있는 길을 따른다".to_owned(),
            }],
        );
        let Some(choice) = projected
            .iter()
            .find(|choice| choice.slot == GUIDE_CHOICE_SLOT)
        else {
            anyhow::bail!("slot 7 choice missing");
        };
        assert_eq!(choice.tag, GUIDE_CHOICE_TAG);
        assert_eq!(choice.label, "7. 판단 위임");
        assert_eq!(choice.intent, GUIDE_CHOICE_REDACTED_INTENT);
        assert!(!choice.label.contains("안내자"));
        assert!(!choice.intent.contains("안내자"));
        Ok(())
    }

    #[test]
    fn webgpt_visual_backend_uses_world_locked_cadence() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_webgpt_visual
title: "웹 이미지 세계"
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
        save_world_backend_selection(
            Some(store.as_path()),
            &WorldBackendSelection::new(
                "stw_vn_webgpt_visual".to_owned(),
                WorldTextBackend::Webgpt,
                WorldVisualBackend::Webgpt,
                "test",
            ),
        )?;
        for _ in 0..2 {
            advance_turn(&AdvanceTurnOptions {
                store_root: Some(store.clone()),
                world_id: "stw_vn_webgpt_visual".to_owned(),
                input: "1".to_owned(),
            })?;
        }

        let mut options = BuildVnPacketOptions::new("stw_vn_webgpt_visual".to_owned());
        options.store_root = Some(store);
        let packet = build_vn_packet(&options)?;

        assert_eq!(packet.turn_id, "turn_0002");
        assert_eq!(packet.image.budget_policy.turn_cg_cadence_min, 2);
        assert_eq!(packet.image.auto_decision.action, "generate_scene");
        assert!(packet.image.image_generation_job.is_some());
        Ok(())
    }

    #[test]
    fn pending_turn_cg_keeps_previous_saved_cg_visible() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_vn_cg_residue
title: "잔존 CG 세계"
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
        save_world_backend_selection(
            Some(store.as_path()),
            &WorldBackendSelection::new(
                "stw_vn_cg_residue".to_owned(),
                WorldTextBackend::Webgpt,
                WorldVisualBackend::Webgpt,
                "test",
            ),
        )?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_vn_cg_residue".to_owned(),
            input: "1".to_owned(),
        })?;
        let turn_cg_dir = initialized
            .world_dir
            .join(crate::visual_assets::VN_ASSETS_DIR)
            .join(super::TURN_CG_DIR);
        std::fs::create_dir_all(turn_cg_dir.as_path())?;
        std::fs::write(turn_cg_dir.join("turn_0001.png"), b"fake previous cg")?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_vn_cg_residue".to_owned(),
            input: "1".to_owned(),
        })?;

        let mut options = BuildVnPacketOptions::new("stw_vn_cg_residue".to_owned());
        options.store_root = Some(store);
        let packet = build_vn_packet(&options)?;

        assert_eq!(packet.turn_id, "turn_0002");
        assert_eq!(packet.image.auto_decision.action, "generate_scene");
        assert!(packet.image.image_generation_job.is_some());
        assert!(!packet.image.exists);
        assert_eq!(
            packet.image.existing_image_url.as_deref(),
            Some("/world-assets/stw_vn_cg_residue/assets/vn/turn_cg/turn_0001.png")
        );
        Ok(())
    }
}
