use crate::models::{CANON_EVENT_SCHEMA_VERSION, CanonEvent};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

pub const WORLD_EVENT_LEDGER_SCHEMA_VERSION: &str = "singulari.world_event_ledger.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEventLedgerAppendReport {
    pub schema_version: String,
    pub world_id: String,
    pub event_id: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEventLedgerVerificationReport {
    pub schema_version: String,
    pub world_id: String,
    pub path: String,
    pub event_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
}

pub(crate) fn append_world_event(
    path: &Path,
    event: &CanonEvent,
) -> Result<WorldEventLedgerAppendReport> {
    validate_event_shape(event, Some(event.world_id.as_str()))?;
    if path.is_file() {
        let existing_events = load_world_events(path)?;
        ensure_event_can_append(path, &existing_events, event)?;
    }
    let parent = path
        .parent()
        .with_context(|| format!("world event ledger path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let body = serde_json::to_string(event).context("failed to serialize world event")?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open world event ledger {}", path.display()))?;
    writeln!(file, "{body}")
        .with_context(|| format!("failed to append world event ledger {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("failed to sync world event ledger {}", path.display()))?;
    Ok(WorldEventLedgerAppendReport {
        schema_version: WORLD_EVENT_LEDGER_SCHEMA_VERSION.to_owned(),
        world_id: event.world_id.clone(),
        event_id: event.event_id.clone(),
        path: path.display().to_string(),
    })
}

pub(crate) fn load_world_events(path: &Path) -> Result<Vec<CanonEvent>> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str::<CanonEvent>(line)
                .with_context(|| format!("failed to parse {} line {}", path.display(), index + 1))
        })
        .collect()
}

/// Verify that `canon_events.jsonl` is a single-world append-only event ledger.
///
/// # Errors
///
/// Returns an error when the ledger cannot be read, any line is malformed, an
/// event has the wrong schema or world id, or event ids are duplicated.
pub fn verify_world_event_ledger(
    path: &Path,
    expected_world_id: &str,
) -> Result<WorldEventLedgerVerificationReport> {
    let events = load_world_events(path)?;
    verify_world_events(expected_world_id, &events)?;
    Ok(WorldEventLedgerVerificationReport {
        schema_version: WORLD_EVENT_LEDGER_SCHEMA_VERSION.to_owned(),
        world_id: expected_world_id.to_owned(),
        path: path.display().to_string(),
        event_count: events.len(),
        first_event_id: events.first().map(|event| event.event_id.clone()),
        last_event_id: events.last().map(|event| event.event_id.clone()),
    })
}

pub(crate) fn verify_world_events(expected_world_id: &str, events: &[CanonEvent]) -> Result<()> {
    let mut seen_event_ids = BTreeSet::new();
    for event in events {
        validate_event_shape(event, Some(expected_world_id))?;
        if !seen_event_ids.insert(event.event_id.clone()) {
            bail!(
                "world event ledger duplicate event_id: world_id={}, event_id={}",
                expected_world_id,
                event.event_id
            );
        }
    }
    Ok(())
}

fn ensure_event_can_append(
    path: &Path,
    existing_events: &[CanonEvent],
    event: &CanonEvent,
) -> Result<()> {
    if let Some(first_event) = existing_events.first()
        && first_event.world_id != event.world_id
    {
        bail!(
            "world event ledger append world_id mismatch: path={}, expected={}, actual={}",
            path.display(),
            first_event.world_id,
            event.world_id
        );
    }
    if existing_events
        .iter()
        .any(|existing| existing.event_id == event.event_id)
    {
        bail!(
            "world event ledger append duplicate event_id: path={}, event_id={}",
            path.display(),
            event.event_id
        );
    }
    verify_world_events(event.world_id.as_str(), existing_events).with_context(|| {
        format!(
            "world event ledger existing entries invalid: {}",
            path.display()
        )
    })
}

fn validate_event_shape(event: &CanonEvent, expected_world_id: Option<&str>) -> Result<()> {
    if event.schema_version != CANON_EVENT_SCHEMA_VERSION {
        bail!(
            "world event schema_version mismatch: event_id={}, expected={}, actual={}",
            event.event_id,
            CANON_EVENT_SCHEMA_VERSION,
            event.schema_version
        );
    }
    if event.event_id.trim().is_empty() {
        bail!("world event ledger event_id must not be empty");
    }
    if event.world_id.trim().is_empty() {
        bail!(
            "world event ledger world_id must not be empty: event_id={}",
            event.event_id
        );
    }
    if let Some(expected_world_id) = expected_world_id
        && event.world_id != expected_world_id
    {
        bail!(
            "world event ledger world_id mismatch: event_id={}, expected={}, actual={}",
            event.event_id,
            expected_world_id,
            event.world_id
        );
    }
    if event.turn_id.trim().is_empty() {
        bail!(
            "world event ledger turn_id must not be empty: event_id={}",
            event.event_id
        );
    }
    if event.kind.trim().is_empty() {
        bail!(
            "world event ledger kind must not be empty: event_id={}",
            event.event_id
        );
    }
    if let Some(event_kind) = event.event_kind
        && !event_kind.matches_legacy_kind(event.kind.as_str())
    {
        bail!(
            "world event ledger kind mismatch: event_id={}, kind={}, event_kind={:?}, expected_kind={}",
            event.event_id,
            event.kind,
            event_kind,
            event_kind.as_legacy_kind()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        append_world_event, load_world_events, verify_world_event_ledger, verify_world_events,
    };
    use crate::models::{
        CANON_EVENT_SCHEMA_VERSION, CanonEvent, EventAuthority, EventEvidence, WorldEventKind,
    };
    use tempfile::tempdir;

    #[test]
    fn appends_and_verifies_world_event_ledger() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("canon_events.jsonl");
        append_world_event(&path, &event("evt_000000", "turn_0000", "world:created"))?;
        append_world_event(&path, &event("evt_000001", "turn_0001", "player:acted"))?;

        let events = load_world_events(&path)?;
        assert_eq!(events.len(), 2);
        let report = verify_world_event_ledger(&path, "stw_event_ledger")?;
        assert_eq!(report.event_count, 2);
        assert_eq!(report.first_event_id.as_deref(), Some("evt_000000"));
        assert_eq!(report.last_event_id.as_deref(), Some("evt_000001"));
        Ok(())
    }

    #[test]
    fn rejects_duplicate_event_ids() {
        let events = vec![
            event("evt_000001", "turn_0001", "first"),
            event("evt_000001", "turn_0002", "duplicate"),
        ];
        let Err(error) = verify_world_events("stw_event_ledger", &events) else {
            panic!("duplicate event ids should be rejected");
        };
        assert!(format!("{error:#}").contains("duplicate event_id"));
    }

    #[test]
    fn rejects_cross_world_append() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("canon_events.jsonl");
        append_world_event(&path, &event("evt_000000", "turn_0000", "world:created"))?;
        let mut other_world = event("evt_000001", "turn_0001", "wrong world");
        other_world.world_id = "stw_other_world".to_owned();

        let Err(error) = append_world_event(&path, &other_world) else {
            panic!("cross-world append should be rejected");
        };
        assert!(format!("{error:#}").contains("world_id mismatch"));
        Ok(())
    }

    #[test]
    fn rejects_mismatched_event_kind_sidecar() {
        let mut event = event("evt_000001", "turn_0001", "numeric_choice");
        event.event_kind = Some(WorldEventKind::FreeformAction);

        let Err(error) = verify_world_events("stw_event_ledger", &[event]) else {
            panic!("mismatched event kind sidecar should be rejected");
        };
        assert!(format!("{error:#}").contains("kind mismatch"));
    }

    fn event(event_id: &str, turn_id: &str, kind: &str) -> CanonEvent {
        CanonEvent {
            schema_version: CANON_EVENT_SCHEMA_VERSION.to_owned(),
            event_id: event_id.to_owned(),
            world_id: "stw_event_ledger".to_owned(),
            turn_id: turn_id.to_owned(),
            occurred_at_world_time: "test-time".to_owned(),
            visibility: "player_visible".to_owned(),
            kind: kind.to_owned(),
            event_kind: WorldEventKind::from_legacy_kind(kind),
            authority: Some(EventAuthority::TurnReducer),
            summary: format!("event {event_id}"),
            entities: Vec::new(),
            location: None,
            evidence: EventEvidence {
                source: "test".to_owned(),
                user_input: "test input".to_owned(),
                narrative_ref: turn_id.to_owned(),
            },
            consequences: Vec::new(),
        }
    }
}
