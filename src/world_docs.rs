use crate::models::{
    CanonEvent, EntityRecords, HiddenState, PlayerKnowledge, TurnSnapshot, WorldRecord,
    redact_guide_choice_public_hints,
};
use crate::store::{
    CANON_EVENTS_FILENAME, ENTITIES_FILENAME, HIDDEN_STATE_FILENAME, LATEST_SNAPSHOT_FILENAME,
    PLAYER_KNOWLEDGE_FILENAME, WORLD_FILENAME, read_json,
};
use crate::world_db::{ChapterSummaryRecord, latest_chapter_summaries};
use anyhow::{Context, Result};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

pub const WORLD_DOCS_DIR: &str = "docs";

/// Refresh human-readable world documents from persisted world state.
///
/// # Errors
///
/// Returns an error when world state cannot be read or any markdown projection
/// cannot be written.
pub fn refresh_world_docs(world_dir: &Path) -> Result<()> {
    let world: WorldRecord = read_json(&world_dir.join(WORLD_FILENAME))?;
    let snapshot: TurnSnapshot = read_json(&world_dir.join(LATEST_SNAPSHOT_FILENAME))?;
    let entities: EntityRecords = read_json(&world_dir.join(ENTITIES_FILENAME))?;
    let hidden_state: HiddenState = read_json(&world_dir.join(HIDDEN_STATE_FILENAME))?;
    let player_knowledge: PlayerKnowledge = read_json(&world_dir.join(PLAYER_KNOWLEDGE_FILENAME))?;
    let canon_events = read_canon_events(&world_dir.join(CANON_EVENTS_FILENAME))?;
    let docs_dir = world_docs_dir(world_dir);
    fs::create_dir_all(&docs_dir)
        .with_context(|| format!("failed to create {}", docs_dir.display()))?;
    write_doc(
        &docs_dir.join("world_bible.md"),
        &render_world_bible(&world),
    )?;
    write_doc(
        &docs_dir.join("timeline.md"),
        &render_timeline(&canon_events),
    )?;
    let chapter_summaries =
        latest_chapter_summaries(world_dir, world.world_id.as_str(), usize::MAX)?;
    write_doc(
        &docs_dir.join("chapters.md"),
        &render_chapters(&chapter_summaries),
    )?;
    write_doc(
        &docs_dir.join("protagonist_timeline.md"),
        &render_protagonist_timeline(&snapshot, &canon_events),
    )?;
    write_doc(
        &docs_dir.join("open_threads.md"),
        &render_open_threads(&snapshot, &player_knowledge, &hidden_state),
    )?;
    write_doc(&docs_dir.join("entities.md"), &render_entities(&entities))?;
    Ok(())
}

#[must_use]
pub fn world_docs_dir(world_dir: &Path) -> PathBuf {
    world_dir.join(WORLD_DOCS_DIR)
}

fn render_world_bible(world: &WorldRecord) -> String {
    let mut markdown = String::new();
    writeln!(markdown, "# {}", world.title).ok();
    writeln!(markdown).ok();
    writeln!(markdown, "- world_id: `{}`", world.world_id).ok();
    writeln!(markdown, "- genre: {}", world.premise.genre).ok();
    writeln!(markdown, "- protagonist: {}", world.premise.protagonist).ok();
    if let Some(special_condition) = &world.premise.special_condition {
        writeln!(markdown, "- special_condition: {special_condition}").ok();
    }
    writeln!(
        markdown,
        "- anchor_invariant: `{}`",
        world.anchor_character.invariant
    )
    .ok();
    writeln!(markdown).ok();
    writeln!(markdown, "## Laws").ok();
    writeln!(markdown, "- death_is_final: {}", world.laws.death_is_final).ok();
    writeln!(
        markdown,
        "- discovery_required: {}",
        world.laws.discovery_required
    )
    .ok();
    writeln!(
        markdown,
        "- bodily_needs_active: {}",
        world.laws.bodily_needs_active
    )
    .ok();
    writeln!(
        markdown,
        "- miracles_forbidden: {}",
        world.laws.miracles_forbidden
    )
    .ok();
    markdown
}

fn render_timeline(events: &[CanonEvent]) -> String {
    let mut markdown = String::from("# Timeline\n\n");
    for event in events {
        writeln!(
            markdown,
            "- `{}` `{}` [{}] {}",
            event.turn_id,
            event.event_id,
            event.kind,
            redact_guide_choice_public_hints(&event.summary)
        )
        .ok();
    }
    markdown
}

fn render_chapters(summaries: &[ChapterSummaryRecord]) -> String {
    let mut markdown = String::from("# Chapters\n\n");
    if summaries.is_empty() {
        markdown.push_str("- no chapter summaries yet\n");
        return markdown;
    }
    for summary in summaries {
        writeln!(
            markdown,
            "- `{}` {} — {}",
            summary.summary_id,
            summary.title,
            redact_guide_choice_public_hints(&summary.summary)
        )
        .ok();
    }
    markdown
}

fn render_protagonist_timeline(snapshot: &TurnSnapshot, events: &[CanonEvent]) -> String {
    let mut markdown = String::from("# Protagonist Timeline\n\n");
    writeln!(markdown, "## Current State").ok();
    writeln!(markdown, "- turn: `{}`", snapshot.turn_id).ok();
    writeln!(markdown, "- phase: {}", snapshot.phase).ok();
    writeln!(
        markdown,
        "- location: `{}`",
        snapshot.protagonist_state.location
    )
    .ok();
    write_list(
        &mut markdown,
        "inventory",
        &snapshot.protagonist_state.inventory,
    );
    write_list(&mut markdown, "body", &snapshot.protagonist_state.body);
    write_list(&mut markdown, "mind", &snapshot.protagonist_state.mind);
    writeln!(markdown).ok();
    writeln!(markdown, "## Lived Events").ok();
    for event in events
        .iter()
        .filter(|event| event.visibility == "player_visible")
    {
        writeln!(
            markdown,
            "- `{}` {}",
            event.turn_id,
            redact_guide_choice_public_hints(&event.summary)
        )
        .ok();
    }
    markdown
}

fn render_open_threads(
    snapshot: &TurnSnapshot,
    knowledge: &PlayerKnowledge,
    hidden_state: &HiddenState,
) -> String {
    let mut markdown = String::from("# Open Threads\n\n");
    writeln!(markdown, "## Player Visible").ok();
    for question in snapshot
        .open_questions
        .iter()
        .chain(knowledge.open_questions.iter())
    {
        writeln!(markdown, "- {}", redact_guide_choice_public_hints(question)).ok();
    }
    writeln!(markdown).ok();
    writeln!(markdown, "## Hidden Ledger").ok();
    writeln!(
        markdown,
        "- hidden_secrets: {} unrevealed records",
        hidden_state.secrets.len()
    )
    .ok();
    writeln!(
        markdown,
        "- hidden_timers: {} active timers",
        hidden_state.timers.len()
    )
    .ok();
    writeln!(
        markdown,
        "\nHidden truths are counted here, not exposed in player-visible prose."
    )
    .ok();
    markdown
}

fn render_entities(entities: &EntityRecords) -> String {
    let mut markdown = String::from("# Entities\n\n");
    writeln!(markdown, "## Characters").ok();
    for character in &entities.characters {
        writeln!(
            markdown,
            "- `{}` {} — {}",
            character.id, character.name.visible, character.role
        )
        .ok();
        write_nested_list(&mut markdown, "speech", &character.voice_anchor.speech);
        write_nested_list(&mut markdown, "gestures", &character.voice_anchor.gestures);
        write_nested_list(&mut markdown, "habits", &character.voice_anchor.habits);
        write_nested_list(&mut markdown, "drift", &character.voice_anchor.drift);
        write_nested_list(&mut markdown, "history", &character.history);
        write_nested_list(&mut markdown, "relationships", &character.relationships);
    }
    writeln!(markdown, "\n## Places").ok();
    for place in &entities.places {
        writeln!(markdown, "- `{}` {}", place.id, place.name).ok();
    }
    writeln!(markdown, "\n## Factions").ok();
    for faction in &entities.factions {
        writeln!(markdown, "- `{}` {}", faction.id, faction.name).ok();
    }
    writeln!(markdown, "\n## Items").ok();
    for item in &entities.items {
        writeln!(markdown, "- `{}` {}", item.id, item.name).ok();
    }
    writeln!(markdown, "\n## Concepts").ok();
    for concept in &entities.concepts {
        writeln!(markdown, "- `{}` {}", concept.id, concept.name).ok();
    }
    markdown
}

fn write_list(markdown: &mut String, label: &str, values: &[String]) {
    if values.is_empty() {
        writeln!(markdown, "- {label}: none").ok();
        return;
    }
    let sanitized = values
        .iter()
        .map(|value| redact_guide_choice_public_hints(value))
        .collect::<Vec<_>>();
    writeln!(markdown, "- {label}: {}", sanitized.join(", ")).ok();
}

fn write_nested_list(markdown: &mut String, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    let sanitized = values
        .iter()
        .map(|value| redact_guide_choice_public_hints(value))
        .collect::<Vec<_>>();
    writeln!(markdown, "  - {label}: {}", sanitized.join(" / ")).ok();
}

fn read_canon_events(path: &Path) -> Result<Vec<CanonEvent>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut events = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let event = serde_json::from_str(line)
            .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))?;
        events.push(event);
    }
    Ok(events)
}

fn write_doc(path: &Path, body: &str) -> Result<()> {
    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::world_docs_dir;
    use crate::store::{InitWorldOptions, init_world};
    use tempfile::tempdir;

    #[test]
    fn init_refreshes_readable_world_docs() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_docs_test
title: "문서 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(temp.path().join("store")),
            session_id: None,
        })?;
        let docs_dir = world_docs_dir(&initialized.world_dir);
        assert!(docs_dir.join("world_bible.md").is_file());
        assert!(docs_dir.join("chapters.md").is_file());
        assert!(docs_dir.join("timeline.md").is_file());
        assert!(docs_dir.join("protagonist_timeline.md").is_file());
        Ok(())
    }
}
