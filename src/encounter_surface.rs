#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::AgentTurnResponse;
use crate::body_resource::BodyResourcePacket;
use crate::hook_ledger::{OmissionProfile, omission_profile_for_choice};
use crate::location_graph::{LocationGraphPacket, LocationNode};
use crate::models::TurnChoice;
use crate::resolution::{GateKind, GateStatus, ResolutionOutcomeKind, ResolutionVisibility};
use crate::scene_pressure::{ScenePressureKind, ScenePressurePacket};
use crate::social_exchange::{AskStatus, SocialExchangePacket};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::{Context, Result, bail};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const ENCOUNTER_SURFACE_PACKET_SCHEMA_VERSION: &str = "singulari.encounter_surface_packet.v1";
pub const ENCOUNTER_SURFACE_SCHEMA_VERSION: &str = "singulari.encounter_surface.v1";
pub const ENCOUNTER_AFFORDANCE_SCHEMA_VERSION: &str = "singulari.encounter_affordance.v1";
pub const ENCOUNTER_CONSTRAINT_SCHEMA_VERSION: &str = "singulari.encounter_constraint.v1";
pub const ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION: &str =
    "singulari.encounter_change_potential.v1";
pub const CHOICE_CONTRACT_SCHEMA_VERSION: &str = "singulari.choice_contract.v1";
pub const ENCOUNTER_SURFACE_EVENT_SCHEMA_VERSION: &str = "singulari.encounter_surface_event.v1";
pub const ENCOUNTER_PROPOSAL_SCHEMA_VERSION: &str = "singulari.encounter_proposal.v1";
pub const ENCOUNTER_SURFACE_EVENTS_FILENAME: &str = "encounter_events.jsonl";
pub const ENCOUNTER_SURFACE_FILENAME: &str = "encounter_surface.json";

const ACTIVE_SURFACE_BUDGET: usize = 12;
const RECENT_CHANGE_BUDGET: usize = 12;
const BLOCKED_INTERACTION_BUDGET: usize = 8;
const REQUIRED_FOLLOWUP_BUDGET: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurfacePacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub scene_id: String,
    #[serde(default)]
    pub active_surfaces: Vec<EncounterSurface>,
    #[serde(default)]
    pub recent_surface_changes: Vec<EncounterSurfaceChange>,
    #[serde(default)]
    pub blocked_interactions: Vec<BlockedInteraction>,
    #[serde(default)]
    pub required_followups: Vec<String>,
    #[serde(default)]
    pub choice_contracts: Vec<ChoiceContract>,
    pub compiler_policy: EncounterSurfacePolicy,
}

impl Default for EncounterSurfacePacket {
    fn default() -> Self {
        Self {
            schema_version: ENCOUNTER_SURFACE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            scene_id: String::new(),
            active_surfaces: Vec::new(),
            recent_surface_changes: Vec::new(),
            blocked_interactions: Vec::new(),
            required_followups: Vec::new(),
            choice_contracts: Vec::new(),
            compiler_policy: EncounterSurfacePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurfacePolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for EncounterSurfacePolicy {
    fn default() -> Self {
        Self {
            source: "materialized_from_encounter_events_and_visible_choices_v1".to_owned(),
            use_rules: vec![
                "Encounter surfaces are current-scene interaction handles, not puzzle solutions."
                    .to_owned(),
                "Use active_surfaces to ground concrete choices, object handling, social access, and probe actions.".to_owned(),
                "HiddenButSignaled surfaces may expose only the visible signal, never the hidden content.".to_owned(),
                "Player-visible fields must not contain hidden adjudication-only facts.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurface {
    pub schema_version: String,
    pub surface_id: String,
    pub label: String,
    pub kind: EncounterSurfaceKind,
    pub status: EncounterSurfaceStatus,
    pub salience: EncounterSalience,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_ref: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub linked_entity_refs: Vec<String>,
    #[serde(default)]
    pub linked_pressure_refs: Vec<String>,
    #[serde(default)]
    pub linked_social_refs: Vec<String>,
    #[serde(default)]
    pub physical_anchors: Vec<EncounterPhysicalAnchor>,
    #[serde(default)]
    pub affordances: Vec<EncounterAffordance>,
    #[serde(default)]
    pub constraints: Vec<EncounterConstraint>,
    #[serde(default)]
    pub change_potential: Vec<EncounterChangePotential>,
    pub lifecycle: EncounterSurfaceLifecycle,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterPhysicalAnchor {
    pub anchor_ref: String,
    pub kind: EncounterPhysicalAnchorKind,
    pub relation: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EncounterPhysicalAnchorKind {
    CurrentLocation,
    NearbyLocation,
    BodyConstraint,
    Resource,
    Actor,
    SocialExchange,
    ChoiceSlot,
    Gate,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EncounterSurfaceKind {
    Barrier,
    AccessController,
    EvidenceTrace,
    MovableObject,
    UsableTool,
    Container,
    Hazard,
    Exit,
    HidingPlace,
    SocialHandle,
    EnvironmentalFeature,
    TimeSensitiveCue,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncounterSurfaceStatus {
    Available,
    Blocked,
    Locked,
    HiddenButSignaled,
    Degraded,
    ClaimedByActor,
    Moving,
    Exhausted,
    Resolved,
    Gone,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EncounterSalience {
    Background,
    Useful,
    Important,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterAffordance {
    pub schema_version: String,
    pub affordance_id: String,
    pub action_kind: EncounterActionKind,
    pub label_seed: String,
    pub intent_seed: String,
    pub availability: AffordanceAvailability,
    #[serde(default)]
    pub required_refs: Vec<String>,
    #[serde(default)]
    pub risk_tags: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum EncounterActionKind {
    Inspect,
    Touch,
    Move,
    Open,
    Close,
    Force,
    Repair,
    Break,
    Take,
    Use,
    TalkAbout,
    TradeOver,
    ThreatenWith,
    HideBehind,
    Follow,
    Wait,
    Listen,
    Smell,
    Compare,
    Mark,
    Bypass,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AffordanceAvailability {
    Available,
    RequiresCondition,
    Risky,
    Blocked,
    UnknownNeedsProbe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterConstraint {
    pub schema_version: String,
    pub constraint_id: String,
    pub kind: EncounterConstraintKind,
    pub summary: String,
    pub visible_reason: String,
    #[serde(default)]
    pub unblock_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncounterConstraintKind {
    Body,
    Resource,
    Tool,
    Knowledge,
    SocialPermission,
    TimePressure,
    Noise,
    Visibility,
    ActorOpposition,
    WorldLaw,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterChangePotential {
    pub schema_version: String,
    pub change_id: String,
    pub kind: EncounterChangeKind,
    pub summary: String,
    pub likely_scope: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoiceContract {
    pub schema_version: String,
    pub choice_id: String,
    pub target_ref: String,
    pub verb: EncounterActionKind,
    pub visible_label: String,
    #[serde(default)]
    pub preconditions: Vec<String>,
    pub expected_cost: ChoiceTimeCost,
    #[serde(default)]
    pub risk_tags: Vec<String>,
    #[serde(default)]
    pub possible_event_kinds: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default)]
    pub forbidden_outcomes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub omission_profile: Option<OmissionProfile>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceTimeCost {
    None,
    Moment,
    Exchange,
    SceneBeat,
    TravelStep,
    LongWait,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncounterChangeKind {
    RevealDetail,
    ChangeAccess,
    ConsumeSurface,
    MoveSurface,
    DamageSurface,
    CreateNoise,
    ShiftActorStance,
    AdvanceClock,
    ProduceResource,
    DestroyEvidence,
    OpenExit,
    CloseExit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurfaceLifecycle {
    pub opened_turn_id: String,
    pub last_changed_turn_id: String,
    pub persistence: EncounterPersistence,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncounterPersistence {
    CurrentBeat,
    CurrentScene,
    UntilChanged,
    SearchOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurfaceChange {
    pub surface_id: String,
    pub summary: String,
    pub turn_id: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockedInteraction {
    pub surface_id: String,
    pub action_kind: EncounterActionKind,
    pub reason: String,
    #[serde(default)]
    pub unblock_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterProposal {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub mutations: Vec<EncounterSurfaceMutation>,
    #[serde(default)]
    pub closures: Vec<EncounterSurfaceClosure>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurfaceMutation {
    pub surface_id: String,
    pub label: String,
    pub kind: EncounterSurfaceKind,
    pub status: EncounterSurfaceStatus,
    pub salience: EncounterSalience,
    pub summary: String,
    pub player_visible_signal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub holder_ref: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub linked_entity_refs: Vec<String>,
    #[serde(default)]
    pub linked_pressure_refs: Vec<String>,
    #[serde(default)]
    pub linked_social_refs: Vec<String>,
    #[serde(default)]
    pub physical_anchors: Vec<EncounterPhysicalAnchor>,
    #[serde(default)]
    pub affordances: Vec<EncounterAffordance>,
    #[serde(default)]
    pub constraints: Vec<EncounterConstraint>,
    #[serde(default)]
    pub change_potential: Vec<EncounterChangePotential>,
    pub persistence: EncounterPersistence,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurfaceClosure {
    pub surface_id: String,
    pub kind: EncounterClosureKind,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_surface_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EncounterClosureKind {
    Resolved,
    Exhausted,
    Destroyed,
    MovedAway,
    SceneTransition,
    Superseded,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncounterSurfaceEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation: Option<EncounterSurfaceMutation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closure: Option<EncounterSurfaceClosure>,
    pub recorded_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EncounterSurfaceEventPlan {
    pub records: Vec<EncounterSurfaceEventRecord>,
}

#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn compile_encounter_surface_packet(
    world_id: &str,
    turn_id: &str,
    scene_id: &str,
    choices: &[TurnChoice],
    scene_pressure: &ScenePressurePacket,
    location_graph: &LocationGraphPacket,
    body_resource: &BodyResourcePacket,
    social_exchange: &SocialExchangePacket,
) -> EncounterSurfacePacket {
    let mut surfaces = choices
        .iter()
        .filter(|choice| (1..=5).contains(&choice.slot))
        .map(|choice| {
            surface_from_choice(turn_id, scene_id, choice, scene_pressure, location_graph)
        })
        .collect::<Vec<_>>();
    surfaces.extend(social_surfaces(turn_id, scene_id, social_exchange));
    surfaces.extend(location_surfaces(turn_id, scene_id, location_graph));
    surfaces.extend(body_constraint_surfaces(turn_id, scene_id, body_resource));
    surfaces.extend(resource_surfaces(turn_id, scene_id, body_resource));
    surfaces.sort_by(|left, right| {
        right
            .salience
            .cmp(&left.salience)
            .then(left.surface_id.cmp(&right.surface_id))
    });
    surfaces.truncate(ACTIVE_SURFACE_BUDGET);
    let choice_contracts = compile_choice_contracts(&surfaces);
    EncounterSurfacePacket {
        schema_version: ENCOUNTER_SURFACE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        scene_id: scene_id.to_owned(),
        active_surfaces: surfaces,
        recent_surface_changes: Vec::new(),
        blocked_interactions: Vec::new(),
        required_followups: Vec::new(),
        choice_contracts,
        compiler_policy: EncounterSurfacePolicy::default(),
    }
}

pub fn prepare_encounter_surface_event_plan(
    current: &EncounterSurfacePacket,
    response: &AgentTurnResponse,
) -> Result<EncounterSurfaceEventPlan> {
    let mut records = Vec::new();
    let recorded_at = Utc::now().to_rfc3339();
    let mut event_context = current.clone();
    let mut known_ids = current
        .active_surfaces
        .iter()
        .map(|surface| surface.surface_id.as_str())
        .collect::<BTreeSet<_>>();
    if let Some(proposal) = &response.encounter_proposal {
        validate_encounter_proposal(current, response, proposal)?;
        event_context.turn_id.clone_from(&proposal.turn_id);
        for mutation in &proposal.mutations {
            records.push(record_from_mutation(
                &event_context,
                mutation.clone(),
                records.len(),
                &recorded_at,
            ));
            known_ids.insert(mutation.surface_id.as_str());
        }
        for closure in &proposal.closures {
            if !known_ids.contains(closure.surface_id.as_str()) {
                bail!(
                    "encounter closure references unknown surface_id: {}",
                    closure.surface_id
                );
            }
            records.push(record_from_closure(
                &event_context,
                closure.clone(),
                records.len(),
                &recorded_at,
            ));
        }
    }
    for closure in derive_scene_transition_closures(&event_context, response) {
        records.push(record_from_closure(
            &event_context,
            closure,
            records.len(),
            &recorded_at,
        ));
    }
    let existing_refs = records
        .iter()
        .filter_map(|record| record.mutation.as_ref())
        .flat_map(|mutation| mutation.source_refs.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    for mutation in derive_response_choice_mutations(current, response, &existing_refs) {
        records.push(record_from_mutation(
            &event_context,
            mutation,
            records.len(),
            &recorded_at,
        ));
    }
    for mutation in derive_resolution_gate_mutations(current, response, records.len()) {
        records.push(record_from_mutation(
            &event_context,
            mutation,
            records.len(),
            &recorded_at,
        ));
    }
    Ok(EncounterSurfaceEventPlan { records })
}

fn derive_scene_transition_closures(
    current: &EncounterSurfacePacket,
    response: &AgentTurnResponse,
) -> Vec<EncounterSurfaceClosure> {
    let Some(proposal) = &response.scene_director_proposal else {
        return Vec::new();
    };
    let Some(transition) = &proposal.transition else {
        return Vec::new();
    };
    current
        .active_surfaces
        .iter()
        .filter(|surface| {
            matches!(
                surface.lifecycle.persistence,
                EncounterPersistence::CurrentBeat | EncounterPersistence::CurrentScene
            )
        })
        .map(|surface| EncounterSurfaceClosure {
            surface_id: surface.surface_id.clone(),
            kind: EncounterClosureKind::SceneTransition,
            summary: format!(
                "Scene transition closed the current interaction surface: {}",
                transition.transition_reason
            ),
            evidence_refs: proposal.evidence_refs.clone(),
            replacement_surface_id: None,
        })
        .collect()
}

pub fn append_encounter_surface_event_plan(
    world_dir: &Path,
    plan: &EncounterSurfaceEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(ENCOUNTER_SURFACE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub fn rebuild_encounter_surface(
    world_dir: &Path,
    base_packet: &EncounterSurfacePacket,
) -> Result<EncounterSurfacePacket> {
    let records = load_encounter_surface_event_records(world_dir)?;
    let mut active_by_id = base_packet
        .active_surfaces
        .iter()
        .map(|surface| (surface.surface_id.clone(), surface.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut changes = Vec::new();
    let mut blocked = Vec::new();
    let mut required_followups = Vec::new();
    for record in records {
        if let Some(closure) = record.closure {
            active_by_id.remove(closure.surface_id.as_str());
            changes.push(EncounterSurfaceChange {
                surface_id: closure.surface_id.clone(),
                summary: closure.summary,
                turn_id: record.turn_id.clone(),
                evidence_refs: closure.evidence_refs,
            });
            if let Some(replacement) = closure.replacement_surface_id {
                required_followups.push(format!("replacement_surface:{replacement}"));
            }
        }
        if let Some(mutation) = record.mutation {
            for constraint in &mutation.constraints {
                if matches!(
                    mutation.status,
                    EncounterSurfaceStatus::Blocked | EncounterSurfaceStatus::Locked
                ) {
                    blocked.push(BlockedInteraction {
                        surface_id: mutation.surface_id.clone(),
                        action_kind: mutation
                            .affordances
                            .first()
                            .map_or(EncounterActionKind::Inspect, |affordance| {
                                affordance.action_kind
                            }),
                        reason: constraint.visible_reason.clone(),
                        unblock_refs: constraint.unblock_refs.clone(),
                        evidence_refs: constraint.evidence_refs.clone(),
                    });
                }
            }
            changes.push(EncounterSurfaceChange {
                surface_id: mutation.surface_id.clone(),
                summary: mutation.summary.clone(),
                turn_id: record.turn_id.clone(),
                evidence_refs: mutation.source_refs.clone(),
            });
            active_by_id.insert(
                mutation.surface_id.clone(),
                EncounterSurface {
                    schema_version: ENCOUNTER_SURFACE_SCHEMA_VERSION.to_owned(),
                    surface_id: mutation.surface_id,
                    label: mutation.label,
                    kind: mutation.kind,
                    status: mutation.status,
                    salience: mutation.salience,
                    summary: mutation.summary,
                    player_visible_signal: mutation.player_visible_signal,
                    location_ref: mutation.location_ref,
                    holder_ref: mutation.holder_ref,
                    source_refs: mutation.source_refs,
                    linked_entity_refs: mutation.linked_entity_refs,
                    linked_pressure_refs: mutation.linked_pressure_refs,
                    linked_social_refs: mutation.linked_social_refs,
                    physical_anchors: mutation.physical_anchors,
                    affordances: mutation.affordances,
                    constraints: mutation.constraints,
                    change_potential: mutation.change_potential,
                    lifecycle: EncounterSurfaceLifecycle {
                        opened_turn_id: record.turn_id.clone(),
                        last_changed_turn_id: record.turn_id,
                        persistence: mutation.persistence,
                    },
                },
            );
        }
    }
    let mut active_surfaces = active_by_id
        .into_values()
        .filter(|surface| {
            !matches!(
                surface.status,
                EncounterSurfaceStatus::Gone
                    | EncounterSurfaceStatus::Resolved
                    | EncounterSurfaceStatus::Exhausted
            )
        })
        .collect::<Vec<_>>();
    active_surfaces.sort_by(|left, right| {
        right
            .salience
            .cmp(&left.salience)
            .then(left.surface_id.cmp(&right.surface_id))
    });
    active_surfaces.truncate(ACTIVE_SURFACE_BUDGET);
    changes.reverse();
    changes.truncate(RECENT_CHANGE_BUDGET);
    blocked.reverse();
    blocked.truncate(BLOCKED_INTERACTION_BUDGET);
    required_followups.sort();
    required_followups.dedup();
    required_followups.truncate(REQUIRED_FOLLOWUP_BUDGET);
    let choice_contracts = compile_choice_contracts(&active_surfaces);
    let packet = EncounterSurfacePacket {
        schema_version: ENCOUNTER_SURFACE_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: base_packet.world_id.clone(),
        turn_id: base_packet.turn_id.clone(),
        scene_id: base_packet.scene_id.clone(),
        active_surfaces,
        recent_surface_changes: changes,
        blocked_interactions: blocked,
        required_followups,
        choice_contracts,
        compiler_policy: EncounterSurfacePolicy::default(),
    };
    write_json(&world_dir.join(ENCOUNTER_SURFACE_FILENAME), &packet)?;
    Ok(packet)
}

pub fn load_encounter_surface_state(
    world_dir: &Path,
    fallback: EncounterSurfacePacket,
) -> Result<EncounterSurfacePacket> {
    let path = world_dir.join(ENCOUNTER_SURFACE_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(fallback)
}

fn validate_encounter_proposal(
    current: &EncounterSurfacePacket,
    response: &AgentTurnResponse,
    proposal: &EncounterProposal,
) -> Result<()> {
    if proposal.schema_version != ENCOUNTER_PROPOSAL_SCHEMA_VERSION {
        bail!("encounter proposal schema_version mismatch");
    }
    let next_turn_id = encounter_next_turn_id(current.turn_id.as_str()).ok();
    let turn_matches = proposal.turn_id == current.turn_id
        || next_turn_id
            .as_deref()
            .is_some_and(|turn_id| proposal.turn_id == turn_id)
        || proposal.turn_id == response.turn_id;
    if proposal.world_id != current.world_id || !turn_matches {
        bail!("encounter proposal world_id/turn_id mismatch");
    }
    let visible_refs = response_visible_refs(response);
    for mutation in &proposal.mutations {
        if mutation.surface_id.trim().is_empty()
            || mutation.label.trim().is_empty()
            || mutation.summary.trim().is_empty()
            || mutation.player_visible_signal.trim().is_empty()
        {
            bail!("encounter mutation requires id, label, summary, and visible signal");
        }
        require_visible_refs(&visible_refs, "encounter mutation", &mutation.source_refs)?;
        for affordance in &mutation.affordances {
            require_visible_refs(
                &visible_refs,
                "encounter affordance",
                &affordance.evidence_refs,
            )?;
            if affordance.label_seed.trim().is_empty() || affordance.intent_seed.trim().is_empty() {
                bail!("encounter affordance requires label_seed and intent_seed");
            }
        }
        for constraint in &mutation.constraints {
            require_visible_refs(
                &visible_refs,
                "encounter constraint",
                &constraint.evidence_refs,
            )?;
            if constraint.visible_reason.trim().is_empty() {
                bail!("encounter constraint requires visible_reason");
            }
        }
    }
    for closure in &proposal.closures {
        require_visible_refs(&visible_refs, "encounter closure", &closure.evidence_refs)?;
        if closure.summary.trim().is_empty() {
            bail!("encounter closure requires summary");
        }
        if closure.kind == EncounterClosureKind::Superseded
            && closure.replacement_surface_id.is_none()
        {
            bail!("superseded encounter closure requires replacement_surface_id");
        }
    }
    Ok(())
}

fn encounter_next_turn_id(turn_id: &str) -> Result<String> {
    let suffix = turn_id
        .strip_prefix("turn_")
        .context("encounter current turn_id missing turn_ prefix")?;
    let index: u64 = suffix
        .parse()
        .context("encounter current turn_id suffix is not numeric")?;
    Ok(format!("turn_{:04}", index + 1))
}

fn derive_response_choice_mutations(
    current: &EncounterSurfacePacket,
    response: &AgentTurnResponse,
    existing_refs: &BTreeSet<&str>,
) -> Vec<EncounterSurfaceMutation> {
    response
        .next_choices
        .iter()
        .filter(|choice| (1..=5).contains(&choice.slot))
        .filter_map(|choice| {
            let source_ref = format!("next_choices[slot={}]", choice.slot);
            if existing_refs.contains(source_ref.as_str()) {
                return None;
            }
            Some(mutation_from_choice(
                current.turn_id.as_str(),
                current.scene_id.as_str(),
                choice,
                &[],
                vec![source_ref],
            ))
        })
        .collect()
}

fn surface_from_choice(
    turn_id: &str,
    scene_id: &str,
    choice: &TurnChoice,
    scene_pressure: &ScenePressurePacket,
    location_graph: &LocationGraphPacket,
) -> EncounterSurface {
    let pressure_refs = pressure_refs_for_choice(choice, scene_pressure);
    let source_refs = vec![format!("next_choices[slot={}]", choice.slot)];
    let location_ref = location_graph
        .current_location
        .as_ref()
        .map(|location| location.location_id.clone())
        .or_else(|| Some(scene_id.to_owned()));
    let mutation = mutation_from_choice(turn_id, scene_id, choice, &pressure_refs, source_refs);
    let mut physical_anchors = choice_physical_anchors(choice, location_graph);
    physical_anchors.extend(mutation.physical_anchors.clone());
    dedupe_physical_anchors(&mut physical_anchors);
    EncounterSurface {
        schema_version: ENCOUNTER_SURFACE_SCHEMA_VERSION.to_owned(),
        surface_id: mutation.surface_id,
        label: mutation.label,
        kind: mutation.kind,
        status: mutation.status,
        salience: mutation.salience,
        summary: mutation.summary,
        player_visible_signal: mutation.player_visible_signal,
        location_ref,
        holder_ref: mutation.holder_ref,
        source_refs: mutation.source_refs,
        linked_entity_refs: mutation.linked_entity_refs,
        linked_pressure_refs: mutation.linked_pressure_refs,
        linked_social_refs: mutation.linked_social_refs,
        physical_anchors,
        affordances: mutation.affordances,
        constraints: mutation.constraints,
        change_potential: mutation.change_potential,
        lifecycle: EncounterSurfaceLifecycle {
            opened_turn_id: turn_id.to_owned(),
            last_changed_turn_id: turn_id.to_owned(),
            persistence: EncounterPersistence::CurrentScene,
        },
    }
}

fn mutation_from_choice(
    turn_id: &str,
    scene_id: &str,
    choice: &TurnChoice,
    pressure_refs: &[String],
    source_refs: Vec<String>,
) -> EncounterSurfaceMutation {
    let text = format!("{} {}", choice.tag, choice.intent).to_ascii_lowercase();
    let kind = kind_from_choice_text(choice.slot, text.as_str());
    let action_kind = action_from_choice_text(choice.slot, text.as_str());
    let status = status_from_choice_text(text.as_str());
    let salience = if choice.slot == 5 || !pressure_refs.is_empty() {
        EncounterSalience::Important
    } else {
        EncounterSalience::Useful
    };
    let surface_id = format!(
        "encounter:{}:slot:{}:{}",
        turn_id,
        choice.slot,
        normalize_id(choice.tag.as_str())
    );
    EncounterSurfaceMutation {
        surface_id: surface_id.clone(),
        label: choice.tag.clone(),
        kind,
        status,
        salience,
        summary: choice.intent.clone(),
        player_visible_signal: choice.tag.clone(),
        location_ref: Some(scene_id.to_owned()),
        holder_ref: None,
        source_refs: if source_refs.is_empty() {
            vec![format!("next_choices[slot={}]", choice.slot)]
        } else {
            source_refs
        },
        linked_entity_refs: Vec::new(),
        linked_pressure_refs: pressure_refs.to_owned(),
        linked_social_refs: Vec::new(),
        physical_anchors: vec![EncounterPhysicalAnchor {
            anchor_ref: format!("next_choices[slot={}]", choice.slot),
            kind: EncounterPhysicalAnchorKind::ChoiceSlot,
            relation: "choice_surface_source".to_owned(),
            evidence_refs: vec![format!("next_choices[slot={}]", choice.slot)],
        }],
        affordances: vec![EncounterAffordance {
            schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
            affordance_id: format!("{surface_id}:affordance"),
            action_kind,
            label_seed: choice.tag.clone(),
            intent_seed: choice.intent.clone(),
            availability: availability_from_status(status),
            required_refs: Vec::new(),
            risk_tags: risk_tags_from_kind(kind, pressure_refs),
            evidence_refs: vec![format!("next_choices[slot={}]", choice.slot)],
        }],
        constraints: constraint_from_choice_status(&surface_id, status),
        change_potential: vec![change_from_action(&surface_id, action_kind)],
        persistence: EncounterPersistence::CurrentScene,
    }
}

fn compile_choice_contracts(surfaces: &[EncounterSurface]) -> Vec<ChoiceContract> {
    surfaces
        .iter()
        .flat_map(|surface| {
            surface
                .affordances
                .iter()
                .map(|affordance| choice_contract_from_affordance(surface, affordance))
        })
        .collect()
}

fn choice_contract_from_affordance(
    surface: &EncounterSurface,
    affordance: &EncounterAffordance,
) -> ChoiceContract {
    ChoiceContract {
        schema_version: CHOICE_CONTRACT_SCHEMA_VERSION.to_owned(),
        choice_id: affordance.affordance_id.clone(),
        target_ref: surface.surface_id.clone(),
        verb: affordance.action_kind,
        visible_label: affordance.label_seed.clone(),
        preconditions: affordance.required_refs.clone(),
        expected_cost: expected_cost_for_action(affordance.action_kind),
        risk_tags: affordance.risk_tags.clone(),
        possible_event_kinds: possible_event_kinds_for_action(affordance.action_kind),
        evidence_refs: affordance.evidence_refs.clone(),
        forbidden_outcomes: forbidden_outcomes_for_action(affordance.action_kind),
        omission_profile: Some(omission_profile_for_choice(
            choice_slot_from_surface(surface, affordance),
            affordance.label_seed.as_str(),
            affordance.intent_seed.as_str(),
            affordance.risk_tags.as_slice(),
            affordance.evidence_refs.as_slice(),
        )),
    }
}

fn choice_slot_from_surface(surface: &EncounterSurface, affordance: &EncounterAffordance) -> u8 {
    surface
        .source_refs
        .iter()
        .chain(affordance.evidence_refs.iter())
        .find_map(|source_ref| choice_slot_from_ref(source_ref))
        .unwrap_or(0)
}

fn choice_slot_from_ref(source_ref: &str) -> Option<u8> {
    let slot_text = source_ref
        .strip_prefix("next_choices[slot=")?
        .strip_suffix(']')?;
    slot_text.parse::<u8>().ok()
}

const fn expected_cost_for_action(action: EncounterActionKind) -> ChoiceTimeCost {
    match action {
        EncounterActionKind::Wait => ChoiceTimeCost::LongWait,
        EncounterActionKind::Move | EncounterActionKind::Follow | EncounterActionKind::Bypass => {
            ChoiceTimeCost::TravelStep
        }
        EncounterActionKind::TalkAbout
        | EncounterActionKind::TradeOver
        | EncounterActionKind::ThreatenWith => ChoiceTimeCost::Exchange,
        EncounterActionKind::Open
        | EncounterActionKind::Force
        | EncounterActionKind::Repair
        | EncounterActionKind::Break
        | EncounterActionKind::Take
        | EncounterActionKind::Use
        | EncounterActionKind::HideBehind => ChoiceTimeCost::SceneBeat,
        EncounterActionKind::Inspect
        | EncounterActionKind::Touch
        | EncounterActionKind::Close
        | EncounterActionKind::Listen
        | EncounterActionKind::Smell
        | EncounterActionKind::Compare
        | EncounterActionKind::Mark => ChoiceTimeCost::Moment,
    }
}

fn possible_event_kinds_for_action(action: EncounterActionKind) -> Vec<String> {
    match action {
        EncounterActionKind::Inspect
        | EncounterActionKind::Listen
        | EncounterActionKind::Smell
        | EncounterActionKind::Compare => {
            vec![
                "knowledge_observed".to_owned(),
                "surface_state_revealed".to_owned(),
            ]
        }
        EncounterActionKind::TalkAbout
        | EncounterActionKind::TradeOver
        | EncounterActionKind::ThreatenWith => {
            vec![
                "dialogue_exchange".to_owned(),
                "relationship_changed".to_owned(),
            ]
        }
        EncounterActionKind::Move | EncounterActionKind::Follow | EncounterActionKind::Bypass => {
            vec![
                "entity_moved".to_owned(),
                "location_access_changed".to_owned(),
            ]
        }
        EncounterActionKind::Wait => {
            vec!["process_ticked".to_owned()]
        }
        EncounterActionKind::Open
        | EncounterActionKind::Close
        | EncounterActionKind::Force
        | EncounterActionKind::Repair
        | EncounterActionKind::Break
        | EncounterActionKind::Touch
        | EncounterActionKind::Take
        | EncounterActionKind::Use
        | EncounterActionKind::HideBehind
        | EncounterActionKind::Mark => {
            vec!["surface_state_changed".to_owned()]
        }
    }
}

fn forbidden_outcomes_for_action(action: EncounterActionKind) -> Vec<String> {
    let mut outcomes = vec!["invent_hidden_truth_without_evidence".to_owned()];
    if matches!(
        action,
        EncounterActionKind::Inspect
            | EncounterActionKind::Listen
            | EncounterActionKind::Smell
            | EncounterActionKind::Compare
    ) {
        outcomes.push("reveal_hidden_truth_directly".to_owned());
    }
    if matches!(
        action,
        EncounterActionKind::Open | EncounterActionKind::Force
    ) {
        outcomes.push("unlock_without_tool_or_permission".to_owned());
    }
    outcomes
}

fn choice_physical_anchors(
    choice: &TurnChoice,
    location_graph: &LocationGraphPacket,
) -> Vec<EncounterPhysicalAnchor> {
    let mut anchors = Vec::new();
    let evidence_refs = vec![format!("next_choices[slot={}]", choice.slot)];
    anchors.push(EncounterPhysicalAnchor {
        anchor_ref: format!("next_choices[slot={}]", choice.slot),
        kind: EncounterPhysicalAnchorKind::ChoiceSlot,
        relation: "choice_surface_source".to_owned(),
        evidence_refs: evidence_refs.clone(),
    });
    if let Some(current_location) = &location_graph.current_location {
        anchors.push(EncounterPhysicalAnchor {
            anchor_ref: current_location.location_id.clone(),
            kind: EncounterPhysicalAnchorKind::CurrentLocation,
            relation: "choice_scene_origin".to_owned(),
            evidence_refs: location_evidence_refs(current_location),
        });
    }

    let choice_text = format!("{} {}", choice.tag, choice.intent);
    for nearby in &location_graph.known_nearby_locations {
        if choice_text.contains(nearby.name.as_str())
            || choice_text.contains(nearby.location_id.as_str())
        {
            anchors.push(EncounterPhysicalAnchor {
                anchor_ref: nearby.location_id.clone(),
                kind: EncounterPhysicalAnchorKind::NearbyLocation,
                relation: "choice_movement_target".to_owned(),
                evidence_refs: location_evidence_refs(nearby),
            });
        }
    }
    anchors
}

fn location_physical_anchors(
    current_location_ref: &str,
    destination: &LocationNode,
    evidence_refs: &[String],
) -> Vec<EncounterPhysicalAnchor> {
    vec![
        EncounterPhysicalAnchor {
            anchor_ref: current_location_ref.to_owned(),
            kind: EncounterPhysicalAnchorKind::CurrentLocation,
            relation: "movement_origin".to_owned(),
            evidence_refs: evidence_refs.to_owned(),
        },
        EncounterPhysicalAnchor {
            anchor_ref: destination.location_id.clone(),
            kind: EncounterPhysicalAnchorKind::NearbyLocation,
            relation: "movement_destination".to_owned(),
            evidence_refs: evidence_refs.to_owned(),
        },
    ]
}

fn location_evidence_refs(location: &LocationNode) -> Vec<String> {
    if location.source_refs.is_empty() {
        vec![format!("location_graph:{}", location.location_id)]
    } else {
        location.source_refs.clone()
    }
}

fn dedupe_physical_anchors(anchors: &mut Vec<EncounterPhysicalAnchor>) {
    let mut seen = BTreeSet::new();
    anchors.retain(|anchor| {
        seen.insert((
            anchor.anchor_ref.clone(),
            anchor.kind,
            anchor.relation.clone(),
        ))
    });
}

fn location_surfaces(
    turn_id: &str,
    scene_id: &str,
    location_graph: &LocationGraphPacket,
) -> Vec<EncounterSurface> {
    let current_location = location_graph
        .current_location
        .as_ref()
        .map_or(scene_id, |location| location.location_id.as_str());
    location_graph
        .known_nearby_locations
        .iter()
        .map(|location| {
            let surface_id = format!(
                "encounter:{turn_id}:location:{}",
                normalize_id(&location.location_id)
            );
            let evidence_refs = location_evidence_refs(location);
            let physical_anchors =
                location_physical_anchors(current_location, location, &evidence_refs);
            EncounterSurface {
                schema_version: ENCOUNTER_SURFACE_SCHEMA_VERSION.to_owned(),
                surface_id: surface_id.clone(),
                label: location.name.clone(),
                kind: EncounterSurfaceKind::Exit,
                status: EncounterSurfaceStatus::Available,
                salience: EncounterSalience::Useful,
                summary: format!("현재 장면에서 도달 가능한 인접 위치: {}", location.name),
                player_visible_signal: location.name.clone(),
                location_ref: Some(current_location.to_owned()),
                holder_ref: None,
                source_refs: evidence_refs.clone(),
                linked_entity_refs: Vec::new(),
                linked_pressure_refs: Vec::new(),
                linked_social_refs: Vec::new(),
                physical_anchors: physical_anchors.clone(),
                affordances: vec![EncounterAffordance {
                    schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
                    affordance_id: format!("{surface_id}:move"),
                    action_kind: EncounterActionKind::Move,
                    label_seed: location.name.clone(),
                    intent_seed: format!("{} 쪽으로 이동한다", location.name),
                    availability: AffordanceAvailability::Available,
                    required_refs: vec![location.location_id.clone()],
                    risk_tags: Vec::new(),
                    evidence_refs: evidence_refs.clone(),
                }],
                constraints: Vec::new(),
                change_potential: vec![EncounterChangePotential {
                    schema_version: ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION.to_owned(),
                    change_id: format!("{surface_id}:move"),
                    kind: EncounterChangeKind::MoveSurface,
                    summary: "플레이어 위치가 바뀔 수 있다.".to_owned(),
                    likely_scope: "current_scene_or_next_scene".to_owned(),
                    evidence_refs,
                }],
                lifecycle: EncounterSurfaceLifecycle {
                    opened_turn_id: turn_id.to_owned(),
                    last_changed_turn_id: turn_id.to_owned(),
                    persistence: EncounterPersistence::CurrentScene,
                },
            }
        })
        .collect()
}

fn body_constraint_surfaces(
    turn_id: &str,
    scene_id: &str,
    body_resource: &BodyResourcePacket,
) -> Vec<EncounterSurface> {
    body_resource
        .body_constraints
        .iter()
        .take(4)
        .map(|constraint| {
            let surface_id = format!(
                "encounter:{turn_id}:body:{}",
                normalize_id(&constraint.constraint_id)
            );
            let physical_anchors = vec![EncounterPhysicalAnchor {
                anchor_ref: constraint.constraint_id.clone(),
                kind: EncounterPhysicalAnchorKind::BodyConstraint,
                relation: "player_body_state".to_owned(),
                evidence_refs: constraint.source_refs.clone(),
            }];
            EncounterSurface {
                schema_version: ENCOUNTER_SURFACE_SCHEMA_VERSION.to_owned(),
                surface_id: surface_id.clone(),
                label: constraint.summary.clone(),
                kind: EncounterSurfaceKind::Hazard,
                status: EncounterSurfaceStatus::Degraded,
                salience: if constraint.severity >= 3 {
                    EncounterSalience::Important
                } else {
                    EncounterSalience::Useful
                },
                summary: constraint.summary.clone(),
                player_visible_signal: constraint.summary.clone(),
                location_ref: Some(scene_id.to_owned()),
                holder_ref: Some("player".to_owned()),
                source_refs: constraint.source_refs.clone(),
                linked_entity_refs: vec!["player".to_owned()],
                linked_pressure_refs: constraint.scene_pressure_kinds.clone(),
                linked_social_refs: Vec::new(),
                physical_anchors,
                affordances: vec![EncounterAffordance {
                    schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
                    affordance_id: format!("{surface_id}:adjust"),
                    action_kind: EncounterActionKind::Inspect,
                    label_seed: constraint.summary.clone(),
                    intent_seed: "몸 상태가 현재 행동에 어떤 제약을 주는지 확인한다".to_owned(),
                    availability: AffordanceAvailability::Risky,
                    required_refs: vec![constraint.constraint_id.clone()],
                    risk_tags: vec!["body".to_owned()],
                    evidence_refs: constraint.source_refs.clone(),
                }],
                constraints: vec![EncounterConstraint {
                    schema_version: ENCOUNTER_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                    constraint_id: format!("{surface_id}:constraint"),
                    kind: EncounterConstraintKind::Body,
                    summary: constraint.summary.clone(),
                    visible_reason: constraint.summary.clone(),
                    unblock_refs: Vec::new(),
                    evidence_refs: constraint.source_refs.clone(),
                }],
                change_potential: vec![EncounterChangePotential {
                    schema_version: ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION.to_owned(),
                    change_id: format!("{surface_id}:cost"),
                    kind: EncounterChangeKind::AdvanceClock,
                    summary: "몸 상태가 행동 비용이나 위험을 바꿀 수 있다.".to_owned(),
                    likely_scope: "current_action".to_owned(),
                    evidence_refs: constraint.source_refs.clone(),
                }],
                lifecycle: EncounterSurfaceLifecycle {
                    opened_turn_id: turn_id.to_owned(),
                    last_changed_turn_id: turn_id.to_owned(),
                    persistence: EncounterPersistence::UntilChanged,
                },
            }
        })
        .collect()
}

#[allow(clippy::too_many_lines)]
fn social_surfaces(
    turn_id: &str,
    scene_id: &str,
    social_exchange: &SocialExchangePacket,
) -> Vec<EncounterSurface> {
    let mut surfaces = Vec::new();
    for stance in &social_exchange.active_stances {
        let surface_id = format!(
            "encounter:{turn_id}:social:{}",
            normalize_id(&stance.stance_id)
        );
        surfaces.push(EncounterSurface {
            schema_version: ENCOUNTER_SURFACE_SCHEMA_VERSION.to_owned(),
            surface_id: surface_id.clone(),
            label: "대화 표면".to_owned(),
            kind: EncounterSurfaceKind::SocialHandle,
            status: EncounterSurfaceStatus::Available,
            salience: EncounterSalience::Important,
            summary: stance.summary.clone(),
            player_visible_signal: stance.player_visible_signal.clone(),
            location_ref: Some(scene_id.to_owned()),
            holder_ref: Some(stance.actor_ref.clone()),
            source_refs: stance.source_refs.clone(),
            linked_entity_refs: vec![stance.actor_ref.clone(), stance.target_ref.clone()],
            linked_pressure_refs: Vec::new(),
            linked_social_refs: vec![stance.stance_id.clone()],
            physical_anchors: vec![
                EncounterPhysicalAnchor {
                    anchor_ref: stance.actor_ref.clone(),
                    kind: EncounterPhysicalAnchorKind::Actor,
                    relation: "stance_actor".to_owned(),
                    evidence_refs: stance.source_refs.clone(),
                },
                EncounterPhysicalAnchor {
                    anchor_ref: stance.stance_id.clone(),
                    kind: EncounterPhysicalAnchorKind::SocialExchange,
                    relation: "relationship_stance".to_owned(),
                    evidence_refs: stance.source_refs.clone(),
                },
            ],
            affordances: vec![EncounterAffordance {
                schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
                affordance_id: format!("{surface_id}:talk"),
                action_kind: EncounterActionKind::TalkAbout,
                label_seed: "말을 건다".to_owned(),
                intent_seed: stance.player_visible_signal.clone(),
                availability: AffordanceAvailability::Risky,
                required_refs: Vec::new(),
                risk_tags: vec!["social".to_owned()],
                evidence_refs: stance.source_refs.clone(),
            }],
            constraints: Vec::new(),
            change_potential: vec![EncounterChangePotential {
                schema_version: ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION.to_owned(),
                change_id: format!("{surface_id}:stance"),
                kind: EncounterChangeKind::ShiftActorStance,
                summary: "대화 태도가 바뀔 수 있다.".to_owned(),
                likely_scope: "current_scene".to_owned(),
                evidence_refs: stance.source_refs.clone(),
            }],
            lifecycle: EncounterSurfaceLifecycle {
                opened_turn_id: turn_id.to_owned(),
                last_changed_turn_id: turn_id.to_owned(),
                persistence: EncounterPersistence::CurrentScene,
            },
        });
    }
    for ask in &social_exchange.unresolved_asks {
        let surface_id = format!("encounter:{turn_id}:ask:{}", normalize_id(&ask.ask_id));
        let blocked =
            ask.current_status == AskStatus::Evaded || ask.current_status == AskStatus::Refused;
        surfaces.push(EncounterSurface {
            schema_version: ENCOUNTER_SURFACE_SCHEMA_VERSION.to_owned(),
            surface_id: surface_id.clone(),
            label: "유보된 질문".to_owned(),
            kind: EncounterSurfaceKind::AccessController,
            status: if blocked {
                EncounterSurfaceStatus::Blocked
            } else {
                EncounterSurfaceStatus::Available
            },
            salience: EncounterSalience::Important,
            summary: ask.question_summary.clone(),
            player_visible_signal: ask.last_response.clone(),
            location_ref: Some(scene_id.to_owned()),
            holder_ref: Some(ask.asked_to_ref.clone()),
            source_refs: ask.source_refs.clone(),
            linked_entity_refs: vec![ask.asked_by_ref.clone(), ask.asked_to_ref.clone()],
            linked_pressure_refs: Vec::new(),
            linked_social_refs: vec![ask.ask_id.clone()],
            physical_anchors: vec![
                EncounterPhysicalAnchor {
                    anchor_ref: ask.asked_to_ref.clone(),
                    kind: EncounterPhysicalAnchorKind::Actor,
                    relation: "ask_controller".to_owned(),
                    evidence_refs: ask.source_refs.clone(),
                },
                EncounterPhysicalAnchor {
                    anchor_ref: ask.ask_id.clone(),
                    kind: EncounterPhysicalAnchorKind::SocialExchange,
                    relation: "unresolved_ask".to_owned(),
                    evidence_refs: ask.source_refs.clone(),
                },
            ],
            affordances: vec![EncounterAffordance {
                schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
                affordance_id: format!("{surface_id}:probe"),
                action_kind: EncounterActionKind::TalkAbout,
                label_seed: "다시 묻는다".to_owned(),
                intent_seed: ask.question_summary.clone(),
                availability: if blocked {
                    AffordanceAvailability::RequiresCondition
                } else {
                    AffordanceAvailability::Available
                },
                required_refs: ask.allowed_next_moves.clone(),
                risk_tags: vec!["repetition".to_owned(), "social".to_owned()],
                evidence_refs: ask.source_refs.clone(),
            }],
            constraints: ask
                .blocked_repetitions
                .iter()
                .enumerate()
                .map(|(index, reason)| EncounterConstraint {
                    schema_version: ENCOUNTER_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                    constraint_id: format!("{surface_id}:repeat:{index}"),
                    kind: EncounterConstraintKind::SocialPermission,
                    summary: reason.clone(),
                    visible_reason: reason.clone(),
                    unblock_refs: ask.allowed_next_moves.clone(),
                    evidence_refs: ask.source_refs.clone(),
                })
                .collect(),
            change_potential: vec![EncounterChangePotential {
                schema_version: ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION.to_owned(),
                change_id: format!("{surface_id}:answer"),
                kind: EncounterChangeKind::RevealDetail,
                summary: "조건이 맞으면 답변 일부가 드러날 수 있다.".to_owned(),
                likely_scope: "current_scene".to_owned(),
                evidence_refs: ask.source_refs.clone(),
            }],
            lifecycle: EncounterSurfaceLifecycle {
                opened_turn_id: turn_id.to_owned(),
                last_changed_turn_id: turn_id.to_owned(),
                persistence: EncounterPersistence::CurrentScene,
            },
        });
    }
    surfaces
}

fn resource_surfaces(
    turn_id: &str,
    scene_id: &str,
    body_resource: &BodyResourcePacket,
) -> Vec<EncounterSurface> {
    body_resource
        .resources
        .iter()
        .take(4)
        .map(|resource| {
            let surface_id = format!(
                "encounter:{turn_id}:resource:{}",
                normalize_id(&resource.resource_id)
            );
            let resource_label = resource.summary.clone();
            EncounterSurface {
                schema_version: ENCOUNTER_SURFACE_SCHEMA_VERSION.to_owned(),
                surface_id: surface_id.clone(),
                label: resource_label.clone(),
                kind: EncounterSurfaceKind::UsableTool,
                status: EncounterSurfaceStatus::Available,
                salience: EncounterSalience::Useful,
                summary: resource.summary.clone(),
                player_visible_signal: resource_label.clone(),
                location_ref: Some(scene_id.to_owned()),
                holder_ref: Some("player".to_owned()),
                source_refs: resource.source_refs.clone(),
                linked_entity_refs: vec!["player".to_owned()],
                linked_pressure_refs: Vec::new(),
                linked_social_refs: Vec::new(),
                physical_anchors: vec![EncounterPhysicalAnchor {
                    anchor_ref: resource.resource_id.clone(),
                    kind: EncounterPhysicalAnchorKind::Resource,
                    relation: "held_resource".to_owned(),
                    evidence_refs: resource.source_refs.clone(),
                }],
                affordances: vec![EncounterAffordance {
                    schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
                    affordance_id: format!("{surface_id}:use"),
                    action_kind: EncounterActionKind::Use,
                    label_seed: resource_label,
                    intent_seed: resource.summary.clone(),
                    availability: AffordanceAvailability::Available,
                    required_refs: vec![resource.resource_id.clone()],
                    risk_tags: Vec::new(),
                    evidence_refs: resource.source_refs.clone(),
                }],
                constraints: Vec::new(),
                change_potential: vec![EncounterChangePotential {
                    schema_version: ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION.to_owned(),
                    change_id: format!("{surface_id}:consume"),
                    kind: EncounterChangeKind::ConsumeSurface,
                    summary: "자원을 쓰거나 잃을 수 있다.".to_owned(),
                    likely_scope: "current_scene".to_owned(),
                    evidence_refs: resource.source_refs.clone(),
                }],
                lifecycle: EncounterSurfaceLifecycle {
                    opened_turn_id: turn_id.to_owned(),
                    last_changed_turn_id: turn_id.to_owned(),
                    persistence: EncounterPersistence::UntilChanged,
                },
            }
        })
        .collect()
}

fn record_from_mutation(
    current: &EncounterSurfacePacket,
    mutation: EncounterSurfaceMutation,
    index: usize,
    recorded_at: &str,
) -> EncounterSurfaceEventRecord {
    EncounterSurfaceEventRecord {
        schema_version: ENCOUNTER_SURFACE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("encounter_event:{}:{index:02}", current.turn_id),
        mutation: Some(mutation),
        closure: None,
        recorded_at: recorded_at.to_owned(),
    }
}

fn record_from_closure(
    current: &EncounterSurfacePacket,
    closure: EncounterSurfaceClosure,
    index: usize,
    recorded_at: &str,
) -> EncounterSurfaceEventRecord {
    EncounterSurfaceEventRecord {
        schema_version: ENCOUNTER_SURFACE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: current.world_id.clone(),
        turn_id: current.turn_id.clone(),
        event_id: format!("encounter_event:{}:{index:02}", current.turn_id),
        mutation: None,
        closure: Some(closure),
        recorded_at: recorded_at.to_owned(),
    }
}

fn load_encounter_surface_event_records(
    world_dir: &Path,
) -> Result<Vec<EncounterSurfaceEventRecord>> {
    let path = world_dir.join(ENCOUNTER_SURFACE_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<EncounterSurfaceEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn pressure_refs_for_choice(
    choice: &TurnChoice,
    scene_pressure: &ScenePressurePacket,
) -> Vec<String> {
    let text = format!("{} {}", choice.tag, choice.intent);
    scene_pressure
        .visible_active
        .iter()
        .filter(|pressure| {
            pressure
                .observable_signals
                .iter()
                .any(|signal| text.contains(signal.as_str()))
                || pressure
                    .choice_affordances
                    .iter()
                    .any(|affordance| text.contains(affordance.as_str()))
                || text.contains(pressure.prose_effect.paragraph_pressure.as_str())
                || matches!(
                    pressure.kind,
                    ScenePressureKind::Threat
                        | ScenePressureKind::TimePressure
                        | ScenePressureKind::SocialPermission
                ) && choice.slot == 5
        })
        .map(|pressure| pressure.pressure_id.clone())
        .collect()
}

fn kind_from_choice_text(slot: u8, text: &str) -> EncounterSurfaceKind {
    if contains_any(text, &["말", "묻", "대화", "설득", "거래", "협상"]) {
        EncounterSurfaceKind::SocialHandle
    } else if contains_any(text, &["문", "길", "통로", "출구", "넘어", "들어", "이동"]) || slot == 1
    {
        EncounterSurfaceKind::Exit
    } else if contains_any(text, &["흔적", "단서", "확인", "살핀", "관찰", "조사"]) || slot == 2
    {
        EncounterSurfaceKind::EvidenceTrace
    } else if contains_any(text, &["도구", "소지", "손", "사용", "꺼내", "쥐"]) || slot == 4
    {
        EncounterSurfaceKind::UsableTool
    } else if contains_any(text, &["위험", "공격", "피", "숨", "막", "압박"]) || slot == 5
    {
        EncounterSurfaceKind::Hazard
    } else {
        EncounterSurfaceKind::EnvironmentalFeature
    }
}

fn action_from_choice_text(slot: u8, text: &str) -> EncounterActionKind {
    if contains_any(text, &["말", "묻", "대화", "설득", "거래"]) {
        EncounterActionKind::TalkAbout
    } else if contains_any(text, &["열", "문"]) {
        EncounterActionKind::Open
    } else if contains_any(text, &["부수", "밀", "억지", "강행"]) {
        EncounterActionKind::Force
    } else if contains_any(text, &["따라"]) {
        EncounterActionKind::Follow
    } else if contains_any(text, &["기다", "버틴"]) {
        EncounterActionKind::Wait
    } else if contains_any(text, &["듣"]) {
        EncounterActionKind::Listen
    } else if slot == 1 {
        EncounterActionKind::Move
    } else if slot == 2 {
        EncounterActionKind::Inspect
    } else if slot == 4 {
        EncounterActionKind::Use
    } else if slot == 5 {
        EncounterActionKind::Bypass
    } else {
        EncounterActionKind::Inspect
    }
}

fn status_from_choice_text(text: &str) -> EncounterSurfaceStatus {
    if contains_any(text, &["막", "불가", "닫혀", "잠겨", "조건"]) {
        EncounterSurfaceStatus::Blocked
    } else if contains_any(text, &["숨", "기척", "희미", "흔적"]) {
        EncounterSurfaceStatus::HiddenButSignaled
    } else {
        EncounterSurfaceStatus::Available
    }
}

fn availability_from_status(status: EncounterSurfaceStatus) -> AffordanceAvailability {
    match status {
        EncounterSurfaceStatus::Available => AffordanceAvailability::Available,
        EncounterSurfaceStatus::Blocked | EncounterSurfaceStatus::Locked => {
            AffordanceAvailability::Blocked
        }
        EncounterSurfaceStatus::HiddenButSignaled => AffordanceAvailability::UnknownNeedsProbe,
        EncounterSurfaceStatus::Degraded | EncounterSurfaceStatus::ClaimedByActor => {
            AffordanceAvailability::Risky
        }
        _ => AffordanceAvailability::RequiresCondition,
    }
}

fn constraint_from_choice_status(
    surface_id: &str,
    status: EncounterSurfaceStatus,
) -> Vec<EncounterConstraint> {
    if !matches!(
        status,
        EncounterSurfaceStatus::Blocked | EncounterSurfaceStatus::Locked
    ) {
        return Vec::new();
    }
    vec![EncounterConstraint {
        schema_version: ENCOUNTER_CONSTRAINT_SCHEMA_VERSION.to_owned(),
        constraint_id: format!("{surface_id}:constraint"),
        kind: EncounterConstraintKind::Knowledge,
        summary: "아직 조건이 충분히 열리지 않았다.".to_owned(),
        visible_reason: "먼저 확인하거나 조건을 바꿔야 한다.".to_owned(),
        unblock_refs: Vec::new(),
        evidence_refs: Vec::new(),
    }]
}

fn change_from_action(
    surface_id: &str,
    action_kind: EncounterActionKind,
) -> EncounterChangePotential {
    let (kind, summary) = match action_kind {
        EncounterActionKind::Inspect
        | EncounterActionKind::Listen
        | EncounterActionKind::Smell
        | EncounterActionKind::Compare => (
            EncounterChangeKind::RevealDetail,
            "조사하면 세부 단서가 드러날 수 있다.",
        ),
        EncounterActionKind::Open | EncounterActionKind::Bypass => {
            (EncounterChangeKind::OpenExit, "접근 경로가 열릴 수 있다.")
        }
        EncounterActionKind::Force | EncounterActionKind::Break => (
            EncounterChangeKind::CreateNoise,
            "강행하면 소음이나 손상이 생길 수 있다.",
        ),
        EncounterActionKind::TalkAbout
        | EncounterActionKind::TradeOver
        | EncounterActionKind::ThreatenWith => (
            EncounterChangeKind::ShiftActorStance,
            "상대 태도나 허가 조건이 바뀔 수 있다.",
        ),
        EncounterActionKind::Take | EncounterActionKind::Use => (
            EncounterChangeKind::ConsumeSurface,
            "자원 상태가 바뀔 수 있다.",
        ),
        _ => (
            EncounterChangeKind::AdvanceClock,
            "시간과 장면 압력이 조금 움직일 수 있다.",
        ),
    };
    EncounterChangePotential {
        schema_version: ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION.to_owned(),
        change_id: format!("{surface_id}:change"),
        kind,
        summary: summary.to_owned(),
        likely_scope: "current_scene".to_owned(),
        evidence_refs: Vec::new(),
    }
}

fn risk_tags_from_kind(kind: EncounterSurfaceKind, pressure_refs: &[String]) -> Vec<String> {
    let mut tags = Vec::new();
    if matches!(
        kind,
        EncounterSurfaceKind::Hazard | EncounterSurfaceKind::Barrier
    ) {
        tags.push("danger".to_owned());
    }
    if matches!(
        kind,
        EncounterSurfaceKind::SocialHandle | EncounterSurfaceKind::AccessController
    ) {
        tags.push("social".to_owned());
    }
    if !pressure_refs.is_empty() {
        tags.push("pressure_linked".to_owned());
    }
    tags
}

fn response_visible_refs(response: &AgentTurnResponse) -> BTreeSet<String> {
    let mut refs = BTreeSet::new();
    for (index, block) in response.visible_scene.text_blocks.iter().enumerate() {
        refs.insert(format!("visible_scene.text_blocks[{index}]"));
        if !block.trim().is_empty() {
            refs.insert(block.clone());
        }
    }
    for choice in &response.next_choices {
        refs.insert(format!("next_choices[slot={}]", choice.slot));
        refs.insert(choice.tag.clone());
        refs.insert(choice.intent.clone());
    }
    if let Some(canon_event) = &response.canon_event {
        refs.insert(canon_event.summary.clone());
    }
    if let Some(resolution) = &response.resolution_proposal {
        refs.extend(resolution.outcome.evidence_refs.iter().cloned());
        for gate in &resolution.gate_results {
            if gate.visibility == ResolutionVisibility::PlayerVisible {
                refs.extend(gate.evidence_refs.iter().cloned());
            }
        }
    }
    refs
}

fn require_visible_refs(
    visible_refs: &BTreeSet<String>,
    label: &str,
    refs: &[String],
) -> Result<()> {
    if refs.is_empty() {
        bail!("{label} requires evidence/source refs");
    }
    let missing = refs
        .iter()
        .filter(|reference| !visible_refs.contains(reference.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        bail!(
            "{label} references non-visible refs: {}",
            missing.join(", ")
        );
    }
    Ok(())
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn normalize_id(text: &str) -> String {
    let normalized = text
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else if ch.is_whitespace() || matches!(ch, '-' | '_' | ':' | '/') {
                '-'
            } else {
                '_'
            }
        })
        .collect::<String>();
    let compact = normalized
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if compact.is_empty() {
        "surface".to_owned()
    } else {
        compact.chars().take(48).collect()
    }
}

#[must_use]
pub fn encounter_status(packet: &EncounterSurfacePacket) -> String {
    if packet
        .active_surfaces
        .iter()
        .any(|surface| surface.salience >= EncounterSalience::Critical)
    {
        return "치명 상호작용 표면 있음".to_owned();
    }
    if packet.active_surfaces.iter().any(|surface| {
        matches!(
            surface.status,
            EncounterSurfaceStatus::Blocked | EncounterSurfaceStatus::Locked
        )
    }) {
        return "막힌 표면을 우회해야 함".to_owned();
    }
    if packet.active_surfaces.iter().any(|surface| {
        surface.kind == EncounterSurfaceKind::EvidenceTrace
            || surface.status == EncounterSurfaceStatus::HiddenButSignaled
    }) {
        return "조사 가능한 표면 있음".to_owned();
    }
    if packet.active_surfaces.is_empty() {
        "뚜렷한 조작 표면 없음".to_owned()
    } else {
        "상호작용 표면 열림".to_owned()
    }
}

#[must_use]
pub fn mutation_from_resolution_gate(
    current: &EncounterSurfacePacket,
    gate_kind: GateKind,
    gate_status: GateStatus,
    gate_ref: &str,
    reason: &str,
    evidence_refs: &[String],
    index: usize,
) -> EncounterSurfaceMutation {
    let surface_id = format!(
        "encounter:{}:gate:{}:{}",
        current.turn_id,
        index,
        normalize_id(gate_ref)
    );
    let kind = match gate_kind {
        GateKind::SocialPermission => EncounterSurfaceKind::AccessController,
        GateKind::Location | GateKind::TimePressure => EncounterSurfaceKind::Barrier,
        GateKind::Knowledge => EncounterSurfaceKind::EvidenceTrace,
        GateKind::Resource => EncounterSurfaceKind::UsableTool,
        GateKind::Body => EncounterSurfaceKind::Hazard,
        GateKind::WorldLaw | GateKind::HiddenConstraint | GateKind::Affordance => {
            EncounterSurfaceKind::EnvironmentalFeature
        }
    };
    let status = match gate_status {
        GateStatus::Passed | GateStatus::Softened => EncounterSurfaceStatus::Available,
        GateStatus::Blocked => EncounterSurfaceStatus::Blocked,
        GateStatus::CostImposed => EncounterSurfaceStatus::Degraded,
        GateStatus::UnknownNeedsProbe => EncounterSurfaceStatus::HiddenButSignaled,
    };
    EncounterSurfaceMutation {
        surface_id: surface_id.clone(),
        label: gate_ref.to_owned(),
        kind,
        status,
        salience: EncounterSalience::Important,
        summary: reason.to_owned(),
        player_visible_signal: reason.to_owned(),
        location_ref: Some(current.scene_id.clone()),
        holder_ref: None,
        source_refs: evidence_refs.to_vec(),
        linked_entity_refs: Vec::new(),
        linked_pressure_refs: Vec::new(),
        linked_social_refs: Vec::new(),
        physical_anchors: vec![EncounterPhysicalAnchor {
            anchor_ref: gate_ref.to_owned(),
            kind: EncounterPhysicalAnchorKind::Gate,
            relation: "resolution_gate".to_owned(),
            evidence_refs: evidence_refs.to_vec(),
        }],
        affordances: vec![EncounterAffordance {
            schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
            affordance_id: format!("{surface_id}:probe"),
            action_kind: EncounterActionKind::Inspect,
            label_seed: gate_ref.to_owned(),
            intent_seed: reason.to_owned(),
            availability: availability_from_status(status),
            required_refs: Vec::new(),
            risk_tags: risk_tags_from_kind(kind, &[]),
            evidence_refs: evidence_refs.to_vec(),
        }],
        constraints: if matches!(
            status,
            EncounterSurfaceStatus::Blocked | EncounterSurfaceStatus::Locked
        ) {
            vec![EncounterConstraint {
                schema_version: ENCOUNTER_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                constraint_id: format!("{surface_id}:constraint"),
                kind: match gate_kind {
                    GateKind::SocialPermission => EncounterConstraintKind::SocialPermission,
                    GateKind::Resource => EncounterConstraintKind::Resource,
                    GateKind::Body => EncounterConstraintKind::Body,
                    GateKind::Knowledge => EncounterConstraintKind::Knowledge,
                    GateKind::TimePressure => EncounterConstraintKind::TimePressure,
                    GateKind::WorldLaw => EncounterConstraintKind::WorldLaw,
                    _ => EncounterConstraintKind::Visibility,
                },
                summary: reason.to_owned(),
                visible_reason: reason.to_owned(),
                unblock_refs: Vec::new(),
                evidence_refs: evidence_refs.to_vec(),
            }]
        } else {
            Vec::new()
        },
        change_potential: vec![EncounterChangePotential {
            schema_version: ENCOUNTER_CHANGE_POTENTIAL_SCHEMA_VERSION.to_owned(),
            change_id: format!("{surface_id}:change"),
            kind: if matches!(gate_status, GateStatus::Passed | GateStatus::Softened) {
                EncounterChangeKind::ChangeAccess
            } else {
                EncounterChangeKind::RevealDetail
            },
            summary: "조건을 건드리면 접근 상태가 바뀔 수 있다.".to_owned(),
            likely_scope: "current_scene".to_owned(),
            evidence_refs: evidence_refs.to_vec(),
        }],
        persistence: EncounterPersistence::CurrentScene,
    }
}

#[must_use]
pub fn derive_resolution_gate_mutations(
    current: &EncounterSurfacePacket,
    response: &AgentTurnResponse,
    offset: usize,
) -> Vec<EncounterSurfaceMutation> {
    let Some(resolution) = &response.resolution_proposal else {
        return Vec::new();
    };
    resolution
        .gate_results
        .iter()
        .enumerate()
        .filter(|(_, gate)| gate.visibility == ResolutionVisibility::PlayerVisible)
        .filter(|(_, gate)| !gate.evidence_refs.is_empty())
        .map(|(index, gate)| {
            mutation_from_resolution_gate(
                current,
                gate.gate_kind,
                gate.status,
                gate.gate_ref.as_str(),
                gate.reason.as_str(),
                &gate.evidence_refs,
                offset + index,
            )
        })
        .chain(
            if matches!(
                resolution.outcome.kind,
                ResolutionOutcomeKind::Blocked | ResolutionOutcomeKind::Delayed
            ) && !resolution.outcome.evidence_refs.is_empty()
            {
                Some(mutation_from_resolution_gate(
                    current,
                    GateKind::Affordance,
                    GateStatus::Blocked,
                    "scene:resolution_outcome",
                    resolution.outcome.summary.as_str(),
                    &resolution.outcome.evidence_refs,
                    offset + resolution.gate_results.len(),
                ))
            } else {
                None
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::{AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentResponseCanonEvent};
    use crate::models::{NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene};
    use tempfile::tempdir;

    #[test]
    fn compiles_choice_surfaces_from_current_scene() {
        let packet = compile_encounter_surface_packet(
            "world",
            "turn_0002",
            "place:gate",
            &[
                TurnChoice {
                    slot: 1,
                    tag: "문 쪽으로 다가간다".to_owned(),
                    intent: "닫히기 전 통로를 확인한다".to_owned(),
                },
                TurnChoice {
                    slot: 2,
                    tag: "흔적을 살핀다".to_owned(),
                    intent: "바닥에 남은 단서를 조사한다".to_owned(),
                },
            ],
            &ScenePressurePacket::default(),
            &LocationGraphPacket::default(),
            &BodyResourcePacket::default(),
            &SocialExchangePacket::default(),
        );

        assert_eq!(packet.active_surfaces.len(), 2);
        assert!(
            packet
                .active_surfaces
                .iter()
                .any(|surface| surface.kind == EncounterSurfaceKind::EvidenceTrace)
        );
        assert_eq!(packet.choice_contracts.len(), 2);
        let Some(inspect_contract) = packet
            .choice_contracts
            .iter()
            .find(|contract| contract.verb == EncounterActionKind::Inspect)
        else {
            panic!("inspect choice contract should exist");
        };
        assert_eq!(inspect_contract.expected_cost, ChoiceTimeCost::Moment);
        assert!(
            inspect_contract
                .possible_event_kinds
                .iter()
                .any(|kind| kind == "knowledge_observed")
        );
        assert!(
            inspect_contract
                .forbidden_outcomes
                .iter()
                .any(|outcome| outcome == "reveal_hidden_truth_directly")
        );
    }

    #[test]
    fn compiles_physical_location_and_body_surfaces() {
        let location_graph = LocationGraphPacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0002".to_owned(),
            current_location: Some(LocationNode {
                schema_version: crate::location_graph::LOCATION_NODE_SCHEMA_VERSION.to_owned(),
                location_id: "place:gate".to_owned(),
                name: "북문".to_owned(),
                knowledge_state: crate::location_graph::LocationKnowledgeState::Visited,
                notes: Vec::new(),
                source_refs: vec!["location_graph.current_location".to_owned()],
            }),
            known_nearby_locations: vec![LocationNode {
                schema_version: crate::location_graph::LOCATION_NODE_SCHEMA_VERSION.to_owned(),
                location_id: "place:courtyard".to_owned(),
                name: "안뜰".to_owned(),
                knowledge_state: crate::location_graph::LocationKnowledgeState::Known,
                notes: Vec::new(),
                source_refs: vec!["location_graph.known_nearby_locations[0]".to_owned()],
            }],
            ..LocationGraphPacket::default()
        };
        let body_resource = BodyResourcePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0002".to_owned(),
            body_constraints: vec![crate::body_resource::BodyConstraint {
                schema_version: crate::body_resource::BODY_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                constraint_id: "body:constraint:left_hand_numb".to_owned(),
                visibility: crate::body_resource::BodyResourceVisibility::PlayerVisible,
                summary: "왼손이 저리다".to_owned(),
                severity: 3,
                source_refs: vec!["latest_snapshot.protagonist_state.body[0]".to_owned()],
                scene_pressure_kinds: vec!["body".to_owned()],
            }],
            ..BodyResourcePacket::default()
        };
        let packet = compile_encounter_surface_packet(
            "world",
            "turn_0002",
            "place:gate",
            &[TurnChoice {
                slot: 1,
                tag: "안뜰 쪽으로 움직인다".to_owned(),
                intent: "북문에서 안뜰로 들어갈 길을 확인한다".to_owned(),
            }],
            &ScenePressurePacket::default(),
            &location_graph,
            &body_resource,
            &SocialExchangePacket::default(),
        );

        let Some(location_surface) = packet
            .active_surfaces
            .iter()
            .find(|surface| surface.surface_id == "encounter:turn_0002:location:place-courtyard")
        else {
            panic!("nearby location should compile into an exit surface");
        };
        assert_eq!(location_surface.kind, EncounterSurfaceKind::Exit);
        assert!(location_surface.physical_anchors.iter().any(|anchor| {
            anchor.kind == EncounterPhysicalAnchorKind::NearbyLocation
                && anchor.anchor_ref == "place:courtyard"
        }));

        let Some(choice_surface) = packet
            .active_surfaces
            .iter()
            .find(|surface| surface.surface_id.contains(":slot:1:"))
        else {
            panic!("choice surface should compile");
        };
        assert!(choice_surface.physical_anchors.iter().any(|anchor| {
            anchor.kind == EncounterPhysicalAnchorKind::NearbyLocation
                && anchor.anchor_ref == "place:courtyard"
                && anchor.relation == "choice_movement_target"
        }));

        let Some(body_surface) = packet
            .active_surfaces
            .iter()
            .find(|surface| surface.kind == EncounterSurfaceKind::Hazard)
        else {
            panic!("body constraint should compile into a hazard surface");
        };
        assert!(body_surface.physical_anchors.iter().any(|anchor| {
            anchor.kind == EncounterPhysicalAnchorKind::BodyConstraint
                && anchor.anchor_ref == "body:constraint:left_hand_numb"
        }));
        assert_eq!(packet.choice_contracts.len(), packet.active_surfaces.len());
    }

    #[test]
    fn proposal_materializes_surface() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let base = EncounterSurfacePacket {
            world_id: "world".to_owned(),
            turn_id: "turn_0002".to_owned(),
            scene_id: "place:gate".to_owned(),
            ..EncounterSurfacePacket::default()
        };
        let mut response = sample_response();
        response.encounter_proposal = Some(EncounterProposal {
            schema_version: ENCOUNTER_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0002".to_owned(),
            mutations: vec![EncounterSurfaceMutation {
                surface_id: "encounter:turn_0002:trace:mud".to_owned(),
                label: "진흙 자국".to_owned(),
                kind: EncounterSurfaceKind::EvidenceTrace,
                status: EncounterSurfaceStatus::Available,
                salience: EncounterSalience::Important,
                summary: "문 아래에 진흙이 끌린 흔적이 남았다.".to_owned(),
                player_visible_signal: "문 아래 진흙 자국".to_owned(),
                location_ref: Some("place:gate".to_owned()),
                holder_ref: None,
                source_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                linked_entity_refs: Vec::new(),
                linked_pressure_refs: Vec::new(),
                linked_social_refs: Vec::new(),
                physical_anchors: Vec::new(),
                affordances: vec![EncounterAffordance {
                    schema_version: ENCOUNTER_AFFORDANCE_SCHEMA_VERSION.to_owned(),
                    affordance_id: "encounter:turn_0002:trace:mud:inspect".to_owned(),
                    action_kind: EncounterActionKind::Inspect,
                    label_seed: "자국을 본다".to_owned(),
                    intent_seed: "진흙이 어디서 왔는지 살핀다".to_owned(),
                    availability: AffordanceAvailability::Available,
                    required_refs: Vec::new(),
                    risk_tags: Vec::new(),
                    evidence_refs: vec!["visible_scene.text_blocks[0]".to_owned()],
                }],
                constraints: Vec::new(),
                change_potential: Vec::new(),
                persistence: EncounterPersistence::CurrentScene,
            }],
            closures: Vec::new(),
        });

        let plan = prepare_encounter_surface_event_plan(&base, &response)?;
        append_encounter_surface_event_plan(temp.path(), &plan)?;
        let rebuilt = rebuild_encounter_surface(temp.path(), &base)?;

        assert!(temp.path().join(ENCOUNTER_SURFACE_FILENAME).is_file());
        assert!(
            rebuilt
                .active_surfaces
                .iter()
                .any(|surface| surface.surface_id == "encounter:turn_0002:trace:mud")
        );
        Ok(())
    }

    fn sample_response() -> AgentTurnResponse {
        AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "world".to_owned(),
            turn_id: "turn_0002".to_owned(),
            resolution_proposal: None,
            scene_director_proposal: None,
            consequence_proposal: None,
            social_exchange_proposal: None,
            encounter_proposal: None,
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec!["문 아래에 진흙이 길게 끌려 있다.".to_owned()],
                tone_notes: Vec::new(),
            },
            adjudication: None,
            canon_event: Some(AgentResponseCanonEvent {
                visibility: "player_visible".to_owned(),
                kind: "observation".to_owned(),
                summary: "문 아래의 진흙 자국을 확인했다.".to_owned(),
            }),
            entity_updates: Vec::new(),
            relationship_updates: Vec::new(),
            plot_thread_events: Vec::new(),
            scene_pressure_events: Vec::new(),
            world_lore_updates: Vec::new(),
            character_text_design_updates: Vec::new(),
            body_resource_events: Vec::new(),
            location_events: Vec::new(),
            extra_contacts: Vec::new(),
            hidden_state_delta: Vec::new(),
            needs_context: Vec::new(),
            next_choices: vec![
                TurnChoice {
                    slot: 1,
                    tag: "문으로 다가간다".to_owned(),
                    intent: "닫힌 문과 주변 틈을 확인한다".to_owned(),
                },
                TurnChoice {
                    slot: 2,
                    tag: "진흙 자국을 살핀다".to_owned(),
                    intent: "자국의 방향을 따라 단서를 확인한다".to_owned(),
                },
                TurnChoice {
                    slot: 3,
                    tag: "문지기에게 묻는다".to_owned(),
                    intent: "방금 지나간 사람을 봤는지 묻는다".to_owned(),
                },
                TurnChoice {
                    slot: 4,
                    tag: "손에 묻은 흙을 비교한다".to_owned(),
                    intent: "가지고 있는 흔적과 바닥 자국을 비교한다".to_owned(),
                },
                TurnChoice {
                    slot: 5,
                    tag: "닫히는 시간을 재촉한다".to_owned(),
                    intent: "더 늦기 전에 빠르게 결정한다".to_owned(),
                },
                TurnChoice {
                    slot: 6,
                    tag: "자유서술".to_owned(),
                    intent: "플레이어가 원하는 행동과 말, 내면 독백을 직접 서술한다".to_owned(),
                },
                TurnChoice {
                    slot: 7,
                    tag: "판단 위임".to_owned(),
                    intent: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
                },
            ],
            actor_goal_events: Vec::new(),
            actor_move_events: Vec::new(),
            hook_events: Vec::new(),
        }
    }
}
