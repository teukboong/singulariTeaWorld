use crate::agent_bridge::{CommittedAgentTurn, PendingAgentTurn};
use crate::store::append_jsonl;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
