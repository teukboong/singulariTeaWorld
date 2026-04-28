use crate::extra_memory::{
    failed_projection_records_after_latest_repair, load_extra_memory_projection_records,
    load_remembered_extras,
};
use crate::job_ledger::{ReadWorldJobsOptions, WorldJobKind, WorldJobStatus, read_world_jobs};
use crate::models::{CanonEvent, TurnSnapshot};
use crate::store::{read_json, resolve_store_paths, world_file_paths};
use crate::turn_commit::{TURN_COMMITS_FILENAME, TurnCommitEnvelope, TurnCommitStatus};
use crate::world_db::{validate_world_db, world_db_stats};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

pub const PROJECTION_HEALTH_SCHEMA_VERSION: &str = "singulari.projection_health.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionHealthStatus {
    Healthy,
    Degraded,
    Failed,
}

impl std::fmt::Display for ProjectionHealthStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Healthy => formatter.write_str("healthy"),
            Self::Degraded => formatter.write_str("degraded"),
            Self::Failed => formatter.write_str("failed"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionComponentHealth {
    pub component: String,
    pub status: ProjectionHealthStatus,
    pub detail: String,
    pub warnings: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionHealthReport {
    pub schema_version: String,
    pub world_id: String,
    pub status: ProjectionHealthStatus,
    pub components: Vec<ProjectionComponentHealth>,
}

/// Build a cross-projection health report for one world.
///
/// # Errors
///
/// Returns an error when the store root cannot be resolved. Component-level
/// parse or consistency failures are represented inside the report.
pub fn build_projection_health_report(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<ProjectionHealthReport> {
    let paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&paths, world_id);
    let world_dir = files.dir.clone();
    let mut components = Vec::new();

    if !world_dir.is_dir() {
        components.push(failed_component(
            "core_store",
            format!("world directory missing: {}", world_dir.display()),
        ));
        return Ok(report_from_components(world_id, components));
    }

    let snapshot = read_component_json::<TurnSnapshot>("latest_snapshot", &files.latest_snapshot);
    let canon_events = read_canon_events(&files.canon_events);
    components.push(core_store_health(
        world_id,
        &snapshot,
        &canon_events,
        &[
            &files.world,
            &files.hidden_state,
            &files.player_knowledge,
            &files.entities,
            &files.canon_events,
        ],
    ));
    components.push(world_db_health(world_id, &world_dir, &canon_events));
    components.push(turn_commit_health(
        world_id,
        &world_dir,
        snapshot.value.as_ref(),
    ));
    components.push(extra_memory_health(world_id, &world_dir));
    components.push(world_jobs_health(store_root, world_id));

    Ok(report_from_components(world_id, components))
}

#[must_use]
pub fn render_projection_health_report(report: &ProjectionHealthReport) -> String {
    let mut lines = vec![
        format!("world: {}", report.world_id),
        format!("status: {}", report.status),
    ];
    for component in &report.components {
        lines.push(format!(
            "{}: {} - {}",
            component.component, component.status, component.detail
        ));
        for warning in &component.warnings {
            lines.push(format!("  warning: {warning}"));
        }
        for error in &component.errors {
            lines.push(format!("  error: {error}"));
        }
    }
    lines.join("\n")
}

fn report_from_components(
    world_id: &str,
    components: Vec<ProjectionComponentHealth>,
) -> ProjectionHealthReport {
    let status = if components
        .iter()
        .any(|component| component.status == ProjectionHealthStatus::Failed)
    {
        ProjectionHealthStatus::Failed
    } else if components
        .iter()
        .any(|component| component.status == ProjectionHealthStatus::Degraded)
    {
        ProjectionHealthStatus::Degraded
    } else {
        ProjectionHealthStatus::Healthy
    };
    ProjectionHealthReport {
        schema_version: PROJECTION_HEALTH_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        status,
        components,
    }
}

fn core_store_health(
    world_id: &str,
    snapshot: &ComponentRead<TurnSnapshot>,
    canon_events: &ComponentRead<Vec<CanonEvent>>,
    required_paths: &[&PathBuf],
) -> ProjectionComponentHealth {
    let mut warnings = Vec::new();
    let mut errors = missing_required_errors(required_paths);
    errors.extend(snapshot.errors.clone());
    errors.extend(canon_events.errors.clone());
    if let Some(snapshot) = &snapshot.value
        && snapshot.world_id != world_id
    {
        errors.push(format!(
            "latest_snapshot world mismatch: expected={world_id}, actual={}",
            snapshot.world_id
        ));
    }
    if let Some(events) = &canon_events.value
        && events.is_empty()
    {
        warnings.push("canon_events.jsonl has no events".to_owned());
    }
    component_from_errors(
        "core_store",
        format!(
            "latest_turn={}, canon_events={}",
            snapshot
                .value
                .as_ref()
                .map_or("<unreadable>", |snapshot| snapshot.turn_id.as_str()),
            canon_events
                .value
                .as_ref()
                .map(Vec::len)
                .map_or_else(|| "<unreadable>".to_owned(), |count| count.to_string())
        ),
        warnings,
        errors,
    )
}

fn world_db_health(
    world_id: &str,
    world_dir: &Path,
    canon_events: &ComponentRead<Vec<CanonEvent>>,
) -> ProjectionComponentHealth {
    let expected_canon_events = canon_events.value.as_ref().map_or(0, Vec::len);
    match validate_world_db(world_dir, world_id, expected_canon_events).and_then(|validation| {
        let stats = world_db_stats(world_dir, world_id)?;
        Ok((validation, stats))
    }) {
        Ok((validation, stats)) => {
            let detail = format!(
                "canon_events={}, character_memories={}, world_facts={}, search_documents={}",
                stats.canon_events,
                stats.character_memories,
                stats.world_facts,
                stats.search_documents
            );
            component_from_errors("world_db", detail, validation.warnings, Vec::new())
        }
        Err(error) => failed_component("world_db", format!("{error:#}")),
    }
}

fn turn_commit_health(
    world_id: &str,
    world_dir: &Path,
    latest_snapshot: Option<&TurnSnapshot>,
) -> ProjectionComponentHealth {
    let ledger_path = world_dir.join(TURN_COMMITS_FILENAME);
    let latest_turn = latest_snapshot.map_or("<unknown>", |snapshot| snapshot.turn_id.as_str());
    if !ledger_path.exists() {
        let status = if latest_turn == "turn_0000" {
            ProjectionHealthStatus::Healthy
        } else {
            ProjectionHealthStatus::Failed
        };
        return ProjectionComponentHealth {
            component: "turn_commit".to_owned(),
            status,
            detail: format!("ledger absent, latest_turn={latest_turn}"),
            warnings: Vec::new(),
            errors: if status == ProjectionHealthStatus::Failed {
                vec![format!(
                    "turn commit ledger missing after initial turn: {}",
                    ledger_path.display()
                )]
            } else {
                Vec::new()
            },
        };
    }

    let envelopes = match read_turn_commit_envelopes(&ledger_path) {
        Ok(envelopes) => envelopes,
        Err(error) => return failed_component("turn_commit", format!("{error:#}")),
    };
    let prepared_count = envelopes
        .iter()
        .filter(|envelope| envelope.status == TurnCommitStatus::Prepared)
        .count();
    let committed_count = envelopes
        .iter()
        .filter(|envelope| envelope.status == TurnCommitStatus::Committed)
        .count();
    let mut errors = Vec::new();
    for envelope in &envelopes {
        if envelope.world_id != world_id {
            errors.push(format!(
                "turn commit world mismatch: expected={world_id}, actual={}, turn={}",
                envelope.world_id, envelope.turn_id
            ));
        }
    }
    if let Some(snapshot) = latest_snapshot {
        let latest_committed = envelopes
            .iter()
            .rev()
            .find(|envelope| envelope.status == TurnCommitStatus::Committed);
        if snapshot.turn_id != "turn_0000"
            && latest_committed.map(|envelope| envelope.turn_id.as_str())
                != Some(snapshot.turn_id.as_str())
        {
            errors.push(format!(
                "latest committed turn mismatch: latest_snapshot={}, latest_committed={}",
                snapshot.turn_id,
                latest_committed.map_or("<none>", |envelope| envelope.turn_id.as_str())
            ));
        }
    }
    component_from_errors(
        "turn_commit",
        format!("prepared={prepared_count}, committed={committed_count}"),
        Vec::new(),
        errors,
    )
}

fn extra_memory_health(world_id: &str, world_dir: &Path) -> ProjectionComponentHealth {
    let store = match load_remembered_extras(world_dir, world_id) {
        Ok(store) => store,
        Err(error) => return failed_component("extra_memory", format!("{error:#}")),
    };
    let records = match load_extra_memory_projection_records(world_dir) {
        Ok(records) => records,
        Err(error) => return failed_component("extra_memory", format!("{error:#}")),
    };
    let repair_start = records
        .iter()
        .rposition(|record| {
            record.status == crate::extra_memory::ExtraMemoryProjectionStatus::Repaired
        })
        .map_or(0, |index| index + 1);
    let failed_records = records[repair_start..]
        .iter()
        .filter(|record| record.status == crate::extra_memory::ExtraMemoryProjectionStatus::Failed)
        .collect::<Vec<_>>();
    let failed_count = failed_projection_records_after_latest_repair(&records);
    let errors = failed_records
        .iter()
        .map(|record| {
            format!(
                "extra memory projection failed: turn={}, error={}",
                record.turn_id,
                record.error.as_deref().unwrap_or("<missing>")
            )
        })
        .collect::<Vec<_>>();
    component_from_errors(
        "extra_memory",
        format!(
            "remembered_extras={}, projection_records={}, failed_projection_records={}",
            store.extras.len(),
            records.len(),
            failed_count
        ),
        Vec::new(),
        errors,
    )
}

fn world_jobs_health(store_root: Option<&Path>, world_id: &str) -> ProjectionComponentHealth {
    match read_world_jobs(&ReadWorldJobsOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id: world_id.to_owned(),
        extra_visual_jobs: Vec::new(),
    }) {
        Ok(jobs) => {
            let text_pending = jobs
                .iter()
                .filter(|job| {
                    job.kind == WorldJobKind::TextTurn && job.status == WorldJobStatus::Pending
                })
                .count();
            let visual_pending = jobs
                .iter()
                .filter(|job| {
                    matches!(
                        job.kind,
                        WorldJobKind::SceneCg
                            | WorldJobKind::ReferenceAsset
                            | WorldJobKind::UiAsset
                    ) && job.status == WorldJobStatus::Pending
                })
                .count();
            let claimed = jobs
                .iter()
                .filter(|job| job.status == WorldJobStatus::Claimed)
                .count();
            let completed = jobs
                .iter()
                .filter(|job| job.status == WorldJobStatus::Completed)
                .count();
            ProjectionComponentHealth {
                component: "world_jobs".to_owned(),
                status: ProjectionHealthStatus::Healthy,
                detail: format!(
                    "text_pending={text_pending}, visual_pending={visual_pending}, claimed={claimed}, completed={completed}"
                ),
                warnings: Vec::new(),
                errors: Vec::new(),
            }
        }
        Err(error) => failed_component("world_jobs", format!("{error:#}")),
    }
}

fn component_from_errors(
    component: &str,
    detail: String,
    warnings: Vec<String>,
    errors: Vec<String>,
) -> ProjectionComponentHealth {
    ProjectionComponentHealth {
        component: component.to_owned(),
        status: if errors.is_empty() {
            ProjectionHealthStatus::Healthy
        } else {
            ProjectionHealthStatus::Failed
        },
        detail,
        warnings,
        errors,
    }
}

fn failed_component(component: &str, detail: String) -> ProjectionComponentHealth {
    ProjectionComponentHealth {
        component: component.to_owned(),
        status: ProjectionHealthStatus::Failed,
        detail,
        warnings: Vec::new(),
        errors: Vec::new(),
    }
}

fn missing_required_errors(required_paths: &[&PathBuf]) -> Vec<String> {
    required_paths
        .iter()
        .filter(|path| !path.exists())
        .map(|path| format!("required projection file missing: {}", path.display()))
        .collect()
}

#[derive(Debug)]
struct ComponentRead<T> {
    value: Option<T>,
    errors: Vec<String>,
}

fn read_component_json<T>(label: &str, path: &Path) -> ComponentRead<T>
where
    T: serde::de::DeserializeOwned,
{
    match read_json(path) {
        Ok(value) => ComponentRead {
            value: Some(value),
            errors: Vec::new(),
        },
        Err(error) => ComponentRead {
            value: None,
            errors: vec![format!("{label} unreadable: {error:#}")],
        },
    }
}

fn read_canon_events(path: &Path) -> ComponentRead<Vec<CanonEvent>> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(error) => {
            return ComponentRead {
                value: None,
                errors: vec![format!("canon_events unreadable: {error:#}")],
            };
        }
    };
    let mut events = Vec::new();
    let mut errors = Vec::new();
    for (index, line) in raw.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<CanonEvent>(line) {
            Ok(event) => events.push(event),
            Err(error) => errors.push(format!("canon_events line {} invalid: {error}", index + 1)),
        }
    }
    ComponentRead {
        value: if errors.is_empty() {
            Some(events)
        } else {
            None
        },
        errors,
    }
}

fn read_turn_commit_envelopes(path: &Path) -> Result<Vec<TurnCommitEnvelope>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<TurnCommitEnvelope>(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::projection_health::{
        ProjectionHealthStatus, build_projection_health_report, render_projection_health_report,
    };
    use crate::store::{InitWorldOptions, init_world};
    use tempfile::tempdir;

    fn seed_body() -> &'static str {
        r#"
schema_version: singulari.world_seed.v1
world_id: stw_projection_health
title: "건강 상태 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#
    }

    #[test]
    fn projection_health_passes_initialized_world() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Healthy);
        assert!(render_projection_health_report(&report).contains("world_db: healthy"));
        Ok(())
    }

    #[test]
    fn projection_health_fails_corrupt_extra_memory_store() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        std::fs::write(initialized.world_dir.join("remembered_extras.json"), "{")?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Failed);
        assert!(report.components.iter().any(|component| {
            component.component == "extra_memory"
                && component.status == ProjectionHealthStatus::Failed
        }));
        Ok(())
    }
}
