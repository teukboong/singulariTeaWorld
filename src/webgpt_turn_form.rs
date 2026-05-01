use crate::agent_bridge::{
    AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentResponseAdjudication, AgentTurnResponse,
    PendingAgentTurn,
};
use crate::models::{
    FREEFORM_CHOICE_SLOT, FREEFORM_CHOICE_TAG, GUIDE_CHOICE_REDACTED_INTENT, GUIDE_CHOICE_SLOT,
    GUIDE_CHOICE_TAG, NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, TurnChoice, TurnInputKind,
    default_freeform_choice, default_guide_choice, normalize_turn_choices,
};
use crate::player_surface::{concrete_delta_is_specific, player_surface_forbidden_terms};
use crate::pre_turn_simulation::CompiledAffordance;
use crate::prompt_context::PromptContextPacket;
use crate::resolution::{
    ActionAmbiguity, ActionInputKind, ActionIntent, ChoicePlan, ChoicePlanKind, NarrativeBrief,
    ProposedEffect, ProposedEffectKind, RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
    ResolutionOutcomeKind, ResolutionProposal, ResolutionVisibility,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const TURN_FORM_SPEC_SCHEMA_VERSION: &str = "singulari.webgpt_turn_form_spec.v1";
pub const TURN_FORM_SUBMISSION_SCHEMA_VERSION: &str = "singulari.webgpt_turn_form_submission.v1";
pub const TURN_FORM_SUBMIT_RESULT_SCHEMA_VERSION: &str = "singulari.webgpt_turn_form_submit.v1";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormSpec {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub player_input: String,
    pub input_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_slot: Option<u8>,
    pub narrative_budget: TurnFormNarrativeBudget,
    #[serde(default)]
    pub allowed_ambiguity: Vec<String>,
    #[serde(default)]
    pub allowed_outcomes: Vec<String>,
    #[serde(default)]
    pub choice_slots: Vec<TurnFormChoiceSlotSpec>,
    #[serde(default)]
    pub pressure_options: Vec<TurnFormPressureOption>,
    #[serde(default)]
    pub allowed_evidence_refs: Vec<String>,
    #[serde(default)]
    pub assembly_rules: Vec<String>,
    pub player_surface_rules: TurnFormPlayerSurfaceRules,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormNarrativeBudget {
    pub narrative_level: u8,
    pub ordinary_turn_blocks: u8,
    pub standard_choice_turn_blocks: u8,
    pub target_chars: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormChoiceSlotSpec {
    pub slot: u8,
    pub affordance_id: String,
    pub affordance_kind: String,
    pub label_contract: String,
    pub action_contract: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormPressureOption {
    pub pressure_id: String,
    pub kind: String,
    pub obligation: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormPlayerSurfaceRules {
    pub product_mode: String,
    #[serde(default)]
    pub forbidden_terms: Vec<String>,
    #[serde(default)]
    pub choice_text_contract: Vec<String>,
    pub concrete_delta_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormSubmission {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub intent_summary: String,
    #[serde(default = "default_ambiguity")]
    pub intent_ambiguity: String,
    #[serde(default = "default_outcome_kind")]
    pub outcome_kind: String,
    pub outcome_summary: String,
    #[serde(default)]
    pub pressure_movements: Vec<TurnFormPressureMovement>,
    pub narrative: TurnFormNarrativeSubmission,
    #[serde(default)]
    pub next_choices: Vec<TurnFormChoiceSubmission>,
    pub director_notes: TurnFormDirectorNotes,
    #[serde(default)]
    pub adjudication_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormPressureMovement {
    pub pressure_id: String,
    #[serde(default = "default_pressure_change")]
    pub change: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormNarrativeSubmission {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default)]
    pub text_blocks: Vec<String>,
    #[serde(default)]
    pub tone_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormChoiceSubmission {
    pub slot: u8,
    pub tag: String,
    pub intent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormDirectorNotes {
    pub beat_type: String,
    pub concrete_delta: String,
    pub scene_exit_progress: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormRejection {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub field_errors: Vec<TurnFormFieldError>,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct TurnFormFieldError {
    pub field_path: String,
    pub message: String,
    #[serde(default)]
    pub allowed_values: Vec<String>,
}

#[must_use]
pub fn build_turn_form_spec(
    pending: &PendingAgentTurn,
    context: &PromptContextPacket,
) -> TurnFormSpec {
    let allowed_evidence_refs = collect_allowed_evidence_refs(context).into_iter().collect();
    TurnFormSpec {
        schema_version: TURN_FORM_SPEC_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        player_input: pending.player_input.clone(),
        input_kind: context.pre_turn_simulation.input_kind.to_string(),
        selected_slot: pending.selected_choice.as_ref().map(|choice| choice.slot),
        narrative_budget: TurnFormNarrativeBudget {
            narrative_level: pending.output_contract.narrative_level,
            ordinary_turn_blocks: pending
                .output_contract
                .narrative_budget
                .ordinary_turn_blocks,
            standard_choice_turn_blocks: pending
                .output_contract
                .narrative_budget
                .standard_choice_turn_blocks,
            target_chars: pending.output_contract.narrative_budget.target_chars,
        },
        allowed_ambiguity: vec!["clear".to_owned(), "minor".to_owned(), "high".to_owned()],
        allowed_outcomes: vec![
            "success".to_owned(),
            "partial_success".to_owned(),
            "blocked".to_owned(),
            "costly_success".to_owned(),
            "delayed".to_owned(),
            "escalated".to_owned(),
        ],
        choice_slots: context
            .pre_turn_simulation
            .available_affordances
            .iter()
            .filter(|affordance| (1..=5).contains(&affordance.slot))
            .map(choice_slot_spec)
            .collect(),
        pressure_options: context
            .pre_turn_simulation
            .pressure_obligations
            .iter()
            .map(|pressure| TurnFormPressureOption {
                pressure_id: pressure.pressure_id.clone(),
                kind: serde_json::to_value(pressure.kind)
                    .ok()
                    .and_then(|value| value.as_str().map(ToOwned::to_owned))
                    .unwrap_or_else(|| "unknown".to_owned()),
                obligation: pressure.obligation.clone(),
                evidence_refs: pressure.evidence_refs.clone(),
            })
            .collect(),
        allowed_evidence_refs,
        assembly_rules: vec![
            "Fill exactly slots 1..5; slots 6 and 7 are appended by the backend.".to_owned(),
            "Do not include hidden, secret, or adjudication-only refs in any visible field."
                .to_owned(),
            "If a visible pressure is not moved, the backend records a no-op reason automatically."
                .to_owned(),
            "Return polished Korean VN scene prose; do not return AgentTurnResponse JSON."
                .to_owned(),
            "Ordinary choices must include surface_text as a diegetic action sentence."
                .to_owned(),
        ],
        player_surface_rules: TurnFormPlayerSurfaceRules {
            product_mode: "visual_novel_engine".to_owned(),
            forbidden_terms: vec![
                "slot",
                "선택지",
                "판정",
                "처리했다",
                "delayed",
                "partial_success",
                "costly_success",
                "visible",
                "evidence",
                "surface",
                "audit",
                "contract",
                "메타",
                "플레이어",
                "턴",
                "압력",
                "가늠",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            choice_text_contract: vec![
                "Write ordinary choices as in-scene Korean action sentences.".to_owned(),
                "Do not name system functions such as records, pressure, audit, slots, or choices."
                    .to_owned(),
                "Use concrete actors, objects, routes, or spoken lines when visible evidence allows it."
                    .to_owned(),
            ],
            concrete_delta_required: true,
        },
    }
}

/// Assemble a canonical agent response from one bounded `WebGPT` turn form.
///
/// # Errors
///
/// Returns field-level rejection data when the form targets the wrong pending
/// turn, omits required prose or choices, uses unsupported enum values, or
/// cites refs outside the current visible prompt context.
#[expect(
    clippy::too_many_lines,
    reason = "form boundary keeps validation and assembly colocated for auditability"
)]
pub fn assemble_agent_turn_response_from_form(
    pending: &PendingAgentTurn,
    context: &PromptContextPacket,
    submission: TurnFormSubmission,
) -> std::result::Result<AgentTurnResponse, TurnFormRejection> {
    let allowed_refs = collect_allowed_evidence_refs(context);
    let mut errors = Vec::new();
    validate_header(pending, &submission, &mut errors);
    validate_narrative(&submission, pending, &mut errors);
    let ambiguity = parse_enum::<ActionAmbiguity>(
        "intent_ambiguity",
        &submission.intent_ambiguity,
        &["clear", "minor", "high"],
        &mut errors,
    );
    let outcome_kind = parse_enum::<ResolutionOutcomeKind>(
        "outcome_kind",
        &submission.outcome_kind,
        &[
            "success",
            "partial_success",
            "blocked",
            "costly_success",
            "delayed",
            "escalated",
        ],
        &mut errors,
    );
    validate_pressure_movements(
        context,
        &submission.pressure_movements,
        &allowed_refs,
        &mut errors,
    );
    validate_next_choices(context, &submission.next_choices, &mut errors);
    validate_director_notes(&submission, &mut errors);
    validate_player_surface(&submission, &mut errors);

    let (Some(ambiguity), Some(outcome_kind)) = (ambiguity, outcome_kind) else {
        return Err(rejection(pending, errors));
    };
    if !errors.is_empty() {
        return Err(rejection(pending, errors));
    }

    let movement_by_pressure = submission
        .pressure_movements
        .iter()
        .map(|movement| (movement.pressure_id.as_str(), movement))
        .collect::<BTreeMap<_, _>>();
    let proposed_effects = submission
        .pressure_movements
        .iter()
        .map(|movement| ProposedEffect {
            effect_kind: ProposedEffectKind::ScenePressureDelta,
            target_ref: movement.pressure_id.clone(),
            visibility: ResolutionVisibility::PlayerVisible,
            knowledge_tier: None,
            summary: format!("{}: {}", movement.change.trim(), movement.summary.trim()),
            evidence_refs: evidence_or_current_turn(&movement.evidence_refs),
        })
        .collect::<Vec<_>>();
    let pressure_noop_reasons = context
        .pre_turn_simulation
        .pressure_obligations
        .iter()
        .filter(|obligation| !movement_by_pressure.contains_key(obligation.pressure_id.as_str()))
        .map(|obligation| crate::resolution::PressureNoopReason {
            pressure_ref: obligation.pressure_id.clone(),
            reason: "이번 입력은 이 압력을 직접 움직이지 않아 다음 장면으로 유지한다.".to_owned(),
            evidence_refs: evidence_or_current_turn(&obligation.evidence_refs),
        })
        .collect::<Vec<_>>();
    let pressure_refs = context
        .pre_turn_simulation
        .pressure_obligations
        .iter()
        .map(|obligation| obligation.pressure_id.clone())
        .collect::<Vec<_>>();
    let target_refs = selected_affordance_id(context, pending)
        .into_iter()
        .collect::<Vec<_>>();
    let next_choice_plan = next_choice_plan(context, &submission.next_choices);
    let next_choices = normalize_turn_choices(
        &submission
            .next_choices
            .iter()
            .map(|choice| TurnChoice {
                slot: choice.slot,
                tag: choice.tag.trim().to_owned(),
                intent: choice
                    .surface_text
                    .as_deref()
                    .unwrap_or(choice.intent.as_str())
                    .trim()
                    .to_owned(),
            })
            .chain([default_freeform_choice(), default_guide_choice()])
            .collect::<Vec<_>>(),
    );

    Ok(AgentTurnResponse {
        schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        resolution_proposal: Some(ResolutionProposal {
            schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
            world_id: pending.world_id.clone(),
            turn_id: pending.turn_id.clone(),
            interpreted_intent: ActionIntent {
                input_kind: action_input_kind(context.pre_turn_simulation.input_kind),
                summary: submission.intent_summary.trim().to_owned(),
                target_refs,
                pressure_refs,
                evidence_refs: vec!["current_turn".to_owned()],
                ambiguity,
            },
            outcome: ResolutionOutcome {
                kind: outcome_kind,
                summary: submission.outcome_summary.trim().to_owned(),
                evidence_refs: vec!["current_turn".to_owned()],
            },
            gate_results: Vec::new(),
            proposed_effects,
            process_ticks: Vec::new(),
            pressure_noop_reasons,
            narrative_brief: NarrativeBrief {
                visible_summary: submission.outcome_summary.trim().to_owned(),
                required_beats: submission.narrative.text_blocks.clone(),
                forbidden_visible_details: pending
                    .private_adjudication_context
                    .unrevealed_constraints
                    .iter()
                    .flat_map(|secret| secret.forbidden_leaks.clone())
                    .collect(),
            },
            next_choice_plan,
        }),
        scene_director_proposal: None,
        consequence_proposal: None,
        social_exchange_proposal: None,
        encounter_proposal: None,
        visible_scene: NarrativeScene {
            schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
            speaker: submission
                .narrative
                .speaker
                .as_deref()
                .map(str::trim)
                .filter(|speaker| !speaker.is_empty())
                .map(ToOwned::to_owned),
            text_blocks: submission
                .narrative
                .text_blocks
                .iter()
                .map(|block| block.trim().to_owned())
                .collect(),
            tone_notes: submission.narrative.tone_notes,
        },
        adjudication: Some(AgentResponseAdjudication {
            outcome: submission.outcome_kind,
            summary: submission
                .adjudication_summary
                .unwrap_or_else(|| submission.outcome_summary.clone()),
            gates: Vec::new(),
            visible_constraints: Vec::new(),
            consequences: submission
                .pressure_movements
                .into_iter()
                .map(|movement| movement.summary)
                .collect(),
        }),
        canon_event: None,
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
        next_choices,
        actor_goal_events: Vec::new(),
        actor_move_events: Vec::new(),
        hook_events: Vec::new(),
    })
}

fn choice_slot_spec(affordance: &CompiledAffordance) -> TurnFormChoiceSlotSpec {
    TurnFormChoiceSlotSpec {
        slot: affordance.slot,
        affordance_id: affordance.affordance_id.clone(),
        affordance_kind: serde_json::to_value(affordance.affordance_kind)
            .ok()
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
            .unwrap_or_else(|| "ordinary".to_owned()),
        label_contract: format!("Short Korean label for slot {}", affordance.slot),
        action_contract: affordance.action_contract.clone(),
        source_refs: affordance.source_refs.clone(),
        pressure_refs: affordance.pressure_refs.clone(),
    }
}

fn validate_header(
    pending: &PendingAgentTurn,
    submission: &TurnFormSubmission,
    errors: &mut Vec<TurnFormFieldError>,
) {
    if submission.schema_version != TURN_FORM_SUBMISSION_SCHEMA_VERSION {
        errors.push(field_error(
            "schema_version",
            "schema_version mismatch",
            &[TURN_FORM_SUBMISSION_SCHEMA_VERSION],
        ));
    }
    if submission.world_id != pending.world_id {
        errors.push(field_error(
            "world_id",
            "world_id does not match pending turn",
            &[],
        ));
    }
    if submission.turn_id != pending.turn_id {
        errors.push(field_error(
            "turn_id",
            "turn_id does not match pending turn",
            &[],
        ));
    }
    require_text("intent_summary", &submission.intent_summary, errors);
    require_text("outcome_summary", &submission.outcome_summary, errors);
}

fn validate_narrative(
    submission: &TurnFormSubmission,
    pending: &PendingAgentTurn,
    errors: &mut Vec<TurnFormFieldError>,
) {
    let blocks = submission
        .narrative
        .text_blocks
        .iter()
        .filter(|block| !block.trim().is_empty())
        .count();
    if blocks == 0 {
        errors.push(field_error(
            "narrative.text_blocks",
            "at least one visible narrative block is required",
            &[],
        ));
    }
    let max_blocks = pending
        .output_contract
        .narrative_budget
        .opening_or_climax_blocks
        .max(1) as usize;
    if blocks > max_blocks {
        errors.push(field_error(
            "narrative.text_blocks",
            "too many narrative blocks for this turn budget",
            &[],
        ));
    }
}

fn validate_pressure_movements(
    context: &PromptContextPacket,
    movements: &[TurnFormPressureMovement],
    allowed_refs: &BTreeSet<String>,
    errors: &mut Vec<TurnFormFieldError>,
) {
    let allowed_pressures = context
        .pre_turn_simulation
        .pressure_obligations
        .iter()
        .map(|obligation| obligation.pressure_id.as_str())
        .collect::<BTreeSet<_>>();
    for (index, movement) in movements.iter().enumerate() {
        if !allowed_pressures.contains(movement.pressure_id.as_str()) {
            errors.push(field_error(
                &format!("pressure_movements[{index}].pressure_id"),
                "pressure_id is not in the current visible pressure options",
                &allowed_pressures.iter().copied().collect::<Vec<_>>(),
            ));
        }
        require_text(
            &format!("pressure_movements[{index}].summary"),
            &movement.summary,
            errors,
        );
        for evidence_ref in &movement.evidence_refs {
            if !allowed_refs.contains(evidence_ref) {
                errors.push(field_error(
                    &format!("pressure_movements[{index}].evidence_refs"),
                    "evidence ref is not present in current visible context",
                    &[],
                ));
            }
        }
    }
}

fn validate_next_choices(
    context: &PromptContextPacket,
    choices: &[TurnFormChoiceSubmission],
    errors: &mut Vec<TurnFormFieldError>,
) {
    let provided_slots = choices
        .iter()
        .map(|choice| choice.slot)
        .collect::<BTreeSet<_>>();
    let expected_slots = context
        .pre_turn_simulation
        .available_affordances
        .iter()
        .filter(|affordance| (1..=5).contains(&affordance.slot))
        .map(|affordance| affordance.slot)
        .collect::<BTreeSet<_>>();
    if provided_slots != expected_slots {
        errors.push(field_error(
            "next_choices",
            "submit exactly the ordinary choice slots 1..5; backend appends slots 6 and 7",
            &expected_slots
                .iter()
                .map(u8::to_string)
                .collect::<Vec<_>>()
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        ));
    }
    for (index, choice) in choices.iter().enumerate() {
        if !(1..=5).contains(&choice.slot) {
            errors.push(field_error(
                &format!("next_choices[{index}].slot"),
                "choice submission may only include ordinary slots 1..5",
                &["1", "2", "3", "4", "5"],
            ));
        }
        require_text(&format!("next_choices[{index}].tag"), &choice.tag, errors);
        require_text(
            &format!("next_choices[{index}].intent"),
            &choice.intent,
            errors,
        );
        require_text(
            &format!("next_choices[{index}].surface_text"),
            choice.surface_text.as_deref().unwrap_or_default(),
            errors,
        );
        if choice.tag == FREEFORM_CHOICE_TAG || choice.tag == GUIDE_CHOICE_TAG {
            errors.push(field_error(
                &format!("next_choices[{index}].tag"),
                "special slot labels are backend-owned",
                &[],
            ));
        }
        if choice.intent == GUIDE_CHOICE_REDACTED_INTENT || choice.slot == FREEFORM_CHOICE_SLOT {
            errors.push(field_error(
                &format!("next_choices[{index}]"),
                "special slot content is backend-owned",
                &[],
            ));
        }
    }
}

fn validate_director_notes(submission: &TurnFormSubmission, errors: &mut Vec<TurnFormFieldError>) {
    require_text(
        "director_notes.beat_type",
        &submission.director_notes.beat_type,
        errors,
    );
    require_text(
        "director_notes.concrete_delta",
        &submission.director_notes.concrete_delta,
        errors,
    );
    require_text(
        "director_notes.scene_exit_progress",
        &submission.director_notes.scene_exit_progress,
        errors,
    );
    if !concrete_delta_is_specific(submission.director_notes.concrete_delta.as_str()) {
        errors.push(field_error(
            "director_notes.concrete_delta",
            "concrete_delta must name a concrete player-visible actor, object, route, evidence, or scene-exit movement",
            &[],
        ));
    }
}

fn validate_player_surface(submission: &TurnFormSubmission, errors: &mut Vec<TurnFormFieldError>) {
    for (index, block) in submission.narrative.text_blocks.iter().enumerate() {
        push_surface_errors(&format!("narrative.text_blocks[{index}]"), block, errors);
    }
    for (field, value) in [
        ("outcome_summary", submission.outcome_summary.as_str()),
        (
            "director_notes.concrete_delta",
            submission.director_notes.concrete_delta.as_str(),
        ),
        (
            "director_notes.scene_exit_progress",
            submission.director_notes.scene_exit_progress.as_str(),
        ),
    ] {
        push_surface_errors(field, value, errors);
    }
    for (index, choice) in submission.next_choices.iter().enumerate() {
        push_surface_errors(&format!("next_choices[{index}].tag"), &choice.tag, errors);
        if let Some(surface_text) = &choice.surface_text {
            push_surface_errors(
                &format!("next_choices[{index}].surface_text"),
                surface_text,
                errors,
            );
        }
        if choice
            .surface_text
            .as_deref()
            .is_some_and(|surface| surface.trim() == choice.tag.trim())
            && choice.tag.chars().count() > 34
        {
            errors.push(field_error(
                &format!("next_choices[{index}].tag"),
                "tag should be a short diegetic label; put the full action sentence in surface_text",
                &[],
            ));
        }
    }
}

fn push_surface_errors(field_path: &str, value: &str, errors: &mut Vec<TurnFormFieldError>) {
    let forbidden = player_surface_forbidden_terms(value);
    if !forbidden.is_empty() {
        errors.push(field_error(
            field_path,
            "player-facing VN surface contains simulator/debug vocabulary",
            &forbidden,
        ));
    }
}

fn next_choice_plan(
    context: &PromptContextPacket,
    submitted_choices: &[TurnFormChoiceSubmission],
) -> Vec<ChoicePlan> {
    let submitted_by_slot = submitted_choices
        .iter()
        .map(|choice| (choice.slot, choice))
        .collect::<BTreeMap<_, _>>();
    let mut plans = context
        .pre_turn_simulation
        .available_affordances
        .iter()
        .filter(|affordance| (1..=5).contains(&affordance.slot))
        .map(|affordance| {
            let submitted = submitted_by_slot.get(&affordance.slot);
            ChoicePlan {
                slot: affordance.slot,
                plan_kind: ChoicePlanKind::OrdinaryAffordance,
                grounding_ref: affordance.affordance_id.clone(),
                label_seed: submitted.map_or_else(
                    || format!("선택 {}", affordance.slot),
                    |choice| choice.tag.trim().to_owned(),
                ),
                intent_seed: submitted.map_or_else(
                    || affordance.action_contract.clone(),
                    |choice| choice.intent.trim().to_owned(),
                ),
                evidence_refs: evidence_or_current_turn(&affordance.source_refs),
            }
        })
        .collect::<Vec<_>>();
    plans.push(ChoicePlan {
        slot: FREEFORM_CHOICE_SLOT,
        plan_kind: ChoicePlanKind::Freeform,
        grounding_ref: "current_turn".to_owned(),
        label_seed: FREEFORM_CHOICE_TAG.to_owned(),
        intent_seed: "직접 행동, 말, 내면 판단을 이어서 서술한다".to_owned(),
        evidence_refs: vec!["current_turn".to_owned()],
    });
    plans.push(ChoicePlan {
        slot: GUIDE_CHOICE_SLOT,
        plan_kind: ChoicePlanKind::DelegatedJudgment,
        grounding_ref: "current_turn".to_owned(),
        label_seed: GUIDE_CHOICE_TAG.to_owned(),
        intent_seed: GUIDE_CHOICE_REDACTED_INTENT.to_owned(),
        evidence_refs: vec!["current_turn".to_owned()],
    });
    plans
}

fn selected_affordance_id(
    context: &PromptContextPacket,
    pending: &PendingAgentTurn,
) -> Option<String> {
    let selected_slot = pending.selected_choice.as_ref()?.slot;
    context
        .pre_turn_simulation
        .available_affordances
        .iter()
        .find(|affordance| affordance.slot == selected_slot)
        .map(|affordance| affordance.affordance_id.clone())
}

fn action_input_kind(kind: TurnInputKind) -> ActionInputKind {
    match kind {
        TurnInputKind::NumericChoice | TurnInputKind::MacroTimeFlow | TurnInputKind::CcCanvas => {
            ActionInputKind::PresentedChoice
        }
        TurnInputKind::GuideChoice => ActionInputKind::DelegatedJudgment,
        TurnInputKind::FreeformAction => ActionInputKind::Freeform,
        TurnInputKind::CodexQuery => ActionInputKind::CodexQuery,
    }
}

fn collect_allowed_evidence_refs(context: &PromptContextPacket) -> BTreeSet<String> {
    let mut refs = BTreeSet::from([
        context.turn_id.clone(),
        format!("turn:{}", context.turn_id),
        "current_turn".to_owned(),
        "visible_scene".to_owned(),
    ]);
    refs.extend(context.pre_turn_simulation.source_refs.iter().cloned());
    for affordance in &context.pre_turn_simulation.available_affordances {
        refs.insert(affordance.affordance_id.clone());
        refs.extend(affordance.source_refs.iter().cloned());
        refs.extend(affordance.pressure_refs.iter().cloned());
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
    refs.into_iter()
        .filter(|item| !item.trim().is_empty())
        .collect()
}

fn parse_enum<T>(
    field_path: &str,
    value: &str,
    allowed_values: &[&str],
    errors: &mut Vec<TurnFormFieldError>,
) -> Option<T>
where
    T: for<'de> Deserialize<'de>,
{
    if let Ok(parsed) = serde_json::from_value(serde_json::Value::String(value.trim().to_owned())) {
        Some(parsed)
    } else {
        errors.push(field_error(field_path, "unsupported value", allowed_values));
        None
    }
}

fn require_text(field_path: &str, value: &str, errors: &mut Vec<TurnFormFieldError>) {
    if value.trim().is_empty() {
        errors.push(field_error(field_path, "field must not be empty", &[]));
    }
}

fn field_error(field_path: &str, message: &str, allowed_values: &[&str]) -> TurnFormFieldError {
    TurnFormFieldError {
        field_path: field_path.to_owned(),
        message: message.to_owned(),
        allowed_values: allowed_values
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
    }
}

fn rejection(
    pending: &PendingAgentTurn,
    field_errors: Vec<TurnFormFieldError>,
) -> TurnFormRejection {
    TurnFormRejection {
        schema_version: TURN_FORM_SUBMIT_RESULT_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        field_errors,
        retryable: true,
    }
}

fn evidence_or_current_turn(evidence_refs: &[String]) -> Vec<String> {
    let evidence_refs = evidence_refs
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if evidence_refs.is_empty() {
        vec!["current_turn".to_owned()]
    } else {
        evidence_refs
    }
}

fn default_ambiguity() -> String {
    "clear".to_owned()
}

fn default_outcome_kind() -> String {
    "partial_success".to_owned()
}

fn default_pressure_change() -> String {
    "moved".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentSubmitTurnOptions, CompilePromptContextPacketOptions, StartWorldOptions,
        compile_prompt_context_packet, enqueue_agent_turn, start_world,
    };
    use tempfile::tempdir;

    #[test]
    fn assembles_agent_response_from_form_without_backend_owned_choices() -> anyhow::Result<()> {
        let (pending, context) = pending_fixture("stw_turn_form_assemble")?;
        let submission = valid_submission(&pending, &context);

        let response = assemble_agent_turn_response_from_form(&pending, &context, submission)
            .map_err(|rejection| anyhow::anyhow!("unexpected form rejection: {rejection:?}"))?;

        assert_eq!(response.world_id, pending.world_id);
        assert_eq!(response.turn_id, pending.turn_id);
        assert_eq!(response.next_choices.len(), 7);
        assert_eq!(response.next_choices[5].slot, FREEFORM_CHOICE_SLOT);
        assert_eq!(response.next_choices[6].slot, GUIDE_CHOICE_SLOT);
        assert!(response.resolution_proposal.is_some());
        Ok(())
    }

    #[test]
    fn rejects_special_choice_slots_in_form_submission() -> anyhow::Result<()> {
        let (pending, context) = pending_fixture("stw_turn_form_reject_special")?;
        let mut submission = valid_submission(&pending, &context);
        submission.next_choices.push(TurnFormChoiceSubmission {
            slot: GUIDE_CHOICE_SLOT,
            tag: GUIDE_CHOICE_TAG.to_owned(),
            intent: GUIDE_CHOICE_REDACTED_INTENT.to_owned(),
            surface_text: Some(GUIDE_CHOICE_REDACTED_INTENT.to_owned()),
        });

        let Err(rejection) = assemble_agent_turn_response_from_form(&pending, &context, submission)
        else {
            anyhow::bail!("special choice slots should be rejected");
        };

        assert!(
            rejection
                .field_errors
                .iter()
                .any(|error| error.field_path.starts_with("next_choices"))
        );
        Ok(())
    }

    #[test]
    fn rejects_unknown_pressure_refs() -> anyhow::Result<()> {
        let (pending, context) = pending_fixture("stw_turn_form_reject_pressure")?;
        let mut submission = valid_submission(&pending, &context);
        submission
            .pressure_movements
            .push(TurnFormPressureMovement {
                pressure_id: "pressure:hidden:unknown".to_owned(),
                change: "moved".to_owned(),
                summary: "없는 압력을 움직였다고 주장한다.".to_owned(),
                evidence_refs: vec!["hidden:timer".to_owned()],
            });

        let Err(rejection) = assemble_agent_turn_response_from_form(&pending, &context, submission)
        else {
            anyhow::bail!("unknown pressure refs should be rejected");
        };

        assert!(
            rejection
                .field_errors
                .iter()
                .any(|error| error.field_path.contains("pressure_movements"))
        );
        Ok(())
    }

    #[test]
    fn rejects_meta_language_in_player_surface() -> anyhow::Result<()> {
        let (pending, context) = pending_fixture("stw_turn_form_reject_meta_surface")?;
        let mut submission = valid_submission(&pending, &context);
        submission.narrative.text_blocks[0] = "slot 4의 판정을 delayed 결과로 처리했다.".to_owned();

        let Err(rejection) = assemble_agent_turn_response_from_form(&pending, &context, submission)
        else {
            anyhow::bail!("meta surface text should be rejected");
        };

        assert!(rejection.field_errors.iter().any(|error| {
            error.field_path.starts_with("narrative.text_blocks")
                && error.message.contains("simulator")
        }));
        Ok(())
    }

    #[test]
    fn rejects_missing_concrete_delta() -> anyhow::Result<()> {
        let (pending, context) = pending_fixture("stw_turn_form_reject_delta")?;
        let mut submission = valid_submission(&pending, &context);
        submission.director_notes.concrete_delta = "기척과 압력이 조금 흔들렸다".to_owned();

        let Err(rejection) = assemble_agent_turn_response_from_form(&pending, &context, submission)
        else {
            anyhow::bail!("abstract-only concrete delta should be rejected");
        };

        assert!(rejection.field_errors.iter().any(|error| {
            error.field_path == "director_notes.concrete_delta"
                && error.message.contains("concrete_delta")
        }));
        Ok(())
    }

    fn pending_fixture(world_id: &str) -> anyhow::Result<(PendingAgentTurn, PromptContextPacket)> {
        let temp = tempdir()?;
        let store_root = temp.path().join("store");
        start_world(&StartWorldOptions {
            seed_text: "중세 판타지 성문 앞에서 시작한다.".to_owned(),
            store_root: Some(store_root.clone()),
            world_id: Some(world_id.to_owned()),
            title: None,
            randomize_opening_seed: false,
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store_root.clone()),
            world_id: world_id.to_owned(),
            input: "1".to_owned(),
            narrative_level: Some(1),
        })?;
        let context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
            store_root: Some(store_root.as_path()),
            pending: &pending,
            engine_session_kind: "turn_form_test",
        })?;
        Ok((pending, context))
    }

    fn valid_submission(
        pending: &PendingAgentTurn,
        context: &PromptContextPacket,
    ) -> TurnFormSubmission {
        TurnFormSubmission {
            schema_version: TURN_FORM_SUBMISSION_SCHEMA_VERSION.to_owned(),
            world_id: pending.world_id.clone(),
            turn_id: pending.turn_id.clone(),
            intent_summary: "플레이어는 눈앞의 선택을 따라 장면을 실제로 전진시킨다.".to_owned(),
            intent_ambiguity: "clear".to_owned(),
            outcome_kind: "partial_success".to_owned(),
            outcome_summary: "문 앞의 정체가 조금 더 구체화되고 다음 행동이 가까워진다.".to_owned(),
            pressure_movements: Vec::new(),
            narrative: TurnFormNarrativeSubmission {
                speaker: None,
                text_blocks: vec![
                    "문고리에 손이 닿자 차가운 금속성이 손바닥으로 올라왔다.".to_owned(),
                    "문 너머에서는 아주 낮은 숨소리 같은 떨림이 이어졌다.".to_owned(),
                ],
                tone_notes: Vec::new(),
            },
            next_choices: context
                .pre_turn_simulation
                .available_affordances
                .iter()
                .filter(|affordance| (1..=5).contains(&affordance.slot))
                .map(|affordance| TurnFormChoiceSubmission {
                    slot: affordance.slot,
                    tag: format!("성문 행동 {}", affordance.slot),
                    intent: format!(
                        "{}를 바탕으로 다음 행동을 고른다.",
                        affordance.action_contract
                    ),
                    surface_text: Some(format!(
                        "성문 아래의 단서 {}를 따라 한 걸음 움직인다.",
                        affordance.slot
                    )),
                })
                .collect(),
            director_notes: TurnFormDirectorNotes {
                beat_type: "discovery".to_owned(),
                concrete_delta: "문지기가 북문 아래 창끝을 낮추며 길을 막았다.".to_owned(),
                scene_exit_progress: "북문을 그냥 지날 수 없다는 사실이 드러났다.".to_owned(),
            },
            adjudication_summary: None,
        }
    }
}
