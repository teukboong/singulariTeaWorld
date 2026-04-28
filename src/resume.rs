use crate::models::{
    ANCHOR_CHARACTER_ID, CharacterRecord, CharacterVoiceAnchor, CodexVoiceAnchorEntry,
    EntityRecords, HiddenState, PROTAGONIST_CHARACTER_ID, PlayerKnowledge, TurnChoice,
    TurnSnapshot, normalize_turn_choices, redact_guide_choice_public_hints,
};
use crate::store::{
    ENTITIES_FILENAME, HIDDEN_STATE_FILENAME, PLAYER_KNOWLEDGE_FILENAME, load_latest_snapshot,
    load_world_record, read_json, resolve_store_paths,
};
use crate::world_db::{
    CanonEventRow, ChapterSummaryRecord, CharacterMemoryRow, latest_chapter_summaries,
    recent_canon_events, recent_character_memories,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;

pub const RESUME_PACK_SCHEMA_VERSION: &str = "singulari.resume_pack.v1";
const DEFAULT_RECENT_EVENTS: usize = 8;
const DEFAULT_RECENT_MEMORIES: usize = 8;
const DEFAULT_CHAPTER_LIMIT: usize = 3;

#[derive(Debug, Clone)]
pub struct BuildResumePackOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub recent_events: usize,
    pub recent_memories: usize,
    pub chapter_limit: usize,
}

impl BuildResumePackOptions {
    #[must_use]
    pub fn new(world_id: String) -> Self {
        Self {
            store_root: None,
            world_id,
            recent_events: DEFAULT_RECENT_EVENTS,
            recent_memories: DEFAULT_RECENT_MEMORIES,
            chapter_limit: DEFAULT_CHAPTER_LIMIT,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumePack {
    pub schema_version: String,
    pub world_id: String,
    pub title: String,
    pub session_id: String,
    pub latest_turn_id: String,
    pub phase: String,
    pub location: String,
    pub anchor_invariant: String,
    pub open_threads: Vec<String>,
    pub active_choices: Vec<TurnChoice>,
    pub recent_events: Vec<CanonEventRow>,
    pub recent_character_memories: Vec<CharacterMemoryRow>,
    pub voice_anchors: Vec<CodexVoiceAnchorEntry>,
    pub chapter_summaries: Vec<ChapterSummaryRecord>,
    pub hidden_state_summary: HiddenStateSummary,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiddenStateSummary {
    pub secret_count: usize,
    pub timer_count: usize,
    pub leak_policy: String,
}

/// Build the compact continuation packet Guide uses before resuming play.
///
/// # Errors
///
/// Returns an error when persisted world state or long-term DB projections
/// cannot be loaded.
pub fn build_resume_pack(options: &BuildResumePackOptions) -> Result<ResumePack> {
    let paths = resolve_store_paths(options.store_root.as_deref())?;
    let world_dir = paths.worlds_dir.join(options.world_id.as_str());
    let world = load_world_record(options.store_root.as_deref(), options.world_id.as_str())?;
    let snapshot = load_latest_snapshot(options.store_root.as_deref(), options.world_id.as_str())?;
    let player_knowledge: PlayerKnowledge = read_json(&world_dir.join(PLAYER_KNOWLEDGE_FILENAME))?;
    let entities: EntityRecords = read_json(&world_dir.join(ENTITIES_FILENAME))?;
    let hidden_state: HiddenState = read_json(&world_dir.join(HIDDEN_STATE_FILENAME))?;
    Ok(ResumePack {
        schema_version: RESUME_PACK_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id.clone(),
        title: world.title.clone(),
        session_id: snapshot.session_id.clone(),
        latest_turn_id: snapshot.turn_id.clone(),
        phase: snapshot.phase.clone(),
        location: snapshot.protagonist_state.location.clone(),
        anchor_invariant: world.anchor_character.invariant.clone(),
        open_threads: merge_open_threads(&snapshot, &player_knowledge),
        active_choices: normalize_turn_choices(&snapshot.last_choices),
        recent_events: recent_canon_events(
            &world_dir,
            world.world_id.as_str(),
            options.recent_events,
        )?,
        recent_character_memories: recent_character_memories(
            &world_dir,
            world.world_id.as_str(),
            options.recent_memories,
        )?,
        voice_anchors: visible_voice_anchors(&entities),
        chapter_summaries: latest_chapter_summaries(
            &world_dir,
            world.world_id.as_str(),
            options.chapter_limit,
        )?,
        hidden_state_summary: HiddenStateSummary {
            secret_count: hidden_state.secrets.len(),
            timer_count: hidden_state.timers.len(),
            leak_policy: "hidden truth counts only; never expose secret contents in resume prose"
                .to_owned(),
        },
        generated_at: chrono::Utc::now().to_rfc3339(),
    })
}

#[must_use]
pub fn render_resume_pack_markdown(pack: &ResumePack) -> String {
    let mut lines = vec![
        format!("# Resume Pack: {}", pack.title),
        String::new(),
        format!("- world_id: `{}`", pack.world_id),
        format!("- session_id: `{}`", pack.session_id),
        format!("- latest_turn: `{}`", pack.latest_turn_id),
        format!("- phase: {}", pack.phase),
        format!("- location: `{}`", pack.location),
        format!("- anchor_invariant: `{}`", pack.anchor_invariant),
        format!(
            "- hidden_state: {} secrets / {} timers",
            pack.hidden_state_summary.secret_count, pack.hidden_state_summary.timer_count
        ),
        String::new(),
        "## Chapter Summaries".to_owned(),
    ];
    push_empty_state(
        &mut lines,
        &pack.chapter_summaries,
        "no chapter summaries yet",
    );
    for summary in &pack.chapter_summaries {
        lines.push(format!(
            "- `{}` {}: {}",
            summary.summary_id,
            summary.title,
            redact_guide_choice_public_hints(&summary.summary)
        ));
    }
    lines.push(String::new());
    lines.push("## Recent Events".to_owned());
    push_empty_state(&mut lines, &pack.recent_events, "no canon events yet");
    for event in &pack.recent_events {
        lines.push(format!(
            "- `{}` `{}` [{}] {}",
            event.turn_id,
            event.event_id,
            event.kind,
            redact_guide_choice_public_hints(&event.summary)
        ));
    }
    lines.push(String::new());
    lines.push("## Character Memories".to_owned());
    push_empty_state(
        &mut lines,
        &pack.recent_character_memories,
        "no character memories yet",
    );
    for memory in &pack.recent_character_memories {
        lines.push(format!(
            "- `{}` {}: {}",
            memory.character_id,
            memory.visibility,
            redact_guide_choice_public_hints(&memory.summary)
        ));
    }
    lines.push(String::new());
    lines.push("## Voice Anchors".to_owned());
    push_empty_state(&mut lines, &pack.voice_anchors, "no voice anchors yet");
    for anchor in &pack.voice_anchors {
        lines.push(format!("- `{}` {}", anchor.character_id, anchor.name));
        push_prefixed_values(&mut lines, "  - 말투", &anchor.speech);
        push_prefixed_values(&mut lines, "  - 제스처", &anchor.gestures);
        push_prefixed_values(&mut lines, "  - 버릇", &anchor.habits);
        push_prefixed_values(&mut lines, "  - 변화", &anchor.drift);
    }
    lines.push(String::new());
    lines.push("## Open Threads".to_owned());
    push_empty_state(&mut lines, &pack.open_threads, "no open threads");
    for thread in &pack.open_threads {
        lines.push(format!("- {}", redact_guide_choice_public_hints(thread)));
    }
    lines.push(String::new());
    lines.push("## Active Choices".to_owned());
    for choice in &pack.active_choices {
        lines.push(format!(
            "{}. {} — {}",
            choice.slot,
            choice.tag,
            choice.player_visible_intent()
        ));
    }
    lines.join("\n")
}

fn visible_voice_anchors(entities: &EntityRecords) -> Vec<CodexVoiceAnchorEntry> {
    entities
        .characters
        .iter()
        .filter(|character| character_voice_anchor_is_visible(character))
        .filter_map(resume_voice_anchor_entry)
        .collect()
}

fn character_voice_anchor_is_visible(character: &CharacterRecord) -> bool {
    matches!(
        character.knowledge_state.as_str(),
        "self" | "known" | "player_visible" | "protagonist_known" | "veiled"
    )
}

fn resume_voice_anchor_entry(character: &CharacterRecord) -> Option<CodexVoiceAnchorEntry> {
    let anchor = effective_voice_anchor(character)?;
    Some(CodexVoiceAnchorEntry {
        character_id: character.id.clone(),
        name: character.name.visible.clone(),
        speech: anchor.speech,
        gestures: anchor.gestures,
        habits: anchor.habits,
        drift: anchor.drift,
    })
}

fn effective_voice_anchor(character: &CharacterRecord) -> Option<CharacterVoiceAnchor> {
    if !character.voice_anchor.is_empty() {
        return Some(character.voice_anchor.clone());
    }
    match character.id.as_str() {
        PROTAGONIST_CHARACTER_ID => Some(CharacterVoiceAnchor::protagonist_default()),
        ANCHOR_CHARACTER_ID => Some(CharacterVoiceAnchor::anchor_default()),
        _ => None,
    }
}

fn push_prefixed_values(lines: &mut Vec<String>, label: &str, values: &[String]) {
    if !values.is_empty() {
        lines.push(format!("{label}: {}", values.join(" / ")));
    }
}

fn merge_open_threads(snapshot: &TurnSnapshot, player_knowledge: &PlayerKnowledge) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut threads = Vec::new();
    for thread in snapshot
        .open_questions
        .iter()
        .chain(player_knowledge.open_questions.iter())
    {
        if is_deprecated_open_thread(thread) {
            continue;
        }
        if seen.insert(thread.clone()) {
            threads.push(thread.clone());
        }
    }
    threads
}

fn is_deprecated_open_thread(thread: &str) -> bool {
    thread == "안내자의 선택은 현재 상태에서 가장 덜 무모한 길을 가리킨다"
}

fn push_empty_state<T>(lines: &mut Vec<String>, values: &[T], label: &str) {
    if values.is_empty() {
        lines.push(format!("- {label}"));
    }
}

#[cfg(test)]
mod tests {
    use super::{BuildResumePackOptions, build_resume_pack};
    use crate::models::ANCHOR_CHARACTER_INVARIANT;
    use crate::store::{InitWorldOptions, init_world};
    use crate::turn::{AdvanceTurnOptions, advance_turn};
    use tempfile::tempdir;

    #[test]
    fn resume_pack_includes_latest_state_and_recent_events() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_resume_test
title: "재개 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: Some("session_resume_test".to_owned()),
        })?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_resume_test".to_owned(),
            input: "4".to_owned(),
        })?;
        let mut options = BuildResumePackOptions::new("stw_resume_test".to_owned());
        options.store_root = Some(store);
        let pack = build_resume_pack(&options)?;
        assert_eq!(pack.latest_turn_id, "turn_0001");
        assert_eq!(pack.anchor_invariant, ANCHOR_CHARACTER_INVARIANT);
        assert!(!pack.recent_events.is_empty());
        assert!(!pack.recent_character_memories.is_empty());
        assert!(!pack.voice_anchors.is_empty());
        Ok(())
    }
}
