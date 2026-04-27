use crate::models::{
    BodyNeeds, CharacterBody, CharacterRecord, CharacterVoiceAnchor, EntityName, EntityRecords,
    EntityUpdateRecord, StructuredEntityUpdates, TraitSet,
};
use crate::store::{append_jsonl, read_json, resolve_store_paths, world_file_paths, write_json};
use crate::world_db::{WorldDbRepairReport, repair_world_db};
use crate::world_docs::refresh_world_docs;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const PLAYER_VISIBLE: &str = "player_visible";
const DEFAULT_KNOWLEDGE_STATE: &str = "known";
const DEFAULT_CHARACTER_ROLE: &str = "등장인물";
const UPDATE_KIND_VOICE_ANCHOR: &str = "voice_anchor";

#[derive(Debug, Clone)]
pub struct ApplyCharacterAnchorOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub character_id: String,
    pub name: Option<String>,
    pub role: Option<String>,
    pub knowledge_state: Option<String>,
    pub speech: Vec<String>,
    pub gestures: Vec<String>,
    pub habits: Vec<String>,
    pub drift: Vec<String>,
    pub replace: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterAnchorReport {
    pub world_id: String,
    pub character_id: String,
    pub name: String,
    pub role: String,
    pub knowledge_state: String,
    pub created_character: bool,
    pub changed_fields: Vec<String>,
    pub voice_anchor: CharacterVoiceAnchor,
    pub update_id: String,
    pub db_repair: WorldDbRepairReport,
    pub docs_dir: PathBuf,
}

/// Upsert a character's voice anchor and mirror the change into world projections.
///
/// # Errors
///
/// Returns an error when the world cannot be loaded, the target character is
/// missing without a replacement name, or persistence/projection refresh fails.
pub fn apply_character_anchor(
    options: &ApplyCharacterAnchorOptions,
) -> Result<CharacterAnchorReport> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let mut entities: EntityRecords = read_json(&files.entities)?;
    let latest_snapshot = crate::store::load_latest_snapshot(
        options.store_root.as_deref(),
        options.world_id.as_str(),
    )?;
    let created_at = Utc::now().to_rfc3339();

    let (target_index, created_character) = find_or_create_character(options, &mut entities)?;
    let character = entities
        .characters
        .get_mut(target_index)
        .context("character anchor target index became invalid")?;
    let mut changed_fields = Vec::new();
    if created_character {
        push_changed_field(&mut changed_fields, "character");
    }
    apply_character_metadata(options, character, &mut changed_fields);
    apply_voice_anchor_values(options, character, &mut changed_fields);

    if changed_fields.is_empty() {
        bail!(
            "character-anchor made no changes: world_id={}, character_id={}",
            options.world_id,
            options.character_id
        );
    }

    let update_id = format!(
        "{}:{}:{}",
        latest_snapshot.turn_id,
        UPDATE_KIND_VOICE_ANCHOR,
        compact_update_suffix(options.character_id.as_str(), created_at.as_str())
    );
    let structured_updates = structured_voice_anchor_update(
        options,
        character,
        changed_fields.as_slice(),
        update_id.as_str(),
        latest_snapshot.turn_id.as_str(),
        created_at.as_str(),
    );
    let report_character = character.clone();

    write_json(&files.entities, &entities)?;
    append_jsonl(&files.entity_updates, &structured_updates)?;
    let db_repair = repair_world_db(&files.dir, options.world_id.as_str())?;
    refresh_world_docs(&files.dir)?;

    Ok(CharacterAnchorReport {
        world_id: options.world_id.clone(),
        character_id: report_character.id,
        name: report_character.name.visible,
        role: report_character.role,
        knowledge_state: report_character.knowledge_state,
        created_character,
        changed_fields,
        voice_anchor: report_character.voice_anchor,
        update_id,
        db_repair,
        docs_dir: files.dir.join(crate::world_docs::WORLD_DOCS_DIR),
    })
}

fn find_or_create_character(
    options: &ApplyCharacterAnchorOptions,
    entities: &mut EntityRecords,
) -> Result<(usize, bool)> {
    if let Some(index) = entities
        .characters
        .iter()
        .position(|character| character.id == options.character_id)
    {
        return Ok((index, false));
    }
    let name = options.name.as_deref().with_context(|| {
        format!(
            "character-anchor requires --name when creating missing character {}",
            options.character_id
        )
    })?;
    entities
        .characters
        .push(new_character_record(options, name));
    Ok((entities.characters.len() - 1, true))
}

fn new_character_record(options: &ApplyCharacterAnchorOptions, name: &str) -> CharacterRecord {
    CharacterRecord {
        id: options.character_id.clone(),
        name: EntityName {
            visible: name.to_owned(),
            native: None,
        },
        role: options
            .role
            .clone()
            .unwrap_or_else(|| DEFAULT_CHARACTER_ROLE.to_owned()),
        knowledge_state: options
            .knowledge_state
            .clone()
            .unwrap_or_else(|| DEFAULT_KNOWLEDGE_STATE.to_owned()),
        traits: TraitSet {
            confirmed: Vec::new(),
            rumored: Vec::new(),
            hidden: Vec::new(),
        },
        voice_anchor: CharacterVoiceAnchor::default(),
        body: CharacterBody {
            injuries: Vec::new(),
            needs: BodyNeeds {
                hunger: "humanly sensed".to_owned(),
                thirst: "humanly sensed".to_owned(),
                fatigue: "humanly sensed".to_owned(),
            },
        },
        history: vec!["캐릭터 음성 앵커 helper로 등록됨".to_owned()],
        relationships: Vec::new(),
    }
}

fn apply_character_metadata(
    options: &ApplyCharacterAnchorOptions,
    character: &mut CharacterRecord,
    changed_fields: &mut Vec<String>,
) {
    if let Some(name) = options.name.as_ref()
        && character.name.visible != *name
    {
        character.name.visible.clone_from(name);
        push_changed_field(changed_fields, "name");
    }
    if let Some(role) = options.role.as_ref()
        && character.role != *role
    {
        character.role.clone_from(role);
        push_changed_field(changed_fields, "role");
    }
    if let Some(knowledge_state) = options.knowledge_state.as_ref()
        && character.knowledge_state != *knowledge_state
    {
        character.knowledge_state.clone_from(knowledge_state);
        push_changed_field(changed_fields, "knowledge_state");
    }
}

fn apply_voice_anchor_values(
    options: &ApplyCharacterAnchorOptions,
    character: &mut CharacterRecord,
    changed_fields: &mut Vec<String>,
) {
    if options.replace {
        character.voice_anchor.speech.clone_from(&options.speech);
        character
            .voice_anchor
            .gestures
            .clone_from(&options.gestures);
        character.voice_anchor.habits.clone_from(&options.habits);
        character.voice_anchor.drift.clone_from(&options.drift);
        push_changed_field(changed_fields, "voice_anchor");
        return;
    }
    append_unique_values(
        &mut character.voice_anchor.speech,
        &options.speech,
        changed_fields,
        "speech",
    );
    append_unique_values(
        &mut character.voice_anchor.gestures,
        &options.gestures,
        changed_fields,
        "gestures",
    );
    append_unique_values(
        &mut character.voice_anchor.habits,
        &options.habits,
        changed_fields,
        "habits",
    );
    append_unique_values(
        &mut character.voice_anchor.drift,
        &options.drift,
        changed_fields,
        "drift",
    );
}

fn append_unique_values(
    target: &mut Vec<String>,
    values: &[String],
    changed_fields: &mut Vec<String>,
    changed_field: &str,
) {
    let initial_len = target.len();
    for value in values {
        if !target.iter().any(|existing| existing == value) {
            target.push(value.clone());
        }
    }
    if target.len() != initial_len {
        push_changed_field(changed_fields, changed_field);
    }
}

fn push_changed_field(changed_fields: &mut Vec<String>, field: &str) {
    if !changed_fields.iter().any(|existing| existing == field) {
        changed_fields.push(field.to_owned());
    }
}

fn structured_voice_anchor_update(
    options: &ApplyCharacterAnchorOptions,
    character: &CharacterRecord,
    changed_fields: &[String],
    update_id: &str,
    turn_id: &str,
    created_at: &str,
) -> StructuredEntityUpdates {
    let mut updates = StructuredEntityUpdates::empty(options.world_id.as_str(), turn_id);
    updates.entity_updates.push(EntityUpdateRecord {
        update_id: update_id.to_owned(),
        world_id: options.world_id.clone(),
        turn_id: turn_id.to_owned(),
        entity_id: character.id.clone(),
        update_kind: UPDATE_KIND_VOICE_ANCHOR.to_owned(),
        visibility: PLAYER_VISIBLE.to_owned(),
        summary: format!(
            "{} 음성 앵커 갱신: {}",
            character.name.visible,
            changed_fields.join(", ")
        ),
        source_event_id: update_id.to_owned(),
        created_at: created_at.to_owned(),
    });
    updates
}

fn compact_update_suffix(character_id: &str, created_at: &str) -> String {
    let id = character_id
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    let timestamp = created_at
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>();
    format!("{id}:{timestamp}")
}

#[cfg(test)]
mod tests {
    use super::{ApplyCharacterAnchorOptions, apply_character_anchor};
    use crate::codex_view::{BuildCodexViewOptions, build_codex_view, render_codex_view_markdown};
    use crate::store::{InitWorldOptions, init_world};
    use tempfile::tempdir;

    #[test]
    fn character_anchor_creates_character_and_updates_codex_view() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_voice_anchor
title: "음성 앵커 세계"
premise:
  genre: "중세 판타지"
  protagonist: "현대인의 전생, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let report = apply_character_anchor(&ApplyCharacterAnchorOptions {
            store_root: Some(store.clone()),
            world_id: "stw_voice_anchor".to_owned(),
            character_id: "char:radin".to_owned(),
            name: Some("라딘".to_owned()),
            role: Some("아르벤 수도원 심부름꾼".to_owned()),
            knowledge_state: Some("known".to_owned()),
            speech: vec!["빠르고 구어체로 말한다".to_owned()],
            gestures: vec!["손을 들고 끼어든다".to_owned()],
            habits: vec!["무서우면 농담이 늘어난다".to_owned()],
            drift: vec!["장난 사이에 판단력이 드러난다".to_owned()],
            replace: false,
        })?;
        assert_eq!(report.name, "라딘");
        assert!(report.created_character);
        let mut options = BuildCodexViewOptions::new("stw_voice_anchor".to_owned());
        options.store_root = Some(store);
        let rendered = render_codex_view_markdown(&build_codex_view(&options)?);
        assert!(rendered.contains("라딘"));
        assert!(rendered.contains("빠르고 구어체로 말한다"));
        Ok(())
    }
}
