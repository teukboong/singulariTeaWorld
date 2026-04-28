use crate::agent_bridge::PendingAgentTurn;
use crate::codex_view::{BuildCodexViewOptions, build_codex_view};
use crate::resume::{BuildResumePackOptions, build_resume_pack};
use crate::store::{resolve_store_paths, world_file_paths};
use crate::world_db::{recent_entity_updates, recent_relationship_updates, search_world_db};
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const AGENT_REVIVAL_PACKET_SCHEMA_VERSION: &str = "singulari.agent_revival_packet.v1";

const WEBGPT_REVIVAL_RECENT_EVENTS: usize = 24;
const WEBGPT_REVIVAL_RECENT_MEMORIES: usize = 24;
const WEBGPT_REVIVAL_CHAPTER_LIMIT: usize = 6;
const WEBGPT_REVIVAL_ARCHIVE_LIMIT: usize = 24;
const WEBGPT_REVIVAL_UPDATE_LIMIT: usize = 16;
const WEBGPT_REVIVAL_SEARCH_LIMIT: usize = 8;

#[derive(Debug, Clone)]
pub struct AgentRevivalCompileOptions<'a> {
    pub store_root: Option<&'a Path>,
    pub pending: &'a PendingAgentTurn,
    pub engine_session_kind: &'a str,
}

/// Compile the deterministic continuity packet sent to `WebGPT` before a text turn.
///
/// # Errors
///
/// Returns an error when the world store, world.db, resume pack, Archive View,
/// or recent update projections cannot be read.
pub fn build_agent_revival_packet(options: &AgentRevivalCompileOptions<'_>) -> Result<Value> {
    let pending = options.pending;
    let store_paths = resolve_store_paths(options.store_root)?;
    let files = world_file_paths(&store_paths, pending.world_id.as_str());
    let mut resume_options = BuildResumePackOptions::new(pending.world_id.clone());
    resume_options.store_root = options.store_root.map(Path::to_path_buf);
    resume_options.recent_events = WEBGPT_REVIVAL_RECENT_EVENTS;
    resume_options.recent_memories = WEBGPT_REVIVAL_RECENT_MEMORIES;
    resume_options.chapter_limit = WEBGPT_REVIVAL_CHAPTER_LIMIT;
    let resume_pack = build_resume_pack(&resume_options).with_context(|| {
        format!(
            "failed to build revival resume pack: world_id={}",
            pending.world_id
        )
    })?;

    let mut codex_view_options = BuildCodexViewOptions::new(pending.world_id.clone());
    codex_view_options.store_root = options.store_root.map(PathBuf::from);
    codex_view_options.query = Some(pending.player_input.clone());
    codex_view_options.limit = WEBGPT_REVIVAL_ARCHIVE_LIMIT;
    let archive_view = build_codex_view(&codex_view_options).with_context(|| {
        format!(
            "failed to build webgpt archive revival view: world_id={}",
            pending.world_id
        )
    })?;

    let query_recall_hits = search_world_db(
        files.dir.as_path(),
        pending.world_id.as_str(),
        pending.player_input.as_str(),
        WEBGPT_REVIVAL_SEARCH_LIMIT,
    )
    .with_context(|| {
        format!(
            "failed to build webgpt query recall hits: world_id={}, turn_id={}",
            pending.world_id, pending.turn_id
        )
    })?;
    let entity_updates = recent_entity_updates(
        files.dir.as_path(),
        pending.world_id.as_str(),
        WEBGPT_REVIVAL_UPDATE_LIMIT,
    )?;
    let relationship_updates = recent_relationship_updates(
        files.dir.as_path(),
        pending.world_id.as_str(),
        WEBGPT_REVIVAL_UPDATE_LIMIT,
    )?;

    Ok(serde_json::json!({
        "schema_version": AGENT_REVIVAL_PACKET_SCHEMA_VERSION,
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "engine_session_kind": options.engine_session_kind,
        "retrieval_profile": {
            "name": "webgpt_active_memory",
            "purpose": "WebGPT context-window and compaction behavior are not the world source of truth, so host-worker proactively surfaces more player-visible continuity from world.db before each turn.",
            "recent_events": WEBGPT_REVIVAL_RECENT_EVENTS,
            "recent_character_memories": WEBGPT_REVIVAL_RECENT_MEMORIES,
            "chapter_summaries": WEBGPT_REVIVAL_CHAPTER_LIMIT,
            "archive_limit": WEBGPT_REVIVAL_ARCHIVE_LIMIT,
            "update_limit": WEBGPT_REVIVAL_UPDATE_LIMIT,
            "query_recall_limit": WEBGPT_REVIVAL_SEARCH_LIMIT
        },
        "current_turn": {
            "schema_version": pending.schema_version,
            "world_id": pending.world_id,
            "turn_id": pending.turn_id,
            "status": pending.status,
            "player_input": pending.player_input,
            "selected_choice": pending.selected_choice,
            "created_at": pending.created_at,
            "pending_ref": pending.pending_ref,
        },
        "memory_revival": {
            "resume_pack": resume_pack,
            "recent_scene_window": pending.visible_context.recent_scene,
            "known_facts": pending.visible_context.known_facts,
            "voice_anchors": pending.visible_context.voice_anchors,
            "extra_memory": pending.visible_context.extra_memory,
            "active_memory_revival": {
                "player_visible_archive_view": archive_view,
                "query_recall": {
                    "query": pending.player_input,
                    "hits": query_recall_hits
                },
                "recent_entity_updates": entity_updates,
                "recent_relationship_updates": relationship_updates
            }
        },
        "private_adjudication_context": pending.private_adjudication_context,
        "output_contract": pending.output_contract,
        "source_of_truth_policy": {
            "world_state_source": "world_store",
            "turn_source": "current_turn",
            "continuity_source": "memory_revival.resume_pack + memory_revival.active_memory_revival",
            "session_context_use": ["prose rhythm", "immediate emotional continuity", "recent dialogue cadence"],
            "session_context_must_not_supply": ["world facts", "current player input", "hidden adjudication", "output contract"],
            "conflict_rule": "revival_packet_wins"
        }
    }))
}
