use crate::models::{
    ANCHOR_CHARACTER_ID, ANCHOR_CHARACTER_INVARIANT, CANON_EVENT_SCHEMA_VERSION, CanonEvent,
    DEFAULT_CHOICE_COUNT, ENTITY_RECORDS_SCHEMA_VERSION, EntityRecords, FREEFORM_CHOICE_SLOT,
    FREEFORM_CHOICE_TAG, GUIDE_CHOICE_SLOT, HIDDEN_STATE_SCHEMA_VERSION, HiddenState,
    PLAYER_KNOWLEDGE_SCHEMA_VERSION, PlayerKnowledge, SINGULARI_WORLD_SCHEMA_VERSION,
    TURN_SNAPSHOT_SCHEMA_VERSION, TurnSnapshot, WorldRecord, is_guide_choice_tag,
};
use crate::store::{read_json, resolve_store_paths, world_file_paths};
use crate::world_db::validate_world_db;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    Passed,
    Failed,
}

impl std::fmt::Display for ValidationStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Passed => formatter.write_str("passed"),
            Self::Failed => formatter.write_str("failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub world_id: String,
    pub status: ValidationStatus,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

impl ValidationReport {
    #[must_use]
    pub fn passed(world_id: &str, warnings: Vec<String>) -> Self {
        Self {
            world_id: world_id.to_owned(),
            status: ValidationStatus::Passed,
            errors: Vec::new(),
            warnings,
        }
    }

    #[must_use]
    pub fn failed(world_id: &str, errors: Vec<String>, warnings: Vec<String>) -> Self {
        Self {
            world_id: world_id.to_owned(),
            status: ValidationStatus::Failed,
            errors,
            warnings,
        }
    }
}

/// Validate a persisted Singulari World world.
///
/// # Errors
///
/// Returns an error when the store root cannot be resolved. Malformed world
/// files are reported as validation errors instead of aborting the full report.
pub fn validate_world(store_root: Option<&Path>, world_id: &str) -> Result<ValidationReport> {
    let paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&paths, world_id);
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if !files.dir.is_dir() {
        errors.push(format!("world directory missing: {}", files.dir.display()));
        return Ok(ValidationReport::failed(world_id, errors, warnings));
    }

    let world = read_or_error::<WorldRecord>(&files.world, &mut errors);
    let entities = read_or_error::<EntityRecords>(&files.entities, &mut errors);
    let hidden_state = read_or_error::<HiddenState>(&files.hidden_state, &mut errors);
    let player_knowledge = read_or_error::<PlayerKnowledge>(&files.player_knowledge, &mut errors);
    let latest_snapshot = read_or_error::<TurnSnapshot>(&files.latest_snapshot, &mut errors);
    let canon_events = read_canon_events(&files.canon_events, &mut errors);

    if let Some(world) = &world {
        check_schema(
            "world",
            &world.schema_version,
            SINGULARI_WORLD_SCHEMA_VERSION,
            &mut errors,
        );
        check_world_id("world", &world.world_id, world_id, &mut errors);
        if world.anchor_character.invariant != ANCHOR_CHARACTER_INVARIANT {
            errors.push(format!(
                "world anchor invariant mismatch: expected {}, got {}",
                ANCHOR_CHARACTER_INVARIANT, world.anchor_character.invariant
            ));
        }
    }
    if let Some(entities) = &entities {
        check_schema(
            "entities",
            &entities.schema_version,
            ENTITY_RECORDS_SCHEMA_VERSION,
            &mut errors,
        );
        check_world_id("entities", &entities.world_id, world_id, &mut errors);
        validate_entities(entities, &mut errors);
    }
    if let Some(hidden_state) = &hidden_state {
        check_schema(
            "hidden_state",
            &hidden_state.schema_version,
            HIDDEN_STATE_SCHEMA_VERSION,
            &mut errors,
        );
        check_world_id(
            "hidden_state",
            &hidden_state.world_id,
            world_id,
            &mut errors,
        );
    }
    if let Some(player_knowledge) = &player_knowledge {
        check_schema(
            "player_knowledge",
            &player_knowledge.schema_version,
            PLAYER_KNOWLEDGE_SCHEMA_VERSION,
            &mut errors,
        );
        check_world_id(
            "player_knowledge",
            &player_knowledge.world_id,
            world_id,
            &mut errors,
        );
    }
    if let Some(snapshot) = &latest_snapshot {
        check_schema(
            "latest_snapshot",
            &snapshot.schema_version,
            TURN_SNAPSHOT_SCHEMA_VERSION,
            &mut errors,
        );
        check_world_id("latest_snapshot", &snapshot.world_id, world_id, &mut errors);
        validate_snapshot_choices(snapshot, &mut errors);
    }
    if let Some(entities) = &entities {
        validate_canon_events(world_id, &canon_events, entities, &mut errors);
    }
    validate_hidden_truth_if_loaded(
        hidden_state.as_ref(),
        player_knowledge.as_ref(),
        latest_snapshot.as_ref(),
        &canon_events,
        &mut errors,
    );
    if canon_events.is_empty() {
        warnings.push("canon_events.jsonl has no events".to_owned());
    }
    validate_db_projection(
        &files.dir,
        world_id,
        canon_events.len(),
        &mut errors,
        &mut warnings,
    );

    Ok(finalize_validation_report(world_id, errors, warnings))
}

fn finalize_validation_report(
    world_id: &str,
    errors: Vec<String>,
    warnings: Vec<String>,
) -> ValidationReport {
    if errors.is_empty() {
        return ValidationReport::passed(world_id, warnings);
    }
    ValidationReport::failed(world_id, errors, warnings)
}

fn validate_db_projection(
    world_dir: &Path,
    world_id: &str,
    expected_canon_events: usize,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) {
    match validate_world_db(world_dir, world_id, expected_canon_events) {
        Ok(report) => warnings.extend(report.warnings),
        Err(error) => errors.push(format!("{error:#}")),
    }
}

fn validate_hidden_truth_if_loaded(
    hidden_state: Option<&HiddenState>,
    player_knowledge: Option<&PlayerKnowledge>,
    latest_snapshot: Option<&TurnSnapshot>,
    canon_events: &[CanonEvent],
    errors: &mut Vec<String>,
) {
    if let (Some(hidden_state), Some(player_knowledge), Some(snapshot)) =
        (hidden_state, player_knowledge, latest_snapshot)
    {
        validate_hidden_truth_filters(
            hidden_state,
            player_knowledge,
            snapshot,
            canon_events,
            errors,
        );
    }
}

#[must_use]
pub fn render_validation_report(report: &ValidationReport) -> String {
    let mut lines = vec![
        format!("world: {}", report.world_id),
        format!("status: {}", report.status),
    ];
    if report.errors.is_empty() {
        lines.push("errors: none".to_owned());
    } else {
        lines.push("errors:".to_owned());
        for error in &report.errors {
            lines.push(format!("  - {error}"));
        }
    }
    if !report.warnings.is_empty() {
        lines.push("warnings:".to_owned());
        for warning in &report.warnings {
            lines.push(format!("  - {warning}"));
        }
    }
    lines.join("\n")
}

fn read_or_error<T>(path: &Path, errors: &mut Vec<String>) -> Option<T>
where
    T: serde::de::DeserializeOwned,
{
    match read_json(path) {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("{error:#}"));
            None
        }
    }
}

fn read_canon_events(path: &Path, errors: &mut Vec<String>) -> Vec<CanonEvent> {
    let raw = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) => {
            errors.push(format!("failed to read {}: {error}", path.display()));
            return Vec::new();
        }
    };
    let mut events = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CanonEvent>(line)
            .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        {
            Ok(event) => events.push(event),
            Err(error) => errors.push(format!("{error:#}")),
        }
    }
    events
}

fn check_schema(label: &str, actual: &str, expected: &str, errors: &mut Vec<String>) {
    if actual != expected {
        errors.push(format!(
            "{label} schema_version mismatch: expected {expected}, got {actual}"
        ));
    }
}

fn check_world_id(label: &str, actual: &str, expected: &str, errors: &mut Vec<String>) {
    if actual != expected {
        errors.push(format!(
            "{label} world_id mismatch: expected {expected}, got {actual}"
        ));
    }
}

fn validate_entities(entities: &EntityRecords, errors: &mut Vec<String>) {
    if !entities
        .characters
        .iter()
        .any(|character| character.id == ANCHOR_CHARACTER_ID)
    {
        errors.push(format!(
            "required anchor character missing: {ANCHOR_CHARACTER_ID}"
        ));
        return;
    }
    let Some(anchor_character) = entities
        .characters
        .iter()
        .find(|character| character.id == ANCHOR_CHARACTER_ID)
    else {
        return;
    };
    if !anchor_character
        .traits
        .confirmed
        .iter()
        .any(|trait_text| trait_text.contains(ANCHOR_CHARACTER_INVARIANT))
    {
        errors.push(format!(
            "{ANCHOR_CHARACTER_ID} is missing confirmed invariant trait: {ANCHOR_CHARACTER_INVARIANT}"
        ));
    }
}

fn validate_canon_events(
    world_id: &str,
    events: &[CanonEvent],
    entities: &EntityRecords,
    errors: &mut Vec<String>,
) {
    let known_refs = known_entity_refs(entities);
    let mut seen_event_ids = BTreeSet::new();
    for event in events {
        check_schema(
            format!("canon event {}", event.event_id).as_str(),
            &event.schema_version,
            CANON_EVENT_SCHEMA_VERSION,
            errors,
        );
        check_world_id(
            format!("canon event {}", event.event_id).as_str(),
            &event.world_id,
            world_id,
            errors,
        );
        if !seen_event_ids.insert(event.event_id.clone()) {
            errors.push(format!("duplicate canon event id: {}", event.event_id));
        }
        for entity_ref in &event.entities {
            if !known_refs.contains(entity_ref) {
                errors.push(format!(
                    "canon event {} references unknown entity {}",
                    event.event_id, entity_ref
                ));
            }
        }
        if let Some(location) = &event.location
            && !known_refs.contains(location)
        {
            errors.push(format!(
                "canon event {} references unknown location {}",
                event.event_id, location
            ));
        }
    }
}

fn validate_snapshot_choices(snapshot: &TurnSnapshot, errors: &mut Vec<String>) {
    if snapshot.last_choices.len() != DEFAULT_CHOICE_COUNT {
        errors.push(format!(
            "latest_snapshot must expose exactly {DEFAULT_CHOICE_COUNT} choices, got {}",
            snapshot.last_choices.len()
        ));
    }
    if !snapshot
        .last_choices
        .iter()
        .any(|choice| choice.slot == GUIDE_CHOICE_SLOT && is_guide_choice_tag(choice.tag.as_str()))
    {
        errors.push(format!(
            "latest_snapshot missing slot {GUIDE_CHOICE_SLOT} 판단 위임"
        ));
    }
    if !snapshot
        .last_choices
        .iter()
        .any(|choice| choice.slot == FREEFORM_CHOICE_SLOT && choice.tag == FREEFORM_CHOICE_TAG)
    {
        errors.push(format!(
            "latest_snapshot missing slot {FREEFORM_CHOICE_SLOT} {FREEFORM_CHOICE_TAG}"
        ));
    }
}

fn validate_hidden_truth_filters(
    hidden_state: &HiddenState,
    player_knowledge: &PlayerKnowledge,
    snapshot: &TurnSnapshot,
    events: &[CanonEvent],
    errors: &mut Vec<String>,
) {
    let visible_text = visible_player_text(player_knowledge, snapshot, events);
    for secret in &hidden_state.secrets {
        let truth = secret.truth.trim();
        if truth.len() >= 12 && visible_text.contains(truth) {
            errors.push(format!(
                "hidden truth leaked into player-visible text: secret_id={}",
                secret.secret_id
            ));
        }
    }
}

fn visible_player_text(
    player_knowledge: &PlayerKnowledge,
    snapshot: &TurnSnapshot,
    events: &[CanonEvent],
) -> String {
    let mut chunks = Vec::new();
    chunks.extend(player_knowledge.known_entities.iter().cloned());
    chunks.extend(player_knowledge.open_questions.iter().cloned());
    chunks.extend(snapshot.open_questions.iter().cloned());
    chunks.extend(snapshot.protagonist_state.body.iter().cloned());
    chunks.extend(snapshot.protagonist_state.mind.iter().cloned());
    chunks.extend(
        events
            .iter()
            .filter(|event| event.visibility == "player_visible")
            .map(|event| event.summary.clone()),
    );
    chunks.join("\n")
}

fn known_entity_refs(entities: &EntityRecords) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    refs.extend(entities.characters.iter().map(|item| item.id.clone()));
    refs.extend(entities.places.iter().map(|item| item.id.clone()));
    refs.extend(entities.factions.iter().map(|item| item.id.clone()));
    refs.extend(entities.items.iter().map(|item| item.id.clone()));
    refs.extend(entities.concepts.iter().map(|item| item.id.clone()));
    refs
}

#[cfg(test)]
mod tests {
    use crate::models::ANCHOR_CHARACTER_ID;
    use crate::store::{InitWorldOptions, init_world};
    use crate::validate::{ValidationStatus, validate_world};
    use tempfile::tempdir;

    fn seed_body() -> &'static str {
        r#"
schema_version: singulari.world_seed.v1
world_id: stw_validate
title: "검증 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#
    }

    #[test]
    fn validate_passes_initialized_world() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(temp.path().join("store")),
            session_id: None,
        })?;
        let report = validate_world(Some(&temp.path().join("store")), "stw_validate")?;
        assert_eq!(report.status, ValidationStatus::Passed);
        Ok(())
    }

    #[test]
    fn validate_fails_when_anchor_character_is_removed() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let entities_path = initialized.world_dir.join("entities.json");
        let mut entities: crate::models::EntityRecords = crate::store::read_json(&entities_path)?;
        entities
            .characters
            .retain(|character| character.id != ANCHOR_CHARACTER_ID);
        std::fs::write(&entities_path, serde_json::to_string_pretty(&entities)?)?;
        let report = validate_world(Some(&store), "stw_validate")?;
        assert_eq!(report.status, ValidationStatus::Failed);
        assert!(
            report
                .errors
                .iter()
                .any(|error| error.contains("required anchor character missing"))
        );
        Ok(())
    }
}
