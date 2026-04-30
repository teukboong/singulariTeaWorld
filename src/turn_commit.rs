use crate::agent_bridge::{CommittedAgentTurn, PendingAgentTurn};
use crate::models::{
    ADJUDICATION_SCHEMA_VERSION, AdjudicationReport, DashboardSummary,
    NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, RENDER_PACKET_SCHEMA_VERSION, RenderPacket,
    TurnChoice, TurnSnapshot, VisibleState,
};
use crate::store::{
    acquire_world_commit_lock, append_jsonl_durable, read_json, resolve_store_paths,
    world_file_paths, write_json,
};
use crate::vn::{BuildVnPacketOptions, VnPacket, build_vn_packet};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const TURN_COMMIT_ENVELOPE_SCHEMA_VERSION: &str = "singulari.turn_commit.v1";
pub const TURN_COMMIT_JOURNAL_RECOVERY_SCHEMA_VERSION: &str =
    "singulari.turn_commit_journal_recovery.v1";
pub const TURN_COMMIT_JOURNAL_RECOVERY_ACTION_SCHEMA_VERSION: &str =
    "singulari.turn_commit_journal_recovery_action.v1";
pub const TURN_COMMITS_FILENAME: &str = "turn_commits.jsonl";
const AGENT_BRIDGE_DIR: &str = "agent_bridge";
const AGENT_COMMITTED_TURNS_DIR: &str = "committed_turns";
const AGENT_PENDING_TURN_FILENAME: &str = "pending_turn.json";
const AGENT_COMMIT_RECORD_FILENAME: &str = "commit_record.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCommitStatus {
    Prepared,
    Committed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCommitEnvelope {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub parent_turn_id: String,
    pub player_input: String,
    pub status: TurnCommitStatus,
    pub pending_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render_packet_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_record_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub world_court_verdict_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_stage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnMaterializationRepairReport {
    pub schema_version: String,
    pub world_id: String,
    pub committed_envelopes: usize,
    pub render_packets_repaired: usize,
    pub commit_records_repaired: usize,
    pub repaired_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCommitJournalRecoveryReport {
    pub schema_version: String,
    pub world_id: String,
    pub inspected_envelopes: usize,
    pub committed_envelopes_appended: usize,
    pub stale_pending_files_removed: usize,
    #[serde(default)]
    pub actions: Vec<TurnCommitJournalRecoveryAction>,
    pub materialization_repair: TurnMaterializationRepairReport,
    pub recovered_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnCommitJournalRecoveryAction {
    pub schema_version: String,
    pub turn_id: String,
    pub action: String,
    pub detail: String,
}

impl TurnCommitEnvelope {
    #[must_use]
    pub fn prepared(pending: &PendingAgentTurn, parent_turn_id: &str, created_at: String) -> Self {
        Self {
            schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
            world_id: pending.world_id.clone(),
            turn_id: pending.turn_id.clone(),
            parent_turn_id: parent_turn_id.to_owned(),
            player_input: pending.player_input.clone(),
            status: TurnCommitStatus::Prepared,
            pending_ref: pending.pending_ref.clone(),
            response_path: None,
            render_packet_path: None,
            commit_record_path: None,
            world_court_verdict_path: None,
            failed_stage: None,
            error: None,
            created_at,
        }
    }

    #[must_use]
    pub fn committed(
        pending: &PendingAgentTurn,
        parent_turn_id: &str,
        committed: &CommittedAgentTurn,
    ) -> Self {
        Self {
            schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
            world_id: pending.world_id.clone(),
            turn_id: pending.turn_id.clone(),
            parent_turn_id: parent_turn_id.to_owned(),
            player_input: pending.player_input.clone(),
            status: TurnCommitStatus::Committed,
            pending_ref: pending.pending_ref.clone(),
            response_path: Some(committed.response_path.clone()),
            render_packet_path: Some(committed.render_packet_path.clone()),
            commit_record_path: Some(committed.commit_record_path.clone()),
            world_court_verdict_path: committed.world_court_verdict_path.clone(),
            failed_stage: None,
            error: None,
            created_at: committed.committed_at.clone(),
        }
    }

    #[must_use]
    pub fn failed(
        pending: &PendingAgentTurn,
        parent_turn_id: &str,
        failed_stage: &str,
        error: String,
        created_at: String,
    ) -> Self {
        Self {
            schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
            world_id: pending.world_id.clone(),
            turn_id: pending.turn_id.clone(),
            parent_turn_id: parent_turn_id.to_owned(),
            player_input: pending.player_input.clone(),
            status: TurnCommitStatus::Failed,
            pending_ref: pending.pending_ref.clone(),
            response_path: None,
            render_packet_path: None,
            commit_record_path: None,
            world_court_verdict_path: None,
            failed_stage: Some(failed_stage.to_owned()),
            error: Some(error),
            created_at,
        }
    }

    #[must_use]
    pub fn committed_from_existing_record(
        source: &TurnCommitEnvelope,
        committed: &CommittedAgentTurn,
    ) -> Self {
        Self {
            schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
            world_id: source.world_id.clone(),
            turn_id: source.turn_id.clone(),
            parent_turn_id: source.parent_turn_id.clone(),
            player_input: source.player_input.clone(),
            status: TurnCommitStatus::Committed,
            pending_ref: source.pending_ref.clone(),
            response_path: Some(committed.response_path.clone()),
            render_packet_path: Some(committed.render_packet_path.clone()),
            commit_record_path: Some(committed.commit_record_path.clone()),
            world_court_verdict_path: committed.world_court_verdict_path.clone(),
            failed_stage: None,
            error: None,
            created_at: committed.committed_at.clone(),
        }
    }
}

/// Append a turn commit envelope to the per-world commit ledger.
///
/// # Errors
///
/// Returns an error when the JSONL append fails.
pub fn append_turn_commit_envelope(world_dir: &Path, envelope: &TurnCommitEnvelope) -> Result<()> {
    append_jsonl_durable(&world_dir.join(TURN_COMMITS_FILENAME), envelope)
}

/// Recover idempotent turn-commit journal state after a crash or worker restart.
///
/// The recovery is intentionally conservative: it appends a missing committed
/// envelope only when the durable `CommittedAgentTurn` record already exists for
/// that turn. It never invents a committed turn from prose, snapshots, or
/// projection files alone.
///
/// # Errors
///
/// Returns an error when the world cannot be loaded, an existing durable commit
/// record disagrees with the journal turn, or materialization repair fails.
pub fn recover_turn_commit_journal(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<TurnCommitJournalRecoveryReport> {
    let store_paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&store_paths, world_id);
    let world_lock = acquire_world_commit_lock(&files.dir, "recover_turn_commit_journal")?;
    let ledger_path = files.dir.join(TURN_COMMITS_FILENAME);
    let envelopes = if ledger_path.is_file() {
        read_turn_commit_envelopes(&ledger_path)?
    } else {
        Vec::new()
    };
    let latest_snapshot: Option<TurnSnapshot> = read_json(&files.latest_snapshot).ok();
    let mut actions = Vec::new();
    let mut committed_envelopes_appended = 0usize;
    let mut stale_pending_files_removed = 0usize;

    for envelope in envelopes.iter().filter(|envelope| {
        matches!(
            envelope.status,
            TurnCommitStatus::Prepared | TurnCommitStatus::Failed
        )
    }) {
        if envelope.world_id != world_id {
            bail!(
                "turn commit journal recovery world mismatch: expected={world_id}, actual={}, turn={}",
                envelope.world_id,
                envelope.turn_id
            );
        }
        if turn_has_committed_envelope(&envelopes, envelope.turn_id.as_str()) {
            continue;
        }
        let commit_record_path = standard_commit_record_path(&files.dir, envelope.turn_id.as_str());
        if commit_record_path.is_file() {
            let committed: CommittedAgentTurn = read_json(&commit_record_path)?;
            ensure_committed_record_matches(envelope, &committed)?;
            append_turn_commit_envelope(
                files.dir.as_path(),
                &TurnCommitEnvelope::committed_from_existing_record(envelope, &committed),
            )?;
            committed_envelopes_appended += 1;
            actions.push(recovery_action(
                envelope.turn_id.as_str(),
                "append_committed_envelope",
                format!(
                    "durable commit record already existed: {}",
                    commit_record_path.display()
                ),
            ));
            if remove_matching_pending_file(&files.dir, envelope)? {
                stale_pending_files_removed += 1;
                actions.push(recovery_action(
                    envelope.turn_id.as_str(),
                    "remove_stale_pending",
                    "pending turn matched a durable committed turn".to_owned(),
                ));
            }
        } else {
            let latest_turn = latest_snapshot
                .as_ref()
                .map_or("<unknown>", |snapshot| snapshot.turn_id.as_str());
            actions.push(recovery_action(
                envelope.turn_id.as_str(),
                "no_recovery_action",
                format!(
                    "no durable commit record found; status={:?}, latest_snapshot={latest_turn}, pending_ref={}",
                    envelope.status, envelope.pending_ref
                ),
            ));
        }
    }

    if let Some(action) =
        remove_stale_standard_pending_after_committed_envelope(&files.dir, &envelopes)?
    {
        stale_pending_files_removed += 1;
        actions.push(action);
    }

    let inspected_envelopes = envelopes.len();
    drop(world_lock);
    let materialization_repair = recovery_materialization_repair(
        store_root,
        world_id,
        &ledger_path,
        committed_envelopes_appended,
    )?;
    Ok(TurnCommitJournalRecoveryReport {
        schema_version: TURN_COMMIT_JOURNAL_RECOVERY_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        inspected_envelopes,
        committed_envelopes_appended,
        stale_pending_files_removed,
        actions,
        materialization_repair,
        recovered_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn remove_stale_standard_pending_after_committed_envelope(
    world_dir: &Path,
    envelopes: &[TurnCommitEnvelope],
) -> Result<Option<TurnCommitJournalRecoveryAction>> {
    let Some(pending) = load_standard_pending_turn(world_dir)? else {
        return Ok(None);
    };
    if !turn_has_committed_envelope(envelopes, pending.turn_id.as_str()) {
        return Ok(None);
    }
    fs::remove_file(standard_pending_turn_path(world_dir)).with_context(|| {
        format!(
            "failed to remove stale pending turn after committed envelope: world_id={}, turn_id={}",
            pending.world_id, pending.turn_id
        )
    })?;
    Ok(Some(recovery_action(
        pending.turn_id.as_str(),
        "remove_stale_pending",
        "pending turn already had committed journal envelope".to_owned(),
    )))
}

fn recovery_materialization_repair(
    store_root: Option<&Path>,
    world_id: &str,
    ledger_path: &Path,
    committed_envelopes_appended: usize,
) -> Result<TurnMaterializationRepairReport> {
    if ledger_path.is_file() || committed_envelopes_appended > 0 {
        repair_turn_materializations(store_root, world_id)
    } else {
        Ok(empty_materialization_repair_report(world_id))
    }
}

/// Rebuild missing materialized files referenced by committed turn envelopes.
///
/// # Errors
///
/// Returns an error when the commit ledger is unreadable, a committed envelope is
/// malformed, or the missing files cannot be reconstructed from existing
/// committed evidence.
pub fn repair_turn_materializations(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<TurnMaterializationRepairReport> {
    let store_paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&store_paths, world_id);
    let _world_lock = acquire_world_commit_lock(&files.dir, "repair_turn_materializations")?;
    let envelopes = read_turn_commit_envelopes(files.dir.join(TURN_COMMITS_FILENAME).as_path())?;
    let committed_envelopes = envelopes
        .iter()
        .filter(|envelope| envelope.status == TurnCommitStatus::Committed)
        .count();
    let mut render_packets_repaired = 0usize;
    let mut commit_records_repaired = 0usize;

    for envelope in envelopes
        .iter()
        .filter(|envelope| envelope.status == TurnCommitStatus::Committed)
    {
        if envelope.world_id != world_id {
            bail!(
                "turn materialization repair world mismatch: expected={world_id}, actual={}, turn={}",
                envelope.world_id,
                envelope.turn_id
            );
        }
        let response_path = required_envelope_path(envelope, "response_path")?;
        let render_packet_path = required_envelope_path(envelope, "render_packet_path")?;
        let commit_record_path = required_envelope_path(envelope, "commit_record_path")?;
        if !response_path.is_file() {
            bail!(
                "turn materialization repair requires existing agent response: turn={}, path={}",
                envelope.turn_id,
                response_path.display()
            );
        }

        if !render_packet_path.is_file() {
            let commit_record = read_json::<CommittedAgentTurn>(&commit_record_path).with_context(
                || {
                    format!(
                        "turn materialization repair cannot rebuild render packet without commit record: turn={}, commit_record={}",
                        envelope.turn_id,
                        commit_record_path.display()
                    )
                },
            )?;
            let render_packet = render_packet_from_committed_vn_packet(&commit_record.packet);
            write_json(&render_packet_path, &render_packet)?;
            render_packets_repaired += 1;
        }

        if !commit_record_path.is_file() {
            let packet = build_vn_packet(&BuildVnPacketOptions {
                store_root: Some(store_paths.root.clone()),
                world_id: world_id.to_owned(),
                turn_id: Some(envelope.turn_id.clone()),
                scene_image_url: None,
            })?;
            let committed = CommittedAgentTurn {
                schema_version: crate::agent_bridge::AGENT_COMMIT_RECORD_SCHEMA_VERSION.to_owned(),
                world_id: world_id.to_owned(),
                turn_id: envelope.turn_id.clone(),
                render_packet_path: render_packet_path.display().to_string(),
                response_path: response_path.display().to_string(),
                commit_record_path: commit_record_path.display().to_string(),
                world_court_verdict_path: envelope.world_court_verdict_path.clone(),
                committed_at: envelope.created_at.clone(),
                packet,
            };
            write_json(&commit_record_path, &committed)?;
            commit_records_repaired += 1;
        }
    }

    Ok(TurnMaterializationRepairReport {
        schema_version: "singulari.turn_materialization_repair.v1".to_owned(),
        world_id: world_id.to_owned(),
        committed_envelopes,
        render_packets_repaired,
        commit_records_repaired,
        repaired_at: chrono::Utc::now().to_rfc3339(),
    })
}

fn read_turn_commit_envelopes(path: &Path) -> Result<Vec<TurnCommitEnvelope>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<TurnCommitEnvelope>(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

fn ensure_committed_record_matches(
    envelope: &TurnCommitEnvelope,
    committed: &CommittedAgentTurn,
) -> Result<()> {
    if committed.world_id != envelope.world_id || committed.turn_id != envelope.turn_id {
        bail!(
            "turn commit journal recovery record mismatch: expected={}/{}, actual={}/{}",
            envelope.world_id,
            envelope.turn_id,
            committed.world_id,
            committed.turn_id
        );
    }
    Ok(())
}

fn turn_has_committed_envelope(envelopes: &[TurnCommitEnvelope], turn_id: &str) -> bool {
    envelopes.iter().any(|envelope| {
        envelope.turn_id == turn_id && envelope.status == TurnCommitStatus::Committed
    })
}

fn standard_commit_record_path(world_dir: &Path, turn_id: &str) -> PathBuf {
    world_dir
        .join(AGENT_BRIDGE_DIR)
        .join(AGENT_COMMITTED_TURNS_DIR)
        .join(turn_id)
        .join(AGENT_COMMIT_RECORD_FILENAME)
}

fn standard_pending_turn_path(world_dir: &Path) -> PathBuf {
    world_dir
        .join(AGENT_BRIDGE_DIR)
        .join(AGENT_PENDING_TURN_FILENAME)
}

fn pending_ref_path(world_dir: &Path, pending_ref: &str) -> PathBuf {
    let path = PathBuf::from(pending_ref);
    if path.is_absolute() {
        path
    } else {
        world_dir.join(path)
    }
}

fn load_standard_pending_turn(world_dir: &Path) -> Result<Option<PendingAgentTurn>> {
    let path = standard_pending_turn_path(world_dir);
    if !path.is_file() {
        return Ok(None);
    }
    read_json(&path).map(Some)
}

fn remove_matching_pending_file(world_dir: &Path, envelope: &TurnCommitEnvelope) -> Result<bool> {
    let pending_path = pending_ref_path(world_dir, envelope.pending_ref.as_str());
    if !pending_path.is_file() {
        return Ok(false);
    }
    let pending: PendingAgentTurn = read_json(&pending_path)?;
    if pending.world_id != envelope.world_id || pending.turn_id != envelope.turn_id {
        bail!(
            "turn commit journal recovery refused to remove nonmatching pending file: path={}, expected={}/{}, actual={}/{}",
            pending_path.display(),
            envelope.world_id,
            envelope.turn_id,
            pending.world_id,
            pending.turn_id
        );
    }
    fs::remove_file(&pending_path)
        .with_context(|| format!("failed to remove {}", pending_path.display()))?;
    Ok(true)
}

fn recovery_action(turn_id: &str, action: &str, detail: String) -> TurnCommitJournalRecoveryAction {
    TurnCommitJournalRecoveryAction {
        schema_version: TURN_COMMIT_JOURNAL_RECOVERY_ACTION_SCHEMA_VERSION.to_owned(),
        turn_id: turn_id.to_owned(),
        action: action.to_owned(),
        detail,
    }
}

fn empty_materialization_repair_report(world_id: &str) -> TurnMaterializationRepairReport {
    TurnMaterializationRepairReport {
        schema_version: "singulari.turn_materialization_repair.v1".to_owned(),
        world_id: world_id.to_owned(),
        committed_envelopes: 0,
        render_packets_repaired: 0,
        commit_records_repaired: 0,
        repaired_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn required_envelope_path(envelope: &TurnCommitEnvelope, field: &str) -> Result<PathBuf> {
    let value = match field {
        "response_path" => envelope.response_path.as_deref(),
        "render_packet_path" => envelope.render_packet_path.as_deref(),
        "commit_record_path" => envelope.commit_record_path.as_deref(),
        "world_court_verdict_path" => envelope.world_court_verdict_path.as_deref(),
        _ => None,
    };
    let Some(value) = value else {
        bail!(
            "turn materialization repair missing committed envelope path: turn={}, field={field}",
            envelope.turn_id
        );
    };
    Ok(PathBuf::from(value))
}

fn render_packet_from_committed_vn_packet(packet: &VnPacket) -> RenderPacket {
    RenderPacket {
        schema_version: RENDER_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: packet.world_id.clone(),
        turn_id: packet.turn_id.clone(),
        mode: packet.mode.clone(),
        narrative_contract: "repaired from committed VN packet".to_owned(),
        narrative_scene: Some(NarrativeScene {
            schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
            speaker: None,
            text_blocks: packet.scene.text_blocks.clone(),
            tone_notes: Vec::new(),
        }),
        visible_state: VisibleState {
            dashboard: DashboardSummary {
                phase: packet.codex_surface.dashboard.phase.clone(),
                location: packet.scene.location.clone(),
                anchor_invariant: packet.codex_surface.dashboard.anchor_invariant.clone(),
                current_event: packet.scene.current_event.clone(),
                status: packet.scene.status.clone(),
            },
            scan_targets: packet
                .scene
                .scan_lines
                .iter()
                .map(|line| scan_line_target(line.as_str()))
                .collect(),
            choices: packet
                .choices
                .iter()
                .map(|choice| TurnChoice {
                    slot: choice.slot,
                    tag: choice.tag.clone(),
                    intent: choice.intent.clone(),
                })
                .collect(),
        },
        adjudication: packet
            .scene
            .adjudication
            .as_ref()
            .map(|adjudication| AdjudicationReport {
                schema_version: ADJUDICATION_SCHEMA_VERSION.to_owned(),
                world_id: packet.world_id.clone(),
                turn_id: packet.turn_id.clone(),
                outcome: adjudication.outcome.clone(),
                summary: adjudication.summary.clone(),
                gates: Vec::new(),
                visible_constraints: adjudication.visible_constraints.clone(),
                consequences: Vec::new(),
            }),
        codex_view: packet.codex_surface.codex_view.clone(),
        canon_delta_refs: Vec::new(),
        forbidden_reveals: Vec::new(),
        style_notes: vec!["repaired_from_committed_vn_packet".to_owned()],
    }
}

fn scan_line_target(line: &str) -> crate::models::ScanTarget {
    crate::models::ScanTarget {
        target: line.to_owned(),
        class: "repaired".to_owned(),
        distance: "unknown".to_owned(),
        thought: "reconstructed from committed VN packet".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::{
        AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentCommitTurnOptions, AgentSubmitTurnOptions,
        AgentTurnResponse, CommittedAgentTurn, PendingAgentTurn, commit_agent_turn,
        enqueue_agent_turn,
    };
    use crate::models::{GUIDE_CHOICE_TAG, TurnChoice};
    use crate::prompt_context::{CompilePromptContextPacketOptions, compile_prompt_context_packet};
    use crate::resolution::{
        ActionAmbiguity, ActionInputKind, ActionIntent, ChoicePlan, ChoicePlanKind, NarrativeBrief,
        PressureNoopReason, RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
        ResolutionOutcomeKind, ResolutionProposal,
    };
    use crate::store::{InitWorldOptions, init_world, write_json};
    use tempfile::tempdir;

    #[test]
    fn repair_turn_materializations_rebuilds_missing_committed_files() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_turn_repair
title: "turn repair"
premise:
  genre: "중세 판타지"
  protagonist: "국경 순찰자"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_turn_repair".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let committed = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_turn_repair".to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id.clone(),
                turn_id: pending.turn_id.clone(),
                resolution_proposal: Some(valid_resolution_proposal_for_pending(
                    store.as_path(),
                    &pending,
                )?),
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["수리 가능한 서사 장면".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: Vec::new(),
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: scene_specific_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        })?;

        std::fs::remove_file(&committed.render_packet_path)?;
        let report = repair_turn_materializations(Some(store.as_path()), "stw_turn_repair")?;
        assert_eq!(report.render_packets_repaired, 1);
        assert!(Path::new(committed.render_packet_path.as_str()).is_file());

        std::fs::remove_file(&committed.commit_record_path)?;
        let report = repair_turn_materializations(Some(store.as_path()), "stw_turn_repair")?;
        assert_eq!(report.commit_records_repaired, 1);
        assert!(Path::new(committed.commit_record_path.as_str()).is_file());
        Ok(())
    }

    #[test]
    fn recover_turn_commit_journal_appends_missing_committed_envelope() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let (pending, committed) = commit_fixture_turn(store.as_path(), "stw_turn_recovery")?;
        let pending_path = PathBuf::from(pending.pending_ref.as_str());
        let world_dir = pending_path
            .parent()
            .context("pending path missing agent bridge dir")?
            .parent()
            .context("pending path missing world dir")?
            .to_path_buf();
        let ledger_path = world_dir.join(TURN_COMMITS_FILENAME);
        let envelopes = read_turn_commit_envelopes(&ledger_path)?;
        let prepared = envelopes
            .iter()
            .find(|envelope| envelope.status == TurnCommitStatus::Prepared)
            .context("prepared envelope missing")?;
        std::fs::write(
            &ledger_path,
            format!("{}\n", serde_json::to_string(prepared)?),
        )?;
        write_json(&pending_path, &pending)?;

        let report = recover_turn_commit_journal(Some(store.as_path()), "stw_turn_recovery")?;

        assert_eq!(report.committed_envelopes_appended, 1);
        assert_eq!(report.stale_pending_files_removed, 1);
        assert!(!pending_path.exists());
        assert_eq!(report.materialization_repair.committed_envelopes, 1);
        let recovered = read_turn_commit_envelopes(&ledger_path)?;
        assert_eq!(recovered.len(), 2);
        assert!(
            recovered
                .iter()
                .any(|envelope| envelope.status == TurnCommitStatus::Committed
                    && envelope.commit_record_path.as_deref()
                        == Some(committed.commit_record_path.as_str()))
        );
        Ok(())
    }

    #[test]
    fn recover_turn_commit_journal_reports_unrecoverable_prepared_turn() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_turn_recovery_blocked
title: "turn recovery blocked"
premise:
  genre: "중세 판타지"
  protagonist: "국경 순찰자"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_turn_recovery_blocked".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let world_dir = PathBuf::from(pending.pending_ref.as_str())
            .parent()
            .context("pending path missing agent bridge dir")?
            .parent()
            .context("pending path missing world dir")?
            .to_path_buf();
        append_turn_commit_envelope(
            &world_dir,
            &TurnCommitEnvelope::prepared(&pending, "turn_0000", "2026-04-30T00:00:00Z".to_owned()),
        )?;

        let report =
            recover_turn_commit_journal(Some(store.as_path()), "stw_turn_recovery_blocked")?;

        assert_eq!(report.committed_envelopes_appended, 0);
        assert!(report.actions.iter().any(|action| {
            action.action == "no_recovery_action"
                && action.detail.contains("no durable commit record found")
        }));
        Ok(())
    }

    fn commit_fixture_turn(
        store: &Path,
        world_id: &str,
    ) -> anyhow::Result<(PendingAgentTurn, CommittedAgentTurn)> {
        let seed_path = store
            .parent()
            .context("store path missing parent")?
            .join(format!("{world_id}.yaml"));
        std::fs::write(
            &seed_path,
            format!(
                r#"
schema_version: singulari.world_seed.v1
world_id: {world_id}
title: "turn recovery"
premise:
  genre: "중세 판타지"
  protagonist: "국경 순찰자"
"#
            ),
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.to_path_buf()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.to_path_buf()),
            world_id: world_id.to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let committed = commit_agent_turn(&AgentCommitTurnOptions {
            store_root: Some(store.to_path_buf()),
            world_id: world_id.to_owned(),
            response: AgentTurnResponse {
                schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
                world_id: pending.world_id.clone(),
                turn_id: pending.turn_id.clone(),
                resolution_proposal: Some(valid_resolution_proposal_for_pending(store, &pending)?),
                scene_director_proposal: None,
                consequence_proposal: None,
                social_exchange_proposal: None,
                encounter_proposal: None,
                visible_scene: NarrativeScene {
                    schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                    speaker: None,
                    text_blocks: vec!["복구 가능한 서사 장면".to_owned()],
                    tone_notes: Vec::new(),
                },
                adjudication: None,
                canon_event: None,
                entity_updates: Vec::new(),
                relationship_updates: Vec::new(),
                plot_thread_events: Vec::new(),
                scene_pressure_events: Vec::new(),
                world_lore_updates: Vec::new(),
                character_text_design_updates: Vec::new(),
                body_resource_events: Vec::new(),
                location_events: Vec::new(),
                extra_contacts: Vec::new(),
                hidden_state_delta: Vec::new(),
                needs_context: Vec::new(),
                next_choices: scene_specific_choices(),
                actor_goal_events: Vec::new(),
                actor_move_events: Vec::new(),
            },
        })?;
        Ok((pending, committed))
    }

    fn scene_specific_choices() -> Vec<TurnChoice> {
        vec![
            TurnChoice {
                slot: 1,
                tag: "단서".to_owned(),
                intent: "이번 장면에서 새로 드러난 단서를 따라 조심스럽게 움직인다".to_owned(),
            },
            TurnChoice {
                slot: 2,
                tag: "몸 상태".to_owned(),
                intent: "현재 몸 상태와 주변 조건이 가능한 행동을 얼마나 제한하는지 살핀다"
                    .to_owned(),
            },
            TurnChoice {
                slot: 3,
                tag: "낮은 부름".to_owned(),
                intent: "가까운 수풀 뒤쪽에 사람이 있는지 낮은 목소리로 확인한다".to_owned(),
            },
            TurnChoice {
                slot: 4,
                tag: "기록".to_owned(),
                intent: "방금 본 단서의 의미를 세계 기록에서 대조한다".to_owned(),
            },
            TurnChoice {
                slot: 5,
                tag: "먼 시야".to_owned(),
                intent: "이 장소를 둘러싼 변화 압력을 더 넓게 본다".to_owned(),
            },
            TurnChoice {
                slot: 6,
                tag: "자유서술".to_owned(),
                intent: "6 뒤에 직접 행동, 말, 내면 독백을 서술한다".to_owned(),
            },
            TurnChoice {
                slot: 7,
                tag: GUIDE_CHOICE_TAG.to_owned(),
                intent: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
            },
        ]
    }

    fn valid_resolution_proposal_for_pending(
        store: &Path,
        pending: &PendingAgentTurn,
    ) -> anyhow::Result<ResolutionProposal> {
        let context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
            store_root: Some(store),
            pending,
            engine_session_kind: "turn_commit_repair_fixture",
        })?;
        let mut next_choice_plan = context
            .pre_turn_simulation
            .available_affordances
            .iter()
            .map(|affordance| ChoicePlan {
                slot: affordance.slot,
                plan_kind: ChoicePlanKind::OrdinaryAffordance,
                grounding_ref: affordance.affordance_id.clone(),
                label_seed: format!("slot {} repair action", affordance.slot),
                intent_seed: affordance.action_contract.clone(),
                evidence_refs: vec![affordance.affordance_id.clone()],
            })
            .collect::<Vec<_>>();
        next_choice_plan.push(ChoicePlan {
            slot: 6,
            plan_kind: ChoicePlanKind::Freeform,
            grounding_ref: "current_turn".to_owned(),
            label_seed: "자유서술".to_owned(),
            intent_seed: "직접 행동을 입력한다.".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        next_choice_plan.push(ChoicePlan {
            slot: 7,
            plan_kind: ChoicePlanKind::DelegatedJudgment,
            grounding_ref: "current_turn".to_owned(),
            label_seed: "판단 위임".to_owned(),
            intent_seed: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        let pressure_refs = context
            .pre_turn_simulation
            .pressure_obligations
            .iter()
            .map(|obligation| obligation.pressure_id.clone())
            .collect::<Vec<_>>();

        Ok(ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: pending.world_id.clone(),
            turn_id: pending.turn_id.clone(),
            interpreted_intent: ActionIntent {
                input_kind: ActionInputKind::PresentedChoice,
                summary: "repair fixture resolves the queued player choice".to_owned(),
                target_refs: Vec::new(),
                pressure_refs: pressure_refs.clone(),
                evidence_refs: vec!["current_turn".to_owned()],
                ambiguity: ActionAmbiguity::Clear,
            },
            outcome: ResolutionOutcome {
                kind: ResolutionOutcomeKind::Success,
                summary: "the repair fixture scene advances".to_owned(),
                evidence_refs: {
                    let mut refs = vec!["current_turn".to_owned()];
                    refs.extend(pressure_refs.iter().cloned());
                    refs
                },
            },
            gate_results: Vec::new(),
            proposed_effects: Vec::new(),
            process_ticks: Vec::new(),
            pressure_noop_reasons: pressure_refs
                .iter()
                .map(|pressure_ref| PressureNoopReason {
                    pressure_ref: pressure_ref.clone(),
                    reason: "repair fixture records no durable pressure movement for this turn"
                        .to_owned(),
                    evidence_refs: vec![pressure_ref.clone()],
                })
                .collect(),
            narrative_brief: NarrativeBrief {
                visible_summary: "the repair fixture scene advances".to_owned(),
                required_beats: Vec::new(),
                forbidden_visible_details: Vec::new(),
            },
            next_choice_plan,
        })
    }
}
