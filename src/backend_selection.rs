use crate::store::resolve_store_paths;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const WORLD_BACKEND_SELECTION_SCHEMA_VERSION: &str = "singulari.world_backend_selection.v1";
pub const WORLD_BACKEND_SELECTION_FILENAME: &str = "backend_selection.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldTextBackend {
    CodexAppServer,
    Webgpt,
}

impl WorldTextBackend {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CodexAppServer => "codex-app-server",
            Self::Webgpt => "webgpt",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorldVisualBackend {
    CodexAppServer,
    Webgpt,
}

impl WorldVisualBackend {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CodexAppServer => "codex-app-server",
            Self::Webgpt => "webgpt",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldBackendSelection {
    pub schema_version: String,
    pub world_id: String,
    pub text_backend: WorldTextBackend,
    pub visual_backend: WorldVisualBackend,
    pub locked: bool,
    pub source: String,
    pub created_at: String,
}

impl WorldBackendSelection {
    #[must_use]
    pub fn new(
        world_id: String,
        text_backend: WorldTextBackend,
        visual_backend: WorldVisualBackend,
        source: impl Into<String>,
    ) -> Self {
        Self {
            schema_version: WORLD_BACKEND_SELECTION_SCHEMA_VERSION.to_owned(),
            world_id,
            text_backend,
            visual_backend,
            locked: true,
            source: source.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Returns the per-world backend selection file path.
///
/// # Errors
///
/// Returns an error when the configured store root cannot be resolved.
pub fn backend_selection_path(store_root: Option<&Path>, world_id: &str) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths
        .worlds_dir
        .join(world_id)
        .join("agent_bridge")
        .join(WORLD_BACKEND_SELECTION_FILENAME))
}

/// Loads the locked backend selection for a world, if one exists.
///
/// # Errors
///
/// Returns an error when the store path cannot be resolved, the file cannot be
/// read or parsed, or the stored world id does not match `world_id`.
pub fn load_world_backend_selection(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<Option<WorldBackendSelection>> {
    let path = backend_selection_path(store_root, world_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path.as_path())
        .with_context(|| format!("failed to read {}", path.display()))?;
    let selection = serde_json::from_str::<WorldBackendSelection>(raw.as_str())
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if selection.world_id != world_id {
        anyhow::bail!(
            "backend selection world mismatch: expected={world_id}, actual={}",
            selection.world_id
        );
    }
    Ok(Some(selection))
}

/// Persists a world's backend selection.
///
/// # Errors
///
/// Returns an error when the store path cannot be resolved, the destination
/// directory or file cannot be written, or an existing locked selection would
/// be changed.
pub fn save_world_backend_selection(
    store_root: Option<&Path>,
    selection: &WorldBackendSelection,
) -> Result<PathBuf> {
    let path = backend_selection_path(store_root, selection.world_id.as_str())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    if path.exists() {
        let existing = load_world_backend_selection(store_root, selection.world_id.as_str())?
            .with_context(|| format!("failed to reload {}", path.display()))?;
        if existing.locked
            && (existing.text_backend != selection.text_backend
                || existing.visual_backend != selection.visual_backend)
        {
            anyhow::bail!(
                "backend selection is locked for world_id={}: text={}, visual={}",
                existing.world_id,
                existing.text_backend.as_str(),
                existing.visual_backend.as_str()
            );
        }
    }
    fs::write(&path, serde_json::to_vec_pretty(selection)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::start::world_seed_from_compact_text;
    use crate::store::init_world_from_seed;

    #[test]
    fn backend_selection_is_locked_after_world_creation() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        let seed = world_seed_from_compact_text(
            "중세 판타지, 변경 순찰자 남주",
            Some("stw_backend_lock"),
            None,
        )?;
        init_world_from_seed(seed, Some(store.as_path()), None)?;

        let selection = WorldBackendSelection::new(
            "stw_backend_lock".to_owned(),
            WorldTextBackend::CodexAppServer,
            WorldVisualBackend::Webgpt,
            "test",
        );
        let path = save_world_backend_selection(Some(store.as_path()), &selection)?;
        assert!(path.ends_with("agent_bridge/backend_selection.json"));

        let Some(loaded) = load_world_backend_selection(Some(store.as_path()), "stw_backend_lock")?
        else {
            anyhow::bail!("selection should exist after save");
        };
        assert!(loaded.locked);
        assert_eq!(loaded.text_backend, WorldTextBackend::CodexAppServer);
        assert_eq!(loaded.visual_backend, WorldVisualBackend::Webgpt);

        let conflicting = WorldBackendSelection::new(
            "stw_backend_lock".to_owned(),
            WorldTextBackend::Webgpt,
            WorldVisualBackend::Webgpt,
            "test",
        );
        let Err(error) = save_world_backend_selection(Some(store.as_path()), &conflicting) else {
            anyhow::bail!("locked selection accepted backend changes");
        };
        assert!(error.to_string().contains("backend selection is locked"));
        Ok(())
    }
}
