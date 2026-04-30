use crate::affordance_graph::{AffordanceGraphPacket, AffordanceKind};
use crate::agent_bridge::{AgentPrivateAdjudicationContext, PendingAgentChoice, PendingAgentTurn};
use crate::body_resource::BodyResourcePacket;
use crate::consequence_spine::ConsequenceSpinePacket;
use crate::encounter_surface::EncounterSurfacePacket;
use crate::location_graph::LocationGraphPacket;
use crate::models::{FREEFORM_CHOICE_SLOT, GUIDE_CHOICE_SLOT, INITIAL_TURN_ID, TurnInputKind};
use crate::relationship_graph::RelationshipGraphPacket;
use crate::scene_pressure::{ScenePressureKind, ScenePressurePacket};
use crate::world_process_clock::{WorldProcessClockPacket, WorldProcessTempo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION: &str = "singulari.pre_turn_simulation_pass.v1";
pub const SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION: &str = "singulari.simulation_source_bundle.v1";

#[derive(Debug, Clone)]
pub struct SimulationSourceBundle {
    pub pending: PendingAgentTurn,
    pub affordance_graph: AffordanceGraphPacket,
    pub body_resource_state: BodyResourcePacket,
    pub location_graph: LocationGraphPacket,
    pub scene_pressure: ScenePressurePacket,
    pub world_process_clock: WorldProcessClockPacket,
    pub relationship_graph: RelationshipGraphPacket,
    pub consequence_spine: ConsequenceSpinePacket,
    pub encounter_surface: EncounterSurfacePacket,
    pub private_adjudication_context: AgentPrivateAdjudicationContext,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreTurnSimulationPass {
    pub schema_version: String,
    pub source_bundle_schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub player_input: String,
    pub input_kind: TurnInputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_choice: Option<PendingAgentChoice>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub available_affordances: Vec<CompiledAffordance>,
    #[serde(default)]
    pub blocked_affordances: Vec<BlockedAffordance>,
    #[serde(default)]
    pub pressure_obligations: Vec<PressureObligation>,
    #[serde(default)]
    pub due_processes: Vec<DueProcess>,
    #[serde(default)]
    pub causal_risks: Vec<CausalRisk>,
    pub required_resolution_fields: RequiredResolutionFields,
    pub hidden_visibility_boundary: HiddenVisibilityBoundary,
    pub compiler_policy: PreTurnSimulationPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompiledAffordance {
    pub slot: u8,
    pub affordance_id: String,
    pub affordance_kind: AffordanceKind,
    pub action_contract: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockedAffordance {
    pub slot: u8,
    pub affordance_id: String,
    #[serde(default)]
    pub forbidden_shortcuts: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PressureObligation {
    pub pressure_id: String,
    pub kind: ScenePressureKind,
    pub obligation: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DueProcess {
    pub process_id: String,
    pub visibility: SimulationVisibility,
    pub tempo: WorldProcessTempo,
    pub tick_condition: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SimulationVisibility {
    PlayerVisible,
    AdjudicationOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CausalRisk {
    pub risk_kind: CausalRiskKind,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CausalRiskKind {
    MissingPressure,
    HiddenLeak,
    ResourceShortcut,
    LocationShortcut,
    SocialShortcut,
    ProcessOvertick,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequiredResolutionFields {
    pub resolution_proposal_required: bool,
    pub next_choice_plan_required: bool,
    pub pressure_movement_or_noop_reason_required: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HiddenVisibilityBoundary {
    pub hidden_timer_count: usize,
    pub unrevealed_constraint_count: usize,
    #[serde(default)]
    pub forbidden_visible_needles: Vec<String>,
    pub render_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PreTurnSimulationPolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for PreTurnSimulationPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_simulation_source_bundle_v1".to_owned(),
            use_rules: vec![
                "PreTurnSimulationPass is a rebuildable compiler artifact, not canonical world truth.".to_owned(),
                "Visible affordances bound slots 1..5 before the text backend rewrites labels.".to_owned(),
                "Adjudication-only processes may shape outcomes but must not be copied into player-visible prose.".to_owned(),
                "Normal player turns should include a ResolutionProposal that satisfies pressure obligations or states a no-op reason.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn compile_pre_turn_simulation_pass(source: &SimulationSourceBundle) -> PreTurnSimulationPass {
    let input_kind = classify_input_kind(
        source.pending.player_input.as_str(),
        source.pending.selected_choice.as_ref(),
    );
    let available_affordances = source
        .affordance_graph
        .ordinary_choice_slots
        .iter()
        .map(|affordance| CompiledAffordance {
            slot: affordance.slot,
            affordance_id: affordance.affordance_id.clone(),
            affordance_kind: affordance.affordance_kind,
            action_contract: affordance.action_contract.clone(),
            source_refs: affordance.source_refs.clone(),
            pressure_refs: affordance.pressure_refs.clone(),
        })
        .collect::<Vec<_>>();
    let blocked_affordances = source
        .affordance_graph
        .ordinary_choice_slots
        .iter()
        .filter(|affordance| !affordance.forbidden_shortcuts.is_empty())
        .map(|affordance| BlockedAffordance {
            slot: affordance.slot,
            affordance_id: affordance.affordance_id.clone(),
            forbidden_shortcuts: affordance.forbidden_shortcuts.clone(),
            reason:
                "Affordance has explicit forbidden shortcuts that the resolution must not bypass."
                    .to_owned(),
        })
        .collect::<Vec<_>>();
    let pressure_obligations = source
        .scene_pressure
        .visible_active
        .iter()
        .map(|pressure| PressureObligation {
            pressure_id: pressure.pressure_id.clone(),
            kind: pressure.kind,
            obligation: "Move this pressure, satisfy it, or state a visible no-op reason."
                .to_owned(),
            evidence_refs: pressure.source_refs.clone(),
        })
        .collect::<Vec<_>>();
    let due_processes = source
        .world_process_clock
        .visible_processes
        .iter()
        .map(|process| DueProcess {
            process_id: process.process_id.clone(),
            visibility: SimulationVisibility::PlayerVisible,
            tempo: process.tempo,
            tick_condition: process.next_tick_contract.clone(),
            evidence_refs: process.source_refs.clone(),
        })
        .chain(
            source
                .world_process_clock
                .adjudication_only_processes
                .iter()
                .map(|process| DueProcess {
                    process_id: process.process_id.clone(),
                    visibility: SimulationVisibility::AdjudicationOnly,
                    tempo: process.tempo,
                    tick_condition: process.next_tick_contract.clone(),
                    evidence_refs: process.source_refs.clone(),
                }),
        )
        .collect::<Vec<_>>();

    PreTurnSimulationPass {
        schema_version: PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION.to_owned(),
        source_bundle_schema_version: SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION.to_owned(),
        world_id: source.pending.world_id.clone(),
        turn_id: source.pending.turn_id.clone(),
        player_input: source.pending.player_input.clone(),
        input_kind,
        selected_choice: source.pending.selected_choice.clone(),
        source_refs: collect_source_refs(source),
        available_affordances,
        blocked_affordances,
        pressure_obligations,
        due_processes,
        causal_risks: compile_causal_risks(source),
        required_resolution_fields: required_resolution_fields(&source.pending, input_kind),
        hidden_visibility_boundary: hidden_visibility_boundary(
            &source.private_adjudication_context,
        ),
        compiler_policy: PreTurnSimulationPolicy::default(),
    }
}

fn classify_input_kind(input: &str, selected_choice: Option<&PendingAgentChoice>) -> TurnInputKind {
    if let Some(choice) = selected_choice {
        if choice.slot == GUIDE_CHOICE_SLOT {
            return TurnInputKind::GuideChoice;
        }
        if choice.slot == FREEFORM_CHOICE_SLOT {
            return TurnInputKind::FreeformAction;
        }
        return TurnInputKind::NumericChoice;
    }
    if input.trim_start().starts_with(".cc") {
        return TurnInputKind::CcCanvas;
    }
    if input.trim().contains("기록") || input.trim().contains("codex") {
        return TurnInputKind::CodexQuery;
    }
    TurnInputKind::FreeformAction
}

fn required_resolution_fields(
    pending: &PendingAgentTurn,
    input_kind: TurnInputKind,
) -> RequiredResolutionFields {
    let mut required = pending.turn_id != INITIAL_TURN_ID;
    if matches!(
        input_kind,
        TurnInputKind::CodexQuery | TurnInputKind::CcCanvas
    ) {
        required = false;
    }
    RequiredResolutionFields {
        resolution_proposal_required: required,
        next_choice_plan_required: required,
        pressure_movement_or_noop_reason_required: required,
        reason: if required {
            "normal player action after bootstrap must be causally resolved before commit"
                .to_owned()
        } else {
            "bootstrap, archive/canvas, or non-mutating route may use a weaker proposal contract"
                .to_owned()
        },
    }
}

fn collect_source_refs(source: &SimulationSourceBundle) -> Vec<String> {
    let mut refs = BTreeSet::new();
    refs.insert(format!("turn:{}", source.pending.turn_id));
    for affordance in &source.affordance_graph.ordinary_choice_slots {
        refs.insert(affordance.affordance_id.clone());
        refs.extend(affordance.source_refs.clone());
        refs.extend(affordance.pressure_refs.clone());
    }
    for pressure in &source.scene_pressure.visible_active {
        refs.insert(pressure.pressure_id.clone());
        refs.extend(pressure.source_refs.clone());
        refs.extend(pressure.observable_signals.clone());
    }
    for pressure in &source.scene_pressure.hidden_adjudication_only {
        refs.insert(pressure.pressure_id.clone());
        refs.extend(pressure.source_refs.clone());
    }
    if let Some(location) = &source.location_graph.current_location {
        refs.insert(location.location_id.clone());
        refs.extend(location.source_refs.clone());
    }
    for location in &source.location_graph.known_nearby_locations {
        refs.insert(location.location_id.clone());
        refs.extend(location.source_refs.clone());
    }
    for process in &source.world_process_clock.visible_processes {
        refs.insert(process.process_id.clone());
        refs.extend(process.source_refs.clone());
    }
    refs.into_iter().filter(|item| !item.is_empty()).collect()
}

fn compile_causal_risks(source: &SimulationSourceBundle) -> Vec<CausalRisk> {
    let mut risks = Vec::new();
    if source.scene_pressure.visible_active.is_empty() {
        risks.push(CausalRisk {
            risk_kind: CausalRiskKind::MissingPressure,
            summary: "No visible scene pressure is active; resolution must still clarify knowledge, cost, relationship, location, body, or time.".to_owned(),
            evidence_refs: vec![format!("turn:{}", source.pending.turn_id)],
        });
    }
    if !source
        .private_adjudication_context
        .unrevealed_constraints
        .is_empty()
    {
        risks.push(CausalRisk {
            risk_kind: CausalRiskKind::HiddenLeak,
            summary: "Private adjudication constraints exist; visible prose must use only observable signals.".to_owned(),
            evidence_refs: Vec::new(),
        });
    }
    if source
        .affordance_graph
        .ordinary_choice_slots
        .iter()
        .any(|affordance| !affordance.forbidden_shortcuts.is_empty())
    {
        risks.push(CausalRisk {
            risk_kind: CausalRiskKind::ProcessOvertick,
            summary: "Compiled affordances include forbidden shortcuts; proposal must not bypass their conditions.".to_owned(),
            evidence_refs: source
                .affordance_graph
                .ordinary_choice_slots
                .iter()
                .flat_map(|affordance| affordance.source_refs.clone())
                .collect(),
        });
    }
    risks
}

fn hidden_visibility_boundary(
    private_context: &AgentPrivateAdjudicationContext,
) -> HiddenVisibilityBoundary {
    let mut forbidden_visible_needles = BTreeSet::new();
    for constraint in &private_context.unrevealed_constraints {
        forbidden_visible_needles.extend(constraint.forbidden_leaks.clone());
    }
    HiddenVisibilityBoundary {
        hidden_timer_count: private_context.hidden_timers.len(),
        unrevealed_constraint_count: private_context.unrevealed_constraints.len(),
        forbidden_visible_needles: forbidden_visible_needles
            .into_iter()
            .filter(|item| !item.is_empty())
            .collect(),
        render_policy: "Hidden timers and constraints may shape adjudication only; render only player-visible signals.".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor_agency::ActorAgencyPacket;
    use crate::affordance_graph::{
        AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION, AFFORDANCE_NODE_SCHEMA_VERSION,
        AffordanceGraphPolicy, AffordanceNode,
    };
    use crate::agent_bridge::{
        AGENT_PENDING_TURN_SCHEMA_VERSION, AgentHiddenSecret, AgentHiddenTimer,
        AgentOutputContract, AgentVisibleContext,
    };
    use crate::autobiographical_index::AutobiographicalIndexPacket;
    use crate::belief_graph::BeliefGraphPacket;
    use crate::body_resource::BodyResourcePacket;
    use crate::change_ledger::ChangeLedgerPacket;
    use crate::character_text_design::CharacterTextDesignPacket;
    use crate::consequence_spine::ConsequenceSpinePacket;
    use crate::context_capsule::ContextCapsuleSelection;
    use crate::encounter_surface::EncounterSurfacePacket;
    use crate::extra_memory::ExtraMemoryPacket;
    use crate::location_graph::LocationGraphPacket;
    use crate::narrative_style_state::NarrativeStyleState;
    use crate::pattern_debt::PatternDebtPacket;
    use crate::player_intent::PlayerIntentTracePacket;
    use crate::plot_thread::PlotThreadPacket;
    use crate::relationship_graph::RelationshipGraphPacket;
    use crate::scene_director::SceneDirectorPacket;
    use crate::scene_pressure::{
        SCENE_PRESSURE_PACKET_SCHEMA_VERSION, SCENE_PRESSURE_SCHEMA_VERSION, ScenePressure,
        ScenePressurePolicy, ScenePressureProseEffect, ScenePressureUrgency,
        ScenePressureVisibility,
    };
    use crate::social_exchange::SocialExchangePacket;
    use crate::turn_retrieval_controller::TurnRetrievalControllerPacket;
    use crate::world_lore::WorldLorePacket;
    use crate::world_process_clock::{
        WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION, WORLD_PROCESS_SCHEMA_VERSION, WorldProcess,
        WorldProcessClockPolicy, WorldProcessVisibility,
    };

    #[test]
    fn compiles_resolution_obligations_from_visible_pressure_and_affordances() {
        let source = sample_source_bundle("turn_0002", Some(sample_choice(1)));
        let pass = compile_pre_turn_simulation_pass(&source);

        assert!(pass.required_resolution_fields.resolution_proposal_required);
        assert_eq!(pass.available_affordances.len(), 1);
        assert_eq!(pass.blocked_affordances.len(), 1);
        assert_eq!(pass.pressure_obligations.len(), 1);
        assert_eq!(pass.due_processes.len(), 1);
        assert_eq!(pass.input_kind, TurnInputKind::NumericChoice);
    }

    #[test]
    fn bootstrap_turn_uses_weaker_resolution_contract() {
        let source = sample_source_bundle(INITIAL_TURN_ID, None);
        let pass = compile_pre_turn_simulation_pass(&source);

        assert!(!pass.required_resolution_fields.resolution_proposal_required);
    }

    #[test]
    fn hidden_boundary_carries_forbidden_leak_needles_without_visible_truth() {
        let source = sample_source_bundle("turn_0003", Some(sample_choice(6)));
        let pass = compile_pre_turn_simulation_pass(&source);

        assert_eq!(pass.input_kind, TurnInputKind::FreeformAction);
        assert_eq!(pass.hidden_visibility_boundary.hidden_timer_count, 1);
        assert_eq!(
            pass.hidden_visibility_boundary.forbidden_visible_needles,
            vec!["밀서".to_owned()]
        );
    }

    fn sample_source_bundle(
        turn_id: &str,
        selected_choice: Option<PendingAgentChoice>,
    ) -> SimulationSourceBundle {
        let pending = PendingAgentTurn {
            schema_version: AGENT_PENDING_TURN_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pre_turn".to_owned(),
            turn_id: turn_id.to_owned(),
            status: "pending".to_owned(),
            player_input: selected_choice
                .as_ref()
                .map_or_else(|| "세계 개막".to_owned(), |choice| choice.slot.to_string()),
            selected_choice,
            visible_context: AgentVisibleContext {
                location: "place:gate".to_owned(),
                recent_scene: Vec::new(),
                known_facts: Vec::new(),
                voice_anchors: Vec::new(),
                extra_memory: ExtraMemoryPacket::default(),
                active_scene_pressure: ScenePressurePacket::default(),
                active_plot_threads: PlotThreadPacket::default(),
                active_body_resource_state: BodyResourcePacket::default(),
                active_location_graph: LocationGraphPacket::default(),
                active_character_text_design: CharacterTextDesignPacket::default(),
                active_world_lore: WorldLorePacket::default(),
                active_relationship_graph: RelationshipGraphPacket::default(),
                active_actor_agency: ActorAgencyPacket::default(),
                active_change_ledger: ChangeLedgerPacket::default(),
                active_pattern_debt: PatternDebtPacket::default(),
                active_belief_graph: BeliefGraphPacket::default(),
                active_world_process_clock: WorldProcessClockPacket::default(),
                active_player_intent_trace: PlayerIntentTracePacket::default(),
                active_narrative_style_state: NarrativeStyleState::default(),
                active_scene_director: SceneDirectorPacket::default(),
                active_consequence_spine: ConsequenceSpinePacket::default(),
                active_social_exchange: SocialExchangePacket::default(),
                active_encounter_surface: EncounterSurfacePacket::default(),
                active_hook_ledger: crate::hook_ledger::HookPacket::default(),
                active_turn_retrieval_controller: TurnRetrievalControllerPacket::default(),
                selected_context_capsules: ContextCapsuleSelection::default(),
                active_autobiographical_index: AutobiographicalIndexPacket::default(),
            },
            private_adjudication_context: private_context(),
            output_contract: AgentOutputContract {
                language: "ko".to_owned(),
                must_return_json: true,
                hidden_truth_must_not_appear_in_visible_text: true,
                narrative_level: 1,
                narrative_budget: crate::agent_bridge::narrative_budget_for_level(Some(1)),
            },
            pending_ref: "agent_bridge/pending_turn.json".to_owned(),
            created_at: "2026-04-30T00:00:00Z".to_owned(),
        };
        SimulationSourceBundle {
            pending,
            affordance_graph: affordance_graph(),
            body_resource_state: BodyResourcePacket::default(),
            location_graph: LocationGraphPacket::default(),
            scene_pressure: scene_pressure(),
            world_process_clock: world_process_clock(),
            relationship_graph: RelationshipGraphPacket::default(),
            consequence_spine: ConsequenceSpinePacket::default(),
            encounter_surface: EncounterSurfacePacket::default(),
            private_adjudication_context: private_context(),
        }
    }

    fn sample_choice(slot: u8) -> PendingAgentChoice {
        PendingAgentChoice {
            slot,
            tag: if slot == FREEFORM_CHOICE_SLOT {
                "자유서술"
            } else {
                "다가섬"
            }
            .to_owned(),
            visible_intent: "문 앞으로 움직인다".to_owned(),
        }
    }

    fn affordance_graph() -> AffordanceGraphPacket {
        AffordanceGraphPacket {
            schema_version: AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pre_turn".to_owned(),
            turn_id: "turn_0002".to_owned(),
            ordinary_choice_slots: vec![AffordanceNode {
                schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
                slot: 1,
                affordance_id: "affordance:slot:1:move".to_owned(),
                affordance_kind: AffordanceKind::Move,
                label_contract: "approach".to_owned(),
                action_contract: "approach the gate".to_owned(),
                source_refs: vec!["place:gate".to_owned()],
                pressure_refs: vec!["pressure:gate".to_owned()],
                forbidden_shortcuts: vec!["teleport through gate".to_owned()],
            }],
            compiler_policy: AffordanceGraphPolicy::default(),
        }
    }

    fn scene_pressure() -> ScenePressurePacket {
        ScenePressurePacket {
            schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pre_turn".to_owned(),
            turn_id: "turn_0002".to_owned(),
            visible_active: vec![ScenePressure {
                schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
                pressure_id: "pressure:gate".to_owned(),
                kind: ScenePressureKind::TimePressure,
                visibility: ScenePressureVisibility::PlayerVisible,
                intensity: 3,
                urgency: ScenePressureUrgency::Immediate,
                source_refs: vec!["visible_scene:gate".to_owned()],
                provenance: None,
                observable_signals: vec!["signal:closing_gate".to_owned()],
                choice_affordances: vec!["move".to_owned()],
                prose_effect: ScenePressureProseEffect {
                    paragraph_pressure: "tight".to_owned(),
                    sensory_focus: Vec::new(),
                    dialogue_style: "short".to_owned(),
                },
            }],
            hidden_adjudication_only: Vec::new(),
            compiler_policy: ScenePressurePolicy::default(),
        }
    }

    fn world_process_clock() -> WorldProcessClockPacket {
        WorldProcessClockPacket {
            schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_pre_turn".to_owned(),
            turn_id: "turn_0002".to_owned(),
            visible_processes: vec![WorldProcess {
                schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
                process_id: "process:gate_closing".to_owned(),
                visibility: WorldProcessVisibility::PlayerVisible,
                tempo: WorldProcessTempo::Immediate,
                summary: "the gate is closing".to_owned(),
                next_tick_contract: "tick when time is spent".to_owned(),
                tick_policy: crate::world_process_clock::WorldProcessTickPolicy::default(),
                source_refs: vec!["pressure:gate".to_owned()],
            }],
            adjudication_only_processes: Vec::new(),
            compiler_policy: WorldProcessClockPolicy::default(),
        }
    }

    fn private_context() -> AgentPrivateAdjudicationContext {
        AgentPrivateAdjudicationContext {
            hidden_timers: vec![AgentHiddenTimer {
                timer_id: "timer:letter".to_owned(),
                kind: "reveal".to_owned(),
                remaining_turns: 2,
                effect: "letter becomes relevant".to_owned(),
            }],
            unrevealed_constraints: vec![AgentHiddenSecret {
                secret_id: "secret:letter".to_owned(),
                status: "unrevealed".to_owned(),
                truth: "sealed royal letter".to_owned(),
                reveal_conditions: vec!["inspect bag".to_owned()],
                forbidden_leaks: vec!["밀서".to_owned()],
            }],
            plausibility_gates: Vec::new(),
        }
    }
}
