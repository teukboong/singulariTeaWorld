use crate::agent_bridge::PendingAgentTurn;
use crate::codex_view::{BuildCodexViewOptions, build_codex_view};
use crate::memory_revival_policy::MemoryRevivalPolicy;
use crate::relationship_graph::compile_relationship_graph_from_projection;
use crate::response_context::{load_agent_context_event_records, load_agent_context_projection};
use crate::resume::{BuildResumePackOptions, build_resume_pack};
use crate::store::{resolve_store_paths, world_file_paths};
use crate::world_db::{
    recent_entity_updates, recent_relationship_updates, search_world_db, visible_world_facts,
};
use crate::world_lore::compile_world_lore_from_projection;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub const AGENT_REVIVAL_PACKET_SCHEMA_VERSION: &str = "singulari.agent_revival_packet.v1";

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
#[allow(clippy::too_many_lines)]
pub fn build_agent_revival_packet(options: &AgentRevivalCompileOptions<'_>) -> Result<Value> {
    let pending = options.pending;
    let policy = MemoryRevivalPolicy::for_engine_session(options.engine_session_kind);
    let store_paths = resolve_store_paths(options.store_root)?;
    let files = world_file_paths(&store_paths, pending.world_id.as_str());
    let mut resume_options = BuildResumePackOptions::new(pending.world_id.clone());
    resume_options.store_root = options.store_root.map(Path::to_path_buf);
    resume_options.recent_events = policy.recent_events;
    resume_options.recent_memories = policy.recent_character_memories;
    resume_options.chapter_limit = policy.chapter_summaries;
    let resume_pack = build_resume_pack(&resume_options).with_context(|| {
        format!(
            "failed to build revival resume pack: world_id={}",
            pending.world_id
        )
    })?;

    let mut codex_view_options = BuildCodexViewOptions::new(pending.world_id.clone());
    codex_view_options.store_root = options.store_root.map(PathBuf::from);
    codex_view_options.query = Some(pending.player_input.clone());
    codex_view_options.limit = policy.archive_limit;
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
        policy.query_recall_limit,
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
        policy.update_limit,
    )?;
    let relationship_updates = recent_relationship_updates(
        files.dir.as_path(),
        pending.world_id.as_str(),
        policy.update_limit,
    )?;
    let context_events = load_agent_context_event_records(files.dir.as_path())?;
    let context_projection = load_agent_context_projection(files.dir.as_path())?;
    let active_relationship_graph = compile_relationship_graph_from_projection(
        pending.world_id.as_str(),
        pending.turn_id.as_str(),
        &context_projection,
        &relationship_updates,
    );
    let world_facts = visible_world_facts(
        files.dir.as_path(),
        pending.world_id.as_str(),
        policy.update_limit,
    )?;
    let active_world_lore = compile_world_lore_from_projection(
        pending.world_id.as_str(),
        pending.turn_id.as_str(),
        &context_projection,
        &world_facts,
    );
    let recent_context_events = context_events
        .iter()
        .rev()
        .take(policy.update_limit)
        .cloned()
        .collect::<Vec<_>>();

    Ok(serde_json::json!({
        "schema_version": AGENT_REVIVAL_PACKET_SCHEMA_VERSION,
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "engine_session_kind": options.engine_session_kind,
        "retrieval_profile": policy,
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
            "active_scene_pressure": pending.visible_context.active_scene_pressure,
            "active_plot_threads": pending.visible_context.active_plot_threads,
            "active_body_resource_state": pending.visible_context.active_body_resource_state,
            "active_location_graph": pending.visible_context.active_location_graph,
            "active_character_text_design": pending.visible_context.active_character_text_design,
            "active_memory_revival": {
                "player_visible_archive_view": archive_view,
                "query_recall": {
                    "query": pending.player_input,
                    "hits": query_recall_hits
                },
                "recent_entity_updates": entity_updates,
                "recent_relationship_updates": relationship_updates,
                "active_relationship_graph": active_relationship_graph,
                "active_world_lore": active_world_lore,
                "recent_agent_context_events": recent_context_events,
                "agent_context_projection": context_projection
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::{AgentSubmitTurnOptions, enqueue_agent_turn};
    use crate::store::{InitWorldOptions, init_world};

    #[test]
    fn agent_revival_packet_orders_source_of_truth_over_session_context() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_revival_packet
title: "revival packet test"
premise:
  genre: "중세 판타지"
  protagonist: "국경 숲의 남자 순찰자"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_revival_packet".to_owned(),
            input: "2".to_owned(),
            narrative_level: Some(2),
        })?;

        let packet = build_agent_revival_packet(&AgentRevivalCompileOptions {
            store_root: Some(store.as_path()),
            pending: &pending,
            engine_session_kind: "webgpt-text",
        })?;

        assert_eq!(
            packet["schema_version"],
            AGENT_REVIVAL_PACKET_SCHEMA_VERSION
        );
        assert_eq!(packet["engine_session_kind"], "webgpt-text");
        assert_eq!(packet["current_turn"]["player_input"], "2");
        assert_eq!(
            packet["source_of_truth_policy"]["conflict_rule"],
            "revival_packet_wins"
        );
        assert_eq!(
            packet["retrieval_profile"]["profile_name"],
            "webgpt_active_memory"
        );
        assert!(
            packet["retrieval_profile"]["anti_repetition_rules"]
                .as_array()
                .is_some_and(|rules| !rules.is_empty())
        );
        assert!(packet["memory_revival"]["resume_pack"].is_object());
        assert!(
            packet["memory_revival"]["active_memory_revival"]["player_visible_archive_view"]
                .is_object()
        );
        Ok(())
    }
}
