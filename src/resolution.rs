use crate::affordance_graph::AffordanceGraphPacket;
use crate::agent_bridge::AgentPrivateAdjudicationContext;
use crate::belief_graph::BeliefGraphPacket;
use crate::body_resource::BodyResourcePacket;
use crate::location_graph::LocationGraphPacket;
use crate::models::TurnChoice;
use crate::prompt_context::PromptContextPacket;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const RESOLUTION_PROPOSAL_SCHEMA_VERSION: &str = "singulari.resolution_proposal.v1";
pub const FREEFORM_GATE_TRACE_SCHEMA_VERSION: &str = "singulari.freeform_gate_trace.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolutionProposal {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub interpreted_intent: ActionIntent,
    pub outcome: ResolutionOutcome,
    #[serde(default)]
    pub gate_results: Vec<GateResult>,
    #[serde(default)]
    pub proposed_effects: Vec<ProposedEffect>,
    #[serde(default)]
    pub process_ticks: Vec<ProcessTickProposal>,
    #[serde(default)]
    pub pressure_noop_reasons: Vec<PressureNoopReason>,
    pub narrative_brief: NarrativeBrief,
    #[serde(default)]
    pub next_choice_plan: Vec<ChoicePlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActionIntent {
    pub input_kind: ActionInputKind,
    pub summary: String,
    #[serde(default)]
    pub target_refs: Vec<String>,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
    pub ambiguity: ActionAmbiguity,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionInputKind {
    #[serde(
        alias = "numeric_choice",
        alias = "macro_time_flow",
        alias = "cc_canvas"
    )]
    PresentedChoice,
    #[serde(alias = "freeform_action")]
    Freeform,
    #[serde(alias = "guide_choice")]
    DelegatedJudgment,
    CodexQuery,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionAmbiguity {
    Clear,
    Minor,
    High,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolutionOutcome {
    pub kind: ResolutionOutcomeKind,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionOutcomeKind {
    Success,
    PartialSuccess,
    Blocked,
    CostlySuccess,
    Delayed,
    Escalated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GateResult {
    pub gate_kind: GateKind,
    pub gate_ref: String,
    pub visibility: ResolutionVisibility,
    pub status: GateStatus,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Body,
    Resource,
    Location,
    SocialPermission,
    Knowledge,
    TimePressure,
    HiddenConstraint,
    WorldLaw,
    Affordance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    Passed,
    Softened,
    Blocked,
    CostImposed,
    UnknownNeedsProbe,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProposedEffect {
    pub effect_kind: ProposedEffectKind,
    pub target_ref: String,
    pub visibility: ResolutionVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProposedEffectKind {
    ScenePressureDelta,
    BodyResourceDelta,
    LocationDelta,
    RelationshipDelta,
    BeliefDelta,
    WorldLoreDelta,
    PatternDebt,
    PlayerIntentTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessTickProposal {
    pub process_ref: String,
    pub cause: ProcessTickCause,
    pub visibility: ResolutionVisibility,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessTickCause {
    PlayerActionTouchedProcess,
    VisibleTimePassage,
    ScenePressureChanged,
    NextTickConditionMet,
    HiddenRevealConditionSatisfied,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PressureNoopReason {
    pub pressure_ref: String,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NarrativeBrief {
    pub visible_summary: String,
    #[serde(default)]
    pub required_beats: Vec<String>,
    #[serde(default)]
    pub forbidden_visible_details: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChoicePlan {
    pub slot: u8,
    pub plan_kind: ChoicePlanKind,
    pub grounding_ref: String,
    pub label_seed: String,
    pub intent_seed: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChoicePlanKind {
    OrdinaryAffordance,
    Freeform,
    DelegatedJudgment,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionVisibility {
    PlayerVisible,
    AdjudicationOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FreeformGateTrace {
    pub schema_version: String,
    pub raw_input: String,
    pub interpreted_intent: ActionIntent,
    #[serde(default)]
    pub gate_results: Vec<GateResult>,
    pub final_outcome: ResolutionOutcomeKind,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResolutionCritique {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub failure_kind: ResolutionFailureKind,
    pub message: String,
    #[serde(default)]
    pub rejected_refs: Vec<String>,
    #[serde(default)]
    pub required_changes: Vec<String>,
    #[serde(default)]
    pub allowed_repair_scope: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionFailureKind {
    Schema,
    TargetRef,
    VisibilityLeak,
    Evidence,
    Gate,
    Causality,
    ChoiceGrounding,
}

/// Audit an LLM-authored resolution proposal against one compiled prompt context.
///
/// This is a pre-commit boundary. It validates that the proposal is grounded in
/// context refs, has evidence for durable deltas, and does not expose hidden
/// adjudication content in player-visible fields.
///
/// # Errors
///
/// Returns a structured critique when the proposal cannot be safely committed.
pub fn audit_resolution_proposal(
    context: &PromptContextPacket,
    proposal: &ResolutionProposal,
) -> std::result::Result<(), Box<ResolutionCritique>> {
    ResolutionAuditor::new(context)
        .and_then(|auditor| auditor.audit(proposal))
        .map_err(|critique| {
            Box::new(ResolutionCritique {
                schema_version: "singulari.resolution_critique.v1".to_owned(),
                world_id: proposal.world_id.clone(),
                turn_id: proposal.turn_id.clone(),
                ..*critique
            })
        })
}

/// Audit player-visible next choices against an already-audited proposal.
///
/// `TurnChoice` stays player-visible and deliberately does not carry internal
/// affordance ids. This audit bridges that gap by requiring the proposal to
/// contain a complete slot plan, then checking that the visible choices do not
/// expose internal refs or copy explicitly forbidden shortcuts.
///
/// # Errors
///
/// Returns a structured critique when a choice plan is incomplete, mismatched
/// with compiled affordances, or leaks internal grounding text.
pub fn audit_resolution_choices(
    context: &PromptContextPacket,
    proposal: &ResolutionProposal,
    choices: &[TurnChoice],
) -> std::result::Result<(), Box<ResolutionCritique>> {
    let affordances = collect_affordances_by_slot(context).map_err(|error| {
        Box::new(ResolutionCritique {
            schema_version: "singulari.resolution_critique.v1".to_owned(),
            world_id: proposal.world_id.clone(),
            turn_id: proposal.turn_id.clone(),
            failure_kind: ResolutionFailureKind::Schema,
            message: format!("resolution choice audit context is invalid: {error}"),
            rejected_refs: Vec::new(),
            required_changes: Vec::new(),
            allowed_repair_scope: Vec::new(),
        })
    })?;
    audit_choice_plan_complete(proposal, &affordances).map_err(|critique| {
        Box::new(ResolutionCritique {
            schema_version: "singulari.resolution_critique.v1".to_owned(),
            world_id: proposal.world_id.clone(),
            turn_id: proposal.turn_id.clone(),
            ..*critique
        })
    })?;
    audit_visible_choice_text_against_affordances(proposal, choices, &affordances).map_err(
        |critique| {
            Box::new(ResolutionCritique {
                schema_version: "singulari.resolution_critique.v1".to_owned(),
                world_id: proposal.world_id.clone(),
                turn_id: proposal.turn_id.clone(),
                ..*critique
            })
        },
    )
}

#[must_use]
pub fn freeform_gate_trace_from_proposal(
    raw_input: &str,
    proposal: &ResolutionProposal,
) -> Option<FreeformGateTrace> {
    if !matches!(
        proposal.interpreted_intent.input_kind,
        ActionInputKind::Freeform
    ) {
        return None;
    }
    Some(FreeformGateTrace {
        schema_version: FREEFORM_GATE_TRACE_SCHEMA_VERSION.to_owned(),
        raw_input: raw_input.trim().to_owned(),
        interpreted_intent: proposal.interpreted_intent.clone(),
        gate_results: proposal.gate_results.clone(),
        final_outcome: proposal.outcome.kind,
        evidence_refs: proposal.outcome.evidence_refs.clone(),
    })
}

struct ResolutionAuditor {
    world_id: String,
    turn_id: String,
    visible_refs: BTreeSet<String>,
    affordances_by_slot: BTreeMap<u8, crate::affordance_graph::AffordanceNode>,
    pressure_obligation_ids: Vec<String>,
    hidden_refs: BTreeSet<String>,
    hidden_needles: Vec<String>,
}

impl ResolutionAuditor {
    fn new(context: &PromptContextPacket) -> std::result::Result<Self, Box<ResolutionCritique>> {
        let visible_refs =
            collect_visible_refs(context).map_err(|error| schema_critique(&error))?;
        let affordances_by_slot =
            collect_affordances_by_slot(context).map_err(|error| schema_critique(&error))?;
        let (hidden_refs, hidden_needles) = collect_hidden_refs_and_needles(context);
        Ok(Self {
            world_id: context.world_id.clone(),
            turn_id: context.turn_id.clone(),
            visible_refs,
            affordances_by_slot,
            pressure_obligation_ids: context
                .pre_turn_simulation
                .pressure_obligations
                .iter()
                .map(|obligation| obligation.pressure_id.clone())
                .collect(),
            hidden_refs,
            hidden_needles,
        })
    }

    fn audit(
        &self,
        proposal: &ResolutionProposal,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        self.audit_header(proposal)?;
        Self::audit_evidence(proposal)?;
        self.audit_refs(proposal)?;
        self.audit_pressure_obligations(proposal)?;
        self.audit_choice_grounding(proposal)?;
        self.audit_hidden_visibility(proposal)
    }

    fn audit_header(
        &self,
        proposal: &ResolutionProposal,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        if proposal.schema_version != RESOLUTION_PROPOSAL_SCHEMA_VERSION {
            return Err(critique(
                ResolutionFailureKind::Schema,
                "resolution proposal schema_version mismatch",
            ));
        }
        if proposal.world_id != self.world_id || proposal.turn_id != self.turn_id {
            return Err(critique(
                ResolutionFailureKind::Schema,
                "resolution proposal target does not match prompt context",
            ));
        }
        Ok(())
    }

    fn audit_evidence(
        proposal: &ResolutionProposal,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        require_evidence(
            "interpreted_intent",
            &proposal.interpreted_intent.evidence_refs,
        )?;
        require_evidence("outcome", &proposal.outcome.evidence_refs)?;
        for gate in &proposal.gate_results {
            require_evidence("gate_result", &gate.evidence_refs)?;
        }
        for effect in &proposal.proposed_effects {
            require_evidence("proposed_effect", &effect.evidence_refs)?;
        }
        for tick in &proposal.process_ticks {
            require_evidence("process_tick", &tick.evidence_refs)?;
        }
        for noop in &proposal.pressure_noop_reasons {
            require_evidence("pressure_noop_reason", &noop.evidence_refs)?;
        }
        for choice in &proposal.next_choice_plan {
            require_evidence("choice_plan", &choice.evidence_refs)?;
        }
        Ok(())
    }

    fn audit_refs(
        &self,
        proposal: &ResolutionProposal,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        for target_ref in &proposal.interpreted_intent.target_refs {
            self.require_known_ref(target_ref, ResolutionVisibility::PlayerVisible)?;
        }
        for pressure_ref in &proposal.interpreted_intent.pressure_refs {
            self.require_known_ref(pressure_ref, ResolutionVisibility::PlayerVisible)?;
        }
        for gate in &proposal.gate_results {
            self.require_known_ref(&gate.gate_ref, gate.visibility)?;
        }
        for effect in &proposal.proposed_effects {
            self.require_known_ref(&effect.target_ref, effect.visibility)?;
        }
        for tick in &proposal.process_ticks {
            self.require_known_ref(&tick.process_ref, tick.visibility)?;
        }
        for noop in &proposal.pressure_noop_reasons {
            self.require_known_ref(&noop.pressure_ref, ResolutionVisibility::PlayerVisible)?;
        }
        for choice in &proposal.next_choice_plan {
            self.require_known_ref(&choice.grounding_ref, ResolutionVisibility::PlayerVisible)?;
        }
        Ok(())
    }

    fn require_known_ref(
        &self,
        item_ref: &str,
        visibility: ResolutionVisibility,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        if self.visible_refs.contains(item_ref) {
            return Ok(());
        }
        if visibility == ResolutionVisibility::AdjudicationOnly
            && self.hidden_refs.contains(item_ref)
        {
            return Ok(());
        }
        if visibility == ResolutionVisibility::PlayerVisible
            && shorthand_affordance_ref_slot(item_ref, &self.turn_id)
                .and_then(|slot| self.affordances_by_slot.get(&slot))
                .is_some()
        {
            return Ok(());
        }
        Err(critique_with_refs(
            ResolutionFailureKind::TargetRef,
            "resolution proposal references an unknown or forbidden ref",
            vec![item_ref.to_owned()],
        ))
    }

    fn audit_choice_grounding(
        &self,
        proposal: &ResolutionProposal,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        audit_choice_plan_complete(proposal, &self.affordances_by_slot)?;
        for choice in &proposal.next_choice_plan {
            match choice.plan_kind {
                ChoicePlanKind::OrdinaryAffordance => {
                    if !(1..=5).contains(&choice.slot) {
                        return Err(critique_with_refs(
                            ResolutionFailureKind::ChoiceGrounding,
                            "ordinary choice plans must use slots 1..5",
                            vec![choice.slot.to_string()],
                        ));
                    }
                }
                ChoicePlanKind::Freeform => {
                    if choice.slot != 6 {
                        return Err(critique_with_refs(
                            ResolutionFailureKind::ChoiceGrounding,
                            "freeform choice plan must use slot 6",
                            vec![choice.slot.to_string()],
                        ));
                    }
                }
                ChoicePlanKind::DelegatedJudgment => {
                    if choice.slot != 7 {
                        return Err(critique_with_refs(
                            ResolutionFailureKind::ChoiceGrounding,
                            "delegated judgment choice plan must use slot 7",
                            vec![choice.slot.to_string()],
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn audit_pressure_obligations(
        &self,
        proposal: &ResolutionProposal,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        for pressure_id in &self.pressure_obligation_ids {
            if proposal_moves_pressure(proposal, pressure_id)
                || proposal_has_pressure_noop_reason(proposal, pressure_id)
            {
                continue;
            }
            return Err(critique_with_refs(
                ResolutionFailureKind::Causality,
                "resolution proposal does not move or explicitly defer a pre-turn pressure obligation",
                vec![pressure_id.clone()],
            ));
        }
        Ok(())
    }

    fn audit_hidden_visibility(
        &self,
        proposal: &ResolutionProposal,
    ) -> std::result::Result<(), Box<ResolutionCritique>> {
        let visible_text = proposal_visible_text(proposal);
        for needle in &self.hidden_needles {
            if hidden_needle_leaks(&visible_text, needle) {
                return Err(critique_with_refs(
                    ResolutionFailureKind::VisibilityLeak,
                    "resolution proposal exposes hidden adjudication content in visible fields",
                    vec![needle.clone()],
                ));
            }
        }
        Ok(())
    }
}

fn audit_choice_plan_complete(
    proposal: &ResolutionProposal,
    affordances: &BTreeMap<u8, crate::affordance_graph::AffordanceNode>,
) -> std::result::Result<(), Box<ResolutionCritique>> {
    let planned_slots = proposal
        .next_choice_plan
        .iter()
        .map(|choice| choice.slot)
        .collect::<BTreeSet<_>>();
    if planned_slots != BTreeSet::from([1, 2, 3, 4, 5, 6, 7]) {
        return Err(critique_with_refs(
            ResolutionFailureKind::ChoiceGrounding,
            "resolution proposal next_choice_plan must cover slots 1..7",
            planned_slots.iter().map(u8::to_string).collect(),
        ));
    }
    let mut grounded_affordance_slots = BTreeSet::new();
    for slot in 1..=5 {
        let Some(choice) = proposal
            .next_choice_plan
            .iter()
            .find(|choice| choice.slot == slot)
        else {
            return Err(critique_with_refs(
                ResolutionFailureKind::ChoiceGrounding,
                "resolution proposal missing ordinary choice slot plan",
                vec![slot.to_string()],
            ));
        };
        if choice.plan_kind != ChoicePlanKind::OrdinaryAffordance {
            return Err(critique_with_refs(
                ResolutionFailureKind::ChoiceGrounding,
                "ordinary choice slots 1..5 must use ordinary_affordance plans",
                vec![slot.to_string()],
            ));
        }
        let Some(affordance_slot) =
            affordance_slot_for_grounding_ref(choice.grounding_ref.as_str(), affordances)
        else {
            return Err(critique_with_refs(
                ResolutionFailureKind::ChoiceGrounding,
                "ordinary choice plan must cite one compiled affordance id",
                vec![choice.grounding_ref.clone()],
            ));
        };
        grounded_affordance_slots.insert(affordance_slot);
    }
    let expected_affordance_slots = affordances.keys().copied().collect::<BTreeSet<_>>();
    if grounded_affordance_slots != expected_affordance_slots {
        return Err(critique_with_refs(
            ResolutionFailureKind::ChoiceGrounding,
            "ordinary choice plan must cite each compiled affordance exactly once",
            grounded_affordance_slots
                .iter()
                .map(u8::to_string)
                .collect(),
        ));
    }

    require_choice_plan_kind(proposal, 6, ChoicePlanKind::Freeform)?;
    require_choice_plan_kind(proposal, 7, ChoicePlanKind::DelegatedJudgment)?;
    Ok(())
}

fn affordance_slot_for_grounding_ref(
    grounding_ref: &str,
    affordances: &BTreeMap<u8, crate::affordance_graph::AffordanceNode>,
) -> Option<u8> {
    for (slot, affordance) in affordances {
        if grounding_ref == affordance.affordance_id {
            return Some(*slot);
        }
    }
    let slot = shorthand_affordance_ref_slot(grounding_ref, "")?;
    affordances
        .get(&slot)
        .is_some_and(|affordance| affordance.affordance_id.starts_with("encounter:"))
        .then_some(slot)
}

fn shorthand_affordance_ref_slot(item_ref: &str, turn_id: &str) -> Option<u8> {
    if !item_ref.starts_with("encounter:") || !item_ref.ends_with("::affordance") {
        return None;
    }
    if !turn_id.is_empty() && !item_ref.starts_with(format!("encounter:{turn_id}:slot:").as_str()) {
        return None;
    }
    let (_, slot_tail) = item_ref.split_once(":slot:")?;
    let slot = slot_tail.strip_suffix("::affordance")?;
    slot.parse().ok()
}

fn require_choice_plan_kind(
    proposal: &ResolutionProposal,
    slot: u8,
    plan_kind: ChoicePlanKind,
) -> std::result::Result<(), Box<ResolutionCritique>> {
    let Some(choice) = proposal
        .next_choice_plan
        .iter()
        .find(|choice| choice.slot == slot)
    else {
        return Err(critique_with_refs(
            ResolutionFailureKind::ChoiceGrounding,
            "resolution proposal missing fixed special slot plan",
            vec![slot.to_string()],
        ));
    };
    if choice.plan_kind != plan_kind {
        return Err(critique_with_refs(
            ResolutionFailureKind::ChoiceGrounding,
            "resolution proposal special slot has wrong plan kind",
            vec![slot.to_string()],
        ));
    }
    Ok(())
}

fn proposal_moves_pressure(proposal: &ResolutionProposal, pressure_id: &str) -> bool {
    proposal.gate_results.iter().any(|gate| {
        gate.gate_ref == pressure_id || gate.evidence_refs.iter().any(|item| item == pressure_id)
    }) || proposal.proposed_effects.iter().any(|effect| {
        effect.target_ref == pressure_id
            || effect.evidence_refs.iter().any(|item| item == pressure_id)
    }) || proposal.process_ticks.iter().any(|tick| {
        tick.process_ref == pressure_id || tick.evidence_refs.iter().any(|item| item == pressure_id)
    })
}

fn proposal_has_pressure_noop_reason(proposal: &ResolutionProposal, pressure_id: &str) -> bool {
    proposal.pressure_noop_reasons.iter().any(|noop| {
        noop.pressure_ref == pressure_id
            && !noop.reason.trim().is_empty()
            && noop
                .evidence_refs
                .iter()
                .any(|item| !item.trim().is_empty())
    })
}

fn audit_visible_choice_text_against_affordances(
    proposal: &ResolutionProposal,
    choices: &[TurnChoice],
    affordances: &BTreeMap<u8, crate::affordance_graph::AffordanceNode>,
) -> std::result::Result<(), Box<ResolutionCritique>> {
    for choice in choices
        .iter()
        .filter(|choice| (1..=5).contains(&choice.slot))
    {
        let Some(plan) = proposal
            .next_choice_plan
            .iter()
            .find(|plan| plan.slot == choice.slot)
        else {
            return Err(critique_with_refs(
                ResolutionFailureKind::ChoiceGrounding,
                "visible ordinary choice is missing a resolution choice plan",
                vec![choice.slot.to_string()],
            ));
        };
        let Some(affordance) = affordances.get(&choice.slot) else {
            continue;
        };
        let visible_text = format!("{}\n{}", choice.tag, choice.intent);
        let mut forbidden_needles = vec![plan.grounding_ref.as_str()];
        forbidden_needles.extend(plan.evidence_refs.iter().map(String::as_str));
        forbidden_needles.extend(affordance.source_refs.iter().map(String::as_str));
        forbidden_needles.extend(affordance.pressure_refs.iter().map(String::as_str));
        forbidden_needles.extend(affordance.forbidden_shortcuts.iter().map(String::as_str));
        for needle in forbidden_needles {
            if !needle.trim().is_empty() && visible_text.contains(needle) {
                return Err(critique_with_refs(
                    ResolutionFailureKind::ChoiceGrounding,
                    "visible choice text exposes internal refs or forbidden shortcut text",
                    vec![needle.to_owned()],
                ));
            }
        }
    }
    Ok(())
}

fn require_evidence(
    label: &str,
    evidence_refs: &[String],
) -> std::result::Result<(), Box<ResolutionCritique>> {
    if evidence_refs.iter().any(|item| !item.trim().is_empty()) {
        return Ok(());
    }
    Err(critique_with_refs(
        ResolutionFailureKind::Evidence,
        "resolution proposal item is missing evidence refs",
        vec![label.to_owned()],
    ))
}

fn collect_visible_refs(context: &PromptContextPacket) -> Result<BTreeSet<String>> {
    let visible = &context.visible_context;
    let mut refs = BTreeSet::new();
    refs.insert(format!("turn:{}", context.turn_id));
    refs.insert("current_turn".to_owned());
    refs.insert("visible_scene".to_owned());

    let scene_pressure: Vec<crate::scene_pressure::ScenePressure> =
        serde_json::from_value(visible.active_scene_pressure.clone())
            .context("prompt context active_scene_pressure is not a visible pressure list")?;
    for pressure in scene_pressure {
        refs.insert(pressure.pressure_id);
        refs.extend(pressure.source_refs);
        refs.extend(pressure.observable_signals);
    }

    let body_resource: BodyResourcePacket =
        serde_json::from_value(visible.active_body_resource_state.clone())
            .context("prompt context active_body_resource_state is invalid")?;
    for constraint in body_resource.body_constraints {
        refs.insert(constraint.constraint_id);
        refs.extend(constraint.source_refs);
    }
    for resource in body_resource.resources {
        refs.insert(resource.resource_id);
        refs.extend(resource.source_refs);
    }

    let location_graph: LocationGraphPacket =
        serde_json::from_value(visible.active_location_graph.clone())
            .context("prompt context active_location_graph is invalid")?;
    if let Some(location) = location_graph.current_location {
        refs.insert(location.location_id);
        refs.extend(location.source_refs);
    }
    for location in location_graph.known_nearby_locations {
        refs.insert(location.location_id);
        refs.extend(location.source_refs);
    }

    let affordance_graph: AffordanceGraphPacket =
        serde_json::from_value(visible.affordance_graph.clone())
            .context("prompt context affordance_graph is invalid")?;
    for affordance in affordance_graph.ordinary_choice_slots {
        refs.insert(affordance.affordance_id);
        refs.extend(affordance.source_refs);
        refs.extend(affordance.pressure_refs);
    }

    let belief_graph: BeliefGraphPacket = serde_json::from_value(visible.belief_graph.clone())
        .context("prompt context belief_graph is invalid")?;
    for belief in belief_graph.protagonist_visible_beliefs {
        refs.insert(belief.belief_id);
        refs.extend(belief.source_refs);
    }

    let world_processes: Vec<crate::world_process_clock::WorldProcess> =
        serde_json::from_value(visible.world_process_clock.clone())
            .context("prompt context world_process_clock is invalid")?;
    for process in world_processes {
        refs.insert(process.process_id);
        refs.extend(process.source_refs);
    }

    collect_string_refs_from_value(&visible.known_facts, &mut refs);
    collect_string_refs_from_value(&visible.active_plot_threads, &mut refs);
    collect_string_refs_from_value(&visible.active_scene_director, &mut refs);
    collect_string_refs_from_value(&visible.selected_context_capsules, &mut refs);
    collect_string_refs_from_value(&visible.selected_memory_items, &mut refs);
    insert_visible_ref_aliases(&mut refs);
    Ok(refs)
}

fn insert_visible_ref_aliases(refs: &mut BTreeSet<String>) {
    let aliases = refs
        .iter()
        .filter_map(|item| relationship_ref_alias(item))
        .collect::<Vec<_>>();
    refs.extend(aliases);
}

fn relationship_ref_alias(item: &str) -> Option<String> {
    let body = item.strip_prefix("relationship:rel_")?;
    let (source, target) = body.split_once("-_")?;
    Some(format!(
        "rel:{}->{}",
        underscored_entity_ref(source)?,
        underscored_entity_ref(target)?
    ))
}

fn underscored_entity_ref(value: &str) -> Option<String> {
    let (prefix, suffix) = value.split_once('_')?;
    Some(format!("{prefix}:{suffix}"))
}

fn collect_affordances_by_slot(
    context: &PromptContextPacket,
) -> Result<BTreeMap<u8, crate::affordance_graph::AffordanceNode>> {
    let affordance_graph: AffordanceGraphPacket =
        serde_json::from_value(context.visible_context.affordance_graph.clone())
            .context("prompt context affordance_graph is invalid")?;
    Ok(affordance_graph
        .ordinary_choice_slots
        .into_iter()
        .map(|affordance| (affordance.slot, affordance))
        .collect())
}

fn collect_hidden_refs_and_needles(
    context: &PromptContextPacket,
) -> (BTreeSet<String>, Vec<String>) {
    let mut refs = BTreeSet::new();
    let mut needles = Vec::new();

    if let Ok(private_context) = serde_json::from_value::<AgentPrivateAdjudicationContext>(
        context
            .adjudication_context
            .private_adjudication_context
            .clone(),
    ) {
        for timer in private_context.hidden_timers {
            refs.insert(timer.timer_id);
            needles.push(timer.effect);
        }
        for secret in private_context.unrevealed_constraints {
            refs.insert(secret.secret_id);
            needles.push(secret.truth);
            needles.extend(secret.forbidden_leaks);
        }
        refs.extend(private_context.plausibility_gates);
    }

    if let Ok(hidden_pressures) = serde_json::from_value::<Vec<crate::scene_pressure::ScenePressure>>(
        context.adjudication_context.hidden_scene_pressure.clone(),
    ) {
        for pressure in hidden_pressures {
            refs.insert(pressure.pressure_id);
            refs.extend(pressure.source_refs);
            needles.extend(pressure.observable_signals);
        }
    }

    if let Ok(hidden_processes) =
        serde_json::from_value::<Vec<crate::world_process_clock::WorldProcess>>(
            context
                .adjudication_context
                .hidden_world_process_clock
                .clone(),
        )
    {
        for process in hidden_processes {
            refs.insert(process.process_id);
            refs.extend(process.source_refs);
            needles.push(process.summary);
        }
    }

    needles.retain(|needle| needle.trim().chars().count() >= 4);
    (refs, needles)
}

fn collect_string_refs_from_value(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::String(text) => {
            if looks_like_ref(text) {
                refs.insert(text.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_string_refs_from_value(item, refs);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_string_refs_from_value(value, refs);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn looks_like_ref(text: &str) -> bool {
    let Some((prefix, suffix)) = text.split_once(':') else {
        return false;
    };
    !prefix.trim().is_empty() && !suffix.trim().is_empty()
}

fn proposal_visible_text(proposal: &ResolutionProposal) -> String {
    let mut values = vec![
        proposal.interpreted_intent.summary.as_str(),
        proposal.outcome.summary.as_str(),
        proposal.narrative_brief.visible_summary.as_str(),
    ];
    values.extend(
        proposal
            .narrative_brief
            .required_beats
            .iter()
            .map(String::as_str),
    );
    values.extend(proposal.gate_results.iter().filter_map(|gate| {
        (gate.visibility == ResolutionVisibility::PlayerVisible).then_some(gate.reason.as_str())
    }));
    values.extend(proposal.proposed_effects.iter().filter_map(|effect| {
        (effect.visibility == ResolutionVisibility::PlayerVisible)
            .then_some(effect.summary.as_str())
    }));
    values.extend(proposal.process_ticks.iter().filter_map(|tick| {
        (tick.visibility == ResolutionVisibility::PlayerVisible).then_some(tick.summary.as_str())
    }));
    values.extend(
        proposal
            .pressure_noop_reasons
            .iter()
            .map(|noop| noop.reason.as_str()),
    );
    for choice in &proposal.next_choice_plan {
        values.push(choice.label_seed.as_str());
        values.push(choice.intent_seed.as_str());
    }
    values.join("\n")
}

fn hidden_needle_leaks(visible_text: &str, needle: &str) -> bool {
    let needle = needle.trim();
    needle.chars().count() >= 4 && visible_text.contains(needle)
}

fn schema_critique(error: &anyhow::Error) -> Box<ResolutionCritique> {
    critique(
        ResolutionFailureKind::Schema,
        format!("resolution audit context is invalid: {error}"),
    )
}

fn critique(
    message_kind: ResolutionFailureKind,
    message: impl Into<String>,
) -> Box<ResolutionCritique> {
    critique_with_refs(message_kind, message, Vec::new())
}

fn critique_with_refs(
    failure_kind: ResolutionFailureKind,
    message: impl Into<String>,
    rejected_refs: Vec<String>,
) -> Box<ResolutionCritique> {
    Box::new(ResolutionCritique {
        schema_version: "singulari.resolution_critique.v1".to_owned(),
        world_id: String::new(),
        turn_id: String::new(),
        failure_kind,
        message: message.into(),
        rejected_refs,
        required_changes: Vec::new(),
        allowed_repair_scope: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affordance_graph::{
        AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION, AFFORDANCE_NODE_SCHEMA_VERSION,
        AffordanceGraphPolicy, AffordanceKind, AffordanceNode,
    };
    use crate::agent_bridge::{AgentHiddenSecret, AgentHiddenTimer, AgentOutputContract};
    use crate::belief_graph::{
        BELIEF_GRAPH_PACKET_SCHEMA_VERSION, BELIEF_NODE_SCHEMA_VERSION, BeliefConfidence,
        BeliefGraphPolicy, BeliefHolder, BeliefNode,
    };
    use crate::body_resource::{
        BODY_CONSTRAINT_SCHEMA_VERSION, BODY_RESOURCE_PACKET_SCHEMA_VERSION, BodyConstraint,
        BodyResourcePolicy, BodyResourceVisibility,
    };
    use crate::location_graph::{
        LOCATION_GRAPH_PACKET_SCHEMA_VERSION, LOCATION_NODE_SCHEMA_VERSION, LocationGraphPolicy,
        LocationKnowledgeState, LocationNode,
    };
    use crate::models::{TurnChoice, TurnInputKind};
    use crate::pre_turn_simulation::{
        HiddenVisibilityBoundary, PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION, PreTurnSimulationPass,
        PreTurnSimulationPolicy, PressureObligation, RequiredResolutionFields,
        SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION,
    };
    use crate::prompt_context::{
        PROMPT_CONTEXT_PACKET_SCHEMA_VERSION, PromptAdjudicationContext, PromptContextPacket,
        PromptContextPolicy, PromptVisibleContext,
    };
    use crate::prompt_context_budget::{
        PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION, PromptContextBudgetPolicy,
        PromptContextBudgetReport,
    };
    use crate::scene_pressure::{
        SCENE_PRESSURE_SCHEMA_VERSION, ScenePressure, ScenePressureKind, ScenePressureProseEffect,
        ScenePressureUrgency, ScenePressureVisibility,
    };
    use crate::world_process_clock::{
        WORLD_PROCESS_SCHEMA_VERSION, WorldProcess, WorldProcessTempo, WorldProcessVisibility,
    };
    use serde::Serialize;
    use std::collections::BTreeMap;

    #[test]
    fn action_input_kind_accepts_runtime_input_aliases() -> anyhow::Result<()> {
        assert_eq!(
            serde_json::from_str::<ActionInputKind>(r#""freeform_action""#)?,
            ActionInputKind::Freeform
        );
        assert_eq!(
            serde_json::from_str::<ActionInputKind>(r#""numeric_choice""#)?,
            ActionInputKind::PresentedChoice
        );
        assert_eq!(
            serde_json::from_str::<ActionInputKind>(r#""guide_choice""#)?,
            ActionInputKind::DelegatedJudgment
        );
        Ok(())
    }

    #[test]
    fn accepts_grounded_llm_resolution_proposal() {
        let context = sample_context();
        let proposal = sample_proposal();

        if let Err(critique) = audit_resolution_proposal(&context, &proposal) {
            panic!("proposal should pass audit: {critique:?}");
        }
    }

    #[test]
    fn rejects_visible_hidden_truth_leak() {
        let context = sample_context();
        let mut proposal = sample_proposal();
        proposal.narrative_brief.visible_summary =
            "문지기가 왕실 밀서가 자루 안에 있다고 말한다.".to_owned();

        let critique = audit_failure(&context, &proposal);
        assert_eq!(critique.failure_kind, ResolutionFailureKind::VisibilityLeak);
    }

    #[test]
    fn rejects_unknown_resolution_target_ref() {
        let context = sample_context();
        let mut proposal = sample_proposal();
        proposal.proposed_effects[0].target_ref = "resource:missing_map".to_owned();

        let critique = audit_failure(&context, &proposal);
        assert_eq!(critique.failure_kind, ResolutionFailureKind::TargetRef);
        assert_eq!(critique.rejected_refs, vec!["resource:missing_map"]);
    }

    #[test]
    fn accepts_selected_context_capsule_refs_as_visible_resolution_refs() {
        let mut context = sample_context();
        context.visible_context.selected_context_capsules = serde_json::json!({
            "selected_capsules": [{
                "capsule_id": "relationship:rel_guard_distance",
                "kind": "relationship",
                "reason": "current_goal_match",
                "body": {
                    "payload": {
                        "edge_id": "rel:guard->protagonist:distance",
                        "source_entity_id": "char:guard",
                        "target_entity_id": "char:protagonist"
                    }
                }
            }],
            "rejected_capsules": [],
            "budget_report": {}
        });
        let mut proposal = sample_proposal();
        proposal.gate_results.push(GateResult {
            gate_kind: GateKind::SocialPermission,
            gate_ref: "rel:guard->protagonist:distance".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            status: GateStatus::Softened,
            reason: "문지기와 주인공 사이의 거리가 조금 좁혀진다.".to_owned(),
            evidence_refs: vec!["rel:guard->protagonist:distance".to_owned()],
        });

        if let Err(critique) = audit_resolution_proposal(&context, &proposal) {
            panic!("selected context capsule ref should pass audit: {critique:?}");
        }
    }

    #[test]
    fn accepts_webgpt_shorthand_encounter_affordance_refs_for_current_turn() {
        let mut context = sample_context();
        context.visible_context.affordance_graph = to_json_value(AffordanceGraphPacket {
            schema_version: AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_resolution".to_owned(),
            turn_id: "turn_0004".to_owned(),
            ordinary_choice_slots: vec![
                sample_affordance_node(1, "___", AffordanceKind::Move),
                sample_affordance_node(2, "__", AffordanceKind::Observe),
                sample_affordance_node(3, "__", AffordanceKind::Contact),
                sample_affordance_node(4, "__", AffordanceKind::ResourceOrBody),
                sample_affordance_node(5, "__", AffordanceKind::PressureResponse),
            ]
            .into_iter()
            .map(|mut affordance| {
                affordance.affordance_id = format!(
                    "encounter:turn_0004:slot:{}:{}:affordance",
                    affordance.slot,
                    if affordance.slot == 1 { "___" } else { "__" }
                );
                affordance
            })
            .collect(),
            compiler_policy: AffordanceGraphPolicy::default(),
        });
        let mut proposal = sample_proposal();
        proposal.gate_results[0].gate_ref = "encounter:turn_0004:slot:4::affordance".to_owned();
        for choice in &mut proposal.next_choice_plan {
            if (1..=5).contains(&choice.slot) {
                choice.grounding_ref =
                    format!("encounter:turn_0004:slot:{}::affordance", choice.slot);
            }
        }
        proposal.next_choice_plan[0].grounding_ref =
            "encounter:turn_0004:slot:5::affordance".to_owned();
        proposal.next_choice_plan[4].grounding_ref =
            "encounter:turn_0004:slot:1::affordance".to_owned();

        if let Err(critique) = audit_resolution_proposal(&context, &proposal) {
            panic!("current-turn shorthand encounter affordance refs should pass: {critique:?}");
        }
    }

    #[test]
    fn rejects_ordinary_choice_without_affordance_grounding() {
        let context = sample_context();
        let mut proposal = sample_proposal();
        proposal.next_choice_plan[0].grounding_ref = "place:west_gate".to_owned();

        let critique = audit_failure(&context, &proposal);
        assert_eq!(
            critique.failure_kind,
            ResolutionFailureKind::ChoiceGrounding
        );
    }

    #[test]
    fn rejects_incomplete_resolution_choice_plan() {
        let context = sample_context();
        let mut proposal = sample_proposal();
        proposal.next_choice_plan.retain(|choice| choice.slot != 5);

        let critique = audit_failure(&context, &proposal);
        assert_eq!(
            critique.failure_kind,
            ResolutionFailureKind::ChoiceGrounding
        );
    }

    #[test]
    fn rejects_visible_choice_exposing_internal_affordance_ref() {
        let context = sample_context();
        let proposal = sample_proposal();
        let mut choices = sample_turn_choices();
        choices[0].intent = "affordance:slot:1:move 그대로 실행한다".to_owned();

        let critique = match audit_resolution_choices(&context, &proposal, &choices) {
            Ok(()) => panic!("visible choices unexpectedly passed audit"),
            Err(critique) => critique,
        };

        assert_eq!(
            critique.failure_kind,
            ResolutionFailureKind::ChoiceGrounding
        );
    }

    #[test]
    fn rejects_missing_effect_evidence() {
        let context = sample_context();
        let mut proposal = sample_proposal();
        proposal.process_ticks[0].evidence_refs.clear();

        let critique = audit_failure(&context, &proposal);
        assert_eq!(critique.failure_kind, ResolutionFailureKind::Evidence);
        assert_eq!(critique.rejected_refs, vec!["process_tick"]);
    }

    #[test]
    fn rejects_uncovered_pre_turn_pressure_obligation() {
        let context = sample_context();
        let mut proposal = sample_proposal();
        proposal
            .interpreted_intent
            .pressure_refs
            .retain(|item| item != "pressure:social:gate");
        proposal
            .outcome
            .evidence_refs
            .retain(|item| item != "pressure:social:gate");
        proposal
            .outcome
            .evidence_refs
            .push("current_turn".to_owned());
        proposal.gate_results.clear();
        proposal.proposed_effects.clear();
        proposal.process_ticks.clear();
        for choice in &mut proposal.next_choice_plan {
            choice
                .evidence_refs
                .retain(|item| item != "pressure:social:gate");
        }

        let critique = audit_failure(&context, &proposal);
        assert_eq!(critique.failure_kind, ResolutionFailureKind::Causality);
        assert_eq!(critique.rejected_refs, vec!["pressure:social:gate"]);
    }

    #[test]
    fn accepts_explicit_pressure_noop_reason() {
        let context = sample_context();
        let mut proposal = sample_proposal();
        proposal.gate_results.clear();
        proposal.proposed_effects.clear();
        proposal.process_ticks.clear();
        proposal.pressure_noop_reasons = vec![PressureNoopReason {
            pressure_ref: "pressure:social:gate".to_owned(),
            reason: "이번 턴은 압력을 관찰만 했고 직접 밀지는 않는다.".to_owned(),
            evidence_refs: vec!["pressure:social:gate".to_owned()],
        }];

        if let Err(critique) = audit_resolution_proposal(&context, &proposal) {
            panic!("explicit pressure noop reason should pass audit: {critique:?}");
        }
    }

    #[test]
    fn derives_freeform_gate_trace_from_resolution_proposal() {
        let proposal = sample_proposal();
        let Some(trace) = freeform_gate_trace_from_proposal("6 문지기에게 둘러댄다", &proposal)
        else {
            panic!("freeform proposal should derive trace");
        };

        assert_eq!(trace.schema_version, FREEFORM_GATE_TRACE_SCHEMA_VERSION);
        assert_eq!(trace.raw_input, "6 문지기에게 둘러댄다");
        assert_eq!(trace.gate_results.len(), 1);
        assert_eq!(trace.final_outcome, ResolutionOutcomeKind::PartialSuccess);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "full prompt-context fixture keeps all audited projection surfaces visible"
    )]
    fn sample_context() -> PromptContextPacket {
        PromptContextPacket {
            schema_version: PROMPT_CONTEXT_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_resolution".to_owned(),
            turn_id: "turn_0004".to_owned(),
            current_turn: serde_json::json!({"player_input": "6 문지기에게 둘러댄다"}),
            opening_randomizer: Value::Null,
            output_contract: to_json_value(AgentOutputContract {
                language: "ko".to_owned(),
                must_return_json: true,
                hidden_truth_must_not_appear_in_visible_text: true,
                narrative_level: 1,
                narrative_budget: crate::agent_bridge::narrative_budget_for_level(Some(1)),
            }),
            pre_turn_simulation: sample_pre_turn_simulation(),
            visible_context: PromptVisibleContext {
                recent_scene_window: serde_json::json!(["서쪽 문 앞에 문지기가 서 있다."]),
                known_facts: serde_json::json!(["place:west_gate", "pressure:social:gate"]),
                active_scene_pressure: to_json_value(vec![ScenePressure {
                    schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
                    pressure_id: "pressure:social:gate".to_owned(),
                    kind: ScenePressureKind::SocialPermission,
                    visibility: ScenePressureVisibility::PlayerVisible,
                    intensity: 3,
                    urgency: ScenePressureUrgency::Immediate,
                    source_refs: vec!["visible_scene:gate".to_owned()],
                    provenance: None,
                    observable_signals: vec!["signal:guard_suspicion".to_owned()],
                    choice_affordances: vec!["speak carefully".to_owned()],
                    prose_effect: ScenePressureProseEffect {
                        paragraph_pressure: "tight".to_owned(),
                        sensory_focus: vec!["voice".to_owned()],
                        dialogue_style: "short".to_owned(),
                    },
                }]),
                active_plot_threads: serde_json::json!({"active_visible": []}),
                active_body_resource_state: to_json_value(
                    crate::body_resource::BodyResourcePacket {
                        schema_version: BODY_RESOURCE_PACKET_SCHEMA_VERSION.to_owned(),
                        world_id: "stw_resolution".to_owned(),
                        turn_id: "turn_0004".to_owned(),
                        body_constraints: vec![BodyConstraint {
                            schema_version: BODY_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                            constraint_id: "body:voice_shaking".to_owned(),
                            visibility: BodyResourceVisibility::PlayerVisible,
                            summary: "목소리가 떨린다.".to_owned(),
                            severity: 1,
                            source_refs: vec!["visible_scene:voice".to_owned()],
                            scene_pressure_kinds: vec!["social_permission".to_owned()],
                        }],
                        resources: Vec::new(),
                        compiler_policy: BodyResourcePolicy::default(),
                    },
                ),
                active_location_graph: to_json_value(LocationGraphPacket {
                    schema_version: LOCATION_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
                    world_id: "stw_resolution".to_owned(),
                    turn_id: "turn_0004".to_owned(),
                    current_location: Some(LocationNode {
                        schema_version: LOCATION_NODE_SCHEMA_VERSION.to_owned(),
                        location_id: "place:west_gate".to_owned(),
                        name: "서쪽 문".to_owned(),
                        knowledge_state: LocationKnowledgeState::Known,
                        notes: vec!["닫히기 직전이다.".to_owned()],
                        source_refs: vec!["visible_scene:gate".to_owned()],
                    }),
                    known_nearby_locations: Vec::new(),
                    compiler_policy: LocationGraphPolicy::default(),
                }),
                affordance_graph: to_json_value(AffordanceGraphPacket {
                    schema_version: AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
                    world_id: "stw_resolution".to_owned(),
                    turn_id: "turn_0004".to_owned(),
                    ordinary_choice_slots: vec![
                        sample_affordance_node(1, "move", AffordanceKind::Move),
                        sample_affordance_node(2, "observe", AffordanceKind::Observe),
                        sample_affordance_node(3, "contact", AffordanceKind::Contact),
                        sample_affordance_node(4, "body_resource", AffordanceKind::ResourceOrBody),
                        sample_affordance_node(
                            5,
                            "pressure_response",
                            AffordanceKind::PressureResponse,
                        ),
                    ],
                    compiler_policy: AffordanceGraphPolicy::default(),
                }),
                belief_graph: to_json_value(BeliefGraphPacket {
                    schema_version: BELIEF_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
                    world_id: "stw_resolution".to_owned(),
                    turn_id: "turn_0004".to_owned(),
                    protagonist_visible_beliefs: vec![BeliefNode {
                        schema_version: BELIEF_NODE_SCHEMA_VERSION.to_owned(),
                        belief_id: "belief:guard:doubt".to_owned(),
                        holder: BeliefHolder::PlayerVisibleNarrator,
                        confidence: BeliefConfidence::Inferred,
                        statement: "문지기는 말을 완전히 믿지 않는다.".to_owned(),
                        source_refs: vec!["visible_scene:gate".to_owned()],
                    }],
                    narrator_knowledge_limits: Vec::new(),
                    compiler_policy: BeliefGraphPolicy::default(),
                }),
                world_process_clock: to_json_value(vec![WorldProcess {
                    schema_version: WORLD_PROCESS_SCHEMA_VERSION.to_owned(),
                    process_id: "process:pressure:pressure:social:gate".to_owned(),
                    visibility: WorldProcessVisibility::PlayerVisible,
                    tempo: WorldProcessTempo::Immediate,
                    summary: "문 닫는 시간이 다가온다.".to_owned(),
                    next_tick_contract: "시간이 지나면 문이 닫힌다.".to_owned(),
                    source_refs: vec!["pressure:social:gate".to_owned()],
                }]),
                active_scene_director: Value::Null,
                active_consequence_spine: Value::Null,
                active_social_exchange: Value::Null,
                active_encounter_surface: Value::Null,
                narrative_style_state: Value::Null,
                active_character_text_design: Value::Null,
                active_change_ledger: Value::Null,
                active_pattern_debt: Value::Null,
                active_belief_graph: Value::Null,
                active_world_process_clock: Value::Null,
                active_actor_agency: Value::Null,
                active_player_intent_trace: Value::Null,
                active_turn_retrieval_controller: Value::Null,
                selected_context_capsules: Value::Null,
                active_autobiographical_index: Value::Null,
                selected_memory_items: serde_json::json!([]),
            },
            adjudication_context: PromptAdjudicationContext {
                private_adjudication_context: to_json_value(AgentPrivateAdjudicationContext {
                    hidden_timers: vec![AgentHiddenTimer {
                        timer_id: "timer:sealed_letter".to_owned(),
                        kind: "reveal".to_owned(),
                        remaining_turns: 2,
                        effect: "왕실 밀서가 자루 안에 있다".to_owned(),
                    }],
                    unrevealed_constraints: vec![AgentHiddenSecret {
                        secret_id: "secret:letter".to_owned(),
                        status: "unrevealed".to_owned(),
                        truth: "왕실 밀서가 자루 안에 있다".to_owned(),
                        reveal_conditions: vec!["inspect_bag".to_owned()],
                        forbidden_leaks: vec!["밀서".to_owned()],
                    }],
                    plausibility_gates: vec!["hidden_gate:sealed_letter".to_owned()],
                }),
                hidden_scene_pressure: to_json_value(Vec::<ScenePressure>::new()),
                hidden_world_process_clock: to_json_value(Vec::<WorldProcess>::new()),
                selected_adjudication_items: serde_json::json!([]),
            },
            source_of_truth_policy: Value::Null,
            prompt_policy: PromptContextPolicy::default(),
            budget_report: PromptContextBudgetReport {
                schema_version: PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION.to_owned(),
                world_id: "stw_resolution".to_owned(),
                turn_id: "turn_0004".to_owned(),
                budgets: BTreeMap::default(),
                included: Vec::new(),
                excluded: Vec::new(),
                compiler_policy: PromptContextBudgetPolicy::default(),
            },
        }
    }

    fn audit_failure(
        context: &PromptContextPacket,
        proposal: &ResolutionProposal,
    ) -> Box<ResolutionCritique> {
        match audit_resolution_proposal(context, proposal) {
            Ok(()) => panic!("resolution proposal unexpectedly passed audit"),
            Err(critique) => critique,
        }
    }

    fn to_json_value<T: Serialize>(value: T) -> Value {
        match serde_json::to_value(value) {
            Ok(value) => value,
            Err(error) => panic!("test fixture should serialize: {error}"),
        }
    }

    fn sample_pre_turn_simulation() -> PreTurnSimulationPass {
        PreTurnSimulationPass {
            schema_version: PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION.to_owned(),
            source_bundle_schema_version: SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_resolution".to_owned(),
            turn_id: "turn_0004".to_owned(),
            player_input: "6 문지기에게 둘러댄다".to_owned(),
            input_kind: TurnInputKind::FreeformAction,
            selected_choice: None,
            source_refs: vec![
                "turn:turn_0004".to_owned(),
                "pressure:social:gate".to_owned(),
                "affordance:slot:1:move".to_owned(),
            ],
            available_affordances: Vec::new(),
            blocked_affordances: Vec::new(),
            pressure_obligations: vec![PressureObligation {
                pressure_id: "pressure:social:gate".to_owned(),
                kind: ScenePressureKind::SocialPermission,
                obligation: "Move this pressure or state a visible no-op reason.".to_owned(),
                evidence_refs: vec!["pressure:social:gate".to_owned()],
            }],
            due_processes: Vec::new(),
            causal_risks: Vec::new(),
            required_resolution_fields: RequiredResolutionFields {
                resolution_proposal_required: true,
                next_choice_plan_required: true,
                pressure_movement_or_noop_reason_required: true,
                reason: "test fixture".to_owned(),
            },
            hidden_visibility_boundary: HiddenVisibilityBoundary {
                hidden_timer_count: 1,
                unrevealed_constraint_count: 1,
                forbidden_visible_needles: vec!["밀서".to_owned()],
                render_policy: "test fixture".to_owned(),
            },
            compiler_policy: PreTurnSimulationPolicy::default(),
        }
    }

    fn sample_affordance_node(slot: u8, suffix: &str, kind: AffordanceKind) -> AffordanceNode {
        AffordanceNode {
            schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
            slot,
            affordance_id: format!("affordance:slot:{slot}:{suffix}"),
            affordance_kind: kind,
            label_contract: format!("slot {slot} contract"),
            action_contract: format!("slot {slot} action"),
            source_refs: vec!["place:west_gate".to_owned()],
            pressure_refs: vec!["pressure:social:gate".to_owned()],
            forbidden_shortcuts: vec![format!("forbidden shortcut {slot}")],
        }
    }

    fn sample_turn_choices() -> Vec<TurnChoice> {
        vec![
            TurnChoice {
                slot: 1,
                tag: "다가섬".to_owned(),
                intent: "문 쪽으로 한 걸음 다가선다".to_owned(),
            },
            TurnChoice {
                slot: 2,
                tag: "눈치".to_owned(),
                intent: "문지기의 의심 어린 표정을 살핀다".to_owned(),
            },
            TurnChoice {
                slot: 3,
                tag: "말걸기".to_owned(),
                intent: "낮은 목소리로 문지기에게 말을 건다".to_owned(),
            },
            TurnChoice {
                slot: 4,
                tag: "숨 고르기".to_owned(),
                intent: "떨리는 목소리를 가다듬는다".to_owned(),
            },
            TurnChoice {
                slot: 5,
                tag: "압박 대응".to_owned(),
                intent: "닫히는 문 앞에서 시간을 벌 방법을 찾는다".to_owned(),
            },
            TurnChoice {
                slot: 6,
                tag: "자유서술".to_owned(),
                intent: "6 뒤에 직접 행동, 말, 내면 독백을 서술한다".to_owned(),
            },
            TurnChoice {
                slot: 7,
                tag: "판단 위임".to_owned(),
                intent: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
            },
        ]
    }

    #[expect(
        clippy::too_many_lines,
        reason = "complete choice-plan fixture keeps all seven slots auditable"
    )]
    fn sample_proposal() -> ResolutionProposal {
        ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "stw_resolution".to_owned(),
            turn_id: "turn_0004".to_owned(),
            interpreted_intent: ActionIntent {
                input_kind: ActionInputKind::Freeform,
                summary: "문지기를 말로 설득해 통과 허가를 얻으려 한다.".to_owned(),
                target_refs: vec![
                    "place:west_gate".to_owned(),
                    "belief:guard:doubt".to_owned(),
                ],
                pressure_refs: vec!["pressure:social:gate".to_owned()],
                evidence_refs: vec!["current_turn".to_owned(), "visible_scene:gate".to_owned()],
                ambiguity: ActionAmbiguity::Minor,
            },
            outcome: ResolutionOutcome {
                kind: ResolutionOutcomeKind::PartialSuccess,
                summary: "문지기는 완전히 믿지 않지만 말을 더 들어보기로 한다.".to_owned(),
                evidence_refs: vec!["pressure:social:gate".to_owned()],
            },
            gate_results: vec![GateResult {
                gate_kind: GateKind::SocialPermission,
                gate_ref: "pressure:social:gate".to_owned(),
                visibility: ResolutionVisibility::PlayerVisible,
                status: GateStatus::Softened,
                reason: "의심은 남았지만 대화를 이어갈 틈이 생겼다.".to_owned(),
                evidence_refs: vec!["signal:guard_suspicion".to_owned()],
            }],
            proposed_effects: vec![ProposedEffect {
                effect_kind: ProposedEffectKind::ScenePressureDelta,
                target_ref: "pressure:social:gate".to_owned(),
                visibility: ResolutionVisibility::PlayerVisible,
                summary: "사회적 허가 압력이 약간 누그러진다.".to_owned(),
                evidence_refs: vec!["pressure:social:gate".to_owned()],
            }],
            process_ticks: vec![ProcessTickProposal {
                process_ref: "process:pressure:pressure:social:gate".to_owned(),
                cause: ProcessTickCause::PlayerActionTouchedProcess,
                visibility: ResolutionVisibility::PlayerVisible,
                summary: "문이 닫히기 전 남은 시간이 더 빡빡해진다.".to_owned(),
                evidence_refs: vec!["pressure:social:gate".to_owned()],
            }],
            pressure_noop_reasons: Vec::new(),
            narrative_brief: NarrativeBrief {
                visible_summary: "문지기의 눈이 좁아지고, 말끝을 붙잡는 침묵이 길어진다."
                    .to_owned(),
                required_beats: vec!["의심이 남은 대화".to_owned()],
                forbidden_visible_details: vec!["hidden secret details".to_owned()],
            },
            next_choice_plan: vec![
                ChoicePlan {
                    slot: 1,
                    plan_kind: ChoicePlanKind::OrdinaryAffordance,
                    grounding_ref: "affordance:slot:1:move".to_owned(),
                    label_seed: "문 쪽으로 한 걸음 다가선다".to_owned(),
                    intent_seed: "위험을 감수하고 거리를 좁힌다.".to_owned(),
                    evidence_refs: vec!["affordance:slot:1:move".to_owned()],
                },
                ChoicePlan {
                    slot: 2,
                    plan_kind: ChoicePlanKind::OrdinaryAffordance,
                    grounding_ref: "affordance:slot:2:observe".to_owned(),
                    label_seed: "문지기의 눈치를 살핀다".to_owned(),
                    intent_seed: "이미 보이는 의심의 신호를 더 읽는다.".to_owned(),
                    evidence_refs: vec!["affordance:slot:2:observe".to_owned()],
                },
                ChoicePlan {
                    slot: 3,
                    plan_kind: ChoicePlanKind::OrdinaryAffordance,
                    grounding_ref: "affordance:slot:3:contact".to_owned(),
                    label_seed: "문지기에게 낮게 말한다".to_owned(),
                    intent_seed: "현장의 사회적 압력에 직접 반응한다.".to_owned(),
                    evidence_refs: vec!["affordance:slot:3:contact".to_owned()],
                },
                ChoicePlan {
                    slot: 4,
                    plan_kind: ChoicePlanKind::OrdinaryAffordance,
                    grounding_ref: "affordance:slot:4:body_resource".to_owned(),
                    label_seed: "떨리는 목소리를 가다듬는다".to_owned(),
                    intent_seed: "현재 몸 제약 안에서 설득력을 확보한다.".to_owned(),
                    evidence_refs: vec!["affordance:slot:4:body_resource".to_owned()],
                },
                ChoicePlan {
                    slot: 5,
                    plan_kind: ChoicePlanKind::OrdinaryAffordance,
                    grounding_ref: "affordance:slot:5:pressure_response".to_owned(),
                    label_seed: "닫히는 문 압박에 대응한다".to_owned(),
                    intent_seed: "가장 강한 시간 압력을 직접 건드린다.".to_owned(),
                    evidence_refs: vec!["affordance:slot:5:pressure_response".to_owned()],
                },
                ChoicePlan {
                    slot: 6,
                    plan_kind: ChoicePlanKind::Freeform,
                    grounding_ref: "current_turn".to_owned(),
                    label_seed: "자유서술".to_owned(),
                    intent_seed: "직접 행동을 입력한다.".to_owned(),
                    evidence_refs: vec!["current_turn".to_owned()],
                },
                ChoicePlan {
                    slot: 7,
                    plan_kind: ChoicePlanKind::DelegatedJudgment,
                    grounding_ref: "current_turn".to_owned(),
                    label_seed: "판단 위임".to_owned(),
                    intent_seed: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
                    evidence_refs: vec!["current_turn".to_owned()],
                },
            ],
        }
    }
}
