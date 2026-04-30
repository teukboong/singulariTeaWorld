use crate::agent_bridge::{ADVISORY_WARNING_EVENT_SCHEMA_VERSION, AdvisoryWarningEvent};
use crate::event_ledger::{
    WorldEventLedgerChainStatus, replay_world_events, verify_world_event_ledger,
};
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
    components.push(event_ledger_replay_health(
        world_id,
        &files.canon_events,
        &canon_events,
        snapshot.value.as_ref(),
    ));
    components.push(world_db_health(world_id, &world_dir, &canon_events));
    components.push(turn_commit_health(
        world_id,
        &world_dir,
        snapshot.value.as_ref(),
    ));
    components.push(advisory_warning_health(world_id, &world_dir));
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

fn event_ledger_replay_health(
    world_id: &str,
    canon_events_path: &Path,
    canon_events: &ComponentRead<Vec<CanonEvent>>,
    latest_snapshot: Option<&TurnSnapshot>,
) -> ProjectionComponentHealth {
    let report = match verify_world_event_ledger(canon_events_path, world_id) {
        Ok(report) => report,
        Err(error) => return failed_component("event_ledger_replay", format!("{error:#}")),
    };
    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    match report.chain_status {
        WorldEventLedgerChainStatus::FullyChained => {}
        WorldEventLedgerChainStatus::LegacyUnchained => {
            warnings.push("event ledger is legacy unchained; replay is accepted but tamper evidence is unavailable".to_owned());
        }
        WorldEventLedgerChainStatus::PartiallyChained => {
            warnings.push("event ledger is partially chained; replay is accepted but older events lack tamper evidence".to_owned());
        }
    }
    if let (Some(snapshot), Some(events)) = (latest_snapshot, canon_events.value.as_ref())
        && snapshot.turn_id != "turn_0000"
        && !events.iter().any(|event| event.turn_id == snapshot.turn_id)
    {
        errors.push(format!(
            "event ledger cannot replay latest snapshot turn: latest_snapshot={}, canon_events missing matching turn_id",
            snapshot.turn_id
        ));
    }
    let replay_detail = if let Some(events) = canon_events.value.as_ref() {
        match replay_world_events(world_id, events) {
            Ok(replay_report) => {
                if replay_report.semantic_summary.legacy_only_event_count > 0 {
                    warnings.push(format!(
                        "event ledger contains {} legacy-only events without typed semantics; replay summary inferred where possible",
                        replay_report.semantic_summary.legacy_only_event_count
                    ));
                }
                if let Some(snapshot) = latest_snapshot {
                    if replay_report.replayed_turn_id.as_deref() != Some(snapshot.turn_id.as_str())
                    {
                        errors.push(format!(
                            "event replay turn mismatch: latest_snapshot={}, replayed_turn={}",
                            snapshot.turn_id,
                            replay_report
                                .replayed_turn_id
                                .as_deref()
                                .unwrap_or("<none>")
                        ));
                    }
                    if let Some(replay_location) = replay_report.current_location.as_deref()
                        && replay_location != snapshot.protagonist_state.location
                    {
                        errors.push(format!(
                            "event replay location mismatch: latest_snapshot={}, replayed_location={}",
                            snapshot.protagonist_state.location, replay_location
                        ));
                    }
                }
                format!(
                    ", replay_turn={}, replay_location={}, observed_entities={}, observed_locations={}, semantic_events={}, legacy_only={}, action_successes={}, action_failures={}, process_ticks={}",
                    replay_report
                        .replayed_turn_id
                        .as_deref()
                        .unwrap_or("<none>"),
                    replay_report
                        .current_location
                        .as_deref()
                        .unwrap_or("<none>"),
                    replay_report.observed_entity_refs.len(),
                    replay_report.observed_location_refs.len(),
                    replay_report.semantic_summary.semantic_event_count,
                    replay_report.semantic_summary.legacy_only_event_count,
                    replay_report.semantic_summary.action_successes,
                    replay_report.semantic_summary.action_failures,
                    replay_report.semantic_summary.process_ticks
                )
            }
            Err(error) => {
                errors.push(format!("event replay failed: {error:#}"));
                String::new()
            }
        }
    } else {
        String::new()
    };
    component_from_errors(
        "event_ledger_replay",
        format!(
            "canon_events={}, chain_status={:?}, last_event_hash={}{}",
            report.event_count,
            report.chain_status,
            report.last_event_hash.as_deref().unwrap_or("<none>"),
            replay_detail
        ),
        warnings,
        errors,
    )
}

fn turn_commit_health(
    world_id: &str,
    world_dir: &Path,
    latest_snapshot: Option<&TurnSnapshot>,
) -> ProjectionComponentHealth {
    let ledger_path = world_dir.join(TURN_COMMITS_FILENAME);
    let latest_turn = latest_snapshot.map_or("<unknown>", |snapshot| snapshot.turn_id.as_str());
    if !ledger_path.exists() {
        let agent_bridge_present = world_dir.join("agent_bridge").is_dir();
        let status = if latest_turn == "turn_0000" || !agent_bridge_present {
            ProjectionHealthStatus::Healthy
        } else {
            ProjectionHealthStatus::Failed
        };
        return ProjectionComponentHealth {
            component: "turn_commit".to_owned(),
            status,
            detail: format!(
                "ledger absent, latest_turn={latest_turn}, agent_bridge_present={agent_bridge_present}"
            ),
            warnings: Vec::new(),
            errors: if status == ProjectionHealthStatus::Failed {
                vec![format!(
                    "agent-authored turn commit ledger missing after initial turn: {}",
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
    let failed_count = envelopes
        .iter()
        .filter(|envelope| envelope.status == TurnCommitStatus::Failed)
        .count();
    let mut errors = Vec::new();
    for envelope in &envelopes {
        if envelope.world_id != world_id {
            errors.push(format!(
                "turn commit world mismatch: expected={world_id}, actual={}, turn={}",
                envelope.world_id, envelope.turn_id
            ));
        }
        match envelope.status {
            TurnCommitStatus::Committed => {
                errors.extend(missing_committed_materialization_errors(envelope));
            }
            TurnCommitStatus::Failed => {
                if !turn_has_committed_envelope(&envelopes, envelope.turn_id.as_str()) {
                    errors.push(format!(
                        "failed turn commit recorded: turn={}, stage={}, error={}",
                        envelope.turn_id,
                        envelope.failed_stage.as_deref().unwrap_or("<unknown>"),
                        envelope.error.as_deref().unwrap_or("<missing>")
                    ));
                }
            }
            TurnCommitStatus::Prepared => {}
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
        format!("prepared={prepared_count}, committed={committed_count}, failed={failed_count}"),
        Vec::new(),
        errors,
    )
}

fn missing_committed_materialization_errors(envelope: &TurnCommitEnvelope) -> Vec<String> {
    let mut errors = Vec::new();
    for (field, value) in [
        ("response_path", envelope.response_path.as_deref()),
        ("render_packet_path", envelope.render_packet_path.as_deref()),
        ("commit_record_path", envelope.commit_record_path.as_deref()),
    ] {
        match value {
            Some(path) if Path::new(path).is_file() => {}
            Some(path) => errors.push(format!(
                "committed turn materialization missing: turn={}, field={field}, path={path}",
                envelope.turn_id
            )),
            None => errors.push(format!(
                "committed turn materialization missing: turn={}, field={field}, path=<none>",
                envelope.turn_id
            )),
        }
    }
    if let Some(path) = envelope.world_court_verdict_path.as_deref()
        && !Path::new(path).is_file()
    {
        errors.push(format!(
            "committed turn optional audit artifact missing: turn={}, field=world_court_verdict_path, path={path}",
            envelope.turn_id
        ));
    }
    errors
}

fn turn_has_committed_envelope(envelopes: &[TurnCommitEnvelope], turn_id: &str) -> bool {
    envelopes.iter().any(|envelope| {
        envelope.turn_id == turn_id && envelope.status == TurnCommitStatus::Committed
    })
}

fn advisory_warning_health(world_id: &str, world_dir: &Path) -> ProjectionComponentHealth {
    let path = world_dir
        .join("agent_bridge")
        .join("advisory_warnings.jsonl");
    if !path.exists() {
        return ProjectionComponentHealth {
            component: "advisory_warnings".to_owned(),
            status: ProjectionHealthStatus::Healthy,
            detail: "warnings=0".to_owned(),
            warnings: Vec::new(),
            errors: Vec::new(),
        };
    }
    let events = match read_advisory_warning_events(&path) {
        Ok(events) => events,
        Err(error) => return failed_component("advisory_warnings", format!("{error:#}")),
    };
    let mut errors = Vec::new();
    for event in &events {
        if event.schema_version != ADVISORY_WARNING_EVENT_SCHEMA_VERSION {
            errors.push(format!(
                "advisory warning schema mismatch: expected={}, actual={}, turn={}",
                ADVISORY_WARNING_EVENT_SCHEMA_VERSION, event.schema_version, event.turn_id
            ));
        }
        if event.world_id != world_id {
            errors.push(format!(
                "advisory warning world mismatch: expected={world_id}, actual={}, turn={}",
                event.world_id, event.turn_id
            ));
        }
    }
    if !errors.is_empty() {
        return component_from_errors(
            "advisory_warnings",
            format!("warnings={}", events.len()),
            Vec::new(),
            errors,
        );
    }
    let Some(latest) = events.last() else {
        return ProjectionComponentHealth {
            component: "advisory_warnings".to_owned(),
            status: ProjectionHealthStatus::Healthy,
            detail: "warnings=0".to_owned(),
            warnings: Vec::new(),
            errors: Vec::new(),
        };
    };
    ProjectionComponentHealth {
        component: "advisory_warnings".to_owned(),
        status: ProjectionHealthStatus::Degraded,
        detail: format!(
            "warnings={}, latest_component={}, latest_kind={}",
            events.len(),
            latest.component,
            latest.warning_kind
        ),
        warnings: vec![format!(
            "latest advisory warning: turn={}, component={}, kind={}, message={}",
            latest.turn_id, latest.component, latest.warning_kind, latest.message
        )],
        errors: Vec::new(),
    }
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
            let failed_terminal = jobs
                .iter()
                .filter(|job| job.status == WorldJobStatus::FailedTerminal)
                .collect::<Vec<_>>();
            let failed_retryable = jobs
                .iter()
                .filter(|job| job.status == WorldJobStatus::FailedRetryable)
                .count();
            let warnings = failed_terminal
                .iter()
                .map(|job| {
                    format!(
                        "terminal world job failure: kind={:?}, slot={}, error={}",
                        job.kind,
                        job.slot,
                        job.last_error.as_deref().unwrap_or("<missing>")
                    )
                })
                .collect::<Vec<_>>();
            ProjectionComponentHealth {
                component: "world_jobs".to_owned(),
                status: if failed_terminal.is_empty() {
                    ProjectionHealthStatus::Healthy
                } else {
                    ProjectionHealthStatus::Degraded
                },
                detail: format!(
                    "text_pending={text_pending}, visual_pending={visual_pending}, claimed={claimed}, completed={completed}, failed_retryable={failed_retryable}, failed_terminal={}",
                    failed_terminal.len()
                ),
                warnings,
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
    match crate::event_ledger::load_world_events(path) {
        Ok(events) => ComponentRead {
            value: Some(events),
            errors: Vec::new(),
        },
        Err(error) => ComponentRead {
            value: None,
            errors: vec![format!("canon_events unreadable: {error:#}")],
        },
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

fn read_advisory_warning_events(path: &Path) -> Result<Vec<AdvisoryWarningEvent>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<AdvisoryWarningEvent>(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::agent_bridge::{ADVISORY_WARNING_EVENT_SCHEMA_VERSION, AdvisoryWarningEvent};
    use crate::agent_bridge::{AgentSubmitTurnOptions, enqueue_agent_turn};
    use crate::job_ledger::{WorldJobStatus, WriteTextTurnJobOptions, write_text_turn_job};
    use crate::projection_health::{
        ProjectionHealthStatus, build_projection_health_report, render_projection_health_report,
    };
    use crate::store::{InitWorldOptions, append_jsonl, init_world, read_json, write_json};
    use crate::turn::{AdvanceTurnOptions, advance_turn};
    use crate::turn_commit::{
        TURN_COMMIT_ENVELOPE_SCHEMA_VERSION, TurnCommitEnvelope, TurnCommitStatus,
    };
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
        assert!(render_projection_health_report(&report).contains("event_ledger_replay: healthy"));
        Ok(())
    }

    #[test]
    fn projection_health_fails_snapshot_without_replayable_event() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let mut snapshot: crate::models::TurnSnapshot = read_json(&initialized.snapshot_path)?;
        snapshot.turn_id = "turn_9999".to_owned();
        write_json(
            &initialized.world_dir.join("latest_snapshot.json"),
            &snapshot,
        )?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Failed);
        assert!(report.components.iter().any(|component| {
            component.component == "event_ledger_replay"
                && component.status == ProjectionHealthStatus::Failed
                && component
                    .errors
                    .iter()
                    .any(|error| error.contains("cannot replay latest snapshot turn"))
        }));
        Ok(())
    }

    #[test]
    fn projection_health_accepts_plain_cli_turn_without_agent_commit_ledger() -> anyhow::Result<()>
    {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_projection_health".to_owned(),
            input: "1".to_owned(),
        })?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Healthy);
        assert!(report.components.iter().any(|component| {
            component.component == "turn_commit"
                && component.status == ProjectionHealthStatus::Healthy
                && component.detail.contains("agent_bridge_present=false")
        }));
        Ok(())
    }

    #[test]
    fn projection_health_fails_agent_bridge_world_without_turn_commit_ledger() -> anyhow::Result<()>
    {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_projection_health".to_owned(),
            input: "1".to_owned(),
        })?;
        std::fs::create_dir_all(initialized.world_dir.join("agent_bridge"))?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Failed);
        assert!(report.components.iter().any(|component| {
            component.component == "turn_commit"
                && component
                    .errors
                    .iter()
                    .any(|error| error.contains("agent-authored turn commit ledger missing"))
        }));
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

    #[test]
    fn projection_health_fails_committed_turn_with_missing_materialization() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        append_jsonl(
            &initialized.world_dir.join("turn_commits.jsonl"),
            &TurnCommitEnvelope {
                schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
                world_id: "stw_projection_health".to_owned(),
                turn_id: "turn_0000".to_owned(),
                parent_turn_id: "turn_0000".to_owned(),
                player_input: "test".to_owned(),
                status: TurnCommitStatus::Committed,
                pending_ref: "agent_bridge/pending_turn.json".to_owned(),
                response_path: Some(
                    initialized
                        .world_dir
                        .join("missing-agent-response.json")
                        .display()
                        .to_string(),
                ),
                render_packet_path: None,
                commit_record_path: None,
                world_court_verdict_path: None,
                failed_stage: None,
                error: None,
                created_at: "2026-04-29T00:00:00Z".to_owned(),
            },
        )?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Failed);
        assert!(report.components.iter().any(|component| {
            component.component == "turn_commit"
                && component.errors.iter().any(|error| {
                    error.contains("committed turn materialization missing")
                        && error.contains("missing-agent-response")
                })
        }));
        Ok(())
    }

    #[test]
    fn projection_health_fails_recorded_failed_turn_commit() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        append_jsonl(
            &initialized.world_dir.join("turn_commits.jsonl"),
            &TurnCommitEnvelope {
                schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
                world_id: "stw_projection_health".to_owned(),
                turn_id: "turn_0001".to_owned(),
                parent_turn_id: "turn_0000".to_owned(),
                player_input: "1".to_owned(),
                status: TurnCommitStatus::Failed,
                pending_ref: "agent_bridge/pending_turn.json".to_owned(),
                response_path: None,
                render_packet_path: None,
                commit_record_path: None,
                world_court_verdict_path: None,
                failed_stage: Some("agent_commit_after_prepare".to_owned()),
                error: Some("simulated materialization failure".to_owned()),
                created_at: "2026-04-29T00:00:00Z".to_owned(),
            },
        )?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Failed);
        assert!(report.components.iter().any(|component| {
            component.component == "turn_commit"
                && component
                    .detail
                    .contains("prepared=0, committed=0, failed=1")
                && component.errors.iter().any(|error| {
                    error.contains("failed turn commit recorded")
                        && error.contains("agent_commit_after_prepare")
                })
        }));
        Ok(())
    }

    #[test]
    fn projection_health_ignores_failed_commit_after_committed_recovery() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let response_path = initialized.world_dir.join("agent-response.json");
        let render_packet_path = initialized.world_dir.join("render-packet.json");
        let commit_record_path = initialized.world_dir.join("commit-record.json");
        std::fs::write(&response_path, "{}\n")?;
        std::fs::write(&render_packet_path, "{}\n")?;
        std::fs::write(&commit_record_path, "{}\n")?;
        append_jsonl(
            &initialized.world_dir.join("turn_commits.jsonl"),
            &TurnCommitEnvelope {
                schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
                world_id: "stw_projection_health".to_owned(),
                turn_id: "turn_0001".to_owned(),
                parent_turn_id: "turn_0000".to_owned(),
                player_input: "1".to_owned(),
                status: TurnCommitStatus::Failed,
                pending_ref: "agent_bridge/pending_turn.json".to_owned(),
                response_path: None,
                render_packet_path: None,
                commit_record_path: None,
                world_court_verdict_path: None,
                failed_stage: Some("agent_commit_after_prepare".to_owned()),
                error: Some("simulated materialization failure".to_owned()),
                created_at: "2026-04-29T00:00:00Z".to_owned(),
            },
        )?;
        append_jsonl(
            &initialized.world_dir.join("turn_commits.jsonl"),
            &TurnCommitEnvelope {
                schema_version: TURN_COMMIT_ENVELOPE_SCHEMA_VERSION.to_owned(),
                world_id: "stw_projection_health".to_owned(),
                turn_id: "turn_0001".to_owned(),
                parent_turn_id: "turn_0000".to_owned(),
                player_input: "1".to_owned(),
                status: TurnCommitStatus::Committed,
                pending_ref: "agent_bridge/pending_turn.json".to_owned(),
                response_path: Some(response_path.display().to_string()),
                render_packet_path: Some(render_packet_path.display().to_string()),
                commit_record_path: Some(commit_record_path.display().to_string()),
                world_court_verdict_path: None,
                failed_stage: None,
                error: None,
                created_at: "2026-04-29T00:00:01Z".to_owned(),
            },
        )?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        let Some(turn_commit) = report
            .components
            .iter()
            .find(|component| component.component == "turn_commit")
        else {
            anyhow::bail!("turn_commit health component missing");
        };
        assert_eq!(turn_commit.status, ProjectionHealthStatus::Healthy);
        assert!(turn_commit.detail.contains("committed=1"));
        assert!(turn_commit.detail.contains("failed=1"));
        assert!(turn_commit.errors.is_empty());
        Ok(())
    }

    #[test]
    fn projection_health_surfaces_advisory_warning_latest() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let warning_path = initialized
            .world_dir
            .join("agent_bridge")
            .join("advisory_warnings.jsonl");
        let Some(warning_dir) = warning_path.parent() else {
            anyhow::bail!("advisory warning path has no parent");
        };
        std::fs::create_dir_all(warning_dir)?;
        append_jsonl(
            &warning_path,
            &AdvisoryWarningEvent {
                schema_version: ADVISORY_WARNING_EVENT_SCHEMA_VERSION.to_owned(),
                world_id: "stw_projection_health".to_owned(),
                turn_id: "turn_0001".to_owned(),
                component: "hook_ledger_promises".to_owned(),
                warning_kind: "advisory_journal_append_failed".to_owned(),
                message: "simulated append loss".to_owned(),
                recorded_at: "2026-04-30T00:00:00Z".to_owned(),
            },
        )?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Degraded);
        let rendered = render_projection_health_report(&report);
        assert!(rendered.contains("advisory_warnings: degraded"));
        assert!(rendered.contains("warnings=1"));
        assert!(rendered.contains("latest_component=hook_ledger_promises"));
        assert!(rendered.contains("latest_kind=advisory_journal_append_failed"));
        assert!(rendered.contains("simulated append loss"));
        Ok(())
    }

    #[test]
    fn projection_health_degrades_on_terminal_text_job_failure() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_projection_health".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        write_text_turn_job(&WriteTextTurnJobOptions {
            store_root: Some(store.as_path()),
            pending: &pending,
            status: WorldJobStatus::FailedTerminal,
            output_ref: Some("agent_bridge/dispatches/turn_0001-webgpt.json".to_owned()),
            claim_owner: Some("webgpt_host_worker".to_owned()),
            attempt_id: Some("webgpt:turn_0001".to_owned()),
            last_error: Some("unknown variant hidden".to_owned()),
        })?;

        let report = build_projection_health_report(Some(&store), "stw_projection_health")?;

        assert_eq!(report.status, ProjectionHealthStatus::Degraded);
        let Some(world_jobs) = report
            .components
            .iter()
            .find(|component| component.component == "world_jobs")
        else {
            anyhow::bail!("world_jobs health component missing");
        };
        assert_eq!(world_jobs.status, ProjectionHealthStatus::Degraded);
        assert!(world_jobs.detail.contains("failed_terminal=1"));
        assert!(world_jobs.warnings.iter().any(|warning| {
            warning.contains("terminal world job failure")
                && warning.contains("turn_0001")
                && warning.contains("unknown variant hidden")
        }));
        Ok(())
    }
}
