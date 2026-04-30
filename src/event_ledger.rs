use crate::models::{CANON_EVENT_SCHEMA_VERSION, CanonEvent, WorldEventKind};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::Path;

pub const WORLD_EVENT_LEDGER_SCHEMA_VERSION: &str = "singulari.world_event_ledger.v1";
pub const WORLD_EVENT_HASH_VERSION: &str = "singulari.world_event_hash.v1";
pub const WORLD_EVENT_HASH_ALGORITHM: &str = "sha256";
pub const WORLD_EVENT_SEMANTIC_REPLAY_SCHEMA_VERSION: &str =
    "singulari.world_event_semantic_replay.v1";
pub const WORLD_EVENT_REPLAY_SCHEMA_VERSION: &str = "singulari.world_event_replay.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEventLedgerAppendReport {
    pub schema_version: String,
    pub world_id: String,
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_event_hash: Option<String>,
    pub event_hash: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEventLedgerVerificationReport {
    pub schema_version: String,
    pub world_id: String,
    pub path: String,
    pub event_count: usize,
    pub chain_status: WorldEventLedgerChainStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_hash: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldEventLedgerChainStatus {
    LegacyUnchained,
    PartiallyChained,
    FullyChained,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEventLedgerChainReport {
    pub schema_version: String,
    pub world_id: String,
    pub event_count: usize,
    pub chain_status: WorldEventLedgerChainStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_hashed_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEventSemanticReplayReport {
    pub schema_version: String,
    pub world_id: String,
    pub event_count: usize,
    pub semantic_event_count: usize,
    pub legacy_only_event_count: usize,
    pub action_attempts: usize,
    pub action_successes: usize,
    pub action_failures: usize,
    pub process_ticks: usize,
    pub knowledge_events: usize,
    pub relationship_events: usize,
    pub body_resource_events: usize,
    pub location_events: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEventReplayReport {
    pub schema_version: String,
    pub world_id: String,
    pub event_count: usize,
    pub semantic_summary: WorldEventSemanticReplayReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replayed_turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_location: Option<String>,
    #[serde(default)]
    pub observed_entity_refs: Vec<String>,
    #[serde(default)]
    pub observed_location_refs: Vec<String>,
    #[serde(default)]
    pub body_resource_refs: Vec<String>,
    #[serde(default)]
    pub relationship_refs: Vec<String>,
    #[serde(default)]
    pub open_consequence_refs: Vec<String>,
}

pub(crate) fn append_world_event(
    path: &Path,
    event: &CanonEvent,
) -> Result<WorldEventLedgerAppendReport> {
    validate_event_shape(event, Some(event.world_id.as_str()))?;
    let existing_events = if path.is_file() {
        let existing_events = load_world_events(path)?;
        ensure_event_can_append(path, &existing_events, event)?;
        existing_events
    } else {
        Vec::new()
    };
    let event_to_append = chain_event_for_append(&existing_events, event)?;
    let parent = path
        .parent()
        .with_context(|| format!("world event ledger path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    let body =
        serde_json::to_string(&event_to_append).context("failed to serialize world event")?;
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
        world_id: event_to_append.world_id.clone(),
        event_id: event_to_append.event_id.clone(),
        previous_event_hash: event_to_append.previous_event_hash.clone(),
        event_hash: event_to_append
            .event_hash
            .clone()
            .context("chained world event missing event_hash after append")?,
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
    let chain_report = verify_world_events(expected_world_id, &events)?;
    Ok(WorldEventLedgerVerificationReport {
        schema_version: WORLD_EVENT_LEDGER_SCHEMA_VERSION.to_owned(),
        world_id: expected_world_id.to_owned(),
        path: path.display().to_string(),
        event_count: events.len(),
        chain_status: chain_report.chain_status,
        first_event_id: events.first().map(|event| event.event_id.clone()),
        last_event_id: events.last().map(|event| event.event_id.clone()),
        last_event_hash: chain_report.last_event_hash,
    })
}

pub(crate) fn verify_world_events(
    expected_world_id: &str,
    events: &[CanonEvent],
) -> Result<WorldEventLedgerChainReport> {
    let mut seen_event_ids = BTreeSet::new();
    let mut first_hashed_event_id = None;
    let mut last_event_hash = None;
    let mut unchained_count = 0usize;
    for event in events {
        validate_event_shape(event, Some(expected_world_id))?;
        if !seen_event_ids.insert(event.event_id.clone()) {
            bail!(
                "world event ledger duplicate event_id: world_id={}, event_id={}",
                expected_world_id,
                event.event_id
            );
        }
        match verify_event_chain_link(event, last_event_hash.as_deref())? {
            EventChainLink::LegacyUnchained => {
                if last_event_hash.is_some() {
                    bail!(
                        "world event ledger unchained event after hashed chain start: world_id={}, event_id={}",
                        expected_world_id,
                        event.event_id
                    );
                }
                unchained_count += 1;
            }
            EventChainLink::Chained { event_hash } => {
                if first_hashed_event_id.is_none() {
                    first_hashed_event_id = Some(event.event_id.clone());
                }
                last_event_hash = Some(event_hash);
            }
        }
    }
    let chain_status = match (unchained_count, events.len()) {
        (_, 0) => WorldEventLedgerChainStatus::LegacyUnchained,
        (0, _) => WorldEventLedgerChainStatus::FullyChained,
        (count, total) if count == total => WorldEventLedgerChainStatus::LegacyUnchained,
        _ => WorldEventLedgerChainStatus::PartiallyChained,
    };
    Ok(WorldEventLedgerChainReport {
        schema_version: WORLD_EVENT_LEDGER_SCHEMA_VERSION.to_owned(),
        world_id: expected_world_id.to_owned(),
        event_count: events.len(),
        chain_status,
        first_hashed_event_id,
        last_event_hash,
    })
}

/// Replay event semantics into a small deterministic summary.
///
/// This is not a full world-state reducer yet; it proves that every modern
/// hashed event carries enough typed meaning for downstream reducers to build on
/// without treating prose as source of truth.
///
/// # Errors
///
/// Returns an error when event shape validation fails.
pub fn replay_world_event_semantics(
    expected_world_id: &str,
    events: &[CanonEvent],
) -> Result<WorldEventSemanticReplayReport> {
    let mut report = WorldEventSemanticReplayReport {
        schema_version: WORLD_EVENT_SEMANTIC_REPLAY_SCHEMA_VERSION.to_owned(),
        world_id: expected_world_id.to_owned(),
        event_count: events.len(),
        semantic_event_count: 0,
        legacy_only_event_count: 0,
        action_attempts: 0,
        action_successes: 0,
        action_failures: 0,
        process_ticks: 0,
        knowledge_events: 0,
        relationship_events: 0,
        body_resource_events: 0,
        location_events: 0,
        current_turn_id: events.last().map(|event| event.turn_id.clone()),
    };
    for event in events {
        validate_event_shape(event, Some(expected_world_id))?;
        let event_kind = event
            .event_kind
            .or_else(|| WorldEventKind::from_legacy_kind(event.kind.as_str()));
        let Some(event_kind) = event_kind else {
            report.legacy_only_event_count += 1;
            continue;
        };
        report.semantic_event_count += 1;
        match event_kind {
            WorldEventKind::PlayerActionAttempted => report.action_attempts += 1,
            WorldEventKind::ActionSucceeded => report.action_successes += 1,
            WorldEventKind::ActionFailed => report.action_failures += 1,
            WorldEventKind::ProcessTicked => report.process_ticks += 1,
            WorldEventKind::KnowledgeObserved | WorldEventKind::KnowledgeInferred => {
                report.knowledge_events += 1;
            }
            WorldEventKind::RelationshipChanged | WorldEventKind::DialogueExchange => {
                report.relationship_events += 1;
            }
            WorldEventKind::BodyStateChanged
            | WorldEventKind::ResourceChanged
            | WorldEventKind::ItemAcquired
            | WorldEventKind::ItemConsumed => {
                report.body_resource_events += 1;
            }
            WorldEventKind::EntityMoved | WorldEventKind::LocationAccessChanged => {
                report.location_events += 1;
            }
            WorldEventKind::WorldInitialized
            | WorldEventKind::NumericChoice
            | WorldEventKind::GuideChoice
            | WorldEventKind::FreeformAction
            | WorldEventKind::CodexQuery
            | WorldEventKind::MacroTimeFlow
            | WorldEventKind::CcCanvas
            | WorldEventKind::Observation
            | WorldEventKind::RepairRebuild
            | WorldEventKind::UnclassifiedLegacy
            | WorldEventKind::ConsequenceOpened
            | WorldEventKind::ConsequenceResolved
            | WorldEventKind::SurfaceStateChanged => {}
        }
    }
    Ok(report)
}

/// Replay the append-only event ledger into a deterministic reducer state.
///
/// This reducer intentionally covers only state that is represented by
/// structured `CanonEvent` fields today. It is therefore a subset replay, but it
/// gives projection health and validation a real state object to compare against
/// instead of trusting prose summaries.
///
/// # Errors
///
/// Returns an error when event shape validation fails.
pub fn replay_world_events(
    expected_world_id: &str,
    events: &[CanonEvent],
) -> Result<WorldEventReplayReport> {
    let semantic_summary = replay_world_event_semantics(expected_world_id, events)?;
    let mut observed_entity_refs = BTreeSet::new();
    let mut observed_location_refs = BTreeSet::new();
    let mut body_resource_refs = BTreeSet::new();
    let mut relationship_refs = BTreeSet::new();
    let mut open_consequence_refs = BTreeSet::new();
    let mut replayed_turn_id = None;
    let mut current_location = None;

    for event in events {
        validate_event_shape(event, Some(expected_world_id))?;
        replayed_turn_id = Some(event.turn_id.clone());
        for entity_ref in &event.entities {
            observed_entity_refs.insert(entity_ref.clone());
            index_replay_ref(
                entity_ref,
                &mut body_resource_refs,
                &mut relationship_refs,
                &mut observed_location_refs,
            );
        }
        if let Some(location) = &event.location {
            current_location = Some(location.clone());
            observed_location_refs.insert(location.clone());
        }
        for consequence in &event.consequences {
            index_replay_ref(
                consequence,
                &mut body_resource_refs,
                &mut relationship_refs,
                &mut observed_location_refs,
            );
            if let Some(resolved) = consequence
                .strip_prefix("resolved:")
                .or_else(|| consequence.strip_prefix("closed:"))
            {
                open_consequence_refs.remove(resolved);
            } else if consequence.starts_with("consequence:") {
                open_consequence_refs.insert(consequence.clone());
            }
        }
    }

    Ok(WorldEventReplayReport {
        schema_version: WORLD_EVENT_REPLAY_SCHEMA_VERSION.to_owned(),
        world_id: expected_world_id.to_owned(),
        event_count: events.len(),
        semantic_summary,
        replayed_turn_id,
        current_location,
        observed_entity_refs: observed_entity_refs.into_iter().collect(),
        observed_location_refs: observed_location_refs.into_iter().collect(),
        body_resource_refs: body_resource_refs.into_iter().collect(),
        relationship_refs: relationship_refs.into_iter().collect(),
        open_consequence_refs: open_consequence_refs.into_iter().collect(),
    })
}

fn index_replay_ref(
    item_ref: &str,
    body_resource_refs: &mut BTreeSet<String>,
    relationship_refs: &mut BTreeSet<String>,
    observed_location_refs: &mut BTreeSet<String>,
) {
    if item_ref.starts_with("body:")
        || item_ref.starts_with("resource:")
        || item_ref.starts_with("inventory:")
    {
        body_resource_refs.insert(item_ref.to_owned());
    }
    if item_ref.starts_with("rel:")
        || item_ref.starts_with("relationship:")
        || item_ref.starts_with("stance:")
        || item_ref.starts_with("social:")
    {
        relationship_refs.insert(item_ref.to_owned());
    }
    if item_ref.starts_with("place:") || item_ref.starts_with("location:") {
        observed_location_refs.insert(item_ref.to_owned());
    }
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
    })?;
    Ok(())
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
    if event.event_hash.is_some() && event.event_kind.is_none() {
        bail!(
            "world event ledger hashed event missing semantic event_kind: event_id={}, kind={}",
            event.event_id,
            event.kind
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

fn chain_event_for_append(
    existing_events: &[CanonEvent],
    event: &CanonEvent,
) -> Result<CanonEvent> {
    let previous_event_hash = existing_events
        .last()
        .and_then(|previous| previous.event_hash.clone());
    let mut event_to_append = event.clone();
    event_to_append.previous_event_hash = previous_event_hash;
    if event_to_append.event_kind.is_none() {
        event_to_append.event_kind = Some(
            WorldEventKind::from_legacy_kind(event_to_append.kind.as_str())
                .unwrap_or(WorldEventKind::UnclassifiedLegacy),
        );
    }
    validate_event_shape(&event_to_append, Some(event_to_append.world_id.as_str()))?;
    event_to_append.event_hash = Some(compute_event_hash(&event_to_append)?);
    Ok(event_to_append)
}

enum EventChainLink {
    LegacyUnchained,
    Chained { event_hash: String },
}

fn verify_event_chain_link(
    event: &CanonEvent,
    previous_event_hash: Option<&str>,
) -> Result<EventChainLink> {
    match (
        event.previous_event_hash.as_deref(),
        event.event_hash.as_deref(),
    ) {
        (None, None) => Ok(EventChainLink::LegacyUnchained),
        (Some(_), None) => bail!(
            "world event ledger previous_event_hash without event_hash: event_id={}",
            event.event_id
        ),
        (stored_previous, Some(stored_event_hash)) => {
            if stored_previous != previous_event_hash {
                bail!(
                    "world event ledger previous_event_hash mismatch: event_id={}, expected={:?}, actual={:?}",
                    event.event_id,
                    previous_event_hash,
                    stored_previous
                );
            }
            let expected_event_hash = compute_event_hash(event)?;
            if stored_event_hash != expected_event_hash {
                bail!(
                    "world event ledger event_hash mismatch: event_id={}, expected={}, actual={}",
                    event.event_id,
                    expected_event_hash,
                    stored_event_hash
                );
            }
            Ok(EventChainLink::Chained {
                event_hash: stored_event_hash.to_owned(),
            })
        }
    }
}

fn compute_event_hash(event: &CanonEvent) -> Result<String> {
    let payload = canonical_event_hash_payload(event)?;
    let mut input = Vec::new();
    input.extend_from_slice(WORLD_EVENT_HASH_VERSION.as_bytes());
    input.push(b'\n');
    input.extend_from_slice(WORLD_EVENT_HASH_ALGORITHM.as_bytes());
    input.push(b'\n');
    input.extend_from_slice(
        event
            .previous_event_hash
            .as_deref()
            .unwrap_or("")
            .as_bytes(),
    );
    input.push(b'\n');
    input.extend_from_slice(payload.as_bytes());
    Ok(format!(
        "{WORLD_EVENT_HASH_ALGORITHM}:{}",
        sha256_hex(&input)?
    ))
}

fn canonical_event_hash_payload(event: &CanonEvent) -> Result<String> {
    let mut value =
        serde_json::to_value(event).context("failed to serialize world event hash payload")?;
    let object = value
        .as_object_mut()
        .context("world event hash payload must be a JSON object")?;
    object.remove("previous_event_hash");
    object.remove("event_hash");
    serde_json::to_string(&value).context("failed to render world event hash payload")
}

// Fixed-width SHA-256 arithmetic is kept local so the ledger can add tamper-evident
// hashes without adding a new dependency to this public-alpha runtime.
#[allow(
    clippy::cast_possible_truncation,
    clippy::many_single_char_names,
    clippy::too_many_lines
)]
fn sha256_hex(input: &[u8]) -> Result<String> {
    const K: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state = [
        0x6a09_e667u32,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_len = u64::try_from(input.len())
        .context("world event hash input length out of range")?
        .checked_mul(8)
        .context("world event hash input bit length overflow")?;
    let mut message = input.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in message.chunks_exact(64) {
        let mut schedule = [0u32; 64];
        for (index, word) in chunk.chunks_exact(4).take(16).enumerate() {
            schedule[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        let mut index = 16usize;
        while index < 64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
            index += 1;
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for (index, constant) in K.iter().enumerate() {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(*constant)
                .wrapping_add(schedule[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(majority);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        for (value, added) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(added);
        }
    }

    let mut digest = [0u8; 32];
    for (index, value) in state.iter().enumerate() {
        digest[index * 4..index * 4 + 4].copy_from_slice(&value.to_be_bytes());
    }
    Ok(hex_lower(&digest))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        WorldEventLedgerChainStatus, append_world_event, load_world_events,
        replay_world_event_semantics, replay_world_events, verify_world_event_ledger,
        verify_world_events,
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
        assert_eq!(events[0].previous_event_hash, None);
        assert!(
            events[0]
                .event_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:"))
        );
        assert_eq!(
            events[1].previous_event_hash.as_deref(),
            events[0].event_hash.as_deref()
        );
        assert!(
            events[1]
                .event_hash
                .as_deref()
                .is_some_and(|hash| hash.starts_with("sha256:"))
        );
        let report = verify_world_event_ledger(&path, "stw_event_ledger")?;
        assert_eq!(report.event_count, 2);
        assert_eq!(
            report.chain_status,
            WorldEventLedgerChainStatus::FullyChained
        );
        assert_eq!(report.first_event_id.as_deref(), Some("evt_000000"));
        assert_eq!(report.last_event_id.as_deref(), Some("evt_000001"));
        assert_eq!(
            report.last_event_hash.as_deref(),
            events[1].event_hash.as_deref()
        );
        Ok(())
    }

    #[test]
    fn accepts_legacy_unchained_events() -> anyhow::Result<()> {
        let events = vec![
            event("evt_000000", "turn_0000", "world:created"),
            event("evt_000001", "turn_0001", "player:acted"),
        ];

        let report = verify_world_events("stw_event_ledger", &events)?;
        assert_eq!(
            report.chain_status,
            WorldEventLedgerChainStatus::LegacyUnchained
        );
        assert_eq!(report.last_event_hash, None);
        Ok(())
    }

    #[test]
    fn rejects_tampered_hashed_event() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("canon_events.jsonl");
        append_world_event(&path, &event("evt_000000", "turn_0000", "world:created"))?;
        let mut events = load_world_events(&path)?;
        events[0].summary = "tampered summary".to_owned();

        let Err(error) = verify_world_events("stw_event_ledger", &events) else {
            panic!("tampered event hash should be rejected");
        };
        assert!(format!("{error:#}").contains("event_hash mismatch"));
        Ok(())
    }

    #[test]
    fn rejects_broken_previous_hash_chain() -> anyhow::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("canon_events.jsonl");
        append_world_event(&path, &event("evt_000000", "turn_0000", "world:created"))?;
        append_world_event(&path, &event("evt_000001", "turn_0001", "player:acted"))?;
        let mut events = load_world_events(&path)?;
        events[1].previous_event_hash = Some("sha256:not-the-previous-event".to_owned());

        let Err(error) = verify_world_events("stw_event_ledger", &events) else {
            panic!("broken previous hash chain should be rejected");
        };
        assert!(format!("{error:#}").contains("previous_event_hash mismatch"));
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

    #[test]
    fn accepts_semantic_event_kind_sidecar_for_legacy_turn_kind() -> anyhow::Result<()> {
        let mut event = event("evt_000001", "turn_0001", "numeric_choice");
        event.event_kind = Some(WorldEventKind::ActionSucceeded);

        let report = verify_world_events("stw_event_ledger", &[event])?;

        assert_eq!(report.event_count, 1);
        Ok(())
    }

    #[test]
    fn replays_semantic_event_counts() -> anyhow::Result<()> {
        let mut success = event("evt_000001", "turn_0001", "numeric_choice");
        success.event_kind = Some(WorldEventKind::ActionSucceeded);
        let mut process = event("evt_000002", "turn_0002", "macro_time_flow");
        process.event_kind = Some(WorldEventKind::ProcessTicked);
        let mut knowledge = event("evt_000003", "turn_0003", "codex_query");
        knowledge.event_kind = Some(WorldEventKind::KnowledgeObserved);

        let report =
            replay_world_event_semantics("stw_event_ledger", &[success, process, knowledge])?;

        assert_eq!(report.event_count, 3);
        assert_eq!(report.semantic_event_count, 3);
        assert_eq!(report.legacy_only_event_count, 0);
        assert_eq!(report.action_successes, 1);
        assert_eq!(report.process_ticks, 1);
        assert_eq!(report.knowledge_events, 1);
        assert_eq!(report.current_turn_id.as_deref(), Some("turn_0003"));
        Ok(())
    }

    #[test]
    fn rejects_hashed_event_without_semantic_event_kind() {
        let mut event = event("evt_000001", "turn_0001", "numeric_choice");
        event.event_kind = None;
        event.event_hash = Some("sha256:not-a-real-event-hash".to_owned());

        let Err(error) = verify_world_events("stw_event_ledger", &[event]) else {
            panic!("hashed event without semantic event_kind should be rejected");
        };
        assert!(format!("{error:#}").contains("missing semantic event_kind"));
    }

    #[test]
    fn replays_unknown_legacy_kind_as_legacy_only() -> anyhow::Result<()> {
        let mut event = event("evt_000001", "turn_0001", "custom_legacy_kind");
        event.event_kind = None;

        let report = replay_world_event_semantics("stw_event_ledger", &[event])?;

        assert_eq!(report.semantic_event_count, 0);
        assert_eq!(report.legacy_only_event_count, 1);
        Ok(())
    }

    #[test]
    fn replays_world_event_state_subset() -> anyhow::Result<()> {
        let mut initial = event("evt_000000", "turn_0000", "note");
        initial.event_kind = Some(WorldEventKind::WorldInitialized);
        initial.location = Some("place:opening_location".to_owned());
        initial.entities = vec!["char:protagonist".to_owned()];
        let mut resource = event("evt_000001", "turn_0001", "resource_changed");
        resource.event_kind = Some(WorldEventKind::ResourceChanged);
        resource.entities = vec!["resource:inventory:00".to_owned()];
        resource.consequences = vec!["consequence:spent_entry_token".to_owned()];
        let mut social = event("evt_000002", "turn_0002", "relationship_changed");
        social.event_kind = Some(WorldEventKind::RelationshipChanged);
        social.entities = vec!["rel:guard->protagonist:distance".to_owned()];
        social.consequences = vec!["resolved:consequence:spent_entry_token".to_owned()];

        let report = replay_world_events("stw_event_ledger", &[initial, resource, social])?;

        assert_eq!(report.event_count, 3);
        assert_eq!(report.replayed_turn_id.as_deref(), Some("turn_0002"));
        assert_eq!(
            report.current_location.as_deref(),
            Some("place:opening_location")
        );
        assert!(
            report
                .observed_entity_refs
                .iter()
                .any(|item| item == "char:protagonist")
        );
        assert!(
            report
                .body_resource_refs
                .iter()
                .any(|item| item == "resource:inventory:00")
        );
        assert!(
            report
                .relationship_refs
                .iter()
                .any(|item| item == "rel:guard->protagonist:distance")
        );
        assert!(report.open_consequence_refs.is_empty());
        assert_eq!(report.semantic_summary.body_resource_events, 1);
        assert_eq!(report.semantic_summary.relationship_events, 1);
        Ok(())
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
            previous_event_hash: None,
            event_hash: None,
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
