use crate::models::WorldRecord;
use crate::store::{
    ActiveWorldBinding, WORLD_FILENAME, read_json, resolve_store_paths, save_active_world,
};
use crate::validate::{ValidationStatus, validate_world};
use crate::world_db::{WorldDbRepairReport, repair_world_db};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const EXPORT_MANIFEST_SCHEMA_VERSION: &str = "singulari.export_manifest.v1";
const EXPORT_MANIFEST_FILENAME: &str = "singulari_export_manifest.json";

#[derive(Debug, Clone)]
pub struct ExportWorldOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub output: PathBuf,
}

#[derive(Debug, Clone)]
pub struct ImportWorldOptions {
    pub store_root: Option<PathBuf>,
    pub bundle: PathBuf,
    pub activate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportManifest {
    pub schema_version: String,
    pub world_id: String,
    pub title: String,
    pub exported_at: String,
    pub files_copied: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportWorldReport {
    pub world_id: String,
    pub bundle_dir: PathBuf,
    pub manifest_path: PathBuf,
    pub files_copied: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportWorldReport {
    pub world_id: String,
    pub world_dir: PathBuf,
    pub active_binding: Option<ActiveWorldBinding>,
    pub repair_report: WorldDbRepairReport,
}

/// Export a world as a filesystem bundle directory.
///
/// # Errors
///
/// Returns an error when the source world is missing, the output already
/// exists, or the bundle cannot be copied.
pub fn export_world(options: &ExportWorldOptions) -> Result<ExportWorldReport> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let source_dir = store_paths.worlds_dir.join(options.world_id.as_str());
    if !source_dir.is_dir() {
        bail!("export-world source missing: {}", source_dir.display());
    }
    if options.output.exists() {
        bail!(
            "export-world refused to overwrite existing output: {}",
            options.output.display()
        );
    }
    if let Some(parent) = options.output.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::create_dir_all(&options.output)
        .with_context(|| format!("failed to create {}", options.output.display()))?;
    let files_copied = copy_dir_contents(&source_dir, &options.output)?;
    let world: WorldRecord = read_json(&source_dir.join(WORLD_FILENAME))?;
    let manifest = ExportManifest {
        schema_version: EXPORT_MANIFEST_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id.clone(),
        title: world.title,
        exported_at: Utc::now().to_rfc3339(),
        files_copied,
    };
    let manifest_path = options.output.join(EXPORT_MANIFEST_FILENAME);
    fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest)?),
    )
    .with_context(|| format!("failed to write {}", manifest_path.display()))?;
    Ok(ExportWorldReport {
        world_id: world.world_id,
        bundle_dir: options.output.clone(),
        manifest_path,
        files_copied,
    })
}

/// Import a filesystem bundle directory into the local world store.
///
/// # Errors
///
/// Returns an error when the bundle is malformed, the destination exists, or
/// repair/validation fails after copy.
pub fn import_world(options: &ImportWorldOptions) -> Result<ImportWorldReport> {
    let bundle_root = resolve_bundle_root(options.bundle.as_path())?;
    let world: WorldRecord = read_json(&bundle_root.join(WORLD_FILENAME))?;
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let world_dir = store_paths.worlds_dir.join(world.world_id.as_str());
    if world_dir.exists() {
        bail!(
            "import-world refused to overwrite existing world: {}",
            world_dir.display()
        );
    }
    fs::create_dir_all(&store_paths.worlds_dir)
        .with_context(|| format!("failed to create {}", store_paths.worlds_dir.display()))?;
    fs::create_dir_all(&world_dir)
        .with_context(|| format!("failed to create {}", world_dir.display()))?;
    copy_dir_contents(&bundle_root, &world_dir)?;
    let repair_report = repair_world_db(&world_dir, world.world_id.as_str())?;
    let validation = validate_world(options.store_root.as_deref(), world.world_id.as_str())?;
    if validation.status == ValidationStatus::Failed {
        bail!(
            "import-world validation failed for {}: {}",
            world.world_id,
            validation.errors.join("; ")
        );
    }
    let active_binding = if options.activate {
        let snapshot: crate::models::TurnSnapshot =
            read_json(&world_dir.join(crate::store::LATEST_SNAPSHOT_FILENAME))?;
        Some(save_active_world(
            options.store_root.as_deref(),
            world.world_id.as_str(),
            snapshot.session_id.as_str(),
        )?)
    } else {
        None
    };
    Ok(ImportWorldReport {
        world_id: world.world_id,
        world_dir,
        active_binding,
        repair_report,
    })
}

fn resolve_bundle_root(bundle: &Path) -> Result<PathBuf> {
    if bundle.join(WORLD_FILENAME).is_file() {
        return Ok(bundle.to_path_buf());
    }
    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(bundle).with_context(|| format!("failed to read {}", bundle.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", bundle.display()))?;
        if entry.path().join(WORLD_FILENAME).is_file() {
            candidates.push(entry.path());
        }
    }
    match candidates.len() {
        1 => Ok(candidates.remove(0)),
        0 => bail!(
            "import-world bundle missing {WORLD_FILENAME}: {}",
            bundle.display()
        ),
        _ => bail!(
            "import-world bundle is ambiguous; multiple child world roots under {}",
            bundle.display()
        ),
    }
}

fn copy_dir_contents(source: &Path, destination: &Path) -> Result<usize> {
    let mut files_copied = 0;
    for entry in
        fs::read_dir(source).with_context(|| format!("failed to read {}", source.display()))?
    {
        let entry = entry.with_context(|| format!("failed to read {}", source.display()))?;
        let source_path = entry.path();
        let file_name = entry.file_name();
        if should_skip_export_file(&file_name.to_string_lossy()) {
            continue;
        }
        let destination_path = destination.join(file_name);
        copy_path(&source_path, &destination_path, &mut files_copied)?;
    }
    Ok(files_copied)
}

fn copy_path(source: &Path, destination: &Path, files_copied: &mut usize) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to stat {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        bail!("bundle copy rejected symlink: {}", source.display());
    }
    if metadata.is_dir() {
        fs::create_dir_all(destination)
            .with_context(|| format!("failed to create {}", destination.display()))?;
        *files_copied += copy_dir_contents(source, destination)?;
        return Ok(());
    }
    if metadata.is_file() {
        fs::copy(source, destination).with_context(|| {
            format!(
                "failed to copy {} -> {}",
                source.display(),
                destination.display()
            )
        })?;
        *files_copied += 1;
        return Ok(());
    }
    bail!(
        "bundle copy rejected unsupported file type: {}",
        source.display()
    )
}

fn should_skip_export_file(file_name: &str) -> bool {
    matches!(
        file_name,
        "world.db-wal" | "world.db-shm" | EXPORT_MANIFEST_FILENAME
    )
}

#[cfg(test)]
mod tests {
    use super::{ExportWorldOptions, ImportWorldOptions, export_world, import_world};
    use crate::store::{InitWorldOptions, init_world, load_active_world};
    use tempfile::tempdir;

    #[test]
    fn export_import_round_trips_world_bundle() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let source_store = temp.path().join("source-store");
        let target_store = temp.path().join("target-store");
        let bundle = temp.path().join("bundle");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_transfer
title: "전송 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(source_store.clone()),
            session_id: Some("session_transfer".to_owned()),
        })?;
        let exported = export_world(&ExportWorldOptions {
            store_root: Some(source_store),
            world_id: "stw_transfer".to_owned(),
            output: bundle.clone(),
        })?;
        assert!(exported.manifest_path.is_file());
        let imported = import_world(&ImportWorldOptions {
            store_root: Some(target_store.clone()),
            bundle,
            activate: true,
        })?;
        assert_eq!(imported.world_id, "stw_transfer");
        let active = load_active_world(Some(target_store.as_path()))?;
        assert_eq!(active.world_id, "stw_transfer");
        Ok(())
    }
}
