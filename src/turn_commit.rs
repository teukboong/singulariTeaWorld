use crate::agent_bridge::{CommittedAgentTurn, PendingAgentTurn};
use crate::models::{
    ADJUDICATION_SCHEMA_VERSION, AdjudicationReport, DashboardSummary,
    NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, RENDER_PACKET_SCHEMA_VERSION, RenderPacket,
    TurnChoice, VisibleState,
};
use crate::store::{append_jsonl, read_json, resolve_store_paths, world_file_paths, write_json};
use crate::vn::{BuildVnPacketOptions, VnPacket, build_vn_packet};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const TURN_COMMIT_ENVELOPE_SCHEMA_VERSION: &str = "singulari.turn_commit.v1";
pub const TURN_COMMITS_FILENAME: &str = "turn_commits.jsonl";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnCommitStatus {
    Prepared,
    Committed,
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
    append_jsonl(&world_dir.join(TURN_COMMITS_FILENAME), envelope)
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

fn required_envelope_path(envelope: &TurnCommitEnvelope, field: &str) -> Result<PathBuf> {
    let value = match field {
        "response_path" => envelope.response_path.as_deref(),
        "render_packet_path" => envelope.render_packet_path.as_deref(),
        "commit_record_path" => envelope.commit_record_path.as_deref(),
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
        AgentTurnResponse, commit_agent_turn, enqueue_agent_turn,
    };
    use crate::models::{GUIDE_CHOICE_TAG, TurnChoice};
    use crate::store::{InitWorldOptions, init_world};
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
                extra_contacts: Vec::new(),
                hidden_state_delta: Vec::new(),
                next_choices: scene_specific_choices(),
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

    fn scene_specific_choices() -> Vec<TurnChoice> {
        vec![
            TurnChoice {
                slot: 1,
                tag: "발소리".to_owned(),
                intent: "젖은 흙 위에 새로 찍힌 발자국을 따라 조심스럽게 움직인다".to_owned(),
            },
            TurnChoice {
                slot: 2,
                tag: "몸 상태".to_owned(),
                intent: "손목의 통증과 낯선 장비가 지금 가능한 행동을 얼마나 제한하는지 살핀다"
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
                intent: "방금 본 문장과 발자국의 의미를 세계 기록에서 대조한다".to_owned(),
            },
            TurnChoice {
                slot: 5,
                tag: "먼 시야".to_owned(),
                intent: "이 장소를 둘러싼 이동 흐름을 한 박자 멀리서 본다".to_owned(),
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
}
