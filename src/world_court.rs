use crate::knowledge_ledger::{
    KnowledgeTier, can_render_knowledge_tier_to_player, render_rule_for_player,
    visible_knowledge_text_is_qualified,
};
use crate::models::TurnChoice;
use crate::prompt_context::PromptContextPacket;
use crate::resolution::{
    GateKind, GateResult, GateStatus, ProposedEffect, ProposedEffectKind, ResolutionCritique,
    ResolutionFailureKind, ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
    audit_resolution_choices, audit_resolution_proposal,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const WORLD_COURT_VERDICT_SCHEMA_VERSION: &str = "singulari.world_court_verdict.v1";
pub const WORLD_COURT_VIOLATION_SCHEMA_VERSION: &str = "singulari.world_court_violation.v1";
pub const WORLD_COURT_REPAIR_ACTION_SCHEMA_VERSION: &str = "singulari.world_court_repair_action.v1";
pub const WORLD_CHANGE_SET_SCHEMA_VERSION: &str = "singulari.world_change_set.v1";
pub const WORLD_CHANGE_EVENT_SCHEMA_VERSION: &str = "singulari.world_change_event.v1";
pub const FACT_MUTATION_SCHEMA_VERSION: &str = "singulari.fact_mutation.v1";
pub const COST_CLAIM_SCHEMA_VERSION: &str = "singulari.cost_claim.v1";
pub const VISIBILITY_CLAIM_SCHEMA_VERSION: &str = "singulari.visibility_claim.v1";

pub struct WorldCourtInput<'a> {
    pub context: &'a PromptContextPacket,
    pub resolution_proposal: Option<&'a ResolutionProposal>,
    pub next_choices: &'a [TurnChoice],
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldCourtVerdict {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub status: WorldCourtVerdictStatus,
    #[serde(default)]
    pub accepted_checks: Vec<String>,
    #[serde(default)]
    pub violations: Vec<WorldCourtViolation>,
    #[serde(default)]
    pub repair_actions: Vec<WorldCourtRepairAction>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldCourtVerdictStatus {
    Accept,
    Reject,
    Repair,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldCourtViolation {
    pub schema_version: String,
    pub layer: WorldCourtLayer,
    pub severity: WorldCourtViolationSeverity,
    pub check: String,
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
pub enum WorldCourtViolationSeverity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldCourtLayer {
    Schema,
    Ontology,
    Visibility,
    Evidence,
    Gate,
    Causality,
    ChoiceGrounding,
    Time,
    Space,
    BodyResource,
    SocialAuthority,
    ConsequenceReturn,
    ProjectionHook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldChangeSet {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub proposed_events: Vec<WorldChangeEvent>,
    #[serde(default)]
    pub fact_mutations: Vec<FactMutation>,
    #[serde(default)]
    pub cost_claims: Vec<CostClaim>,
    #[serde(default)]
    pub visibility_claims: Vec<VisibilityClaim>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldChangeEvent {
    pub schema_version: String,
    pub event_kind: WorldChangeEventKind,
    pub target_ref: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorldChangeEventKind {
    PlayerActionAttempted,
    ActionSucceeded,
    ActionFailed,
    ProcessTicked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FactMutation {
    pub schema_version: String,
    pub mutation_kind: FactMutationKind,
    pub target_ref: String,
    pub visibility: ResolutionVisibility,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_tier: Option<KnowledgeTier>,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FactMutationKind {
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
pub struct CostClaim {
    pub schema_version: String,
    pub cost_kind: CostClaimKind,
    pub target_ref: String,
    pub status: GateStatus,
    pub visibility: ResolutionVisibility,
    pub reason: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CostClaimKind {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VisibilityClaim {
    pub schema_version: String,
    pub target_ref: String,
    pub visibility: ResolutionVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_tier: Option<KnowledgeTier>,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldCourtRepairAction {
    pub schema_version: String,
    pub action_id: String,
    pub summary: String,
    #[serde(default)]
    pub target_refs: Vec<String>,
}

#[must_use]
pub fn adjudicate_world_changes(input: &WorldCourtInput<'_>) -> WorldCourtVerdict {
    let mut accepted_checks = Vec::new();
    let mut violations = Vec::new();
    let required_fields = &input.context.pre_turn_simulation.required_resolution_fields;

    if required_fields.resolution_proposal_required && input.resolution_proposal.is_none() {
        violations.push(WorldCourtViolation {
            schema_version: WORLD_COURT_VIOLATION_SCHEMA_VERSION.to_owned(),
            layer: WorldCourtLayer::Schema,
            severity: WorldCourtViolationSeverity::Blocking,
            check: "resolution_proposal_required".to_owned(),
            message: format!(
                "agent response missing required resolution_proposal before commit: world_id={}, turn_id={}, reason={}",
                input.context.world_id, input.context.turn_id, required_fields.reason
            ),
            rejected_refs: Vec::new(),
            required_changes: vec!["provide a grounded resolution_proposal".to_owned()],
            allowed_repair_scope: vec!["resolution_proposal".to_owned()],
        });
    } else if input.resolution_proposal.is_none() {
        accepted_checks.push("resolution_proposal_optional_absent".to_owned());
    }

    if let Some(proposal) = input.resolution_proposal {
        let resolution_accepted = match audit_resolution_proposal(input.context, proposal) {
            Ok(()) => {
                accepted_checks.extend(accepted_resolution_checks());
                true
            }
            Err(critique) => {
                violations.push(violation_from_resolution_critique(
                    "resolution_proposal",
                    &critique,
                ));
                false
            }
        };
        if resolution_accepted {
            let change_set = world_change_set_from_resolution(proposal);
            audit_world_change_set(&change_set, &mut accepted_checks, &mut violations);
            audit_world_change_set_against_context(
                input.context,
                &change_set,
                &mut accepted_checks,
                &mut violations,
            );
            audit_court_semantic_layers(proposal, &mut accepted_checks, &mut violations);
            audit_court_causality(proposal, &mut accepted_checks, &mut violations);
            match audit_resolution_choices(input.context, proposal, input.next_choices) {
                Ok(()) => accepted_checks.push("visible_choice_text".to_owned()),
                Err(critique) => {
                    violations.push(violation_from_resolution_critique(
                        "visible_choice_text",
                        &critique,
                    ));
                }
            }
        }
    }

    let status = if violations.is_empty() {
        WorldCourtVerdictStatus::Accept
    } else {
        WorldCourtVerdictStatus::Reject
    };
    WorldCourtVerdict {
        schema_version: WORLD_COURT_VERDICT_SCHEMA_VERSION.to_owned(),
        world_id: input.context.world_id.clone(),
        turn_id: input.context.turn_id.clone(),
        status,
        accepted_checks,
        violations,
        repair_actions: Vec::new(),
    }
}

/// Return an accepted world-court verdict or fail closed with a rendered verdict.
///
/// # Errors
///
/// Returns an error when any court check records a blocking violation.
pub fn enforce_world_court_acceptance(input: &WorldCourtInput<'_>) -> Result<WorldCourtVerdict> {
    let verdict = adjudicate_world_changes(input);
    if verdict.status == WorldCourtVerdictStatus::Accept {
        return Ok(verdict);
    }
    bail!("{}", render_world_court_verdict(&verdict));
}

#[must_use]
pub fn render_world_court_verdict(verdict: &WorldCourtVerdict) -> String {
    let mut lines = vec![
        format!("world court verdict: {:?}", verdict.status),
        format!("world_id: {}", verdict.world_id),
        format!("turn_id: {}", verdict.turn_id),
    ];
    for check in &verdict.accepted_checks {
        lines.push(format!("accepted_check: {check}"));
    }
    for violation in &verdict.violations {
        lines.push(format!(
            "violation: layer={:?}, check={}, severity={:?}, message={}",
            violation.layer, violation.check, violation.severity, violation.message
        ));
        if !violation.rejected_refs.is_empty() {
            lines.push(format!("  rejected_refs: {:?}", violation.rejected_refs));
        }
        if !violation.required_changes.is_empty() {
            lines.push(format!(
                "  required_changes: {:?}",
                violation.required_changes
            ));
        }
    }
    lines.join("\n")
}

fn audit_court_semantic_layers(
    proposal: &ResolutionProposal,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    for gate in &proposal.gate_results {
        if gate.gate_kind == GateKind::HiddenConstraint
            && gate.visibility == ResolutionVisibility::PlayerVisible
        {
            violations.push(domain_violation(
                WorldCourtLayer::Visibility,
                "hidden_constraint_gate_visibility",
                "hidden constraint gates must stay adjudication-only",
                gate.gate_ref.as_str(),
                "mark hidden constraints adjudication_only and render only symptoms",
            ));
            continue;
        }
        if ref_matches_gate_kind(gate.gate_kind, gate.gate_ref.as_str()) {
            push_accepted_check(accepted_checks, semantic_gate_check_name(gate.gate_kind));
        } else {
            violations.push(domain_violation(
                layer_for_gate_kind(gate.gate_kind),
                semantic_gate_check_name(gate.gate_kind),
                "gate kind does not match the referenced world domain",
                gate.gate_ref.as_str(),
                "use a gate_ref from the matching body/resource/location/social/time domain",
            ));
        }
    }

    for effect in &proposal.proposed_effects {
        audit_effect_knowledge_tier(effect, accepted_checks, violations);
        if ref_matches_effect_kind(effect.effect_kind, effect.target_ref.as_str()) {
            push_accepted_check(
                accepted_checks,
                semantic_effect_check_name(effect.effect_kind),
            );
        } else {
            violations.push(domain_violation(
                layer_for_effect_kind(effect.effect_kind),
                semantic_effect_check_name(effect.effect_kind),
                "effect kind does not match the target world domain",
                effect.target_ref.as_str(),
                "retarget the effect to the matching projection domain",
            ));
        }
    }

    for tick in &proposal.process_ticks {
        if tick.process_ref.starts_with("process:") {
            push_accepted_check(accepted_checks, "time_process_tick_right");
        } else {
            violations.push(domain_violation(
                WorldCourtLayer::Time,
                "time_process_tick_right",
                "process ticks must target process refs",
                tick.process_ref.as_str(),
                "retarget the tick to an active process:* ref",
            ));
        }
    }
}

fn audit_court_causality(
    proposal: &ResolutionProposal,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    let blocked_refs = proposal
        .gate_results
        .iter()
        .filter(|gate| gate.status == GateStatus::Blocked)
        .map(|gate| gate.gate_ref.clone())
        .collect::<Vec<_>>();
    if blocked_refs.is_empty() {
        push_accepted_check(accepted_checks, "blocked_gate_outcome_consistency");
        return;
    }
    if matches!(
        proposal.outcome.kind,
        ResolutionOutcomeKind::Success | ResolutionOutcomeKind::CostlySuccess
    ) {
        violations.push(WorldCourtViolation {
            schema_version: WORLD_COURT_VIOLATION_SCHEMA_VERSION.to_owned(),
            layer: WorldCourtLayer::Causality,
            severity: WorldCourtViolationSeverity::Blocking,
            check: "blocked_gate_outcome_consistency".to_owned(),
            message: "blocked gates cannot produce a full success or costly success outcome"
                .to_owned(),
            rejected_refs: blocked_refs,
            required_changes: vec![
                "change the outcome to blocked/partial/delayed/escalated or remove the blocked gate"
                    .to_owned(),
            ],
            allowed_repair_scope: vec!["resolution_proposal.outcome".to_owned()],
        });
    } else {
        push_accepted_check(accepted_checks, "blocked_gate_outcome_consistency");
    }
}

#[must_use]
pub fn world_change_set_from_resolution(proposal: &ResolutionProposal) -> WorldChangeSet {
    let mut evidence_refs = proposal.interpreted_intent.evidence_refs.clone();
    evidence_refs.extend(proposal.outcome.evidence_refs.clone());
    let proposed_events = world_change_events_from_resolution(proposal);
    let fact_mutations = proposal
        .proposed_effects
        .iter()
        .map(fact_mutation_from_effect)
        .collect::<Vec<_>>();
    let cost_claims = proposal
        .gate_results
        .iter()
        .map(cost_claim_from_gate)
        .collect::<Vec<_>>();
    let visibility_claims = visibility_claims_from_resolution(proposal);
    WorldChangeSet {
        schema_version: WORLD_CHANGE_SET_SCHEMA_VERSION.to_owned(),
        world_id: proposal.world_id.clone(),
        turn_id: proposal.turn_id.clone(),
        proposed_events,
        fact_mutations,
        cost_claims,
        visibility_claims,
        evidence_refs: dedupe_strings(evidence_refs),
    }
}

fn world_change_events_from_resolution(proposal: &ResolutionProposal) -> Vec<WorldChangeEvent> {
    let mut events = vec![WorldChangeEvent {
        schema_version: WORLD_CHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        event_kind: WorldChangeEventKind::PlayerActionAttempted,
        target_ref: proposal
            .interpreted_intent
            .target_refs
            .first()
            .cloned()
            .unwrap_or_else(|| "current_turn".to_owned()),
        summary: proposal.interpreted_intent.summary.clone(),
        evidence_refs: proposal.interpreted_intent.evidence_refs.clone(),
    }];
    events.push(WorldChangeEvent {
        schema_version: WORLD_CHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        event_kind: outcome_event_kind(proposal.outcome.kind),
        target_ref: "current_turn".to_owned(),
        summary: proposal.outcome.summary.clone(),
        evidence_refs: proposal.outcome.evidence_refs.clone(),
    });
    events.extend(proposal.process_ticks.iter().map(|tick| WorldChangeEvent {
        schema_version: WORLD_CHANGE_EVENT_SCHEMA_VERSION.to_owned(),
        event_kind: WorldChangeEventKind::ProcessTicked,
        target_ref: tick.process_ref.clone(),
        summary: tick.summary.clone(),
        evidence_refs: tick.evidence_refs.clone(),
    }));
    events
}

const fn outcome_event_kind(kind: ResolutionOutcomeKind) -> WorldChangeEventKind {
    match kind {
        ResolutionOutcomeKind::Success
        | ResolutionOutcomeKind::PartialSuccess
        | ResolutionOutcomeKind::CostlySuccess
        | ResolutionOutcomeKind::Delayed
        | ResolutionOutcomeKind::Escalated => WorldChangeEventKind::ActionSucceeded,
        ResolutionOutcomeKind::Blocked => WorldChangeEventKind::ActionFailed,
    }
}

fn fact_mutation_from_effect(effect: &ProposedEffect) -> FactMutation {
    FactMutation {
        schema_version: FACT_MUTATION_SCHEMA_VERSION.to_owned(),
        mutation_kind: fact_mutation_kind_from_effect(effect.effect_kind),
        target_ref: effect.target_ref.clone(),
        visibility: effect.visibility,
        summary: effect.summary.clone(),
        knowledge_tier: effect.knowledge_tier,
        evidence_refs: effect.evidence_refs.clone(),
    }
}

const fn fact_mutation_kind_from_effect(kind: ProposedEffectKind) -> FactMutationKind {
    match kind {
        ProposedEffectKind::ScenePressureDelta => FactMutationKind::ScenePressureDelta,
        ProposedEffectKind::BodyResourceDelta => FactMutationKind::BodyResourceDelta,
        ProposedEffectKind::LocationDelta => FactMutationKind::LocationDelta,
        ProposedEffectKind::RelationshipDelta => FactMutationKind::RelationshipDelta,
        ProposedEffectKind::BeliefDelta => FactMutationKind::BeliefDelta,
        ProposedEffectKind::WorldLoreDelta => FactMutationKind::WorldLoreDelta,
        ProposedEffectKind::PatternDebt => FactMutationKind::PatternDebt,
        ProposedEffectKind::PlayerIntentTrace => FactMutationKind::PlayerIntentTrace,
    }
}

fn cost_claim_from_gate(gate: &GateResult) -> CostClaim {
    CostClaim {
        schema_version: COST_CLAIM_SCHEMA_VERSION.to_owned(),
        cost_kind: cost_claim_kind_from_gate(gate.gate_kind),
        target_ref: gate.gate_ref.clone(),
        status: gate.status,
        visibility: gate.visibility,
        reason: gate.reason.clone(),
        evidence_refs: gate.evidence_refs.clone(),
    }
}

const fn cost_claim_kind_from_gate(kind: GateKind) -> CostClaimKind {
    match kind {
        GateKind::Body => CostClaimKind::Body,
        GateKind::Resource => CostClaimKind::Resource,
        GateKind::Location => CostClaimKind::Location,
        GateKind::SocialPermission => CostClaimKind::SocialPermission,
        GateKind::Knowledge => CostClaimKind::Knowledge,
        GateKind::TimePressure => CostClaimKind::TimePressure,
        GateKind::HiddenConstraint => CostClaimKind::HiddenConstraint,
        GateKind::WorldLaw => CostClaimKind::WorldLaw,
        GateKind::Affordance => CostClaimKind::Affordance,
    }
}

fn visibility_claims_from_resolution(proposal: &ResolutionProposal) -> Vec<VisibilityClaim> {
    let mut claims = proposal
        .proposed_effects
        .iter()
        .map(|effect| VisibilityClaim {
            schema_version: VISIBILITY_CLAIM_SCHEMA_VERSION.to_owned(),
            target_ref: effect.target_ref.clone(),
            visibility: effect.visibility,
            knowledge_tier: effect.knowledge_tier,
            summary: effect.summary.clone(),
            evidence_refs: effect.evidence_refs.clone(),
        })
        .collect::<Vec<_>>();
    claims.extend(proposal.process_ticks.iter().map(|tick| VisibilityClaim {
        schema_version: VISIBILITY_CLAIM_SCHEMA_VERSION.to_owned(),
        target_ref: tick.process_ref.clone(),
        visibility: tick.visibility,
        knowledge_tier: None,
        summary: tick.summary.clone(),
        evidence_refs: tick.evidence_refs.clone(),
    }));
    claims
}

fn audit_world_change_set(
    change_set: &WorldChangeSet,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    if change_set.proposed_events.is_empty() {
        violations.push(change_set_violation(
            WorldCourtLayer::Schema,
            "world_change_set_events",
            "world change set must include at least one proposed event",
            "current_turn",
            "derive attempted/resolved events before court adjudication",
        ));
    } else {
        push_accepted_check(accepted_checks, "world_change_set_events");
    }
    for event in &change_set.proposed_events {
        if event.evidence_refs.is_empty() {
            violations.push(change_set_violation(
                WorldCourtLayer::Evidence,
                "world_change_event_evidence",
                "world change events must be evidence-backed",
                event.target_ref.as_str(),
                "attach evidence_refs to the event source",
            ));
        }
    }
    for mutation in &change_set.fact_mutations {
        audit_fact_mutation(mutation, accepted_checks, violations);
    }
    for cost in &change_set.cost_claims {
        audit_cost_claim(cost, accepted_checks, violations);
    }
    for claim in &change_set.visibility_claims {
        audit_visibility_claim(claim, accepted_checks, violations);
    }
}

fn audit_world_change_set_against_context(
    context: &PromptContextPacket,
    change_set: &WorldChangeSet,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    let refs = court_reference_index(context);
    for event in &change_set.proposed_events {
        if event.event_kind == WorldChangeEventKind::ProcessTicked
            && !refs.contains(event.target_ref.as_str())
        {
            violations.push(change_set_violation(
                WorldCourtLayer::Time,
                "world_change_set_process_tick_due",
                "process tick events must target a process compiled as due for this turn",
                event.target_ref.as_str(),
                "retarget the tick to a due_process or remove the tick",
            ));
        }
    }
    for cost in &change_set.cost_claims {
        if cost.cost_kind == CostClaimKind::Affordance && !refs.contains(cost.target_ref.as_str()) {
            violations.push(change_set_violation(
                WorldCourtLayer::ChoiceGrounding,
                "world_change_set_affordance_exists",
                "affordance costs must reference a compiled available or blocked affordance",
                cost.target_ref.as_str(),
                "use an affordance_id from the pre-turn simulation pass",
            ));
        }
    }
    if !change_set
        .proposed_events
        .iter()
        .any(|event| event.event_kind == WorldChangeEventKind::ProcessTicked)
        || !violations
            .iter()
            .any(|violation| violation.check == "world_change_set_process_tick_due")
    {
        push_accepted_check(accepted_checks, "world_change_set_process_tick_due");
    }
    if !change_set
        .cost_claims
        .iter()
        .any(|cost| cost.cost_kind == CostClaimKind::Affordance)
        || !violations
            .iter()
            .any(|violation| violation.check == "world_change_set_affordance_exists")
    {
        push_accepted_check(accepted_checks, "world_change_set_affordance_exists");
    }
}

fn court_reference_index(context: &PromptContextPacket) -> BTreeSet<String> {
    let mut refs = BTreeSet::from(["current_turn".to_owned()]);
    refs.extend(context.pre_turn_simulation.source_refs.iter().cloned());
    if let Some(choice) = &context.pre_turn_simulation.selected_choice {
        refs.insert(format!("choice:{}", choice.slot));
    }
    for affordance in &context.pre_turn_simulation.available_affordances {
        refs.insert(affordance.affordance_id.clone());
        refs.extend(affordance.source_refs.iter().cloned());
        refs.extend(affordance.pressure_refs.iter().cloned());
    }
    for affordance in &context.pre_turn_simulation.blocked_affordances {
        refs.insert(affordance.affordance_id.clone());
    }
    for pressure in &context.pre_turn_simulation.pressure_obligations {
        refs.insert(pressure.pressure_id.clone());
        refs.extend(pressure.evidence_refs.iter().cloned());
    }
    for process in &context.pre_turn_simulation.due_processes {
        refs.insert(process.process_id.clone());
        refs.extend(process.evidence_refs.iter().cloned());
    }
    for risk in &context.pre_turn_simulation.causal_risks {
        refs.extend(risk.evidence_refs.iter().cloned());
    }
    refs
}

fn audit_fact_mutation(
    mutation: &FactMutation,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    if ref_matches_fact_mutation_kind(mutation.mutation_kind, mutation.target_ref.as_str()) {
        push_accepted_check(accepted_checks, "world_change_set_fact_domains");
    } else {
        violations.push(change_set_violation(
            layer_for_fact_mutation_kind(mutation.mutation_kind),
            "world_change_set_fact_domains",
            "fact mutation kind does not match the target world domain",
            mutation.target_ref.as_str(),
            "retarget the fact mutation to the matching world ref domain",
        ));
    }
}

fn audit_cost_claim(
    cost: &CostClaim,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    if cost.cost_kind == CostClaimKind::HiddenConstraint
        && cost.visibility == ResolutionVisibility::PlayerVisible
    {
        violations.push(change_set_violation(
            WorldCourtLayer::Visibility,
            "world_change_set_hidden_cost_visibility",
            "hidden constraint costs must stay adjudication-only",
            cost.target_ref.as_str(),
            "render symptoms instead of the hidden cost claim",
        ));
        return;
    }
    if ref_matches_cost_claim_kind(cost.cost_kind, cost.target_ref.as_str()) {
        push_accepted_check(accepted_checks, "world_change_set_cost_domains");
    } else {
        violations.push(change_set_violation(
            layer_for_cost_claim_kind(cost.cost_kind),
            "world_change_set_cost_domains",
            "cost claim kind does not match the target world domain",
            cost.target_ref.as_str(),
            "retarget the cost claim to the matching gate domain",
        ));
    }
}

fn audit_visibility_claim(
    claim: &VisibilityClaim,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    let tier = claim
        .knowledge_tier
        .unwrap_or(KnowledgeTier::PlayerObserved);
    if claim.visibility != ResolutionVisibility::PlayerVisible {
        push_accepted_check(accepted_checks, "world_change_set_visibility_scope");
        return;
    }
    if !can_render_knowledge_tier_to_player(tier) {
        violations.push(change_set_violation(
            WorldCourtLayer::Visibility,
            "world_change_set_visibility_tier",
            "player-visible change claims cannot render world-true hidden knowledge",
            claim.target_ref.as_str(),
            "keep hidden knowledge adjudication-only",
        ));
        return;
    }
    if !visible_knowledge_text_is_qualified(tier, claim.summary.as_str()) {
        violations.push(change_set_violation(
            WorldCourtLayer::Visibility,
            "world_change_set_visibility_render_rule",
            "player-visible change claim violates knowledge tier render rule",
            claim.target_ref.as_str(),
            "rewrite the claim with the required uncertainty/source/belief framing",
        ));
        return;
    }
    push_accepted_check(accepted_checks, "world_change_set_visibility_claims");
}

fn change_set_violation(
    layer: WorldCourtLayer,
    check: &str,
    message: &str,
    rejected_ref: &str,
    required_change: &str,
) -> WorldCourtViolation {
    WorldCourtViolation {
        schema_version: WORLD_COURT_VIOLATION_SCHEMA_VERSION.to_owned(),
        layer,
        severity: WorldCourtViolationSeverity::Blocking,
        check: check.to_owned(),
        message: message.to_owned(),
        rejected_refs: vec![rejected_ref.to_owned()],
        required_changes: vec![required_change.to_owned()],
        allowed_repair_scope: vec![
            "world_change_set".to_owned(),
            "resolution_proposal".to_owned(),
        ],
    }
}

fn audit_effect_knowledge_tier(
    effect: &ProposedEffect,
    accepted_checks: &mut Vec<String>,
    violations: &mut Vec<WorldCourtViolation>,
) {
    if effect.effect_kind != ProposedEffectKind::BeliefDelta {
        if effect.knowledge_tier.is_some() {
            violations.push(domain_violation(
                WorldCourtLayer::ProjectionHook,
                "knowledge_tier_effect_domain",
                "knowledge_tier may only be attached to belief/knowledge effects",
                effect.target_ref.as_str(),
                "remove knowledge_tier or change effect_kind to belief_delta",
            ));
        }
        return;
    }

    let tier = effect
        .knowledge_tier
        .unwrap_or(KnowledgeTier::PlayerObserved);
    if effect.visibility != ResolutionVisibility::PlayerVisible {
        push_accepted_check(accepted_checks, "knowledge_tier_adjudication_scope");
        return;
    }
    if !can_render_knowledge_tier_to_player(tier) {
        violations.push(domain_violation(
            WorldCourtLayer::Visibility,
            "knowledge_tier_visibility",
            "world-true hidden knowledge cannot be rendered to player-visible effects",
            effect.target_ref.as_str(),
            "keep hidden knowledge adjudication-only and expose only observable symptoms",
        ));
        return;
    }
    if !visible_knowledge_text_is_qualified(tier, effect.summary.as_str()) {
        let message = format!(
            "player-visible knowledge summary violates tier render rule: tier={tier:?}, rule={}",
            render_rule_for_player(tier)
        );
        violations.push(domain_violation(
            WorldCourtLayer::Visibility,
            "knowledge_tier_render_rule",
            message.as_str(),
            effect.target_ref.as_str(),
            "rewrite the visible summary with the required uncertainty/source/belief framing",
        ));
        return;
    }
    push_accepted_check(accepted_checks, "knowledge_tier_render_rule");
}

fn push_accepted_check(accepted_checks: &mut Vec<String>, check: &str) {
    if !accepted_checks.iter().any(|existing| existing == check) {
        accepted_checks.push(check.to_owned());
    }
}

fn domain_violation(
    layer: WorldCourtLayer,
    check: &str,
    message: &str,
    rejected_ref: &str,
    required_change: &str,
) -> WorldCourtViolation {
    WorldCourtViolation {
        schema_version: WORLD_COURT_VIOLATION_SCHEMA_VERSION.to_owned(),
        layer,
        severity: WorldCourtViolationSeverity::Blocking,
        check: check.to_owned(),
        message: message.to_owned(),
        rejected_refs: vec![rejected_ref.to_owned()],
        required_changes: vec![required_change.to_owned()],
        allowed_repair_scope: vec!["resolution_proposal".to_owned()],
    }
}

const fn semantic_gate_check_name(gate_kind: GateKind) -> &'static str {
    match gate_kind {
        GateKind::Body | GateKind::Resource => "body_resource_gate_ref_domain",
        GateKind::Location => "space_gate_ref_domain",
        GateKind::SocialPermission => "social_authority_gate_ref_domain",
        GateKind::Knowledge => "knowledge_gate_ref_domain",
        GateKind::TimePressure => "time_gate_ref_domain",
        GateKind::HiddenConstraint => "hidden_constraint_gate_visibility",
        GateKind::WorldLaw => "world_law_gate_ref_domain",
        GateKind::Affordance => "affordance_gate_ref_domain",
    }
}

const fn layer_for_gate_kind(gate_kind: GateKind) -> WorldCourtLayer {
    match gate_kind {
        GateKind::Body | GateKind::Resource => WorldCourtLayer::BodyResource,
        GateKind::Location => WorldCourtLayer::Space,
        GateKind::SocialPermission => WorldCourtLayer::SocialAuthority,
        GateKind::Knowledge => WorldCourtLayer::Evidence,
        GateKind::TimePressure => WorldCourtLayer::Time,
        GateKind::HiddenConstraint => WorldCourtLayer::Visibility,
        GateKind::WorldLaw => WorldCourtLayer::Causality,
        GateKind::Affordance => WorldCourtLayer::ChoiceGrounding,
    }
}

fn ref_matches_gate_kind(gate_kind: GateKind, item_ref: &str) -> bool {
    match gate_kind {
        GateKind::Body => ref_has_prefix(item_ref, &["body:"]),
        GateKind::Resource => ref_has_prefix(item_ref, &["resource:", "inventory:"]),
        GateKind::Location => ref_has_prefix(item_ref, &["place:", "location:"]),
        GateKind::SocialPermission => ref_has_prefix(
            item_ref,
            &[
                "pressure:social:",
                "rel:",
                "relationship:",
                "stance:",
                "social:",
            ],
        ),
        GateKind::Knowledge => ref_has_prefix(
            item_ref,
            &["belief:", "knowledge:", "visible_scene:", "current_turn"],
        ),
        GateKind::TimePressure => ref_has_prefix(item_ref, &["pressure:time:", "process:"]),
        GateKind::HiddenConstraint => true,
        GateKind::WorldLaw => ref_has_prefix(item_ref, &["law:", "world_law:"]),
        GateKind::Affordance => ref_has_prefix(item_ref, &["affordance:"]),
    }
}

const fn semantic_effect_check_name(effect_kind: ProposedEffectKind) -> &'static str {
    match effect_kind {
        ProposedEffectKind::ScenePressureDelta => "scene_pressure_effect_ref_domain",
        ProposedEffectKind::BodyResourceDelta => "body_resource_effect_ref_domain",
        ProposedEffectKind::LocationDelta => "space_effect_ref_domain",
        ProposedEffectKind::RelationshipDelta => "social_authority_effect_ref_domain",
        ProposedEffectKind::BeliefDelta => "knowledge_effect_ref_domain",
        ProposedEffectKind::WorldLoreDelta => "world_lore_effect_ref_domain",
        ProposedEffectKind::PatternDebt => "pattern_debt_effect_ref_domain",
        ProposedEffectKind::PlayerIntentTrace => "player_intent_effect_ref_domain",
    }
}

const fn layer_for_effect_kind(effect_kind: ProposedEffectKind) -> WorldCourtLayer {
    match effect_kind {
        ProposedEffectKind::ScenePressureDelta => WorldCourtLayer::Causality,
        ProposedEffectKind::BodyResourceDelta => WorldCourtLayer::BodyResource,
        ProposedEffectKind::LocationDelta => WorldCourtLayer::Space,
        ProposedEffectKind::RelationshipDelta => WorldCourtLayer::SocialAuthority,
        ProposedEffectKind::BeliefDelta => WorldCourtLayer::Evidence,
        ProposedEffectKind::WorldLoreDelta => WorldCourtLayer::Ontology,
        ProposedEffectKind::PatternDebt => WorldCourtLayer::ConsequenceReturn,
        ProposedEffectKind::PlayerIntentTrace => WorldCourtLayer::ProjectionHook,
    }
}

fn ref_matches_effect_kind(effect_kind: ProposedEffectKind, item_ref: &str) -> bool {
    match effect_kind {
        ProposedEffectKind::ScenePressureDelta => ref_has_prefix(item_ref, &["pressure:"]),
        ProposedEffectKind::BodyResourceDelta => {
            ref_has_prefix(item_ref, &["body:", "resource:", "inventory:"])
        }
        ProposedEffectKind::LocationDelta => ref_has_prefix(item_ref, &["place:", "location:"]),
        ProposedEffectKind::RelationshipDelta => {
            ref_has_prefix(item_ref, &["rel:", "relationship:", "stance:", "social:"])
        }
        ProposedEffectKind::BeliefDelta => ref_has_prefix(item_ref, &["belief:", "knowledge:"]),
        ProposedEffectKind::WorldLoreDelta => ref_has_prefix(item_ref, &["lore:", "world_fact:"]),
        ProposedEffectKind::PatternDebt => ref_has_prefix(item_ref, &["pattern_debt:"]),
        ProposedEffectKind::PlayerIntentTrace => {
            ref_has_prefix(item_ref, &["intent:", "player_intent:", "current_turn"])
        }
    }
}

const fn layer_for_fact_mutation_kind(mutation_kind: FactMutationKind) -> WorldCourtLayer {
    match mutation_kind {
        FactMutationKind::ScenePressureDelta => WorldCourtLayer::Causality,
        FactMutationKind::BodyResourceDelta => WorldCourtLayer::BodyResource,
        FactMutationKind::LocationDelta => WorldCourtLayer::Space,
        FactMutationKind::RelationshipDelta => WorldCourtLayer::SocialAuthority,
        FactMutationKind::BeliefDelta => WorldCourtLayer::Evidence,
        FactMutationKind::WorldLoreDelta => WorldCourtLayer::Ontology,
        FactMutationKind::PatternDebt => WorldCourtLayer::ConsequenceReturn,
        FactMutationKind::PlayerIntentTrace => WorldCourtLayer::ProjectionHook,
    }
}

fn ref_matches_fact_mutation_kind(mutation_kind: FactMutationKind, item_ref: &str) -> bool {
    match mutation_kind {
        FactMutationKind::ScenePressureDelta => ref_has_prefix(item_ref, &["pressure:"]),
        FactMutationKind::BodyResourceDelta => {
            ref_has_prefix(item_ref, &["body:", "resource:", "inventory:"])
        }
        FactMutationKind::LocationDelta => ref_has_prefix(item_ref, &["place:", "location:"]),
        FactMutationKind::RelationshipDelta => {
            ref_has_prefix(item_ref, &["rel:", "relationship:", "stance:", "social:"])
        }
        FactMutationKind::BeliefDelta => ref_has_prefix(item_ref, &["belief:", "knowledge:"]),
        FactMutationKind::WorldLoreDelta => ref_has_prefix(item_ref, &["lore:", "world_fact:"]),
        FactMutationKind::PatternDebt => ref_has_prefix(item_ref, &["pattern_debt:"]),
        FactMutationKind::PlayerIntentTrace => {
            ref_has_prefix(item_ref, &["intent:", "player_intent:", "current_turn"])
        }
    }
}

const fn layer_for_cost_claim_kind(cost_kind: CostClaimKind) -> WorldCourtLayer {
    match cost_kind {
        CostClaimKind::Body | CostClaimKind::Resource => WorldCourtLayer::BodyResource,
        CostClaimKind::Location => WorldCourtLayer::Space,
        CostClaimKind::SocialPermission => WorldCourtLayer::SocialAuthority,
        CostClaimKind::Knowledge => WorldCourtLayer::Evidence,
        CostClaimKind::TimePressure => WorldCourtLayer::Time,
        CostClaimKind::HiddenConstraint => WorldCourtLayer::Visibility,
        CostClaimKind::WorldLaw => WorldCourtLayer::Causality,
        CostClaimKind::Affordance => WorldCourtLayer::ChoiceGrounding,
    }
}

fn ref_matches_cost_claim_kind(cost_kind: CostClaimKind, item_ref: &str) -> bool {
    match cost_kind {
        CostClaimKind::Body => ref_has_prefix(item_ref, &["body:"]),
        CostClaimKind::Resource => ref_has_prefix(item_ref, &["resource:", "inventory:"]),
        CostClaimKind::Location => ref_has_prefix(item_ref, &["place:", "location:"]),
        CostClaimKind::SocialPermission => ref_has_prefix(
            item_ref,
            &[
                "pressure:social:",
                "rel:",
                "relationship:",
                "stance:",
                "social:",
            ],
        ),
        CostClaimKind::Knowledge => ref_has_prefix(
            item_ref,
            &["belief:", "knowledge:", "visible_scene:", "current_turn"],
        ),
        CostClaimKind::TimePressure => ref_has_prefix(item_ref, &["pressure:time:", "process:"]),
        CostClaimKind::HiddenConstraint => true,
        CostClaimKind::WorldLaw => ref_has_prefix(item_ref, &["law:", "world_law:"]),
        CostClaimKind::Affordance => ref_has_prefix(item_ref, &["affordance:"]),
    }
}

fn ref_has_prefix(item_ref: &str, prefixes: &[&str]) -> bool {
    prefixes
        .iter()
        .any(|prefix| item_ref == *prefix || item_ref.starts_with(*prefix))
}

fn accepted_resolution_checks() -> Vec<String> {
    [
        "resolution_schema",
        "evidence_refs",
        "ontology_refs",
        "pressure_causality",
        "choice_grounding",
        "hidden_visibility",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn dedupe_strings(values: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for value in values {
        if !deduped.iter().any(|existing| existing == &value) {
            deduped.push(value);
        }
    }
    deduped
}

fn violation_from_resolution_critique(
    check: &str,
    critique: &ResolutionCritique,
) -> WorldCourtViolation {
    WorldCourtViolation {
        schema_version: WORLD_COURT_VIOLATION_SCHEMA_VERSION.to_owned(),
        layer: layer_from_resolution_failure(critique.failure_kind),
        severity: WorldCourtViolationSeverity::Blocking,
        check: check.to_owned(),
        message: format!(
            "resolution proposal audit failed: failure_kind={:?}, message={}, rejected_refs={:?}",
            critique.failure_kind, critique.message, critique.rejected_refs
        ),
        rejected_refs: critique.rejected_refs.clone(),
        required_changes: critique.required_changes.clone(),
        allowed_repair_scope: critique.allowed_repair_scope.clone(),
    }
}

const fn layer_from_resolution_failure(failure_kind: ResolutionFailureKind) -> WorldCourtLayer {
    match failure_kind {
        ResolutionFailureKind::Schema => WorldCourtLayer::Schema,
        ResolutionFailureKind::TargetRef => WorldCourtLayer::Ontology,
        ResolutionFailureKind::VisibilityLeak => WorldCourtLayer::Visibility,
        ResolutionFailureKind::Evidence => WorldCourtLayer::Evidence,
        ResolutionFailureKind::Gate => WorldCourtLayer::Gate,
        ResolutionFailureKind::Causality => WorldCourtLayer::Causality,
        ResolutionFailureKind::ChoiceGrounding => WorldCourtLayer::ChoiceGrounding,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WorldCourtInput, WorldCourtLayer, WorldCourtVerdictStatus, adjudicate_world_changes,
        enforce_world_court_acceptance,
    };
    use crate::TurnInputKind;
    use crate::affordance_graph::AffordanceKind;
    use crate::knowledge_ledger::KnowledgeTier;
    use crate::pre_turn_simulation::{
        CompiledAffordance, DueProcess, HiddenVisibilityBoundary,
        PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION, PreTurnSimulationPass, PreTurnSimulationPolicy,
        PressureObligation, RequiredResolutionFields, SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION,
        SimulationVisibility,
    };
    use crate::prompt_context::{
        PROMPT_CONTEXT_PACKET_SCHEMA_VERSION, PromptAdjudicationContext, PromptContextPacket,
        PromptContextPolicy, PromptVisibleContext,
    };
    use crate::prompt_context_budget::{PromptContextBudgetPolicy, PromptContextBudgetReport};
    use crate::resolution::{
        ActionAmbiguity, ActionInputKind, GateKind, GateResult, GateStatus, NarrativeBrief,
        ProcessTickCause, ProcessTickProposal, ProposedEffect, ProposedEffectKind,
        RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionCritique, ResolutionFailureKind,
        ResolutionOutcome, ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
    };
    use crate::scene_pressure::ScenePressureKind;
    use crate::world_process_clock::WorldProcessTempo;
    use std::collections::BTreeMap;

    #[test]
    fn accepts_absent_resolution_when_not_required() {
        let context = minimal_context(false);
        let verdict = adjudicate_world_changes(&WorldCourtInput {
            context: &context,
            resolution_proposal: None,
            next_choices: &[],
        });

        assert_eq!(verdict.status, WorldCourtVerdictStatus::Accept);
        assert_eq!(
            verdict.accepted_checks,
            vec!["resolution_proposal_optional_absent"]
        );
    }

    #[test]
    fn rejects_missing_required_resolution_proposal() {
        let context = minimal_context(true);
        let verdict = adjudicate_world_changes(&WorldCourtInput {
            context: &context,
            resolution_proposal: None,
            next_choices: &[],
        });

        assert_eq!(verdict.status, WorldCourtVerdictStatus::Reject);
        assert_eq!(verdict.violations[0].layer, WorldCourtLayer::Schema);
        assert!(
            verdict.violations[0]
                .message
                .contains("missing required resolution_proposal before commit")
        );
        let result = enforce_world_court_acceptance(&WorldCourtInput {
            context: &context,
            resolution_proposal: None,
            next_choices: &[],
        });
        let Err(error) = result else {
            panic!("missing required resolution proposal should fail world court");
        };
        assert!(format!("{error:#}").contains("world court verdict"));
    }

    #[test]
    fn maps_target_ref_failure_to_ontology_layer() {
        let violation = super::violation_from_resolution_critique(
            "resolution_proposal",
            &ResolutionCritique {
                schema_version: "singulari.resolution_critique.v1".to_owned(),
                world_id: "stw_court".to_owned(),
                turn_id: "turn_0001".to_owned(),
                failure_kind: ResolutionFailureKind::TargetRef,
                message: "resolution proposal references an unknown or forbidden ref".to_owned(),
                rejected_refs: vec!["resource:missing_map".to_owned()],
                required_changes: Vec::new(),
                allowed_repair_scope: Vec::new(),
            },
        );

        assert_eq!(violation.layer, WorldCourtLayer::Ontology);
        assert_eq!(violation.rejected_refs, vec!["resource:missing_map"]);
    }

    #[test]
    fn accepted_resolution_checks_name_ontology_refs() {
        assert!(
            super::accepted_resolution_checks()
                .iter()
                .any(|check| check == "ontology_refs")
        );
    }

    #[test]
    fn semantic_layers_reject_mismatched_body_effect_ref() {
        let mut proposal = semantic_probe();
        proposal.proposed_effects.push(ProposedEffect {
            effect_kind: ProposedEffectKind::BodyResourceDelta,
            target_ref: "place:west_gate".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            knowledge_tier: None,
            summary: "wrong domain".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_court_semantic_layers(&proposal, &mut accepted_checks, &mut violations);

        assert!(accepted_checks.is_empty());
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].layer, WorldCourtLayer::BodyResource);
        assert_eq!(violations[0].check, "body_resource_effect_ref_domain");
    }

    #[test]
    fn semantic_layers_reject_player_visible_hidden_gate() {
        let mut proposal = semantic_probe();
        proposal.gate_results.push(GateResult {
            gate_kind: GateKind::HiddenConstraint,
            gate_ref: "hidden:assassin_waiting".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            status: GateStatus::Blocked,
            reason: "should not render".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_court_semantic_layers(&proposal, &mut accepted_checks, &mut violations);

        assert!(accepted_checks.is_empty());
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].layer, WorldCourtLayer::Visibility);
        assert_eq!(violations[0].check, "hidden_constraint_gate_visibility");
    }

    #[test]
    fn semantic_layers_accept_named_layer_domains() {
        let mut proposal = semantic_probe();
        proposal.gate_results.push(GateResult {
            gate_kind: GateKind::Location,
            gate_ref: "place:west_gate".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            status: GateStatus::Passed,
            reason: "reachable".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        proposal.proposed_effects.push(ProposedEffect {
            effect_kind: ProposedEffectKind::RelationshipDelta,
            target_ref: "rel:guard->protagonist:distance".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            knowledge_tier: None,
            summary: "stance shifts".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        proposal.process_ticks.push(ProcessTickProposal {
            process_ref: "process:gate_closing".to_owned(),
            cause: ProcessTickCause::PlayerActionTouchedProcess,
            visibility: ResolutionVisibility::PlayerVisible,
            summary: "gate clock advances".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_court_semantic_layers(&proposal, &mut accepted_checks, &mut violations);

        assert!(violations.is_empty());
        assert!(
            accepted_checks
                .iter()
                .any(|item| item == "space_gate_ref_domain")
        );
        assert!(
            accepted_checks
                .iter()
                .any(|item| item == "social_authority_effect_ref_domain")
        );
        assert!(
            accepted_checks
                .iter()
                .any(|item| item == "time_process_tick_right")
        );
    }

    #[test]
    fn semantic_layers_reject_hidden_knowledge_visible_effect() {
        let mut proposal = semantic_probe();
        proposal.proposed_effects.push(ProposedEffect {
            effect_kind: ProposedEffectKind::BeliefDelta,
            target_ref: "knowledge:hidden_assassin".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            knowledge_tier: Some(KnowledgeTier::WorldTrueHidden),
            summary: "북문 뒤에 암살자가 숨어 있다.".to_owned(),
            evidence_refs: vec!["hidden:assassin_waiting".to_owned()],
        });
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_court_semantic_layers(&proposal, &mut accepted_checks, &mut violations);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].layer, WorldCourtLayer::Visibility);
        assert_eq!(violations[0].check, "knowledge_tier_visibility");
    }

    #[test]
    fn semantic_layers_require_inferred_knowledge_qualification() {
        let mut proposal = semantic_probe();
        proposal.proposed_effects.push(ProposedEffect {
            effect_kind: ProposedEffectKind::BeliefDelta,
            target_ref: "knowledge:north_gate_noise".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            knowledge_tier: Some(KnowledgeTier::PlayerInferred),
            summary: "북문 뒤에 누군가 있다.".to_owned(),
            evidence_refs: vec!["signal:metal_noise".to_owned()],
        });
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_court_semantic_layers(&proposal, &mut accepted_checks, &mut violations);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].check, "knowledge_tier_render_rule");

        proposal.proposed_effects[0].summary = "북문 뒤에 누군가 있을 가능성이 있다.".to_owned();
        accepted_checks.clear();
        violations.clear();

        super::audit_court_semantic_layers(&proposal, &mut accepted_checks, &mut violations);

        assert!(violations.is_empty());
        assert!(
            accepted_checks
                .iter()
                .any(|check| check == "knowledge_tier_render_rule")
        );
    }

    #[test]
    fn world_change_set_from_resolution_maps_events_facts_costs_and_visibility() {
        let mut proposal = semantic_probe();
        proposal.gate_results.push(GateResult {
            gate_kind: GateKind::Resource,
            gate_ref: "inventory:oil".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            status: GateStatus::CostImposed,
            reason: "uses oil".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        proposal.proposed_effects.push(ProposedEffect {
            effect_kind: ProposedEffectKind::BodyResourceDelta,
            target_ref: "inventory:oil".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            knowledge_tier: None,
            summary: "oil is spent".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        proposal.process_ticks.push(ProcessTickProposal {
            process_ref: "process:gate_closing".to_owned(),
            cause: ProcessTickCause::PlayerActionTouchedProcess,
            visibility: ResolutionVisibility::PlayerVisible,
            summary: "gate clock advances".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });

        let change_set = super::world_change_set_from_resolution(&proposal);

        assert_eq!(change_set.proposed_events.len(), 3);
        assert_eq!(change_set.fact_mutations.len(), 1);
        assert_eq!(change_set.cost_claims.len(), 1);
        assert_eq!(change_set.visibility_claims.len(), 2);
        assert!(
            change_set
                .proposed_events
                .iter()
                .any(|event| event.event_kind == super::WorldChangeEventKind::ProcessTicked)
        );
    }

    #[test]
    fn world_change_set_rejects_mismatched_fact_mutation_domain() {
        let change_set = super::WorldChangeSet {
            schema_version: super::WORLD_CHANGE_SET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_court".to_owned(),
            turn_id: "turn_0001".to_owned(),
            proposed_events: vec![super::WorldChangeEvent {
                schema_version: super::WORLD_CHANGE_EVENT_SCHEMA_VERSION.to_owned(),
                event_kind: super::WorldChangeEventKind::PlayerActionAttempted,
                target_ref: "current_turn".to_owned(),
                summary: "attempt".to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            }],
            fact_mutations: vec![super::FactMutation {
                schema_version: super::FACT_MUTATION_SCHEMA_VERSION.to_owned(),
                mutation_kind: super::FactMutationKind::BodyResourceDelta,
                target_ref: "place:west_gate".to_owned(),
                visibility: ResolutionVisibility::PlayerVisible,
                summary: "wrong target domain".to_owned(),
                knowledge_tier: None,
                evidence_refs: vec!["current_turn".to_owned()],
            }],
            cost_claims: Vec::new(),
            visibility_claims: Vec::new(),
            evidence_refs: vec!["current_turn".to_owned()],
        };
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_world_change_set(&change_set, &mut accepted_checks, &mut violations);

        assert!(
            accepted_checks
                .iter()
                .any(|check| check == "world_change_set_events")
        );
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].layer, WorldCourtLayer::BodyResource);
        assert_eq!(violations[0].check, "world_change_set_fact_domains");
    }

    #[test]
    fn world_change_set_rejects_process_tick_that_is_not_due() {
        let mut context = minimal_context(false);
        context.pre_turn_simulation.due_processes.push(DueProcess {
            process_id: "process:gate_closing".to_owned(),
            visibility: SimulationVisibility::PlayerVisible,
            tempo: WorldProcessTempo::Immediate,
            tick_condition: "player touches the gate".to_owned(),
            evidence_refs: vec!["pressure:test".to_owned()],
        });
        let change_set = super::WorldChangeSet {
            schema_version: super::WORLD_CHANGE_SET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_court".to_owned(),
            turn_id: "turn_0001".to_owned(),
            proposed_events: vec![super::WorldChangeEvent {
                schema_version: super::WORLD_CHANGE_EVENT_SCHEMA_VERSION.to_owned(),
                event_kind: super::WorldChangeEventKind::ProcessTicked,
                target_ref: "process:unknown_alarm".to_owned(),
                summary: "unknown process tick".to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            }],
            fact_mutations: Vec::new(),
            cost_claims: Vec::new(),
            visibility_claims: Vec::new(),
            evidence_refs: vec!["current_turn".to_owned()],
        };
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_world_change_set_against_context(
            &context,
            &change_set,
            &mut accepted_checks,
            &mut violations,
        );

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].layer, WorldCourtLayer::Time);
        assert_eq!(violations[0].check, "world_change_set_process_tick_due");
    }

    #[test]
    fn world_change_set_accepts_due_process_and_compiled_affordance_refs() {
        let mut context = minimal_context(false);
        context
            .pre_turn_simulation
            .available_affordances
            .push(CompiledAffordance {
                slot: 1,
                affordance_id: "affordance:inspect_gate".to_owned(),
                affordance_kind: AffordanceKind::Observe,
                action_contract: "inspect the gate".to_owned(),
                source_refs: vec!["current_turn".to_owned()],
                pressure_refs: Vec::new(),
            });
        context.pre_turn_simulation.due_processes.push(DueProcess {
            process_id: "process:gate_closing".to_owned(),
            visibility: SimulationVisibility::PlayerVisible,
            tempo: WorldProcessTempo::Immediate,
            tick_condition: "player touches the gate".to_owned(),
            evidence_refs: vec!["pressure:test".to_owned()],
        });
        let change_set = super::WorldChangeSet {
            schema_version: super::WORLD_CHANGE_SET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_court".to_owned(),
            turn_id: "turn_0001".to_owned(),
            proposed_events: vec![super::WorldChangeEvent {
                schema_version: super::WORLD_CHANGE_EVENT_SCHEMA_VERSION.to_owned(),
                event_kind: super::WorldChangeEventKind::ProcessTicked,
                target_ref: "process:gate_closing".to_owned(),
                summary: "gate clock advances".to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            }],
            fact_mutations: Vec::new(),
            cost_claims: vec![super::CostClaim {
                schema_version: super::COST_CLAIM_SCHEMA_VERSION.to_owned(),
                cost_kind: super::CostClaimKind::Affordance,
                target_ref: "affordance:inspect_gate".to_owned(),
                status: GateStatus::Passed,
                visibility: ResolutionVisibility::PlayerVisible,
                reason: "compiled choice".to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            }],
            visibility_claims: Vec::new(),
            evidence_refs: vec!["current_turn".to_owned()],
        };
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_world_change_set_against_context(
            &context,
            &change_set,
            &mut accepted_checks,
            &mut violations,
        );

        assert!(violations.is_empty());
        assert!(
            accepted_checks
                .iter()
                .any(|check| check == "world_change_set_process_tick_due")
        );
        assert!(
            accepted_checks
                .iter()
                .any(|check| check == "world_change_set_affordance_exists")
        );
    }

    #[test]
    fn semantic_causality_rejects_full_success_when_gate_is_blocked() {
        let mut proposal = semantic_probe();
        proposal.outcome.kind = ResolutionOutcomeKind::Success;
        proposal.gate_results.push(GateResult {
            gate_kind: GateKind::Resource,
            gate_ref: "inventory:missing_key".to_owned(),
            visibility: ResolutionVisibility::PlayerVisible,
            status: GateStatus::Blocked,
            reason: "missing key".to_owned(),
            evidence_refs: vec!["current_turn".to_owned()],
        });
        let mut accepted_checks = Vec::new();
        let mut violations = Vec::new();

        super::audit_court_causality(&proposal, &mut accepted_checks, &mut violations);

        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].layer, WorldCourtLayer::Causality);
        assert_eq!(violations[0].check, "blocked_gate_outcome_consistency");
    }

    fn minimal_context(resolution_required: bool) -> PromptContextPacket {
        PromptContextPacket {
            schema_version: PROMPT_CONTEXT_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_court".to_owned(),
            turn_id: "turn_0001".to_owned(),
            current_turn: serde_json::Value::Null,
            opening_randomizer: serde_json::Value::Null,
            output_contract: serde_json::Value::Null,
            pre_turn_simulation: PreTurnSimulationPass {
                schema_version: PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION.to_owned(),
                source_bundle_schema_version: SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION.to_owned(),
                world_id: "stw_court".to_owned(),
                turn_id: "turn_0001".to_owned(),
                player_input: "1".to_owned(),
                input_kind: TurnInputKind::NumericChoice,
                selected_choice: None,
                source_refs: Vec::new(),
                available_affordances: Vec::new(),
                blocked_affordances: Vec::new(),
                pressure_obligations: vec![PressureObligation {
                    pressure_id: "pressure:test".to_owned(),
                    kind: ScenePressureKind::Knowledge,
                    obligation: "test pressure".to_owned(),
                    evidence_refs: vec!["pressure:test".to_owned()],
                }],
                due_processes: Vec::new(),
                causal_risks: Vec::new(),
                required_resolution_fields: RequiredResolutionFields {
                    resolution_proposal_required: resolution_required,
                    next_choice_plan_required: resolution_required,
                    pressure_movement_or_noop_reason_required: resolution_required,
                    reason: "test fixture".to_owned(),
                },
                hidden_visibility_boundary: HiddenVisibilityBoundary {
                    hidden_timer_count: 0,
                    unrevealed_constraint_count: 0,
                    forbidden_visible_needles: Vec::new(),
                    render_policy: "test fixture".to_owned(),
                },
                compiler_policy: PreTurnSimulationPolicy::default(),
            },
            visible_context: PromptVisibleContext {
                recent_scene_window: serde_json::Value::Null,
                known_facts: serde_json::Value::Null,
                active_scene_pressure: serde_json::Value::Null,
                active_plot_threads: serde_json::Value::Null,
                active_body_resource_state: serde_json::Value::Null,
                active_location_graph: serde_json::Value::Null,
                affordance_graph: serde_json::Value::Null,
                belief_graph: serde_json::Value::Null,
                world_process_clock: serde_json::Value::Null,
                active_scene_director: serde_json::Value::Null,
                active_consequence_spine: serde_json::Value::Null,
                active_social_exchange: serde_json::Value::Null,
                active_encounter_surface: serde_json::Value::Null,
                narrative_style_state: serde_json::Value::Null,
                active_character_text_design: serde_json::Value::Null,
                active_change_ledger: serde_json::Value::Null,
                active_pattern_debt: serde_json::Value::Null,
                active_belief_graph: serde_json::Value::Null,
                active_world_process_clock: serde_json::Value::Null,
                active_actor_agency: serde_json::Value::Null,
                active_player_intent_trace: serde_json::Value::Null,
                active_turn_retrieval_controller: serde_json::Value::Null,
                selected_context_capsules: serde_json::Value::Null,
                active_autobiographical_index: serde_json::Value::Null,
                selected_memory_items: serde_json::Value::Null,
            },
            adjudication_context: PromptAdjudicationContext {
                private_adjudication_context: serde_json::Value::Null,
                hidden_scene_pressure: serde_json::Value::Null,
                hidden_world_process_clock: serde_json::Value::Null,
                selected_adjudication_items: serde_json::Value::Null,
            },
            source_of_truth_policy: serde_json::Value::Null,
            prompt_policy: PromptContextPolicy::default(),
            budget_report: PromptContextBudgetReport {
                schema_version:
                    crate::prompt_context_budget::PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION
                        .to_owned(),
                world_id: "stw_court".to_owned(),
                turn_id: "turn_0001".to_owned(),
                budgets: BTreeMap::default(),
                included: Vec::new(),
                excluded: Vec::new(),
                compiler_policy: PromptContextBudgetPolicy::default(),
            },
        }
    }

    fn semantic_probe() -> ResolutionProposal {
        ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: "stw_court".to_owned(),
            turn_id: "turn_0001".to_owned(),
            interpreted_intent: crate::resolution::ActionIntent {
                input_kind: ActionInputKind::Freeform,
                summary: "probe".to_owned(),
                target_refs: Vec::new(),
                pressure_refs: Vec::new(),
                evidence_refs: vec!["current_turn".to_owned()],
                ambiguity: ActionAmbiguity::Clear,
            },
            outcome: ResolutionOutcome {
                kind: ResolutionOutcomeKind::Blocked,
                summary: "probe".to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            },
            gate_results: Vec::new(),
            proposed_effects: Vec::new(),
            process_ticks: Vec::new(),
            pressure_noop_reasons: Vec::new(),
            narrative_brief: NarrativeBrief {
                visible_summary: "probe".to_owned(),
                required_beats: Vec::new(),
                forbidden_visible_details: Vec::new(),
            },
            next_choice_plan: Vec::new(),
        }
    }
}
