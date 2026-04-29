use crate::belief_graph::BELIEF_GRAPH_FILENAME;
use crate::body_resource::BODY_RESOURCE_STATE_FILENAME;
use crate::change_ledger::CHANGE_LEDGER_FILENAME;
use crate::character_text_design::{CHARACTER_TEXT_DESIGN_FILENAME, CharacterTextDesignPacket};
use crate::location_graph::LOCATION_GRAPH_FILENAME;
use crate::models::{
    ANCHOR_CHARACTER_ID, CanonEvent, CharacterRecord, EntityRecords, EntityUpdateRecord,
    HiddenState, NamedEntity, PlaceRecord, PlayerKnowledge, RelationshipUpdateRecord, RenderPacket,
    StructuredEntityUpdates, TurnLogEntry, TurnSnapshot, WorldRecord,
    redact_guide_choice_public_hints,
};
use crate::narrative_style_state::NARRATIVE_STYLE_STATE_FILENAME;
use crate::pattern_debt::PATTERN_DEBT_FILENAME;
use crate::player_intent::PLAYER_INTENT_TRACE_FILENAME;
use crate::plot_thread::PLOT_THREADS_FILENAME;
use crate::relationship_graph::{RELATIONSHIP_GRAPH_FILENAME, RelationshipGraphPacket};
use crate::scene_pressure::ACTIVE_SCENE_PRESSURES_FILENAME;
use crate::sqlite::{
    Connection, OpenFlags, OptionalExtension, SQLITE_BUSY_TIMEOUT_MS, SqliteConnectionOptions,
    configure_sqlite_connection, params,
};
use crate::store::{
    CANON_EVENTS_FILENAME, ENTITIES_FILENAME, ENTITY_UPDATES_FILENAME, HIDDEN_STATE_FILENAME,
    LATEST_SNAPSHOT_FILENAME, PLAYER_KNOWLEDGE_FILENAME, WORLD_FILENAME, read_json,
};
use crate::visual_asset_graph::VISUAL_ASSET_GRAPH_FILENAME;
use crate::world_lore::{WORLD_LORE_FILENAME, WorldLorePacket};
use crate::world_process_clock::WORLD_PROCESSES_FILENAME;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const WORLD_DB_FILENAME: &str = "world.db";
pub const WORLD_DB_SCHEMA_VERSION: &str = "singulari.world_db.v2";

const DEFAULT_AUTO_CHAPTER_EVENT_COUNT: usize = 5;
const DB_NAME: &str = "singulari-world";
const OPERATION_OPEN: &str = "open_world_db";
const OPERATION_VALIDATE: &str = "validate_world_db";
const PLAYER_VISIBLE: &str = "player_visible";
const SYSTEM_VISIBLE: &str = "system";
const SEARCH_SNIPPET_TOKENS: i64 = 12;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldDbStats {
    pub world_id: String,
    pub db_path: PathBuf,
    pub schema_version: String,
    pub world_facts: u64,
    pub canon_events: u64,
    pub character_memories: u64,
    pub state_changes: u64,
    pub entity_records: u64,
    pub entity_updates: u64,
    pub relationship_updates: u64,
    pub materialized_projections: u64,
    pub snapshots: u64,
    pub chapter_summaries: u64,
    pub search_documents: u64,
}

#[derive(Debug, Clone)]
pub struct WorldDbValidation {
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonEventRow {
    pub event_id: String,
    pub turn_id: String,
    pub kind: String,
    pub visibility: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterMemoryRow {
    pub memory_id: String,
    pub character_id: String,
    pub visibility: String,
    pub summary: String,
    pub source_event_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterSummaryRecord {
    pub summary_id: String,
    pub chapter_index: u32,
    pub title: String,
    pub summary: String,
    pub v2: ChapterSummaryV2,
    pub source_turn_start: String,
    pub source_turn_end: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChapterSummaryV2 {
    pub schema_version: String,
    pub summary_id: String,
    pub source_turn_start: String,
    pub source_turn_end: String,
    #[serde(default)]
    pub facts: Vec<String>,
    #[serde(default)]
    pub open_ambiguities: Vec<String>,
    #[serde(default)]
    pub state_changes: Vec<String>,
    #[serde(default)]
    pub relationship_changes: Vec<String>,
    #[serde(default)]
    pub belief_changes: Vec<String>,
    #[serde(default)]
    pub process_changes: Vec<String>,
    #[serde(default)]
    pub retired_for_now: Vec<String>,
    #[serde(default)]
    pub revival_triggers: Vec<String>,
    #[serde(default)]
    pub summary_bias_risks: Vec<String>,
}

impl Default for ChapterSummaryV2 {
    fn default() -> Self {
        Self {
            schema_version: "singulari.chapter_summary_v2.v1".to_owned(),
            summary_id: String::new(),
            source_turn_start: String::new(),
            source_turn_end: String::new(),
            facts: Vec::new(),
            open_ambiguities: Vec::new(),
            state_changes: Vec::new(),
            relationship_changes: Vec::new(),
            belief_changes: Vec::new(),
            process_changes: Vec::new(),
            retired_for_now: Vec::new(),
            revival_triggers: Vec::new(),
            summary_bias_risks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldFactRow {
    pub fact_id: String,
    pub category: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecordRow {
    pub entity_id: String,
    pub entity_type: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSearchHit {
    pub source_table: String,
    pub source_id: String,
    pub title: String,
    pub snippet: String,
    pub rank: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldDbRepairReport {
    pub world_id: String,
    pub db_path: PathBuf,
    pub rebuilt: bool,
    pub canon_events: usize,
    pub snapshots: usize,
    pub render_packets: usize,
    pub search_documents: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct RecordTurnDbInput<'a> {
    pub world_dir: &'a Path,
    pub world: &'a WorldRecord,
    pub entities: &'a EntityRecords,
    pub snapshot: &'a TurnSnapshot,
    pub canon_event: &'a CanonEvent,
    pub render_packet: &'a RenderPacket,
    pub turn_log_entry: &'a TurnLogEntry,
    pub structured_updates: &'a StructuredEntityUpdates,
}

/// Initialize the per-world `SQLite` database and seed its first projections.
///
/// # Errors
///
/// Returns an error when the database cannot be opened, migrated, or seeded.
pub fn initialize_world_db(
    world_dir: &Path,
    world: &WorldRecord,
    snapshot: &TurnSnapshot,
    entities: &EntityRecords,
    hidden_state: &HiddenState,
    player_knowledge: &PlayerKnowledge,
    initial_event: &CanonEvent,
) -> Result<()> {
    let conn = open_world_db(world_dir)?;
    migrate_world_db(&conn)?;
    upsert_world(&conn, world)?;
    upsert_player_knowledge(&conn, player_knowledge)?;
    upsert_hidden_state(&conn, hidden_state)?;
    upsert_entity_records(&conn, entities, world.updated_at.as_str())?;
    upsert_world_facts(&conn, world, initial_event.event_id.as_str())?;
    insert_canon_event(&conn, initial_event)?;
    insert_timeline_event(&conn, initial_event)?;
    insert_state_changes(&conn, initial_event, world.updated_at.as_str())?;
    insert_character_memory(
        &conn,
        world.world_id.as_str(),
        ANCHOR_CHARACTER_ID,
        SYSTEM_VISIBLE,
        "초기 초점은 아직 고정되지 않았다",
        initial_event.event_id.as_str(),
        world.updated_at.as_str(),
    )?;
    upsert_snapshot(&conn, snapshot, world.created_at.as_str())?;
    rebuild_world_search_index_for_conn(&conn, world.world_id.as_str())?;
    Ok(())
}

/// Sync closed JSON projection files into `world.db` and refresh search.
///
/// # Errors
///
/// Returns an error when the database cannot be opened or an existing
/// projection file is malformed.
pub fn sync_world_db_materialized_projections(
    world_dir: &Path,
    world_id: &str,
    updated_at: &str,
) -> Result<()> {
    let conn = open_world_db(world_dir)?;
    migrate_world_db(&conn)?;
    upsert_materialized_projection_files(&conn, world_dir, world_id, updated_at)?;
    rebuild_world_search_index_for_conn(&conn, world_id)?;
    Ok(())
}

/// Record one advanced turn into the per-world `SQLite` database.
///
/// # Errors
///
/// Returns an error when the database is missing, cannot be migrated, or any
/// projection write fails.
pub fn record_turn_in_world_db(input: &RecordTurnDbInput<'_>) -> Result<()> {
    let conn = open_world_db(input.world_dir)?;
    migrate_world_db(&conn)?;
    upsert_world(&conn, input.world)?;
    upsert_entity_records(
        &conn,
        input.entities,
        input.turn_log_entry.created_at.as_str(),
    )?;
    insert_canon_event(&conn, input.canon_event)?;
    insert_timeline_event(&conn, input.canon_event)?;
    insert_state_changes(
        &conn,
        input.canon_event,
        input.turn_log_entry.created_at.as_str(),
    )?;
    insert_structured_entity_updates(&conn, input.structured_updates)?;
    insert_character_memory(
        &conn,
        input.world.world_id.as_str(),
        "char:protagonist",
        PLAYER_VISIBLE,
        input.canon_event.summary.as_str(),
        input.canon_event.event_id.as_str(),
        input.turn_log_entry.created_at.as_str(),
    )?;
    upsert_snapshot(
        &conn,
        input.snapshot,
        input.turn_log_entry.created_at.as_str(),
    )?;
    upsert_render_packet(
        &conn,
        input.render_packet,
        input.turn_log_entry.created_at.as_str(),
    )?;
    upsert_materialized_projection_files(
        &conn,
        input.world_dir,
        input.world.world_id.as_str(),
        input.turn_log_entry.created_at.as_str(),
    )?;
    refresh_due_chapter_summary_for_conn(
        &conn,
        input.world.world_id.as_str(),
        DEFAULT_AUTO_CHAPTER_EVENT_COUNT,
        input.turn_log_entry.created_at.as_str(),
    )?;
    rebuild_world_search_index_for_conn(&conn, input.world.world_id.as_str())?;
    Ok(())
}

/// Return counts for the long-term per-world database.
///
/// # Errors
///
/// Returns an error when the active database cannot be read.
pub fn world_db_stats(world_dir: &Path, world_id: &str) -> Result<WorldDbStats> {
    let db_path = world_db_path(world_dir);
    let conn = open_readonly_world_db(&db_path)?;
    let schema_version = schema_version(&conn)?;
    Ok(WorldDbStats {
        world_id: world_id.to_owned(),
        db_path,
        schema_version,
        world_facts: count_world_rows(&conn, "world_facts", world_id)?,
        canon_events: count_world_rows(&conn, "canon_events", world_id)?,
        character_memories: count_world_rows(&conn, "character_memories", world_id)?,
        state_changes: count_world_rows(&conn, "state_changes", world_id)?,
        entity_records: count_world_rows(&conn, "entity_records", world_id)?,
        entity_updates: count_world_rows(&conn, "entity_updates", world_id)?,
        relationship_updates: count_world_rows(&conn, "relationship_updates", world_id)?,
        materialized_projections: count_world_rows(&conn, "materialized_projections", world_id)?,
        snapshots: count_world_rows(&conn, "snapshots", world_id)?,
        chapter_summaries: count_world_rows(&conn, "chapter_summaries", world_id)?,
        search_documents: count_world_rows(&conn, "world_search_fts", world_id)?,
    })
}

/// Validate the per-world database projection.
///
/// # Errors
///
/// Returns an error when the database is missing, malformed, or out of sync
/// with core JSON/JSONL state.
pub fn validate_world_db(
    world_dir: &Path,
    world_id: &str,
    expected_canon_events: usize,
) -> Result<WorldDbValidation> {
    let db_path = world_db_path(world_dir);
    if !db_path.is_file() {
        bail!("world.db missing: {}", db_path.display());
    }
    let conn = open_readonly_world_db(&db_path)?;
    let schema_version = schema_version(&conn)?;
    if schema_version != WORLD_DB_SCHEMA_VERSION {
        bail!(
            "world.db schema_version mismatch: expected {WORLD_DB_SCHEMA_VERSION}, got {schema_version}"
        );
    }
    let world_rows = count_world_rows(&conn, "worlds", world_id)?;
    if world_rows != 1 {
        bail!("world.db worlds row count mismatch: expected 1, got {world_rows}");
    }
    let canon_events = count_world_rows(&conn, "canon_events", world_id)?;
    if canon_events != expected_canon_events as u64 {
        bail!(
            "world.db canon event count mismatch: expected {expected_canon_events}, got {canon_events}"
        );
    }
    let anchor_rows: u64 = conn
        .query_row(
            "SELECT COUNT(*) FROM entity_records WHERE world_id = ?1 AND entity_id = ?2",
            params![world_id, ANCHOR_CHARACTER_ID],
            |row| row.get(0),
        )
        .context("world.db count anchor character entity failed")?;
    if anchor_rows != 1 {
        bail!("world.db missing entity record: {ANCHOR_CHARACTER_ID}");
    }
    let stats = world_db_stats(world_dir, world_id)?;
    let mut warnings = Vec::new();
    if stats.character_memories == 0 {
        warnings.push("world.db has no character memories yet".to_owned());
    }
    if stats.world_facts == 0 {
        warnings.push("world.db has no world facts yet".to_owned());
    }
    Ok(WorldDbValidation { warnings })
}

/// Force a deterministic chapter summary for currently unsummarized events.
///
/// # Errors
///
/// Returns an error when the world database cannot be opened or queried.
pub fn force_chapter_summary(
    world_dir: &Path,
    world_id: &str,
) -> Result<Option<ChapterSummaryRecord>> {
    let conn = open_world_db(world_dir)?;
    migrate_world_db(&conn)?;
    let created_at = chrono::Utc::now().to_rfc3339();
    create_chapter_summary_for_open_events(&conn, world_id, 1, true, created_at.as_str())
}

/// Load recent canon events in chronological order.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn recent_canon_events(
    world_dir: &Path,
    world_id: &str,
    limit: usize,
) -> Result<Vec<CanonEventRow>> {
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT event_id, turn_id, kind, visibility, summary
             FROM canon_events
             WHERE world_id = ?1
             ORDER BY turn_id DESC
             LIMIT ?2",
        )
        .context("world.db recent canon events prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, limit_to_i64(limit)], |row| {
            Ok(CanonEventRow {
                event_id: row.get(0)?,
                turn_id: row.get(1)?,
                kind: row.get(2)?,
                visibility: row.get(3)?,
                summary: redact_guide_choice_public_hints(&row.get::<_, String>(4)?),
            })
        })
        .context("world.db recent canon events query failed")?;
    let mut events = Vec::new();
    for row in rows {
        events.push(row.context("world.db recent canon event row failed")?);
    }
    events.reverse();
    Ok(events)
}

/// Load recent character memories in chronological order.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn recent_character_memories(
    world_dir: &Path,
    world_id: &str,
    limit: usize,
) -> Result<Vec<CharacterMemoryRow>> {
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, character_id, visibility, summary, source_event_id, created_at
             FROM character_memories
             WHERE world_id = ?1
             ORDER BY created_at DESC, memory_id DESC
             LIMIT ?2",
        )
        .context("world.db recent character memories prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, limit_to_i64(limit)], |row| {
            Ok(CharacterMemoryRow {
                memory_id: row.get(0)?,
                character_id: row.get(1)?,
                visibility: row.get(2)?,
                summary: redact_guide_choice_public_hints(&row.get::<_, String>(3)?),
                source_event_id: row.get(4)?,
                created_at: row.get(5)?,
            })
        })
        .context("world.db recent character memories query failed")?;
    let mut memories = Vec::new();
    for row in rows {
        memories.push(row.context("world.db recent character memory row failed")?);
    }
    memories.reverse();
    Ok(memories)
}

/// Load recent structured entity updates in chronological order.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn recent_entity_updates(
    world_dir: &Path,
    world_id: &str,
    limit: usize,
) -> Result<Vec<EntityUpdateRecord>> {
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT update_id, world_id, turn_id, entity_id, update_kind,
                    visibility, summary, source_event_id, created_at
             FROM entity_updates
             WHERE world_id = ?1
             ORDER BY created_at DESC, update_id DESC
             LIMIT ?2",
        )
        .context("world.db recent entity updates prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, limit_to_i64(limit)], |row| {
            Ok(EntityUpdateRecord {
                update_id: row.get(0)?,
                world_id: row.get(1)?,
                turn_id: row.get(2)?,
                entity_id: row.get(3)?,
                update_kind: row.get(4)?,
                visibility: row.get(5)?,
                summary: redact_guide_choice_public_hints(&row.get::<_, String>(6)?),
                source_event_id: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .context("world.db recent entity updates query failed")?;
    let mut updates = collect_rows(rows, "world.db recent entity update row failed")?;
    updates.reverse();
    Ok(updates)
}

/// Load recent structured relationship updates in chronological order.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn recent_relationship_updates(
    world_dir: &Path,
    world_id: &str,
    limit: usize,
) -> Result<Vec<RelationshipUpdateRecord>> {
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT update_id, world_id, turn_id, source_entity_id, target_entity_id,
                    relation_kind, visibility, summary, source_event_id, created_at
             FROM relationship_updates
             WHERE world_id = ?1
             ORDER BY created_at DESC, update_id DESC
             LIMIT ?2",
        )
        .context("world.db recent relationship updates prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, limit_to_i64(limit)], |row| {
            Ok(RelationshipUpdateRecord {
                update_id: row.get(0)?,
                world_id: row.get(1)?,
                turn_id: row.get(2)?,
                source_entity_id: row.get(3)?,
                target_entity_id: row.get(4)?,
                relation_kind: row.get(5)?,
                visibility: row.get(6)?,
                summary: redact_guide_choice_public_hints(&row.get::<_, String>(7)?),
                source_event_id: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .context("world.db recent relationship updates query failed")?;
    let mut updates = collect_rows(rows, "world.db recent relationship update row failed")?;
    updates.reverse();
    Ok(updates)
}

/// Load latest chapter summaries in chronological order.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn latest_chapter_summaries(
    world_dir: &Path,
    world_id: &str,
    limit: usize,
) -> Result<Vec<ChapterSummaryRecord>> {
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT summary_id, chapter_index, title, summary, raw_json, source_turn_start, source_turn_end, created_at
             FROM chapter_summaries
             WHERE world_id = ?1
             ORDER BY chapter_index DESC
             LIMIT ?2",
        )
        .context("world.db latest chapter summaries prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, limit_to_i64(limit)], |row| {
            let raw_json: String = row.get(4)?;
            let mut v2 = serde_json::from_str::<ChapterSummaryV2>(&raw_json).unwrap_or_default();
            v2.summary_id = row.get(0)?;
            v2.source_turn_start = row.get(5)?;
            v2.source_turn_end = row.get(6)?;
            Ok(ChapterSummaryRecord {
                summary_id: v2.summary_id.clone(),
                chapter_index: row.get(1)?,
                title: row.get(2)?,
                summary: redact_guide_choice_public_hints(&row.get::<_, String>(3)?),
                v2,
                source_turn_start: row.get(5)?,
                source_turn_end: row.get(6)?,
                created_at: row.get(7)?,
            })
        })
        .context("world.db latest chapter summaries query failed")?;
    let mut summaries = Vec::new();
    for row in rows {
        summaries.push(row.context("world.db latest chapter summary row failed")?);
    }
    summaries.reverse();
    Ok(summaries)
}

/// Load player-visible world facts for Archive View.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn visible_world_facts(
    world_dir: &Path,
    world_id: &str,
    limit: usize,
) -> Result<Vec<WorldFactRow>> {
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT fact_id, category, subject, predicate, object
             FROM world_facts
             WHERE world_id = ?1 AND visibility = ?2
             ORDER BY category ASC, fact_id ASC
             LIMIT ?3",
        )
        .context("world.db visible world facts prepare failed")?;
    let rows = stmt
        .query_map(
            params![world_id, PLAYER_VISIBLE, limit_to_i64(limit)],
            |row| {
                Ok(WorldFactRow {
                    fact_id: row.get(0)?,
                    category: row.get(1)?,
                    subject: row.get(2)?,
                    predicate: row.get(3)?,
                    object: row.get(4)?,
                })
            },
        )
        .context("world.db visible world facts query failed")?;
    collect_rows(rows, "world.db visible world fact row failed")
}

/// Load protagonist-known entity records for Archive View.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn visible_entity_records(
    world_dir: &Path,
    world_id: &str,
    limit: usize,
) -> Result<Vec<EntityRecordRow>> {
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT entity_id, entity_type, name, status
             FROM entity_records
             WHERE world_id = ?1 AND known_to_protagonist = 1
             ORDER BY entity_type ASC, entity_id ASC
             LIMIT ?2",
        )
        .context("world.db visible entity records prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, limit_to_i64(limit)], |row| {
            Ok(EntityRecordRow {
                entity_id: row.get(0)?,
                entity_type: row.get(1)?,
                name: row.get(2)?,
                status: row.get(3)?,
            })
        })
        .context("world.db visible entity records query failed")?;
    collect_rows(rows, "world.db visible entity record row failed")
}

/// Search player-visible world projections with `SQLite` FTS.
///
/// # Errors
///
/// Returns an error when the world database cannot be queried.
pub fn search_world_db(
    world_dir: &Path,
    world_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<WorldSearchHit>> {
    let normalized_query = normalize_fts_query(query);
    if normalized_query.is_empty() {
        return Ok(Vec::new());
    }
    let conn = open_readonly_world_db(&world_db_path(world_dir))?;
    let mut stmt = conn
        .prepare(
            "SELECT source_table, source_id, title,
                    snippet(world_search_fts, 4, '', '', '...', ?3),
                    bm25(world_search_fts)
             FROM world_search_fts
             WHERE world_id = ?1 AND world_search_fts MATCH ?2
             ORDER BY bm25(world_search_fts)
             LIMIT ?4",
        )
        .context("world.db search prepare failed")?;
    let rows = stmt
        .query_map(
            params![
                world_id,
                normalized_query,
                SEARCH_SNIPPET_TOKENS,
                limit_to_i64(limit)
            ],
            |row| {
                Ok(WorldSearchHit {
                    source_table: row.get(0)?,
                    source_id: row.get(1)?,
                    title: redact_guide_choice_public_hints(&row.get::<_, String>(2)?),
                    snippet: redact_guide_choice_public_hints(&row.get::<_, String>(3)?),
                    rank: row.get(4)?,
                })
            },
        )
        .context("world.db search query failed")?;
    let hits = collect_rows(rows, "world.db search row failed")?;
    if hits.is_empty() {
        return search_world_index_like(&conn, world_id, query, limit);
    }
    Ok(hits)
}

/// Rebuild the per-world database from JSON/JSONL evidence files.
///
/// # Errors
///
/// Returns an error when evidence files cannot be read or any projection cannot
/// be rebuilt.
pub fn repair_world_db(world_dir: &Path, world_id: &str) -> Result<WorldDbRepairReport> {
    let world: WorldRecord = read_json(&world_dir.join(WORLD_FILENAME))?;
    if world.world_id != world_id {
        bail!(
            "repair-db world_id mismatch: path={}, world.json={}",
            world_id,
            world.world_id
        );
    }
    let entities: EntityRecords = read_json(&world_dir.join(ENTITIES_FILENAME))?;
    let hidden_state: HiddenState = read_json(&world_dir.join(HIDDEN_STATE_FILENAME))?;
    let player_knowledge: PlayerKnowledge = read_json(&world_dir.join(PLAYER_KNOWLEDGE_FILENAME))?;
    let latest_snapshot: TurnSnapshot = read_json(&world_dir.join(LATEST_SNAPSHOT_FILENAME))?;
    let canon_events = read_canon_events_jsonl(&world_dir.join(CANON_EVENTS_FILENAME))?;
    let structured_updates =
        read_structured_updates_jsonl(&world_dir.join(ENTITY_UPDATES_FILENAME))?;
    let snapshots = read_session_snapshots(world_dir)?;
    let render_packets = read_session_render_packets(world_dir)?;
    let conn = open_world_db(world_dir)?;
    migrate_world_db(&conn)?;
    clear_world_rows(&conn, world_id)?;
    upsert_world(&conn, &world)?;
    upsert_player_knowledge(&conn, &player_knowledge)?;
    upsert_hidden_state(&conn, &hidden_state)?;
    upsert_entity_records(&conn, &entities, world.updated_at.as_str())?;
    if let Some(first_event) = canon_events.first() {
        upsert_world_facts(&conn, &world, first_event.event_id.as_str())?;
    }
    for event in &canon_events {
        insert_canon_event(&conn, event)?;
        insert_timeline_event(&conn, event)?;
        insert_state_changes(&conn, event, world.updated_at.as_str())?;
        insert_character_memory(
            &conn,
            world_id,
            "char:protagonist",
            if event.visibility == SYSTEM_VISIBLE {
                SYSTEM_VISIBLE
            } else {
                PLAYER_VISIBLE
            },
            event.summary.as_str(),
            event.event_id.as_str(),
            world.updated_at.as_str(),
        )?;
    }
    for updates in &structured_updates {
        insert_structured_entity_updates(&conn, updates)?;
    }
    if snapshots.is_empty() {
        upsert_snapshot(&conn, &latest_snapshot, world.updated_at.as_str())?;
    } else {
        for snapshot in &snapshots {
            upsert_snapshot(&conn, snapshot, world.updated_at.as_str())?;
        }
    }
    for packet in &render_packets {
        upsert_render_packet(&conn, packet, world.updated_at.as_str())?;
    }
    upsert_materialized_projection_files(&conn, world_dir, world_id, world.updated_at.as_str())?;
    create_chapter_summary_for_open_events(&conn, world_id, 1, true, world.updated_at.as_str())?;
    rebuild_world_search_index_for_conn(&conn, world_id)?;
    let search_documents = count_world_rows(&conn, "world_search_fts", world_id)?;
    Ok(WorldDbRepairReport {
        world_id: world_id.to_owned(),
        db_path: world_db_path(world_dir),
        rebuilt: true,
        canon_events: canon_events.len(),
        snapshots: snapshots.len().max(1),
        render_packets: render_packets.len(),
        search_documents,
    })
}

#[must_use]
pub fn world_db_path(world_dir: &Path) -> PathBuf {
    world_dir.join(WORLD_DB_FILENAME)
}

fn open_world_db(world_dir: &Path) -> Result<Connection> {
    let db_path = world_db_path(world_dir);
    let conn = Connection::open(&db_path)
        .with_context(|| format!("failed to open world db {}", db_path.display()))?;
    configure_world_db_connection(&conn, OPERATION_OPEN)?;
    Ok(conn)
}

fn open_readonly_world_db(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open readonly world db {}", db_path.display()))?;
    configure_world_db_connection(&conn, OPERATION_VALIDATE)?;
    Ok(conn)
}

fn configure_world_db_connection(conn: &Connection, operation: &'static str) -> Result<()> {
    configure_sqlite_connection(
        conn,
        SqliteConnectionOptions {
            db_name: DB_NAME,
            operation,
            busy_timeout_ms: SQLITE_BUSY_TIMEOUT_MS,
            journal_mode: Some("WAL"),
            synchronous: Some("NORMAL"),
            foreign_keys: true,
        },
    )
}

const WORLD_DB_SCHEMA_SQL: &str = r"
        CREATE TABLE IF NOT EXISTS schema_meta (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS worlds (
            world_id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            created_by TEXT NOT NULL,
            genre TEXT NOT NULL,
            protagonist TEXT NOT NULL,
            special_condition TEXT,
            anchor_invariant TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            raw_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS world_facts (
            fact_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            category TEXT NOT NULL,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL,
            object TEXT NOT NULL,
            visibility TEXT NOT NULL,
            source_event_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS canon_events (
            event_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            occurred_at_world_time TEXT NOT NULL,
            visibility TEXT NOT NULL,
            kind TEXT NOT NULL,
            summary TEXT NOT NULL,
            location TEXT,
            evidence_source TEXT NOT NULL,
            user_input TEXT NOT NULL,
            narrative_ref TEXT NOT NULL,
            raw_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS timeline_events (
            event_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            timeline_scope TEXT NOT NULL,
            summary TEXT NOT NULL,
            raw_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS state_changes (
            change_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            event_id TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            summary TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS entity_records (
            world_id TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            entity_type TEXT NOT NULL,
            name TEXT NOT NULL,
            known_to_protagonist INTEGER NOT NULL,
            status TEXT NOT NULL,
            raw_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (world_id, entity_id)
        );

        CREATE TABLE IF NOT EXISTS character_memories (
            memory_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            character_id TEXT NOT NULL,
            visibility TEXT NOT NULL,
            summary TEXT NOT NULL,
            source_event_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS entity_updates (
            update_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            update_kind TEXT NOT NULL,
            visibility TEXT NOT NULL,
            summary TEXT NOT NULL,
            source_event_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS relationship_updates (
            update_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            source_entity_id TEXT NOT NULL,
            target_entity_id TEXT NOT NULL,
            relation_kind TEXT NOT NULL,
            visibility TEXT NOT NULL,
            summary TEXT NOT NULL,
            source_event_id TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS materialized_projections (
            world_id TEXT NOT NULL,
            projection_id TEXT NOT NULL,
            projection_kind TEXT NOT NULL,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            raw_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (world_id, projection_id)
        );

        CREATE TABLE IF NOT EXISTS world_lore_entries (
            world_id TEXT NOT NULL,
            lore_id TEXT NOT NULL,
            domain TEXT NOT NULL,
            name TEXT NOT NULL,
            summary TEXT NOT NULL,
            visibility TEXT NOT NULL,
            confidence TEXT NOT NULL,
            authority TEXT NOT NULL,
            raw_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (world_id, lore_id)
        );

        CREATE TABLE IF NOT EXISTS relationship_edges (
            world_id TEXT NOT NULL,
            edge_id TEXT NOT NULL,
            source_entity_id TEXT NOT NULL,
            target_entity_id TEXT NOT NULL,
            stance TEXT NOT NULL,
            visibility TEXT NOT NULL,
            visible_summary TEXT NOT NULL,
            raw_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (world_id, edge_id)
        );

        CREATE TABLE IF NOT EXISTS character_text_designs (
            world_id TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            visible_name TEXT NOT NULL,
            role TEXT NOT NULL,
            visibility TEXT NOT NULL,
            summary TEXT NOT NULL,
            raw_json TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (world_id, entity_id)
        );

        CREATE TABLE IF NOT EXISTS player_knowledge (
            world_id TEXT PRIMARY KEY,
            raw_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS hidden_state (
            world_id TEXT PRIMARY KEY,
            raw_json TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS snapshots (
            world_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            phase TEXT NOT NULL,
            current_event_id TEXT,
            raw_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (world_id, session_id, turn_id)
        );

        CREATE TABLE IF NOT EXISTS render_packets (
            world_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            mode TEXT NOT NULL,
            raw_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (world_id, turn_id)
        );

        CREATE TABLE IF NOT EXISTS chapter_summaries (
            summary_id TEXT PRIMARY KEY,
            world_id TEXT NOT NULL,
            chapter_index INTEGER NOT NULL,
            title TEXT NOT NULL,
            summary TEXT NOT NULL,
            raw_json TEXT NOT NULL DEFAULT '{}',
            source_turn_start TEXT NOT NULL,
            source_turn_end TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS world_search_fts USING fts5(
            world_id UNINDEXED,
            source_table UNINDEXED,
            source_id UNINDEXED,
            title,
            body,
            visibility UNINDEXED
        );

        CREATE INDEX IF NOT EXISTS idx_canon_events_world_turn
            ON canon_events(world_id, turn_id);
        CREATE INDEX IF NOT EXISTS idx_character_memories_world_character
            ON character_memories(world_id, character_id);
        CREATE INDEX IF NOT EXISTS idx_entity_updates_world_turn
            ON entity_updates(world_id, turn_id);
        CREATE INDEX IF NOT EXISTS idx_relationship_updates_world_turn
            ON relationship_updates(world_id, turn_id);
        CREATE INDEX IF NOT EXISTS idx_materialized_projections_world_kind
            ON materialized_projections(world_id, projection_kind);
        CREATE INDEX IF NOT EXISTS idx_world_lore_entries_world_domain
            ON world_lore_entries(world_id, domain);
        CREATE INDEX IF NOT EXISTS idx_relationship_edges_world_source
            ON relationship_edges(world_id, source_entity_id);
        CREATE INDEX IF NOT EXISTS idx_character_text_designs_world_role
            ON character_text_designs(world_id, role);
        CREATE INDEX IF NOT EXISTS idx_state_changes_world_turn
            ON state_changes(world_id, turn_id);
        CREATE INDEX IF NOT EXISTS idx_world_facts_world_category
            ON world_facts(world_id, category);
        ";

fn migrate_world_db(conn: &Connection) -> Result<()> {
    conn.execute_batch(WORLD_DB_SCHEMA_SQL)
        .context("world.db migration failed")?;
    ensure_chapter_summary_raw_json_column(conn)?;
    conn.execute(
        "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('schema_version', ?1)",
        params![WORLD_DB_SCHEMA_VERSION],
    )
    .context("world.db schema version upsert failed")?;
    Ok(())
}

fn ensure_chapter_summary_raw_json_column(conn: &Connection) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(chapter_summaries)")
        .context("world.db chapter_summaries table_info prepare failed")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .context("world.db chapter_summaries table_info query failed")?;
    let mut has_raw_json = false;
    for row in rows {
        if row.context("world.db chapter_summaries table_info row failed")? == "raw_json" {
            has_raw_json = true;
            break;
        }
    }
    if has_raw_json {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE chapter_summaries ADD COLUMN raw_json TEXT NOT NULL DEFAULT '{}'",
        [],
    )
    .context("world.db chapter_summaries raw_json migration failed")?;
    Ok(())
}

fn refresh_due_chapter_summary_for_conn(
    conn: &Connection,
    world_id: &str,
    min_events: usize,
    created_at: &str,
) -> Result<Option<ChapterSummaryRecord>> {
    create_chapter_summary_for_open_events(conn, world_id, min_events, false, created_at)
}

fn create_chapter_summary_for_open_events(
    conn: &Connection,
    world_id: &str,
    min_events: usize,
    force: bool,
    created_at: &str,
) -> Result<Option<ChapterSummaryRecord>> {
    let after_turn = latest_summary_end_turn(conn, world_id)?;
    let events = open_chapter_events(conn, world_id, after_turn.as_deref())?;
    if events.is_empty() || (!force && events.len() < min_events) {
        return Ok(None);
    }
    let chapter_index = next_chapter_index(conn, world_id)?;
    let Some(first_event) = events.first() else {
        return Ok(None);
    };
    let Some(last_event) = events.last() else {
        return Ok(None);
    };
    let summary_id = format!("chapter_{chapter_index:04}");
    let title = format!(
        "Chapter {chapter_index}: {} -> {}",
        first_event.turn_id, last_event.turn_id
    );
    let v2 = deterministic_chapter_summary_v2(&summary_id, &events);
    let summary = render_chapter_summary_v2(&v2);
    let record = ChapterSummaryRecord {
        summary_id,
        chapter_index,
        title,
        summary,
        v2,
        source_turn_start: first_event.turn_id.clone(),
        source_turn_end: last_event.turn_id.clone(),
        created_at: created_at.to_owned(),
    };
    insert_chapter_summary(conn, world_id, &record)?;
    Ok(Some(record))
}

fn latest_summary_end_turn(conn: &Connection, world_id: &str) -> Result<Option<String>> {
    conn.query_row(
        "SELECT source_turn_end FROM chapter_summaries WHERE world_id = ?1 ORDER BY chapter_index DESC LIMIT 1",
        params![world_id],
        |row| row.get(0),
    )
    .optional()
    .context("world.db latest chapter summary end query failed")
}

fn open_chapter_events(
    conn: &Connection,
    world_id: &str,
    after_turn: Option<&str>,
) -> Result<Vec<CanonEventRow>> {
    let mut events = if let Some(turn_id) = after_turn {
        let mut stmt = conn
            .prepare(
                "SELECT event_id, turn_id, kind, visibility, summary
                 FROM canon_events
                 WHERE world_id = ?1 AND turn_id > ?2
                 ORDER BY turn_id ASC",
            )
            .context("world.db open chapter events prepare failed")?;
        let rows = stmt
            .query_map(params![world_id, turn_id], canon_event_row_from_sql)
            .context("world.db open chapter events query failed")?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row.context("world.db open chapter event row failed")?);
        }
        values
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT event_id, turn_id, kind, visibility, summary
                 FROM canon_events
                 WHERE world_id = ?1
                 ORDER BY turn_id ASC",
            )
            .context("world.db open chapter events prepare failed")?;
        let rows = stmt
            .query_map(params![world_id], canon_event_row_from_sql)
            .context("world.db open chapter events query failed")?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row.context("world.db open chapter event row failed")?);
        }
        values
    };
    events.retain(|event| event.visibility == PLAYER_VISIBLE || event.visibility == SYSTEM_VISIBLE);
    Ok(events)
}

fn canon_event_row_from_sql(
    row: &crate::sqlite::Row<'_>,
) -> crate::sqlite::RusqliteResult<CanonEventRow> {
    Ok(CanonEventRow {
        event_id: row.get(0)?,
        turn_id: row.get(1)?,
        kind: row.get(2)?,
        visibility: row.get(3)?,
        summary: row.get(4)?,
    })
}

fn next_chapter_index(conn: &Connection, world_id: &str) -> Result<u32> {
    let value: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(chapter_index), 0) + 1 FROM chapter_summaries WHERE world_id = ?1",
            params![world_id],
            |row| row.get(0),
        )
        .context("world.db next chapter index query failed")?;
    u32::try_from(value).context("world.db next chapter index out of range")
}

fn deterministic_chapter_summary_v2(
    summary_id: &str,
    events: &[CanonEventRow],
) -> ChapterSummaryV2 {
    let Some(first_event) = events.first() else {
        return ChapterSummaryV2 {
            summary_id: summary_id.to_owned(),
            ..ChapterSummaryV2::default()
        };
    };
    let Some(last_event) = events.last() else {
        return ChapterSummaryV2 {
            summary_id: summary_id.to_owned(),
            ..ChapterSummaryV2::default()
        };
    };
    let facts = events
        .iter()
        .take(8)
        .map(|event| {
            format!(
                "{} [{}] {}",
                event.turn_id,
                event.kind,
                redact_guide_choice_public_hints(&event.summary)
            )
        })
        .collect::<Vec<_>>();
    let open_ambiguities = events
        .iter()
        .filter(|event| event.kind.contains("question") || event.summary.contains('?'))
        .map(|event| redact_guide_choice_public_hints(&event.summary))
        .collect::<Vec<_>>();
    let revival_triggers = events
        .iter()
        .rev()
        .take(4)
        .map(|event| format!("{}:{}", event.kind, event.turn_id))
        .collect::<Vec<_>>();
    let summary_bias_risks = vec![
        "This summary is a compression hint, not an authority over omitted events.".to_owned(),
        "retired_for_now means inactive in the current prompt, not deleted canon.".to_owned(),
    ];
    ChapterSummaryV2 {
        schema_version: ChapterSummaryV2::default().schema_version,
        summary_id: summary_id.to_owned(),
        source_turn_start: first_event.turn_id.clone(),
        source_turn_end: last_event.turn_id.clone(),
        facts,
        open_ambiguities,
        state_changes: Vec::new(),
        relationship_changes: Vec::new(),
        belief_changes: Vec::new(),
        process_changes: Vec::new(),
        retired_for_now: Vec::new(),
        revival_triggers,
        summary_bias_risks,
    }
}

fn render_chapter_summary_v2(v2: &ChapterSummaryV2) -> String {
    let fact_summary = if v2.facts.is_empty() {
        "facts: none".to_owned()
    } else {
        format!("facts: {}", v2.facts.join(" / "))
    };
    let ambiguity_summary = if v2.open_ambiguities.is_empty() {
        "ambiguities: none".to_owned()
    } else {
        format!("ambiguities: {}", v2.open_ambiguities.join(" / "))
    };
    format!(
        "{} to {}. {fact_summary}. {ambiguity_summary}. revival_triggers: {}",
        v2.source_turn_start,
        v2.source_turn_end,
        v2.revival_triggers.join(", ")
    )
}

fn insert_chapter_summary(
    conn: &Connection,
    world_id: &str,
    record: &ChapterSummaryRecord,
) -> Result<()> {
    conn.execute(
        r"
        INSERT OR IGNORE INTO chapter_summaries(
            summary_id, world_id, chapter_index, title, summary, raw_json,
            source_turn_start, source_turn_end, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ",
        params![
            record.summary_id,
            world_id,
            record.chapter_index,
            record.title,
            record.summary,
            to_json(&record.v2)?,
            record.source_turn_start,
            record.source_turn_end,
            record.created_at,
        ],
    )
    .context("world.db chapter summary insert failed")?;
    Ok(())
}

fn upsert_world(conn: &Connection, world: &WorldRecord) -> Result<()> {
    conn.execute(
        r"
        INSERT OR REPLACE INTO worlds(
            world_id, title, created_by, genre, protagonist, special_condition,
            anchor_invariant, created_at, updated_at, raw_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ",
        params![
            world.world_id,
            world.title,
            world.created_by,
            world.premise.genre,
            world.premise.protagonist,
            world.premise.special_condition,
            world.anchor_character.invariant,
            world.created_at,
            world.updated_at,
            to_json(world)?,
        ],
    )
    .context("world.db world upsert failed")?;
    Ok(())
}

fn upsert_world_facts(conn: &Connection, world: &WorldRecord, source_event_id: &str) -> Result<()> {
    let created_at = world.created_at.as_str();
    upsert_world_fact(
        conn,
        &WorldFactInsert {
            fact_id: "fact:premise:genre",
            world_id: world.world_id.as_str(),
            category: "premise",
            subject: "world",
            predicate: "genre",
            object: world.premise.genre.as_str(),
            visibility: PLAYER_VISIBLE,
            source_event_id,
            created_at,
        },
    )?;
    upsert_world_fact(
        conn,
        &WorldFactInsert {
            fact_id: "fact:premise:protagonist",
            world_id: world.world_id.as_str(),
            category: "premise",
            subject: "protagonist",
            predicate: "identity",
            object: world.premise.protagonist.as_str(),
            visibility: PLAYER_VISIBLE,
            source_event_id,
            created_at,
        },
    )?;
    if let Some(special_condition) = &world.premise.special_condition {
        upsert_world_fact(
            conn,
            &WorldFactInsert {
                fact_id: "fact:premise:special_condition",
                world_id: world.world_id.as_str(),
                category: "premise",
                subject: "protagonist",
                predicate: "special_condition",
                object: special_condition.as_str(),
                visibility: PLAYER_VISIBLE,
                source_event_id,
                created_at,
            },
        )?;
    }
    upsert_world_fact(
        conn,
        &WorldFactInsert {
            fact_id: "fact:anchor_character:invariant",
            world_id: world.world_id.as_str(),
            category: "anchor_character",
            subject: ANCHOR_CHARACTER_ID,
            predicate: "invariant",
            object: world.anchor_character.invariant.as_str(),
            visibility: PLAYER_VISIBLE,
            source_event_id,
            created_at,
        },
    )
}

struct WorldFactInsert<'a> {
    fact_id: &'a str,
    world_id: &'a str,
    category: &'a str,
    subject: &'a str,
    predicate: &'a str,
    object: &'a str,
    visibility: &'a str,
    source_event_id: &'a str,
    created_at: &'a str,
}

fn upsert_world_fact(conn: &Connection, fact: &WorldFactInsert<'_>) -> Result<()> {
    conn.execute(
        r"
        INSERT OR REPLACE INTO world_facts(
            fact_id, world_id, category, subject, predicate, object,
            visibility, source_event_id, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ",
        params![
            fact.fact_id,
            fact.world_id,
            fact.category,
            fact.subject,
            fact.predicate,
            fact.object,
            fact.visibility,
            fact.source_event_id,
            fact.created_at,
        ],
    )
    .context("world.db world fact upsert failed")?;
    Ok(())
}

fn insert_canon_event(conn: &Connection, event: &CanonEvent) -> Result<()> {
    conn.execute(
        r"
        INSERT OR IGNORE INTO canon_events(
            event_id, world_id, turn_id, occurred_at_world_time, visibility, kind,
            summary, location, evidence_source, user_input, narrative_ref, raw_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)
        ",
        params![
            event.event_id,
            event.world_id,
            event.turn_id,
            event.occurred_at_world_time,
            event.visibility,
            event.kind,
            event.summary,
            event.location,
            event.evidence.source,
            event.evidence.user_input,
            event.evidence.narrative_ref,
            to_json(event)?,
        ],
    )
    .context("world.db canon event insert failed")?;
    Ok(())
}

fn insert_timeline_event(conn: &Connection, event: &CanonEvent) -> Result<()> {
    conn.execute(
        r"
        INSERT OR IGNORE INTO timeline_events(
            event_id, world_id, turn_id, timeline_scope, summary, raw_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ",
        params![
            event.event_id,
            event.world_id,
            event.turn_id,
            event.kind,
            event.summary,
            to_json(event)?,
        ],
    )
    .context("world.db timeline event insert failed")?;
    Ok(())
}

fn insert_state_changes(conn: &Connection, event: &CanonEvent, created_at: &str) -> Result<()> {
    for (index, consequence) in event.consequences.iter().enumerate() {
        let change_id = format!("{}:{index:02}", event.event_id);
        conn.execute(
            r"
            INSERT OR IGNORE INTO state_changes(
                change_id, world_id, turn_id, event_id, change_kind, summary, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                change_id,
                event.world_id,
                event.turn_id,
                event.event_id,
                event.kind,
                consequence,
                created_at,
            ],
        )
        .context("world.db state change insert failed")?;
    }
    Ok(())
}

fn insert_structured_entity_updates(
    conn: &Connection,
    updates: &StructuredEntityUpdates,
) -> Result<()> {
    for update in &updates.entity_updates {
        insert_entity_update(conn, update)?;
    }
    for update in &updates.relationship_updates {
        insert_relationship_update(conn, update)?;
    }
    Ok(())
}

fn insert_entity_update(conn: &Connection, update: &EntityUpdateRecord) -> Result<()> {
    conn.execute(
        r"
        INSERT OR IGNORE INTO entity_updates(
            update_id, world_id, turn_id, entity_id, update_kind,
            visibility, summary, source_event_id, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ",
        params![
            update.update_id,
            update.world_id,
            update.turn_id,
            update.entity_id,
            update.update_kind,
            update.visibility,
            update.summary,
            update.source_event_id,
            update.created_at,
        ],
    )
    .context("world.db entity update insert failed")?;
    Ok(())
}

fn insert_relationship_update(conn: &Connection, update: &RelationshipUpdateRecord) -> Result<()> {
    conn.execute(
        r"
        INSERT OR IGNORE INTO relationship_updates(
            update_id, world_id, turn_id, source_entity_id, target_entity_id,
            relation_kind, visibility, summary, source_event_id, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ",
        params![
            update.update_id,
            update.world_id,
            update.turn_id,
            update.source_entity_id,
            update.target_entity_id,
            update.relation_kind,
            update.visibility,
            update.summary,
            update.source_event_id,
            update.created_at,
        ],
    )
    .context("world.db relationship update insert failed")?;
    Ok(())
}

fn upsert_entity_records(
    conn: &Connection,
    entities: &EntityRecords,
    updated_at: &str,
) -> Result<()> {
    for character in &entities.characters {
        upsert_character_record(conn, entities.world_id.as_str(), character, updated_at)?;
    }
    for place in &entities.places {
        upsert_place_record(conn, entities.world_id.as_str(), place, updated_at)?;
    }
    for faction in &entities.factions {
        upsert_named_entity(
            conn,
            entities.world_id.as_str(),
            "faction",
            faction,
            updated_at,
        )?;
    }
    for item in &entities.items {
        upsert_named_entity(conn, entities.world_id.as_str(), "item", item, updated_at)?;
    }
    for concept in &entities.concepts {
        upsert_named_entity(
            conn,
            entities.world_id.as_str(),
            "concept",
            concept,
            updated_at,
        )?;
    }
    Ok(())
}

fn upsert_character_record(
    conn: &Connection,
    world_id: &str,
    character: &CharacterRecord,
    updated_at: &str,
) -> Result<()> {
    upsert_entity_record(
        conn,
        &EntityRecordInsert {
            world_id,
            entity_id: character.id.as_str(),
            entity_type: "character",
            name: character.name.visible.as_str(),
            known_to_protagonist: character.knowledge_state == "self",
            status: character.role.as_str(),
            raw_json: to_json(character)?,
            updated_at,
        },
    )
}

fn upsert_place_record(
    conn: &Connection,
    world_id: &str,
    place: &PlaceRecord,
    updated_at: &str,
) -> Result<()> {
    upsert_entity_record(
        conn,
        &EntityRecordInsert {
            world_id,
            entity_id: place.id.as_str(),
            entity_type: "place",
            name: place.name.as_str(),
            known_to_protagonist: place.known_to_protagonist,
            status: "place",
            raw_json: to_json(place)?,
            updated_at,
        },
    )
}

fn upsert_named_entity(
    conn: &Connection,
    world_id: &str,
    entity_type: &str,
    entity: &NamedEntity,
    updated_at: &str,
) -> Result<()> {
    upsert_entity_record(
        conn,
        &EntityRecordInsert {
            world_id,
            entity_id: entity.id.as_str(),
            entity_type,
            name: entity.name.as_str(),
            known_to_protagonist: entity.known_to_protagonist,
            status: entity_type,
            raw_json: to_json(entity)?,
            updated_at,
        },
    )
}

struct EntityRecordInsert<'a> {
    world_id: &'a str,
    entity_id: &'a str,
    entity_type: &'a str,
    name: &'a str,
    known_to_protagonist: bool,
    status: &'a str,
    raw_json: String,
    updated_at: &'a str,
}

fn upsert_entity_record(conn: &Connection, entity: &EntityRecordInsert<'_>) -> Result<()> {
    let known_to_protagonist = i64::from(entity.known_to_protagonist);
    conn.execute(
        r"
        INSERT OR REPLACE INTO entity_records(
            world_id, entity_id, entity_type, name, known_to_protagonist,
            status, raw_json, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ",
        params![
            entity.world_id,
            entity.entity_id,
            entity.entity_type,
            entity.name,
            known_to_protagonist,
            entity.status,
            entity.raw_json,
            entity.updated_at,
        ],
    )
    .context("world.db entity record upsert failed")?;
    Ok(())
}

fn insert_character_memory(
    conn: &Connection,
    world_id: &str,
    character_id: &str,
    visibility: &str,
    summary: &str,
    source_event_id: &str,
    created_at: &str,
) -> Result<()> {
    let memory_id = format!("{source_event_id}:{character_id}");
    conn.execute(
        r"
        INSERT OR IGNORE INTO character_memories(
            memory_id, world_id, character_id, visibility, summary, source_event_id, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
        params![
            memory_id,
            world_id,
            character_id,
            visibility,
            summary,
            source_event_id,
            created_at,
        ],
    )
    .context("world.db character memory insert failed")?;
    Ok(())
}

fn upsert_player_knowledge(conn: &Connection, knowledge: &PlayerKnowledge) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO player_knowledge(world_id, raw_json, updated_at) VALUES (?1, ?2, ?3)",
        params![knowledge.world_id, to_json(knowledge)?, chrono::Utc::now().to_rfc3339()],
    )
    .context("world.db player knowledge upsert failed")?;
    Ok(())
}

fn upsert_hidden_state(conn: &Connection, hidden_state: &HiddenState) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO hidden_state(world_id, raw_json, updated_at) VALUES (?1, ?2, ?3)",
        params![
            hidden_state.world_id,
            to_json(hidden_state)?,
            chrono::Utc::now().to_rfc3339()
        ],
    )
    .context("world.db hidden state upsert failed")?;
    Ok(())
}

fn upsert_snapshot(conn: &Connection, snapshot: &TurnSnapshot, created_at: &str) -> Result<()> {
    let current_event_id = snapshot
        .current_event
        .as_ref()
        .map(|event| event.event_id.as_str());
    conn.execute(
        r"
        INSERT OR REPLACE INTO snapshots(
            world_id, session_id, turn_id, phase, current_event_id, raw_json, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ",
        params![
            snapshot.world_id,
            snapshot.session_id,
            snapshot.turn_id,
            snapshot.phase,
            current_event_id,
            to_json(snapshot)?,
            created_at,
        ],
    )
    .context("world.db snapshot upsert failed")?;
    Ok(())
}

fn upsert_render_packet(conn: &Connection, packet: &RenderPacket, created_at: &str) -> Result<()> {
    conn.execute(
        r"
        INSERT OR REPLACE INTO render_packets(world_id, turn_id, mode, raw_json, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ",
        params![
            packet.world_id,
            packet.turn_id,
            packet.mode,
            to_json(packet)?,
            created_at,
        ],
    )
    .context("world.db render packet upsert failed")?;
    Ok(())
}

fn upsert_materialized_projection_files(
    conn: &Connection,
    world_dir: &Path,
    world_id: &str,
    updated_at: &str,
) -> Result<()> {
    for projection in MATERIALIZED_PROJECTION_FILES {
        let path = world_dir.join(projection.filename);
        if !path.is_file() {
            continue;
        }
        let value: serde_json::Value = read_json(&path).with_context(|| {
            format!(
                "world.db materialized projection read failed: {}",
                path.display()
            )
        })?;
        let summary = summarize_materialized_projection(projection.kind, &value);
        conn.execute(
            r"
            INSERT OR REPLACE INTO materialized_projections(
                world_id, projection_id, projection_kind, title, summary, raw_json, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                world_id,
                projection.id,
                projection.kind,
                projection.title,
                summary,
                to_json(&value)?,
                updated_at,
            ],
        )
        .with_context(|| {
            format!(
                "world.db materialized projection upsert failed: projection_id={}",
                projection.id
            )
        })?;
        upsert_typed_memory_projection(conn, projection.kind, world_id, &value, updated_at)?;
    }
    Ok(())
}

fn upsert_typed_memory_projection(
    conn: &Connection,
    kind: &str,
    world_id: &str,
    value: &serde_json::Value,
    updated_at: &str,
) -> Result<()> {
    match kind {
        "world_lore" => upsert_world_lore_entries(conn, world_id, value, updated_at),
        "relationship_graph" => upsert_relationship_edges(conn, world_id, value, updated_at),
        "character_text_design" => upsert_character_text_designs(conn, world_id, value, updated_at),
        _ => Ok(()),
    }
}

fn upsert_world_lore_entries(
    conn: &Connection,
    world_id: &str,
    value: &serde_json::Value,
    updated_at: &str,
) -> Result<()> {
    let packet: WorldLorePacket = serde_json::from_value(value.clone())
        .context("world.db world_lore materialized projection parse failed")?;
    conn.execute(
        "DELETE FROM world_lore_entries WHERE world_id = ?1",
        params![world_id],
    )
    .context("world.db world_lore_entries clear failed")?;
    for entry in packet.entries {
        let raw_json = to_json(&entry)?;
        conn.execute(
            r"
            INSERT OR REPLACE INTO world_lore_entries(
                world_id, lore_id, domain, name, summary, visibility,
                confidence, authority, raw_json, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            ",
            params![
                world_id,
                entry.lore_id,
                format!("{:?}", entry.domain),
                entry.name,
                entry.summary,
                entry.visibility,
                entry.confidence,
                entry.authority,
                raw_json,
                updated_at,
            ],
        )
        .context("world.db world_lore_entry upsert failed")?;
    }
    Ok(())
}

fn upsert_relationship_edges(
    conn: &Connection,
    world_id: &str,
    value: &serde_json::Value,
    updated_at: &str,
) -> Result<()> {
    let packet: RelationshipGraphPacket = serde_json::from_value(value.clone())
        .context("world.db relationship_graph materialized projection parse failed")?;
    conn.execute(
        "DELETE FROM relationship_edges WHERE world_id = ?1",
        params![world_id],
    )
    .context("world.db relationship_edges clear failed")?;
    for edge in packet.active_edges {
        let raw_json = to_json(&edge)?;
        conn.execute(
            r"
            INSERT OR REPLACE INTO relationship_edges(
                world_id, edge_id, source_entity_id, target_entity_id, stance,
                visibility, visible_summary, raw_json, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ",
            params![
                world_id,
                edge.edge_id,
                edge.source_entity_id,
                edge.target_entity_id,
                edge.stance,
                edge.visibility,
                edge.visible_summary,
                raw_json,
                updated_at,
            ],
        )
        .context("world.db relationship_edge upsert failed")?;
    }
    Ok(())
}

fn upsert_character_text_designs(
    conn: &Connection,
    world_id: &str,
    value: &serde_json::Value,
    updated_at: &str,
) -> Result<()> {
    let packet: CharacterTextDesignPacket = serde_json::from_value(value.clone())
        .context("world.db character_text_design materialized projection parse failed")?;
    conn.execute(
        "DELETE FROM character_text_designs WHERE world_id = ?1",
        params![world_id],
    )
    .context("world.db character_text_designs clear failed")?;
    for design in packet.active_designs {
        let summary = character_text_design_summary(&design);
        let raw_json = to_json(&design)?;
        conn.execute(
            r"
            INSERT OR REPLACE INTO character_text_designs(
                world_id, entity_id, visible_name, role, visibility, summary,
                raw_json, updated_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ",
            params![
                world_id,
                design.entity_id,
                design.visible_name,
                design.role,
                design.visibility,
                summary,
                raw_json,
                updated_at,
            ],
        )
        .context("world.db character_text_design upsert failed")?;
    }
    Ok(())
}

fn character_text_design_summary(
    design: &crate::character_text_design::CharacterTextDesign,
) -> String {
    [
        design.speech.join(", "),
        design.endings.join(", "),
        design.tone.join(", "),
        design.gestures.join(", "),
        design.habits.join(", "),
        design.drift.join(", "),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" | ")
}

fn summarize_materialized_projection(kind: &str, value: &serde_json::Value) -> String {
    match kind {
        "body_resource" => {
            summarize_count_fields(value, &[("body", "body"), ("resources", "resources")])
        }
        "location_graph" => summarize_count_fields(
            value,
            &[
                ("current_location", "current"),
                ("nearby_locations", "nearby"),
            ],
        ),
        "plot_threads" => summarize_count_fields(value, &[("threads", "threads")]),
        "scene_pressure" => summarize_count_fields(value, &[("pressures", "pressures")]),
        "visual_asset_graph" => summarize_count_fields(
            value,
            &[
                ("display_assets", "display"),
                ("reference_assets", "reference"),
                ("pending_jobs", "pending"),
            ],
        ),
        "world_lore" => summarize_count_fields(value, &[("entries", "entries")]),
        "relationship_graph" => summarize_count_fields(value, &[("active_edges", "edges")]),
        "character_text_design" => summarize_count_fields(value, &[("active_designs", "designs")]),
        "change_ledger" => summarize_count_fields(value, &[("active_changes", "changes")]),
        "pattern_debt" => summarize_count_fields(value, &[("active_patterns", "patterns")]),
        "belief_graph" => {
            summarize_count_fields(value, &[("protagonist_visible_beliefs", "beliefs")])
        }
        "world_process_clock" => {
            summarize_count_fields(value, &[("visible_processes", "visible_processes")])
        }
        "player_intent_trace" => summarize_count_fields(value, &[("active_intents", "intents")]),
        "narrative_style_state" => {
            summarize_count_fields(value, &[("active_style_events", "style_events")])
        }
        _ => "projection available".to_owned(),
    }
}

fn summarize_count_fields(value: &serde_json::Value, fields: &[(&str, &str)]) -> String {
    fields
        .iter()
        .map(|(field, label)| {
            let count = value
                .get(*field)
                .and_then(serde_json::Value::as_array)
                .map_or_else(
                    || usize::from(value.get(*field).is_some()),
                    std::vec::Vec::len,
                );
            format!("{label}={count}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn rebuild_world_search_index_for_conn(conn: &Connection, world_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM world_search_fts WHERE world_id = ?1",
        params![world_id],
    )
    .context("world.db search index clear failed")?;
    index_world_facts(conn, world_id)?;
    index_canon_events(conn, world_id)?;
    index_character_memories(conn, world_id)?;
    index_entity_records(conn, world_id)?;
    index_entity_updates(conn, world_id)?;
    index_relationship_updates(conn, world_id)?;
    index_materialized_projections(conn, world_id)?;
    index_world_lore_entries(conn, world_id)?;
    index_relationship_edges(conn, world_id)?;
    index_character_text_designs(conn, world_id)?;
    Ok(())
}

fn index_world_facts(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT fact_id, category, subject, predicate, object
             FROM world_facts
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index world facts prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("world.db search index world facts query failed")?;
    for row in rows {
        let (fact_id, category, subject, predicate, object) =
            row.context("world.db search index world fact row failed")?;
        insert_search_document(
            conn,
            world_id,
            "world_facts",
            fact_id.as_str(),
            format!("{category} {subject} {predicate}").as_str(),
            object.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_canon_events(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT event_id, turn_id, kind, summary
             FROM canon_events
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index canon events prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .context("world.db search index canon events query failed")?;
    for row in rows {
        let (event_id, turn_id, kind, summary) =
            row.context("world.db search index canon event row failed")?;
        insert_search_document(
            conn,
            world_id,
            "canon_events",
            event_id.as_str(),
            format!("{turn_id} {kind}").as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_character_memories(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, character_id, summary
             FROM character_memories
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index character memories prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .context("world.db search index character memories query failed")?;
    for row in rows {
        let (memory_id, character_id, summary) =
            row.context("world.db search index character memory row failed")?;
        insert_search_document(
            conn,
            world_id,
            "character_memories",
            memory_id.as_str(),
            character_id.as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_entity_records(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT entity_id, entity_type, name, status
             FROM entity_records
             WHERE world_id = ?1 AND known_to_protagonist = 1",
        )
        .context("world.db search index entity records prepare failed")?;
    let rows = stmt
        .query_map(params![world_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .context("world.db search index entity records query failed")?;
    for row in rows {
        let (entity_id, entity_type, name, status) =
            row.context("world.db search index entity record row failed")?;
        insert_search_document(
            conn,
            world_id,
            "entity_records",
            entity_id.as_str(),
            format!("{entity_type} {name}").as_str(),
            status.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_entity_updates(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT update_id, entity_id, update_kind, summary
             FROM entity_updates
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index entity updates prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .context("world.db search index entity updates query failed")?;
    for row in rows {
        let (update_id, entity_id, update_kind, summary) =
            row.context("world.db search index entity update row failed")?;
        insert_search_document(
            conn,
            world_id,
            "entity_updates",
            update_id.as_str(),
            format!("{entity_id} {update_kind}").as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_relationship_updates(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT update_id, source_entity_id, target_entity_id, relation_kind, summary
             FROM relationship_updates
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index relationship updates prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("world.db search index relationship updates query failed")?;
    for row in rows {
        let (update_id, source_id, target_id, relation_kind, summary) =
            row.context("world.db search index relationship update row failed")?;
        insert_search_document(
            conn,
            world_id,
            "relationship_updates",
            update_id.as_str(),
            format!("{source_id} {target_id} {relation_kind}").as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_materialized_projections(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT projection_id, projection_kind, title, summary
             FROM materialized_projections
             WHERE world_id = ?1",
        )
        .context("world.db search index materialized projections prepare failed")?;
    let rows = stmt
        .query_map(params![world_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .context("world.db search index materialized projections query failed")?;
    for row in rows {
        let (projection_id, projection_kind, title, summary) =
            row.context("world.db search index materialized projection row failed")?;
        insert_search_document(
            conn,
            world_id,
            "materialized_projections",
            projection_id.as_str(),
            format!("{projection_kind} {title}").as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_world_lore_entries(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT lore_id, domain, name, summary
             FROM world_lore_entries
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index world_lore_entries prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .context("world.db search index world_lore_entries query failed")?;
    for row in rows {
        let (lore_id, domain, name, summary) =
            row.context("world.db search index world_lore_entry row failed")?;
        insert_search_document(
            conn,
            world_id,
            "world_lore_entries",
            lore_id.as_str(),
            format!("{domain} {name}").as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_relationship_edges(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT edge_id, source_entity_id, target_entity_id, stance, visible_summary
             FROM relationship_edges
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index relationship_edges prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .context("world.db search index relationship_edges query failed")?;
    for row in rows {
        let (edge_id, source_id, target_id, stance, summary) =
            row.context("world.db search index relationship_edge row failed")?;
        insert_search_document(
            conn,
            world_id,
            "relationship_edges",
            edge_id.as_str(),
            format!("{source_id} {target_id} {stance}").as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn index_character_text_designs(conn: &Connection, world_id: &str) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT entity_id, visible_name, role, summary
             FROM character_text_designs
             WHERE world_id = ?1 AND visibility = ?2",
        )
        .context("world.db search index character_text_designs prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, PLAYER_VISIBLE], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .context("world.db search index character_text_designs query failed")?;
    for row in rows {
        let (entity_id, visible_name, role, summary) =
            row.context("world.db search index character_text_design row failed")?;
        insert_search_document(
            conn,
            world_id,
            "character_text_designs",
            entity_id.as_str(),
            format!("{visible_name} {role}").as_str(),
            summary.as_str(),
            PLAYER_VISIBLE,
        )?;
    }
    Ok(())
}

fn insert_search_document(
    conn: &Connection,
    world_id: &str,
    source_table: &str,
    source_id: &str,
    title: &str,
    body: &str,
    visibility: &str,
) -> Result<()> {
    let title = redact_guide_choice_public_hints(title);
    let body = redact_guide_choice_public_hints(body);
    conn.execute(
        r"
        INSERT INTO world_search_fts(world_id, source_table, source_id, title, body, visibility)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ",
        params![world_id, source_table, source_id, title, body, visibility],
    )
    .context("world.db search document insert failed")?;
    Ok(())
}

fn search_world_index_like(
    conn: &Connection,
    world_id: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<WorldSearchHit>> {
    let like_query = format!("%{}%", query.trim());
    let mut stmt = conn
        .prepare(
            "SELECT source_table, source_id, title, body
             FROM world_search_fts
             WHERE world_id = ?1 AND (title LIKE ?2 OR body LIKE ?2)
             LIMIT ?3",
        )
        .context("world.db LIKE search prepare failed")?;
    let rows = stmt
        .query_map(params![world_id, like_query, limit_to_i64(limit)], |row| {
            Ok(WorldSearchHit {
                source_table: row.get(0)?,
                source_id: row.get(1)?,
                title: redact_guide_choice_public_hints(&row.get::<_, String>(2)?),
                snippet: redact_guide_choice_public_hints(&row.get::<_, String>(3)?),
                rank: 0.0,
            })
        })
        .context("world.db LIKE search query failed")?;
    collect_rows(rows, "world.db LIKE search row failed")
}

fn schema_version(conn: &Connection) -> Result<String> {
    conn.query_row(
        "SELECT value FROM schema_meta WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )
    .optional()
    .context("world.db schema version query failed")?
    .with_context(|| "world.db missing schema_version")
}

fn count_world_rows(conn: &Connection, table: &str, world_id: &str) -> Result<u64> {
    if !WORLD_SCOPED_TABLES.contains(&table) {
        bail!("unsupported world-scoped table count: {table}");
    }
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE world_id = ?1");
    conn.query_row(sql.as_str(), params![world_id], |row| row.get(0))
        .with_context(|| format!("world.db count failed: table={table}"))
}

fn limit_to_i64(limit: usize) -> i64 {
    i64::try_from(limit).unwrap_or(i64::MAX)
}

fn normalize_fts_query(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("\"{}\"", trimmed.replace('"', "\"\""))
}

fn collect_rows<T, I>(rows: I, row_context: &str) -> Result<Vec<T>>
where
    I: IntoIterator<Item = crate::sqlite::RusqliteResult<T>>,
{
    let mut values = Vec::new();
    for row in rows {
        values.push(row.context(row_context.to_owned())?);
    }
    Ok(values)
}

fn to_json<T>(value: &T) -> Result<String>
where
    T: serde::Serialize,
{
    serde_json::to_string(value).context("world.db JSON serialization failed")
}

fn read_canon_events_jsonl(path: &Path) -> Result<Vec<CanonEvent>> {
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

fn read_structured_updates_jsonl(path: &Path) -> Result<Vec<StructuredEntityUpdates>> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut updates = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let update = serde_json::from_str(line)
            .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))?;
        updates.push(update);
    }
    Ok(updates)
}

fn read_session_snapshots(world_dir: &Path) -> Result<Vec<TurnSnapshot>> {
    read_session_json_files(world_dir, "snapshots")
}

fn read_session_render_packets(world_dir: &Path) -> Result<Vec<RenderPacket>> {
    read_session_json_files(world_dir, "render_packets")
}

fn read_session_json_files<T>(world_dir: &Path, subdir: &str) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    let sessions_dir = world_dir.join("sessions");
    if !sessions_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for session_entry in fs::read_dir(&sessions_dir)
        .with_context(|| format!("failed to read {}", sessions_dir.display()))?
    {
        let session_entry =
            session_entry.with_context(|| format!("failed to read {}", sessions_dir.display()))?;
        let candidate_dir = session_entry.path().join(subdir);
        if !candidate_dir.is_dir() {
            continue;
        }
        for file_entry in fs::read_dir(&candidate_dir)
            .with_context(|| format!("failed to read {}", candidate_dir.display()))?
        {
            let file_entry = file_entry
                .with_context(|| format!("failed to read {}", candidate_dir.display()))?;
            let path = file_entry.path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut values = Vec::new();
    for path in files {
        values.push(read_json(&path)?);
    }
    Ok(values)
}

fn clear_world_rows(conn: &Connection, world_id: &str) -> Result<()> {
    for table in WORLD_REPAIR_CLEAR_TABLES {
        let sql = format!("DELETE FROM {table} WHERE world_id = ?1");
        conn.execute(sql.as_str(), params![world_id])
            .with_context(|| format!("world.db repair clear failed: table={table}"))?;
    }
    Ok(())
}

const WORLD_SCOPED_TABLES: &[&str] = &[
    "worlds",
    "world_facts",
    "canon_events",
    "character_memories",
    "state_changes",
    "entity_records",
    "entity_updates",
    "relationship_updates",
    "materialized_projections",
    "world_lore_entries",
    "relationship_edges",
    "character_text_designs",
    "snapshots",
    "chapter_summaries",
    "world_search_fts",
];

const WORLD_REPAIR_CLEAR_TABLES: &[&str] = &[
    "world_search_fts",
    "chapter_summaries",
    "render_packets",
    "snapshots",
    "relationship_updates",
    "materialized_projections",
    "world_lore_entries",
    "relationship_edges",
    "character_text_designs",
    "entity_updates",
    "character_memories",
    "state_changes",
    "timeline_events",
    "canon_events",
    "entity_records",
    "hidden_state",
    "player_knowledge",
    "world_facts",
    "worlds",
];

struct MaterializedProjectionFile {
    id: &'static str,
    kind: &'static str,
    title: &'static str,
    filename: &'static str,
}

const MATERIALIZED_PROJECTION_FILES: &[MaterializedProjectionFile] = &[
    MaterializedProjectionFile {
        id: "body_resource_state",
        kind: "body_resource",
        title: "몸과 소지품",
        filename: BODY_RESOURCE_STATE_FILENAME,
    },
    MaterializedProjectionFile {
        id: "location_graph",
        kind: "location_graph",
        title: "장소 그래프",
        filename: LOCATION_GRAPH_FILENAME,
    },
    MaterializedProjectionFile {
        id: "plot_threads",
        kind: "plot_threads",
        title: "열린 서사 스레드",
        filename: PLOT_THREADS_FILENAME,
    },
    MaterializedProjectionFile {
        id: "active_scene_pressures",
        kind: "scene_pressure",
        title: "장면 압력",
        filename: ACTIVE_SCENE_PRESSURES_FILENAME,
    },
    MaterializedProjectionFile {
        id: "visual_asset_graph",
        kind: "visual_asset_graph",
        title: "시각 자료 그래프",
        filename: VISUAL_ASSET_GRAPH_FILENAME,
    },
    MaterializedProjectionFile {
        id: "world_lore",
        kind: "world_lore",
        title: "월드 로어",
        filename: WORLD_LORE_FILENAME,
    },
    MaterializedProjectionFile {
        id: "relationship_graph",
        kind: "relationship_graph",
        title: "관계망",
        filename: RELATIONSHIP_GRAPH_FILENAME,
    },
    MaterializedProjectionFile {
        id: "character_text_design",
        kind: "character_text_design",
        title: "인물 텍스트 디자인",
        filename: CHARACTER_TEXT_DESIGN_FILENAME,
    },
    MaterializedProjectionFile {
        id: "change_ledger",
        kind: "change_ledger",
        title: "변화 장부",
        filename: CHANGE_LEDGER_FILENAME,
    },
    MaterializedProjectionFile {
        id: "pattern_debt",
        kind: "pattern_debt",
        title: "반복 부채",
        filename: PATTERN_DEBT_FILENAME,
    },
    MaterializedProjectionFile {
        id: "belief_graph",
        kind: "belief_graph",
        title: "믿음 그래프",
        filename: BELIEF_GRAPH_FILENAME,
    },
    MaterializedProjectionFile {
        id: "world_process_clock",
        kind: "world_process_clock",
        title: "세계 진행 시계",
        filename: WORLD_PROCESSES_FILENAME,
    },
    MaterializedProjectionFile {
        id: "player_intent_trace",
        kind: "player_intent_trace",
        title: "플레이어 의도 흔적",
        filename: PLAYER_INTENT_TRACE_FILENAME,
    },
    MaterializedProjectionFile {
        id: "narrative_style_state",
        kind: "narrative_style_state",
        title: "서사 문체 상태",
        filename: NARRATIVE_STYLE_STATE_FILENAME,
    },
];

#[cfg(test)]
mod tests {
    use super::{
        force_chapter_summary, latest_chapter_summaries, repair_world_db, search_world_db,
        sync_world_db_materialized_projections, validate_world_db, world_db_stats,
    };
    use crate::store::{InitWorldOptions, init_world};
    use crate::turn::{AdvanceTurnOptions, advance_turn};
    use crate::world_lore::{
        WORLD_LORE_ENTRY_SCHEMA_VERSION, WORLD_LORE_FILENAME, WorldLoreDomain, WorldLoreEntry,
        WorldLorePacket,
    };
    use tempfile::tempdir;

    #[test]
    fn init_creates_queryable_world_db() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_db_test
title: "DB 세계"
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
        let stats = world_db_stats(&initialized.world_dir, "stw_db_test")?;
        assert_eq!(stats.canon_events, 1);
        assert!(stats.world_facts >= 3);
        assert!(stats.entity_records >= 3);
        assert!(stats.search_documents >= 3);
        let report = validate_world_db(&initialized.world_dir, "stw_db_test", 1)?;
        assert!(report.warnings.is_empty());
        Ok(())
    }

    #[test]
    fn force_chapter_summary_records_open_events() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_chapter_test
title: "챕터 세계"
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
        let summary = force_chapter_summary(&initialized.world_dir, "stw_chapter_test")?;
        let Some(summary) = summary else {
            anyhow::bail!("expected forced chapter summary");
        };
        assert_eq!(summary.chapter_index, 1);
        assert_eq!(summary.source_turn_start, "turn_0000");
        let summaries = latest_chapter_summaries(&initialized.world_dir, "stw_chapter_test", 3)?;
        assert_eq!(summaries.len(), 1);
        Ok(())
    }

    #[test]
    fn search_indexes_visible_turn_updates() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_search_test
title: "검색 세계"
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
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store),
            world_id: "stw_search_test".to_owned(),
            input: "7".to_owned(),
        })?;
        let hits = search_world_db(&initialized.world_dir, "stw_search_test", "판단 위임", 10)?;
        assert!(!hits.is_empty());
        Ok(())
    }

    #[test]
    fn search_indexes_typed_blueprint_projection_entries() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_blueprint_search
title: "블루프린트 검색 세계"
premise:
  genre: "중세 판타지"
  protagonist: "문서 보관자"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store),
            session_id: None,
        })?;
        crate::store::write_json(
            &initialized.world_dir.join(WORLD_LORE_FILENAME),
            &WorldLorePacket {
                world_id: "stw_blueprint_search".to_owned(),
                turn_id: "turn_0001".to_owned(),
                entries: vec![WorldLoreEntry {
                    schema_version: WORLD_LORE_ENTRY_SCHEMA_VERSION.to_owned(),
                    lore_id: "lore:customs:gate_tax".to_owned(),
                    domain: WorldLoreDomain::Customs,
                    name: "gate tax".to_owned(),
                    summary: "gate tax requires a stamped token".to_owned(),
                    visibility: "player_visible".to_owned(),
                    confidence: "confirmed".to_owned(),
                    authority: "world_lore_updates".to_owned(),
                    source_refs: vec!["world_lore_update:turn_0001:00".to_owned()],
                    mechanical_axis: vec!["social_permission".to_owned()],
                }],
                ..WorldLorePacket::default()
            },
        )?;
        sync_world_db_materialized_projections(
            &initialized.world_dir,
            "stw_blueprint_search",
            "2026-04-29T00:00:00Z",
        )?;

        let hits = search_world_db(
            &initialized.world_dir,
            "stw_blueprint_search",
            "stamped token",
            10,
        )?;

        assert!(
            hits.iter()
                .any(|hit| hit.source_table == "world_lore_entries")
        );
        Ok(())
    }

    #[test]
    fn repair_world_db_rebuilds_projection_counts() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_repair_test
title: "수리 세계"
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
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store),
            world_id: "stw_repair_test".to_owned(),
            input: "7".to_owned(),
        })?;
        let report = repair_world_db(&initialized.world_dir, "stw_repair_test")?;
        assert!(report.rebuilt);
        assert_eq!(report.canon_events, 2);
        assert!(report.search_documents >= 3);
        let stats = world_db_stats(&initialized.world_dir, "stw_repair_test")?;
        assert!(stats.entity_updates >= 2);
        assert!(stats.relationship_updates >= 1);
        Ok(())
    }
}
