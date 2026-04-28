use crate::agent_bridge::AgentExtraContact;
use crate::models::{CharacterVoiceAnchor, TurnSnapshot};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const EXTRA_TRACE_SCHEMA_VERSION: &str = "singulari.extra_trace.v1";
pub const REMEMBERED_EXTRAS_SCHEMA_VERSION: &str = "singulari.remembered_extras.v1";
pub const REMEMBERED_EXTRA_SCHEMA_VERSION: &str = "singulari.remembered_extra.v1";
pub const EXTRA_MEMORY_PROJECTION_RECORD_SCHEMA_VERSION: &str =
    "singulari.extra_memory_projection.v1";
pub const EXTRA_TRACES_FILENAME: &str = "extra_traces.jsonl";
pub const REMEMBERED_EXTRAS_FILENAME: &str = "remembered_extras.json";
pub const EXTRA_MEMORY_PROJECTIONS_FILENAME: &str = "extra_memory_projections.jsonl";

const REMEMBERED_EXTRA_LIMIT: usize = 7;
const RECENT_TRACE_LIMIT: usize = 5;
const MAX_EXTRA_MEMORY_ITEMS: usize = 8;
const MAX_OPEN_HOOKS: usize = 6;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraTrace {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub trace_id: String,
    pub surface_label: String,
    pub location_id: String,
    pub scene_role: String,
    pub contact_summary: String,
    #[serde(default)]
    pub pressure_tags: Vec<String>,
    pub promotion_hint: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RememberedExtrasStore {
    pub schema_version: String,
    pub world_id: String,
    #[serde(default)]
    pub extras: Vec<RememberedExtra>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RememberedExtra {
    pub schema_version: String,
    pub world_id: String,
    pub extra_id: String,
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub known_name: Option<String>,
    pub role: String,
    pub home_location_id: String,
    pub visibility: String,
    pub first_seen_turn: String,
    pub last_seen_turn: String,
    pub contact_count: u32,
    pub disposition: String,
    #[serde(default)]
    pub memory: Vec<String>,
    pub text_design: CharacterVoiceAnchor,
    #[serde(default)]
    pub open_hooks: Vec<String>,
    pub promotion_score: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExtraMemoryPacket {
    #[serde(default)]
    pub remembered_extras: Vec<RememberedExtra>,
    #[serde(default)]
    pub recent_extra_traces: Vec<ExtraTrace>,
    pub extra_memory_policy: ExtraMemoryPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraMemoryPolicy {
    pub evidence_tiers: Vec<String>,
    pub retrieval_budget: ExtraMemoryRetrievalBudget,
    pub use_rules: Vec<String>,
}

impl Default for ExtraMemoryPolicy {
    fn default() -> Self {
        Self {
            evidence_tiers: vec![
                "CharacterRecord".to_owned(),
                "RememberedExtra".to_owned(),
                "ExtraTrace".to_owned(),
                "DerivedCandidate".to_owned(),
            ],
            retrieval_budget: ExtraMemoryRetrievalBudget {
                remembered_extras: REMEMBERED_EXTRA_LIMIT,
                recent_extra_traces: RECENT_TRACE_LIMIT,
            },
            use_rules: vec![
                "Use retrieved extras only when they naturally fit the current scene.".to_owned(),
                "Do not force every remembered extra to appear.".to_owned(),
                "Never use extra memory to leak hidden truth.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraMemoryRetrievalBudget {
    pub remembered_extras: usize,
    pub recent_extra_traces: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalFaceEntry {
    pub extra_id: String,
    pub display_name: String,
    pub role: String,
    pub home_location_id: String,
    pub last_seen_turn: String,
    pub disposition: String,
    pub last_contact: String,
    #[serde(default)]
    pub open_hooks: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExtraMemoryProjectionPlan {
    world_id: String,
    turn_id: String,
    traces_path: PathBuf,
    remembered_extras_path: PathBuf,
    projection_records_path: PathBuf,
    traces_to_append: Vec<ExtraTrace>,
    remembered_store: RememberedExtrasStore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtraMemoryProjectionStatus {
    Committed,
    Failed,
    Repaired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraMemoryProjectionRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub status: ExtraMemoryProjectionStatus,
    pub traces_planned: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remembered_extras_planned: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtraMemoryRepairReport {
    pub world_id: String,
    pub traces_read: usize,
    pub remembered_extras_rebuilt: usize,
    pub projection_records_read: usize,
    pub repaired_failed_records: usize,
    pub repaired_at: String,
}

/// Preflight agent-authored extra contact records before the turn state advances.
///
/// # Errors
///
/// Returns an error when existing memory files cannot be read or a contact is
/// missing required player-visible fields.
pub fn compile_extra_memory_projection(
    world_dir: &Path,
    world_id: &str,
    turn_id: &str,
    location_id: &str,
    contacts: &[AgentExtraContact],
) -> Result<ExtraMemoryProjectionPlan> {
    let mut store = load_remembered_extras(world_dir, world_id)?;
    let mut existing_traces = load_extra_traces(world_dir)?;
    let mut traces_to_append = Vec::new();
    let created_at = Utc::now().to_rfc3339();

    for contact in contacts {
        let trace = extra_trace_from_contact(world_id, turn_id, location_id, contact, &created_at)?;
        let matching_prior_traces =
            matching_prior_traces(&trace, contact, &existing_traces).collect::<Vec<_>>();
        let should_remember = should_remember_contact(contact, &store, &matching_prior_traces);
        if should_remember {
            upsert_remembered_extra(&mut store, &trace, contact, &matching_prior_traces);
        }
        existing_traces.push(trace.clone());
        traces_to_append.push(trace);
    }

    store.updated_at = Utc::now().to_rfc3339();
    Ok(ExtraMemoryProjectionPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        traces_path: extra_traces_path(world_dir),
        remembered_extras_path: remembered_extras_path(world_dir),
        projection_records_path: extra_memory_projections_path(world_dir),
        traces_to_append,
        remembered_store: store,
    })
}

/// Persist a preflighted extra-memory projection.
///
/// # Errors
///
/// Returns an error when trace or remembered-extra writes fail.
pub fn commit_extra_memory_projection(plan: &ExtraMemoryProjectionPlan) -> Result<()> {
    for trace in &plan.traces_to_append {
        append_jsonl(&plan.traces_path, trace)?;
    }
    write_json(&plan.remembered_extras_path, &plan.remembered_store)
}

/// Persist a preflighted extra-memory projection and append its terminal status.
///
/// # Errors
///
/// Returns an error only when the terminal status record itself cannot be
/// written. Projection write failures are captured as `failed` records so the
/// already-committed turn can close and projection repair can handle the fault.
pub fn commit_extra_memory_projection_terminal(
    plan: &ExtraMemoryProjectionPlan,
) -> Result<ExtraMemoryProjectionRecord> {
    let projection_result = commit_extra_memory_projection(plan);
    let record = match projection_result {
        Ok(()) => plan.projection_record(ExtraMemoryProjectionStatus::Committed, None),
        Err(error) => plan.projection_record(
            ExtraMemoryProjectionStatus::Failed,
            Some(format!("{error:#}")),
        ),
    };
    append_jsonl(&plan.projection_records_path, &record)?;
    Ok(record)
}

/// Load terminal extra-memory projection records for health checks.
///
/// # Errors
///
/// Returns an error when the record file exists but cannot be parsed.
pub fn load_extra_memory_projection_records(
    world_dir: &Path,
) -> Result<Vec<ExtraMemoryProjectionRecord>> {
    let path = extra_memory_projections_path(world_dir);
    let Ok(raw) = fs::read_to_string(path.as_path()) else {
        return Ok(Vec::new());
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<ExtraMemoryProjectionRecord>(line)
                .context("failed to parse extra memory projection record")
        })
        .collect()
}

/// Rebuild remembered extras from trace evidence and close failed projection records.
///
/// # Errors
///
/// Returns an error when trace/projection records cannot be parsed or when the
/// rebuilt remembered-extra store cannot be written.
pub fn repair_extra_memory_projection(
    world_dir: &Path,
    world_id: &str,
) -> Result<ExtraMemoryRepairReport> {
    let traces = load_extra_traces(world_dir)?;
    for trace in &traces {
        if trace.world_id != world_id {
            bail!(
                "extra trace world mismatch: expected={}, actual={}, trace_id={}",
                world_id,
                trace.world_id,
                trace.trace_id
            );
        }
    }
    let records = load_extra_memory_projection_records(world_dir)?;
    let repaired_failed_records = failed_projection_records_after_latest_repair(&records);
    let repaired_at = Utc::now().to_rfc3339();
    let store = rebuild_remembered_extras_from_traces(world_id, &traces, repaired_at.as_str());
    write_json(&remembered_extras_path(world_dir), &store)?;
    let repair_record = ExtraMemoryProjectionRecord {
        schema_version: EXTRA_MEMORY_PROJECTION_RECORD_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: "__repair__".to_owned(),
        status: ExtraMemoryProjectionStatus::Repaired,
        traces_planned: traces.len(),
        remembered_extras_planned: Some(store.extras.len()),
        error: None,
        recorded_at: repaired_at.clone(),
    };
    append_jsonl(&extra_memory_projections_path(world_dir), &repair_record)?;
    Ok(ExtraMemoryRepairReport {
        world_id: world_id.to_owned(),
        traces_read: traces.len(),
        remembered_extras_rebuilt: store.extras.len(),
        projection_records_read: records.len(),
        repaired_failed_records,
        repaired_at,
    })
}

/// Apply agent-authored extra contact records after a turn commit.
///
/// # Errors
///
/// Returns an error when existing memory files cannot be read, a contact is
/// missing required player-visible fields, or trace/store writes fail.
pub fn apply_extra_memory_projection(
    world_dir: &Path,
    world_id: &str,
    turn_id: &str,
    location_id: &str,
    contacts: &[AgentExtraContact],
) -> Result<()> {
    if contacts.is_empty() {
        return Ok(());
    }
    let plan =
        compile_extra_memory_projection(world_dir, world_id, turn_id, location_id, contacts)?;
    commit_extra_memory_projection(&plan)
}

impl ExtraMemoryProjectionPlan {
    fn projection_record(
        &self,
        status: ExtraMemoryProjectionStatus,
        error: Option<String>,
    ) -> ExtraMemoryProjectionRecord {
        ExtraMemoryProjectionRecord {
            schema_version: EXTRA_MEMORY_PROJECTION_RECORD_SCHEMA_VERSION.to_owned(),
            world_id: self.world_id.clone(),
            turn_id: self.turn_id.clone(),
            status,
            traces_planned: self.traces_to_append.len(),
            remembered_extras_planned: Some(self.remembered_store.extras.len()),
            error,
            recorded_at: Utc::now().to_rfc3339(),
        }
    }
}

#[must_use]
pub fn failed_projection_records_after_latest_repair(
    records: &[ExtraMemoryProjectionRecord],
) -> usize {
    let start = records
        .iter()
        .rposition(|record| record.status == ExtraMemoryProjectionStatus::Repaired)
        .map_or(0, |index| index + 1);
    records[start..]
        .iter()
        .filter(|record| record.status == ExtraMemoryProjectionStatus::Failed)
        .count()
}

/// Retrieve the compact extra memory packet for the next pending turn.
///
/// # Errors
///
/// Returns an error when remembered-extra or trace files exist but cannot be
/// parsed.
pub fn retrieve_extra_memory_packet(
    world_dir: &Path,
    world_id: &str,
    snapshot: &TurnSnapshot,
    player_input: &str,
) -> Result<ExtraMemoryPacket> {
    let store = load_remembered_extras(world_dir, world_id)?;
    let traces = load_extra_traces(world_dir)?;
    let remembered_extras = select_remembered_extras(
        &store.extras,
        &snapshot.protagonist_state.location,
        player_input,
    );
    let recent_extra_traces = select_recent_traces(
        &traces,
        &snapshot.protagonist_state.location,
        player_input,
        &remembered_extras,
    );
    Ok(ExtraMemoryPacket {
        remembered_extras,
        recent_extra_traces,
        extra_memory_policy: ExtraMemoryPolicy {
            evidence_tiers: vec![
                "CharacterRecord".to_owned(),
                "RememberedExtra".to_owned(),
                "ExtraTrace".to_owned(),
                "DerivedCandidate".to_owned(),
            ],
            retrieval_budget: ExtraMemoryRetrievalBudget {
                remembered_extras: REMEMBERED_EXTRA_LIMIT,
                recent_extra_traces: RECENT_TRACE_LIMIT,
            },
            use_rules: vec![
                "Use retrieved extras only when they naturally fit the current scene.".to_owned(),
                "Do not force every remembered extra to appear.".to_owned(),
                "Preserve speech, endings, tone, gestures, habits, and drift if an extra reappears."
                    .to_owned(),
                "Never use extra memory to leak hidden truth or imply an authorial guide.".to_owned(),
            ],
        },
    })
}

/// Build the player-visible Local Faces section for Archive View.
///
/// # Errors
///
/// Returns an error when the remembered-extra store exists but cannot be read.
pub fn local_faces_for_codex_view(
    world_dir: &Path,
    world_id: &str,
    location_id: &str,
    limit: usize,
) -> Result<Vec<LocalFaceEntry>> {
    let store = load_remembered_extras(world_dir, world_id)?;
    Ok(select_remembered_extras(&store.extras, location_id, "")
        .into_iter()
        .take(limit)
        .map(|extra| LocalFaceEntry {
            extra_id: extra.extra_id,
            display_name: extra.display_name,
            role: extra.role,
            home_location_id: extra.home_location_id,
            last_seen_turn: extra.last_seen_turn,
            disposition: extra.disposition,
            last_contact: extra.memory.last().cloned().unwrap_or_default(),
            open_hooks: extra.open_hooks,
        })
        .collect())
}

/// Load the remembered-extra store for a world.
///
/// # Errors
///
/// Returns an error when the store exists but cannot be read or targets a
/// different world.
pub fn load_remembered_extras(world_dir: &Path, world_id: &str) -> Result<RememberedExtrasStore> {
    let path = remembered_extras_path(world_dir);
    if !path.exists() {
        return Ok(RememberedExtrasStore {
            schema_version: REMEMBERED_EXTRAS_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            extras: Vec::new(),
            updated_at: Utc::now().to_rfc3339(),
        });
    }
    let store: RememberedExtrasStore = read_json(path.as_path())?;
    if store.world_id != world_id {
        anyhow::bail!(
            "remembered extras world mismatch: expected={}, actual={}",
            world_id,
            store.world_id
        );
    }
    Ok(store)
}

fn load_extra_traces(world_dir: &Path) -> Result<Vec<ExtraTrace>> {
    let path = extra_traces_path(world_dir);
    let Ok(raw) = fs::read_to_string(path.as_path()) else {
        return Ok(Vec::new());
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<ExtraTrace>(line).context("failed to parse extra trace"))
        .collect()
}

fn extra_trace_from_contact(
    world_id: &str,
    turn_id: &str,
    fallback_location_id: &str,
    contact: &AgentExtraContact,
    created_at: &str,
) -> Result<ExtraTrace> {
    let surface_label = normalized_required("extra surface_label", contact.surface_label.as_str())?;
    let contact_summary =
        normalized_required("extra contact_summary", contact.contact_summary.as_str())?;
    let location_id = contact
        .location_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback_location_id)
        .to_owned();
    let scene_role = contact
        .scene_role
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("background_contact")
        .to_owned();
    Ok(ExtraTrace {
        schema_version: EXTRA_TRACE_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        trace_id: format!(
            "extra_trace:{turn_id}:{}",
            stable_slug(surface_label.as_str())
        ),
        surface_label,
        location_id,
        scene_role,
        contact_summary,
        pressure_tags: normalized_list(&contact.pressure_tags),
        promotion_hint: contact
            .promotion_hint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("player-visible contact")
            .to_owned(),
        created_at: created_at.to_owned(),
    })
}

fn should_remember_contact(
    contact: &AgentExtraContact,
    store: &RememberedExtrasStore,
    matching_prior_traces: &[&ExtraTrace],
) -> bool {
    matches!(
        contact.memory_action.as_deref(),
        Some("remember" | "promote" | "update")
    ) || store.extras.iter().any(|extra| {
        extra.display_name == contact.surface_label || extra.known_name == contact.known_name
    }) || !matching_prior_traces.is_empty()
}

fn upsert_remembered_extra(
    store: &mut RememberedExtrasStore,
    trace: &ExtraTrace,
    contact: &AgentExtraContact,
    matching_prior_traces: &[&ExtraTrace],
) {
    let extra_id = format!(
        "extra:{}:{}",
        stable_slug(trace.location_id.as_str()),
        stable_slug(trace.surface_label.as_str())
    );
    let Some(extra) = store
        .extras
        .iter_mut()
        .find(|extra| extra.extra_id == extra_id || extra.display_name == trace.surface_label)
    else {
        store.extras.push(RememberedExtra {
            schema_version: REMEMBERED_EXTRA_SCHEMA_VERSION.to_owned(),
            world_id: trace.world_id.clone(),
            extra_id,
            display_name: trace.surface_label.clone(),
            known_name: contact.known_name.clone(),
            role: contact
                .role
                .clone()
                .unwrap_or_else(|| trace.scene_role.clone()),
            home_location_id: trace.location_id.clone(),
            visibility: "player_visible".to_owned(),
            first_seen_turn: matching_prior_traces
                .first()
                .map_or_else(|| trace.turn_id.clone(), |trace| trace.turn_id.clone()),
            last_seen_turn: trace.turn_id.clone(),
            contact_count: u32::try_from(matching_prior_traces.len() + 1).unwrap_or(u32::MAX),
            disposition: contact
                .disposition
                .clone()
                .unwrap_or_else(|| "unsettled".to_owned()),
            memory: contact_memory_from_traces(matching_prior_traces, trace),
            text_design: contact.text_design.clone().unwrap_or_default(),
            open_hooks: normalized_list(&contact.open_hooks),
            promotion_score: promotion_score(contact),
        });
        return;
    };

    extra.last_seen_turn.clone_from(&trace.turn_id);
    extra.contact_count = extra.contact_count.saturating_add(1);
    extra.promotion_score = extra
        .promotion_score
        .saturating_add(promotion_score(contact).max(1));
    push_limited_unique(
        &mut extra.memory,
        trace.contact_summary.as_str(),
        MAX_EXTRA_MEMORY_ITEMS,
    );
    if let Some(disposition) = &contact.disposition
        && !disposition.trim().is_empty()
    {
        disposition.trim().clone_into(&mut extra.disposition);
    }
    merge_voice_anchor(&mut extra.text_design, contact.text_design.as_ref());
    for hook in normalized_list(&contact.open_hooks) {
        push_limited_unique(&mut extra.open_hooks, hook.as_str(), MAX_OPEN_HOOKS);
    }
}

fn matching_prior_traces<'a>(
    trace: &ExtraTrace,
    _contact: &AgentExtraContact,
    traces: &'a [ExtraTrace],
) -> impl Iterator<Item = &'a ExtraTrace> {
    traces.iter().filter(|prior| {
        prior.surface_label == trace.surface_label && prior.location_id == trace.location_id
    })
}

fn contact_memory_from_traces(
    matching_prior_traces: &[&ExtraTrace],
    trace: &ExtraTrace,
) -> Vec<String> {
    let mut memory = Vec::new();
    for prior in matching_prior_traces {
        push_limited_unique(
            &mut memory,
            prior.contact_summary.as_str(),
            MAX_EXTRA_MEMORY_ITEMS,
        );
    }
    push_limited_unique(
        &mut memory,
        trace.contact_summary.as_str(),
        MAX_EXTRA_MEMORY_ITEMS,
    );
    memory
}

fn rebuild_remembered_extras_from_traces(
    world_id: &str,
    traces: &[ExtraTrace],
    updated_at: &str,
) -> RememberedExtrasStore {
    let mut grouped: BTreeMap<(String, String), Vec<&ExtraTrace>> = BTreeMap::new();
    for trace in traces {
        grouped
            .entry((trace.location_id.clone(), trace.surface_label.clone()))
            .or_default()
            .push(trace);
    }
    let extras = grouped
        .into_iter()
        .filter_map(|((location_id, surface_label), traces)| {
            if traces.len() < 2 {
                return None;
            }
            let first = traces.first()?;
            let last = traces.last()?;
            let mut memory = Vec::new();
            for trace in &traces {
                push_limited_unique(
                    &mut memory,
                    trace.contact_summary.as_str(),
                    MAX_EXTRA_MEMORY_ITEMS,
                );
            }
            Some(RememberedExtra {
                schema_version: REMEMBERED_EXTRA_SCHEMA_VERSION.to_owned(),
                world_id: world_id.to_owned(),
                extra_id: format!(
                    "extra:{}:{}",
                    stable_slug(location_id.as_str()),
                    stable_slug(surface_label.as_str())
                ),
                display_name: surface_label,
                known_name: None,
                role: first.scene_role.clone(),
                home_location_id: location_id,
                visibility: "player_visible".to_owned(),
                first_seen_turn: first.turn_id.clone(),
                last_seen_turn: last.turn_id.clone(),
                contact_count: u32::try_from(traces.len()).unwrap_or(u32::MAX),
                disposition: "unsettled".to_owned(),
                memory,
                text_design: CharacterVoiceAnchor::default(),
                open_hooks: Vec::new(),
                promotion_score: u32::try_from(traces.len()).unwrap_or(u32::MAX),
            })
        })
        .collect();
    RememberedExtrasStore {
        schema_version: REMEMBERED_EXTRAS_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        extras,
        updated_at: updated_at.to_owned(),
    }
}

fn select_remembered_extras(
    extras: &[RememberedExtra],
    location_id: &str,
    player_input: &str,
) -> Vec<RememberedExtra> {
    let mut scored = extras
        .iter()
        .map(|extra| (extra_score(extra, location_id, player_input), extra))
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.last_seen_turn.cmp(&left.last_seen_turn))
    });
    scored
        .into_iter()
        .map(|(_, extra)| extra.clone())
        .take(REMEMBERED_EXTRA_LIMIT)
        .collect()
}

fn select_recent_traces(
    traces: &[ExtraTrace],
    location_id: &str,
    player_input: &str,
    remembered_extras: &[RememberedExtra],
) -> Vec<ExtraTrace> {
    let remembered_labels = remembered_extras
        .iter()
        .map(|extra| extra.display_name.as_str())
        .collect::<BTreeSet<_>>();
    let mut scored = traces
        .iter()
        .filter(|trace| !remembered_labels.contains(trace.surface_label.as_str()))
        .map(|trace| (trace_score(trace, location_id, player_input), trace))
        .filter(|(score, _)| *score > 0)
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.turn_id.cmp(&left.turn_id))
    });
    scored
        .into_iter()
        .map(|(_, trace)| trace.clone())
        .take(RECENT_TRACE_LIMIT)
        .collect()
}

fn extra_score(extra: &RememberedExtra, location_id: &str, player_input: &str) -> u32 {
    let mut score = extra.promotion_score + extra.contact_count;
    if extra.home_location_id == location_id {
        score += 6;
    }
    if text_matches(extra.display_name.as_str(), player_input)
        || extra
            .known_name
            .as_deref()
            .is_some_and(|name| text_matches(name, player_input))
        || text_matches(extra.role.as_str(), player_input)
    {
        score += 4;
    }
    if extra
        .open_hooks
        .iter()
        .any(|hook| text_matches(hook, player_input))
    {
        score += 3;
    }
    score
}

fn trace_score(trace: &ExtraTrace, location_id: &str, player_input: &str) -> u32 {
    let mut score = 1;
    if trace.location_id == location_id {
        score += 4;
    }
    if text_matches(trace.surface_label.as_str(), player_input)
        || text_matches(trace.scene_role.as_str(), player_input)
        || text_matches(trace.contact_summary.as_str(), player_input)
    {
        score += 3;
    }
    score
}

fn promotion_score(contact: &AgentExtraContact) -> u32 {
    let mut score = 1;
    if matches!(
        contact.memory_action.as_deref(),
        Some("remember" | "promote" | "update")
    ) {
        score += 2;
    }
    score += u32::try_from(contact.pressure_tags.len().min(3)).unwrap_or(3);
    if contact.known_name.is_some() {
        score += 1;
    }
    if !contact.open_hooks.is_empty() {
        score += 1;
    }
    score
}

fn merge_voice_anchor(target: &mut CharacterVoiceAnchor, source: Option<&CharacterVoiceAnchor>) {
    let Some(source) = source else {
        return;
    };
    for value in &source.speech {
        push_limited_unique(&mut target.speech, value, 6);
    }
    for value in &source.endings {
        push_limited_unique(&mut target.endings, value, 6);
    }
    for value in &source.tone {
        push_limited_unique(&mut target.tone, value, 6);
    }
    for value in &source.gestures {
        push_limited_unique(&mut target.gestures, value, 6);
    }
    for value in &source.habits {
        push_limited_unique(&mut target.habits, value, 6);
    }
    for value in &source.drift {
        push_limited_unique(&mut target.drift, value, 6);
    }
}

fn push_limited_unique(values: &mut Vec<String>, value: &str, limit: usize) {
    let normalized = value.trim();
    if normalized.is_empty() || values.iter().any(|existing| existing == normalized) {
        return;
    }
    values.push(normalized.to_owned());
    if values.len() > limit {
        values.remove(0);
    }
}

fn normalized_required(field: &str, value: &str) -> Result<String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        bail!("{field} must not be empty");
    }
    if is_placeholder_extra_field(normalized) {
        bail!("{field} contains schema placeholder text: {normalized}");
    }
    Ok(normalized.to_owned())
}

fn is_placeholder_extra_field(value: &str) -> bool {
    matches!(
        value,
        "player-visible 주변 인물 표지"
            | "이번 턴에서 플레이어-visible로 남은 접촉/목격/거래/감정 흔적"
            | "장소/사건 안 역할"
            | "현재 장소 id 또는 null"
            | "왜 나중에 다시 떠오를 수 있는지"
            | "플레이어-visible 후속 가능성"
    )
}

fn normalized_list(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn text_matches(needle: &str, haystack: &str) -> bool {
    let needle = needle.trim();
    !needle.is_empty() && haystack.contains(needle)
}

fn stable_slug(value: &str) -> String {
    let slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = slug.trim_matches('_');
    if trimmed.is_empty() {
        "extra".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn remembered_extras_path(world_dir: &Path) -> PathBuf {
    world_dir.join(REMEMBERED_EXTRAS_FILENAME)
}

fn extra_traces_path(world_dir: &Path) -> PathBuf {
    world_dir.join(EXTRA_TRACES_FILENAME)
}

fn extra_memory_projections_path(world_dir: &Path) -> PathBuf {
    world_dir.join(EXTRA_MEMORY_PROJECTIONS_FILENAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_trace_promotes_extra_to_remembered() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let first = AgentExtraContact {
            surface_label: "gate porter".to_owned(),
            known_name: None,
            role: Some("porter near the west gate".to_owned()),
            location_id: Some("place:west_gate".to_owned()),
            scene_role: Some("witness".to_owned()),
            contact_summary: "saw the protagonist hide a bloodied sleeve".to_owned(),
            pressure_tags: vec!["social".to_owned()],
            promotion_hint: Some("witnessed risky action".to_owned()),
            memory_action: None,
            disposition: Some("wary".to_owned()),
            text_design: None,
            open_hooks: Vec::new(),
        };
        apply_extra_memory_projection(
            temp.path(),
            "stw_extra",
            "turn_0001",
            "place:west_gate",
            std::slice::from_ref(&first),
        )?;
        assert!(
            load_remembered_extras(temp.path(), "stw_extra")?
                .extras
                .is_empty()
        );

        let second = AgentExtraContact {
            contact_summary: "later blocked the side door after recognizing the same sleeve"
                .to_owned(),
            ..first.clone()
        };
        apply_extra_memory_projection(
            temp.path(),
            "stw_extra",
            "turn_0002",
            "place:west_gate",
            &[second],
        )?;
        let remembered = load_remembered_extras(temp.path(), "stw_extra")?;
        assert_eq!(remembered.extras.len(), 1);
        assert_eq!(remembered.extras[0].contact_count, 2);
        assert_eq!(remembered.extras[0].first_seen_turn, "turn_0001");
        assert_eq!(
            remembered.extras[0].memory,
            vec![
                "saw the protagonist hide a bloodied sleeve",
                "later blocked the side door after recognizing the same sleeve"
            ]
        );
        Ok(())
    }

    #[test]
    fn repair_extra_memory_rebuilds_from_traces_and_closes_failed_records() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let first = AgentExtraContact {
            surface_label: "gate porter".to_owned(),
            known_name: None,
            role: Some("porter near the west gate".to_owned()),
            location_id: Some("place:west_gate".to_owned()),
            scene_role: Some("witness".to_owned()),
            contact_summary: "saw the protagonist hide a bloodied sleeve".to_owned(),
            pressure_tags: vec!["social".to_owned()],
            promotion_hint: Some("witnessed risky action".to_owned()),
            memory_action: None,
            disposition: Some("wary".to_owned()),
            text_design: None,
            open_hooks: Vec::new(),
        };
        apply_extra_memory_projection(
            temp.path(),
            "stw_extra",
            "turn_0001",
            "place:west_gate",
            std::slice::from_ref(&first),
        )?;
        let second = AgentExtraContact {
            contact_summary: "later blocked the side door after recognizing the same sleeve"
                .to_owned(),
            ..first
        };
        apply_extra_memory_projection(
            temp.path(),
            "stw_extra",
            "turn_0002",
            "place:west_gate",
            &[second],
        )?;
        std::fs::write(remembered_extras_path(temp.path()), "{")?;
        append_jsonl(
            &extra_memory_projections_path(temp.path()),
            &ExtraMemoryProjectionRecord {
                schema_version: EXTRA_MEMORY_PROJECTION_RECORD_SCHEMA_VERSION.to_owned(),
                world_id: "stw_extra".to_owned(),
                turn_id: "turn_0002".to_owned(),
                status: ExtraMemoryProjectionStatus::Failed,
                traces_planned: 1,
                remembered_extras_planned: None,
                error: Some("simulated failure".to_owned()),
                recorded_at: Utc::now().to_rfc3339(),
            },
        )?;

        let report = repair_extra_memory_projection(temp.path(), "stw_extra")?;

        assert_eq!(report.traces_read, 2);
        assert_eq!(report.remembered_extras_rebuilt, 1);
        assert_eq!(report.repaired_failed_records, 1);
        let records = load_extra_memory_projection_records(temp.path())?;
        assert_eq!(failed_projection_records_after_latest_repair(&records), 0);
        let remembered = load_remembered_extras(temp.path(), "stw_extra")?;
        assert_eq!(remembered.extras[0].contact_count, 2);
        assert_eq!(
            remembered.extras[0].memory,
            vec![
                "saw the protagonist hide a bloodied sleeve",
                "later blocked the side door after recognizing the same sleeve"
            ]
        );
        Ok(())
    }
}
