use crate::models::TurnChoice;
use crate::prompt_context::PromptContextPacket;
use crate::resolution::{
    ResolutionCritique, ResolutionFailureKind, ResolutionProposal, audit_resolution_choices,
    audit_resolution_proposal,
};
use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

pub const WORLD_COURT_VERDICT_SCHEMA_VERSION: &str = "singulari.world_court_verdict.v1";
pub const WORLD_COURT_VIOLATION_SCHEMA_VERSION: &str = "singulari.world_court_violation.v1";
pub const WORLD_COURT_REPAIR_ACTION_SCHEMA_VERSION: &str = "singulari.world_court_repair_action.v1";

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
    use crate::pre_turn_simulation::{
        HiddenVisibilityBoundary, PRE_TURN_SIMULATION_PASS_SCHEMA_VERSION, PreTurnSimulationPass,
        PreTurnSimulationPolicy, PressureObligation, RequiredResolutionFields,
        SIMULATION_SOURCE_BUNDLE_SCHEMA_VERSION,
    };
    use crate::prompt_context::{
        PROMPT_CONTEXT_PACKET_SCHEMA_VERSION, PromptAdjudicationContext, PromptContextPacket,
        PromptContextPolicy, PromptVisibleContext,
    };
    use crate::prompt_context_budget::{PromptContextBudgetPolicy, PromptContextBudgetReport};
    use crate::resolution::{ResolutionCritique, ResolutionFailureKind};
    use crate::scene_pressure::ScenePressureKind;
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
}
