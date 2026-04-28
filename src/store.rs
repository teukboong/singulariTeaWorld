use crate::models::{
    ANCHOR_CHARACTER_INVARIANT, CanonEvent, DashboardSummary, EntityRecords, HiddenState,
    NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, PlayerKnowledge, RENDER_PACKET_SCHEMA_VERSION,
    RenderPacket, TurnSnapshot, VisibleState, WorldRecord, WorldSeed, default_turn_choices,
    initial_canon_event,
};
use crate::world_db::initialize_world_db;
use crate::world_docs::refresh_world_docs;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

pub const ACTIVE_WORLD_BINDING_SCHEMA_VERSION: &str = "singulari.active_world.v1";
pub const SINGULARI_WORLD_HOME_ENV: &str = "SINGULARI_WORLD_HOME";
const DEFAULT_STORE_SUBDIR: &str = ".local/share/singulari-world";
const WORLDS_DIR: &str = "worlds";
pub const ACTIVE_WORLD_FILENAME: &str = "active_world.json";
pub(crate) const WORLD_FILENAME: &str = "world.json";
const LAWS_FILENAME: &str = "laws.md";
pub(crate) const CANON_EVENTS_FILENAME: &str = "canon_events.jsonl";
pub(crate) const ENTITY_UPDATES_FILENAME: &str = "entity_updates.jsonl";
pub(crate) const HIDDEN_STATE_FILENAME: &str = "hidden_state.json";
pub(crate) const PLAYER_KNOWLEDGE_FILENAME: &str = "player_knowledge.json";
pub(crate) const ENTITIES_FILENAME: &str = "entities.json";
pub(crate) const LATEST_SNAPSHOT_FILENAME: &str = "latest_snapshot.json";
pub(crate) const TURN_LOG_FILENAME: &str = "turn_log.jsonl";

#[derive(Debug, Clone)]
pub struct StorePaths {
    pub root: PathBuf,
    pub worlds_dir: PathBuf,
}

#[derive(Debug, Clone)]
pub struct InitWorldOptions {
    pub seed_path: PathBuf,
    pub store_root: Option<PathBuf>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InitializedWorld {
    pub world: WorldRecord,
    pub world_dir: PathBuf,
    pub session_id: String,
    pub snapshot_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveWorldBinding {
    pub schema_version: String,
    pub world_id: String,
    pub session_id: String,
    pub updated_at: String,
}

/// Initialize a file-backed Singulari World world.
///
/// # Errors
///
/// Returns an error when the seed cannot be read or parsed, the anchor
/// invariant is unsupported, the world id is unsafe, or the destination world
/// already exists.
pub fn init_world(options: &InitWorldOptions) -> Result<InitializedWorld> {
    let seed = load_world_seed(&options.seed_path)?;
    init_world_from_seed(
        seed,
        options.store_root.as_deref(),
        options.session_id.clone(),
    )
}

pub(crate) fn init_world_from_seed(
    seed: WorldSeed,
    store_root: Option<&Path>,
    session_id: Option<String>,
) -> Result<InitializedWorld> {
    validate_seed_before_init(&seed)?;
    let now = Utc::now().to_rfc3339();
    let world = WorldRecord::from_seed(seed, now);
    validate_safe_id("world_id", world.world_id.as_str())?;
    let store_paths = resolve_store_paths(store_root)?;
    fs::create_dir_all(&store_paths.worlds_dir)
        .with_context(|| format!("failed to create {}", store_paths.worlds_dir.display()))?;
    let world_dir = world_dir(&store_paths, world.world_id.as_str());
    if world_dir.exists() {
        bail!(
            "singulari-world init refused to overwrite existing world: {}",
            world_dir.display()
        );
    }

    let all_sessions_dir = world_dir.join("sessions");
    let session_id = session_id.unwrap_or_else(|| default_session_id(world.world_id.as_str()));
    validate_safe_id("session_id", session_id.as_str())?;
    let active_session_dir = all_sessions_dir.join(&session_id);
    let snapshot_dir = active_session_dir.join("snapshots");
    let render_packet_dir = active_session_dir.join("render_packets");

    fs::create_dir_all(world_dir.join("timelines"))
        .with_context(|| format!("failed to create {}", world_dir.join("timelines").display()))?;
    fs::create_dir_all(&snapshot_dir)
        .with_context(|| format!("failed to create {}", snapshot_dir.display()))?;
    fs::create_dir_all(&render_packet_dir)
        .with_context(|| format!("failed to create {}", render_packet_dir.display()))?;
    write_json(&world_dir.join(WORLD_FILENAME), &world)?;
    fs::write(world_dir.join(LAWS_FILENAME), render_laws(&world)).with_context(|| {
        format!(
            "failed to write {}",
            world_dir.join(LAWS_FILENAME).display()
        )
    })?;
    let hidden_state = HiddenState::initial(world.world_id.as_str());
    let player_knowledge = PlayerKnowledge::initial(world.world_id.as_str());
    let entities = EntityRecords::initial(&world);
    let initial_event = initial_canon_event(&world);
    write_json(&world_dir.join(HIDDEN_STATE_FILENAME), &hidden_state)?;
    write_json(
        &world_dir.join(PLAYER_KNOWLEDGE_FILENAME),
        &player_knowledge,
    )?;
    write_json(&world_dir.join(ENTITIES_FILENAME), &entities)?;
    append_canon_event(&world_dir.join(CANON_EVENTS_FILENAME), &initial_event)?;
    fs::write(world_dir.join(ENTITY_UPDATES_FILENAME), "").with_context(|| {
        format!(
            "failed to write {}",
            world_dir.join(ENTITY_UPDATES_FILENAME).display()
        )
    })?;

    let snapshot = TurnSnapshot::initial(&world, session_id.clone());
    let snapshot_path = snapshot_dir.join("turn_0000.json");
    let render_packet_path = render_packet_dir.join("turn_0000.json");
    write_json(&snapshot_path, &snapshot)?;
    write_json(&world_dir.join(LATEST_SNAPSHOT_FILENAME), &snapshot)?;
    write_json(
        &render_packet_path,
        &initial_render_packet_for_waiting_turn(&world, &snapshot),
    )?;
    initialize_world_db(
        &world_dir,
        &world,
        &snapshot,
        &entities,
        &hidden_state,
        &player_knowledge,
        &initial_event,
    )?;
    refresh_world_docs(&world_dir)?;
    fs::write(active_session_dir.join(TURN_LOG_FILENAME), "").with_context(|| {
        format!(
            "failed to write {}",
            active_session_dir.join(TURN_LOG_FILENAME).display()
        )
    })?;
    Ok(InitializedWorld {
        world,
        world_dir,
        session_id,
        snapshot_path,
    })
}

fn initial_render_packet_for_waiting_turn(
    world: &WorldRecord,
    snapshot: &TurnSnapshot,
) -> RenderPacket {
    RenderPacket {
        schema_version: RENDER_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id.clone(),
        turn_id: snapshot.turn_id.clone(),
        mode: "normal".to_owned(),
        narrative_contract: "initial VN packet waits for the first agent-authored turn".to_owned(),
        narrative_scene: Some(NarrativeScene {
            schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
            speaker: None,
            text_blocks: Vec::new(),
            tone_notes: vec!["initial_turn_generation_pending".to_owned()],
        }),
        visible_state: VisibleState {
            dashboard: DashboardSummary {
                phase: snapshot.phase.clone(),
                location: snapshot.protagonist_state.location.clone(),
                anchor_invariant: world.anchor_character.invariant.clone(),
                current_event: "interlude".to_owned(),
                status: "흐름 수렴 중".to_owned(),
            },
            scan_targets: Vec::new(),
            choices: default_turn_choices(),
        },
        adjudication: None,
        codex_view: None,
        canon_delta_refs: Vec::new(),
        forbidden_reveals: Vec::new(),
        style_notes: vec!["initial_turn_generation_pending".to_owned()],
    }
}

/// Resolve the simulator store paths.
///
/// # Errors
///
/// Returns an error when `HOME` is unavailable and no explicit store root is
/// provided.
pub fn resolve_store_paths(store_root: Option<&Path>) -> Result<StorePaths> {
    let root = match store_root {
        Some(path) => path.to_path_buf(),
        None => default_store_root()?,
    };
    Ok(StorePaths {
        worlds_dir: root.join(WORLDS_DIR),
        root,
    })
}

/// Load a persisted world record.
///
/// # Errors
///
/// Returns an error when the world record cannot be found, read, or parsed.
pub fn load_world_record(store_root: Option<&Path>, world_id: &str) -> Result<WorldRecord> {
    let paths = resolve_store_paths(store_root)?;
    let path = world_dir(&paths, world_id).join(WORLD_FILENAME);
    read_json(&path)
}

/// Load the latest snapshot pointer for a world.
///
/// # Errors
///
/// Returns an error when the snapshot pointer cannot be found, read, or parsed.
pub fn load_latest_snapshot(store_root: Option<&Path>, world_id: &str) -> Result<TurnSnapshot> {
    read_json(&latest_snapshot_path(store_root, world_id)?)
}

/// Resolve the latest snapshot path for a world.
///
/// # Errors
///
/// Returns an error when store paths cannot be resolved.
pub fn latest_snapshot_path(store_root: Option<&Path>, world_id: &str) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(world_dir(&paths, world_id).join(LATEST_SNAPSHOT_FILENAME))
}

/// Save the active world pointer used by worldsim chat continuation.
///
/// # Errors
///
/// Returns an error when ids are unsafe, store paths cannot be resolved, or the
/// active binding file cannot be written.
pub fn save_active_world(
    store_root: Option<&Path>,
    world_id: &str,
    session_id: &str,
) -> Result<ActiveWorldBinding> {
    validate_safe_id("world_id", world_id)?;
    validate_safe_id("session_id", session_id)?;
    let paths = resolve_store_paths(store_root)?;
    fs::create_dir_all(&paths.root)
        .with_context(|| format!("failed to create {}", paths.root.display()))?;
    let binding = ActiveWorldBinding {
        schema_version: ACTIVE_WORLD_BINDING_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        session_id: session_id.to_owned(),
        updated_at: Utc::now().to_rfc3339(),
    };
    write_json(&active_world_path(&paths), &binding)?;
    Ok(binding)
}

/// Load the active world pointer used when a command omits `--world-id`.
///
/// # Errors
///
/// Returns an error when no active binding exists, the binding is malformed, or
/// the referenced world cannot be loaded.
pub fn load_active_world(store_root: Option<&Path>) -> Result<ActiveWorldBinding> {
    let paths = resolve_store_paths(store_root)?;
    let binding: ActiveWorldBinding = read_json(&active_world_path(&paths))?;
    if binding.schema_version != ACTIVE_WORLD_BINDING_SCHEMA_VERSION {
        bail!(
            "active world schema_version mismatch: expected {}, got {}",
            ACTIVE_WORLD_BINDING_SCHEMA_VERSION,
            binding.schema_version
        );
    }
    validate_safe_id("world_id", binding.world_id.as_str())?;
    validate_safe_id("session_id", binding.session_id.as_str())?;
    let snapshot = load_latest_snapshot(store_root, binding.world_id.as_str())?;
    if snapshot.session_id != binding.session_id {
        bail!(
            "active world session mismatch: active={}, latest_snapshot={}",
            binding.session_id,
            snapshot.session_id
        );
    }
    Ok(binding)
}

/// Resolve an explicit world id, or fall back to the active world binding.
///
/// # Errors
///
/// Returns an error when the explicit id is unsafe or no valid active world is
/// available.
pub fn resolve_world_id(store_root: Option<&Path>, world_id: Option<&str>) -> Result<String> {
    if let Some(world_id) = world_id {
        validate_safe_id("world_id", world_id)?;
        return Ok(world_id.to_owned());
    }
    Ok(load_active_world(store_root)?.world_id)
}

pub(crate) fn world_dir(paths: &StorePaths, world_id: &str) -> PathBuf {
    paths.worlds_dir.join(world_id)
}

fn active_world_path(paths: &StorePaths) -> PathBuf {
    paths.root.join(ACTIVE_WORLD_FILENAME)
}

pub(crate) fn world_file_paths(paths: &StorePaths, world_id: &str) -> WorldFilePaths {
    let dir = world_dir(paths, world_id);
    WorldFilePaths {
        dir: dir.clone(),
        world: dir.join(WORLD_FILENAME),
        canon_events: dir.join(CANON_EVENTS_FILENAME),
        entity_updates: dir.join(ENTITY_UPDATES_FILENAME),
        hidden_state: dir.join(HIDDEN_STATE_FILENAME),
        player_knowledge: dir.join(PLAYER_KNOWLEDGE_FILENAME),
        entities: dir.join(ENTITIES_FILENAME),
        latest_snapshot: dir.join(LATEST_SNAPSHOT_FILENAME),
    }
}

#[derive(Debug, Clone)]
pub(crate) struct WorldFilePaths {
    pub(crate) dir: PathBuf,
    pub(crate) world: PathBuf,
    pub(crate) canon_events: PathBuf,
    pub(crate) entity_updates: PathBuf,
    pub(crate) hidden_state: PathBuf,
    pub(crate) player_knowledge: PathBuf,
    pub(crate) entities: PathBuf,
    pub(crate) latest_snapshot: PathBuf,
}

pub(crate) fn read_json<T>(path: &Path) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("failed to parse {}", path.display()))
}

pub(crate) fn write_json<T>(path: &Path, value: &T) -> Result<()>
where
    T: serde::Serialize,
{
    let body = serde_json::to_string_pretty(value)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    fs::write(path, format!("{body}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

pub(crate) fn append_jsonl<T>(path: &Path, value: &T) -> Result<()>
where
    T: serde::Serialize,
{
    let body = serde_json::to_string(value).context("failed to serialize JSONL value")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    writeln!(file, "{body}").with_context(|| format!("failed to append {}", path.display()))
}

pub(crate) fn append_canon_event(path: &Path, event: &CanonEvent) -> Result<()> {
    let body = serde_json::to_string(event).context("failed to serialize canon event")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    writeln!(file, "{body}").with_context(|| format!("failed to append {}", path.display()))
}

fn load_world_seed(path: &Path) -> Result<WorldSeed> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read seed {}", path.display()))?;
    if path.extension().and_then(|value| value.to_str()) == Some("json") {
        return serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse JSON seed {}", path.display()));
    }
    serde_yaml::from_str(&raw)
        .with_context(|| format!("failed to parse YAML seed {}", path.display()))
}

fn validate_seed_before_init(seed: &WorldSeed) -> Result<()> {
    if seed.schema_version != crate::models::WORLD_SEED_SCHEMA_VERSION {
        bail!(
            "seed schema_version mismatch: expected {}, got {}",
            crate::models::WORLD_SEED_SCHEMA_VERSION,
            seed.schema_version
        );
    }
    if seed.anchor_character.invariant.trim().is_empty() {
        return Ok(());
    }
    if seed.anchor_character.invariant != ANCHOR_CHARACTER_INVARIANT {
        bail!(
            "anchor invariant mismatch: expected {}, got {}",
            ANCHOR_CHARACTER_INVARIANT,
            seed.anchor_character.invariant
        );
    }
    Ok(())
}

fn validate_safe_id(label: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        bail!("{label} must not be empty");
    }
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Ok(());
    }
    bail!("{label} must contain only ASCII letters, digits, '-' or '_': {value}");
}

fn default_store_root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os(SINGULARI_WORLD_HOME_ENV) {
        return Ok(PathBuf::from(root));
    }
    let home = std::env::var_os("HOME").context("HOME is not set; pass --store-root")?;
    Ok(PathBuf::from(home).join(DEFAULT_STORE_SUBDIR))
}

fn default_session_id(world_id: &str) -> String {
    format!("stw_session_{world_id}_0001")
}

fn render_laws(world: &WorldRecord) -> String {
    format!(
        "# Singulari World World Laws\n\n- death_is_final: {}\n- discovery_required: {}\n- bodily_needs_active: {}\n- miracles_forbidden: {}\n- anchor_invariant: {}\n",
        world.laws.death_is_final,
        world.laws.discovery_required,
        world.laws.bodily_needs_active,
        world.laws.miracles_forbidden,
        world.anchor_character.invariant
    )
}

#[cfg(test)]
mod tests {
    use super::{InitWorldOptions, init_world, load_active_world, save_active_world};
    use crate::models::ANCHOR_CHARACTER_ID;
    use crate::store::read_json;
    use tempfile::tempdir;

    #[test]
    fn init_world_creates_anchor_character_record() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_test
title: "테스트 세계"
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
        let entities: crate::models::EntityRecords =
            read_json(&initialized.world_dir.join("entities.json"))?;
        assert!(
            entities
                .characters
                .iter()
                .any(|character| character.id == ANCHOR_CHARACTER_ID)
        );
        Ok(())
    }

    #[test]
    fn init_world_rejects_wrong_anchor_invariant() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_test
title: "테스트 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
anchor_character:
  invariant: "ordinary_companion"
"#,
        )?;
        let result = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(temp.path().join("store")),
            session_id: None,
        });
        let Err(error) = result else {
            anyhow::bail!("wrong anchor invariant should fail init");
        };
        assert!(format!("{error:#}").contains("anchor invariant mismatch"));
        Ok(())
    }

    #[test]
    fn active_world_binding_round_trips_after_init() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store_root = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_active_test
title: "활성 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store_root.clone()),
            session_id: Some("session_active_test".to_owned()),
        })?;
        save_active_world(
            Some(store_root.as_path()),
            initialized.world.world_id.as_str(),
            initialized.session_id.as_str(),
        )?;
        let active = load_active_world(Some(store_root.as_path()))?;
        assert_eq!(active.world_id, "stw_active_test");
        assert_eq!(active.session_id, "session_active_test");
        Ok(())
    }
}
