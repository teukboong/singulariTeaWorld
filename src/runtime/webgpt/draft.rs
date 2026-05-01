use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use singulari_world::{
    AGENT_TURN_RESPONSE_SCHEMA_VERSION, ActionAmbiguity, ActionInputKind, ActionIntent,
    AffordanceKind, AgentTurnResponse, ChoicePlan, ChoicePlanKind, ChoiceStrategy,
    DramaticBeatKind, FREEFORM_CHOICE_SLOT, FREEFORM_CHOICE_TAG, GUIDE_CHOICE_REDACTED_INTENT,
    GUIDE_CHOICE_SLOT, GUIDE_CHOICE_TAG, NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeBrief,
    NarrativeScene, ParagraphStrategy, PendingAgentTurn, PressureNoopReason, PromptContextPacket,
    RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome, ResolutionOutcomeKind,
    ResolutionProposal, ResolutionVisibility, SceneDirectorPacket, SceneDirectorProposal,
    SceneEffect, ScenePhase, SceneTransitionProposal, TensionLevel, TransitionPressure, TurnChoice,
    TurnInputKind,
};
use std::collections::{BTreeSet, HashMap};

use super::json_extract::extract_json_object_text_for_schema;

pub(super) const WEBGPT_TURN_DRAFT_SCHEMA_VERSION: &str = "singulari.webgpt_turn_draft.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebgptTurnDraft {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub interpreted_intent: WebgptDraftIntent,
    pub outcome: WebgptDraftOutcome,
    pub visible_scene: WebgptDraftScene,
    #[serde(default)]
    pub choices: Vec<WebgptDraftChoice>,
    #[serde(default)]
    pub pressure_movements: Vec<WebgptDraftPressureMovement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebgptDraftIntent {
    pub summary: String,
    #[serde(default)]
    pub target_hints: Vec<String>,
    #[serde(default)]
    pub pressure_hints: Vec<String>,
    #[serde(default)]
    pub evidence_hints: Vec<String>,
    #[serde(default = "default_clear_ambiguity")]
    pub ambiguity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebgptDraftOutcome {
    #[serde(default = "default_success_kind")]
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebgptDraftScene {
    #[serde(default)]
    pub text_blocks: Vec<String>,
    #[serde(default)]
    pub tone_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebgptDraftChoice {
    pub slot: u8,
    pub label: String,
    pub intent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct WebgptDraftPressureMovement {
    pub pressure_id: String,
    #[serde(default)]
    pub change: String,
    pub summary: String,
    #[serde(default)]
    pub evidence_hints: Vec<String>,
}

#[derive(Debug, Default)]
struct ReferenceIndex {
    canonical_refs: BTreeSet<String>,
    aliases: HashMap<String, String>,
}

pub(super) fn parse_webgpt_turn_draft(raw: &str) -> Result<WebgptTurnDraft> {
    let draft_json = extract_json_object_text_for_schema(
        raw,
        WEBGPT_TURN_DRAFT_SCHEMA_VERSION,
        &[
            "world_id",
            "turn_id",
            "interpreted_intent",
            "outcome",
            "visible_scene",
        ],
    )
    .context("webgpt answer did not contain one complete WebgptTurnDraft JSON object")?;
    serde_json::from_str(draft_json.as_str()).context("failed to parse WebgptTurnDraft JSON")
}

pub(super) fn assemble_agent_turn_response(
    pending: &PendingAgentTurn,
    prompt_context: &PromptContextPacket,
    draft: &WebgptTurnDraft,
) -> Result<AgentTurnResponse> {
    validate_draft_identity(pending, draft)?;
    let reference_index = ReferenceIndex::from_prompt_context(prompt_context)?;
    let visible_scene = assemble_visible_scene(&draft.visible_scene)?;
    let resolution_proposal =
        assemble_resolution_proposal(pending, prompt_context, draft, &reference_index)?;
    let next_choices = assemble_next_choices(prompt_context, draft)?;
    let scene_director_proposal =
        assemble_scene_director_proposal(pending, prompt_context, draft, &resolution_proposal);

    Ok(AgentTurnResponse {
        schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        resolution_proposal: Some(resolution_proposal),
        scene_director_proposal,
        consequence_proposal: None,
        social_exchange_proposal: None,
        encounter_proposal: None,
        visible_scene,
        adjudication: None,
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

fn validate_draft_identity(pending: &PendingAgentTurn, draft: &WebgptTurnDraft) -> Result<()> {
    if draft.schema_version != WEBGPT_TURN_DRAFT_SCHEMA_VERSION {
        bail!(
            "webgpt draft schema_version mismatch: expected={}, got={}",
            WEBGPT_TURN_DRAFT_SCHEMA_VERSION,
            draft.schema_version
        );
    }
    if draft.world_id != pending.world_id {
        bail!(
            "webgpt draft world_id mismatch: expected={}, got={}",
            pending.world_id,
            draft.world_id
        );
    }
    if draft.turn_id != pending.turn_id {
        bail!(
            "webgpt draft turn_id mismatch: expected={}, got={}",
            pending.turn_id,
            draft.turn_id
        );
    }
    Ok(())
}

fn assemble_visible_scene(scene: &WebgptDraftScene) -> Result<NarrativeScene> {
    let text_blocks = scene
        .text_blocks
        .iter()
        .map(|block| block.trim())
        .filter(|block| !block.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if text_blocks.is_empty() {
        bail!("webgpt draft visible_scene.text_blocks must contain player-visible prose");
    }
    Ok(NarrativeScene {
        schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
        speaker: None,
        text_blocks,
        tone_notes: scene
            .tone_notes
            .iter()
            .map(|note| note.trim())
            .filter(|note| !note.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    })
}

fn assemble_resolution_proposal(
    pending: &PendingAgentTurn,
    prompt_context: &PromptContextPacket,
    draft: &WebgptTurnDraft,
    reference_index: &ReferenceIndex,
) -> Result<ResolutionProposal> {
    let pressure_refs = pressure_refs(prompt_context, draft, reference_index);
    let pressure_movements = pressure_movements(prompt_context, draft, reference_index);
    let moved_pressure_refs = pressure_movements
        .iter()
        .map(|movement| movement.pressure_id.clone())
        .collect::<BTreeSet<_>>();
    let pressure_noop_reasons = prompt_context
        .pre_turn_simulation
        .pressure_obligations
        .iter()
        .filter(|obligation| !moved_pressure_refs.contains(&obligation.pressure_id))
        .map(|obligation| PressureNoopReason {
            pressure_ref: obligation.pressure_id.clone(),
            reason: "이번 턴은 압력을 직접 해소하지 않고 다음 행동 압력으로 유지한다.".to_owned(),
            evidence_refs: evidence_refs(
                &obligation.evidence_refs,
                reference_index,
                &[obligation.pressure_id.as_str()],
            ),
        })
        .collect::<Vec<_>>();

    Ok(ResolutionProposal {
        schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        interpreted_intent: ActionIntent {
            input_kind: action_input_kind(prompt_context.pre_turn_simulation.input_kind),
            summary: required_text(
                draft.interpreted_intent.summary.as_str(),
                "interpreted_intent.summary",
            )?,
            target_refs: canonical_target_refs(
                &draft.interpreted_intent.target_hints,
                reference_index,
            ),
            pressure_refs,
            evidence_refs: evidence_refs(
                &draft.interpreted_intent.evidence_hints,
                reference_index,
                &["current_turn"],
            ),
            ambiguity: parse_ambiguity(draft.interpreted_intent.ambiguity.as_str())?,
        },
        outcome: ResolutionOutcome {
            kind: parse_outcome_kind(draft.outcome.kind.as_str())?,
            summary: required_text(draft.outcome.summary.as_str(), "outcome.summary")?,
            evidence_refs: evidence_refs(
                &draft.outcome.evidence_hints,
                reference_index,
                &["current_turn"],
            ),
        },
        gate_results: Vec::new(),
        proposed_effects: pressure_movements
            .into_iter()
            .map(|movement| singulari_world::ProposedEffect {
                effect_kind: singulari_world::ProposedEffectKind::ScenePressureDelta,
                target_ref: movement.pressure_id,
                visibility: ResolutionVisibility::PlayerVisible,
                knowledge_tier: None,
                summary: movement.summary,
                evidence_refs: movement.evidence_refs,
            })
            .collect(),
        process_ticks: Vec::new(),
        pressure_noop_reasons,
        narrative_brief: NarrativeBrief {
            visible_summary: draft.outcome.summary.trim().to_owned(),
            required_beats: Vec::new(),
            forbidden_visible_details: Vec::new(),
        },
        next_choice_plan: assemble_choice_plan(prompt_context, draft),
    })
}

fn pressure_refs(
    prompt_context: &PromptContextPacket,
    draft: &WebgptTurnDraft,
    reference_index: &ReferenceIndex,
) -> Vec<String> {
    let mut refs = BTreeSet::new();
    for hint in &draft.interpreted_intent.pressure_hints {
        if let Some(reference) = reference_index.canonicalize(hint)
            && reference.starts_with("pressure:")
        {
            refs.insert(reference);
        }
    }
    for obligation in &prompt_context.pre_turn_simulation.pressure_obligations {
        refs.insert(obligation.pressure_id.clone());
    }
    refs.into_iter().collect()
}

fn pressure_movements(
    prompt_context: &PromptContextPacket,
    draft: &WebgptTurnDraft,
    reference_index: &ReferenceIndex,
) -> Vec<AssembledPressureMovement> {
    let allowed = prompt_context
        .pre_turn_simulation
        .pressure_obligations
        .iter()
        .map(|obligation| obligation.pressure_id.clone())
        .chain(
            prompt_context
                .pre_turn_simulation
                .available_affordances
                .iter()
                .flat_map(|affordance| affordance.pressure_refs.iter().cloned()),
        )
        .collect::<BTreeSet<_>>();
    draft
        .pressure_movements
        .iter()
        .filter_map(|movement| {
            let pressure_id = reference_index.canonicalize(movement.pressure_id.as_str())?;
            if !allowed.contains(&pressure_id) {
                return None;
            }
            Some(AssembledPressureMovement {
                pressure_id,
                summary: movement.summary.trim().to_owned(),
                evidence_refs: evidence_refs(
                    &movement.evidence_hints,
                    reference_index,
                    &["current_turn"],
                ),
            })
        })
        .filter(|movement| !movement.summary.is_empty())
        .collect()
}

struct AssembledPressureMovement {
    pressure_id: String,
    summary: String,
    evidence_refs: Vec<String>,
}

fn assemble_next_choices(
    prompt_context: &PromptContextPacket,
    draft: &WebgptTurnDraft,
) -> Result<Vec<TurnChoice>> {
    let mut choices = Vec::new();
    for affordance in &prompt_context.pre_turn_simulation.available_affordances {
        if !(1..FREEFORM_CHOICE_SLOT).contains(&affordance.slot) {
            continue;
        }
        let draft_choice = draft
            .choices
            .iter()
            .find(|choice| choice.slot == affordance.slot);
        let tag = choice_tag(draft_choice, affordance.affordance_kind);
        let intent = draft_choice
            .map_or(affordance.action_contract.as_str(), |choice| {
                choice.intent.as_str()
            })
            .trim();
        choices.push(TurnChoice {
            slot: affordance.slot,
            tag: required_text(tag, "choice.tag")?,
            intent: required_text(intent, "choice.intent")?,
        });
    }
    choices.push(TurnChoice {
        slot: FREEFORM_CHOICE_SLOT,
        tag: FREEFORM_CHOICE_TAG.to_owned(),
        intent: "6 뒤에 직접 행동, 말, 내면 판단을 이어서 서술한다".to_owned(),
    });
    choices.push(TurnChoice {
        slot: GUIDE_CHOICE_SLOT,
        tag: GUIDE_CHOICE_TAG.to_owned(),
        intent: GUIDE_CHOICE_REDACTED_INTENT.to_owned(),
    });
    Ok(choices)
}

fn choice_tag(draft_choice: Option<&WebgptDraftChoice>, affordance_kind: AffordanceKind) -> &str {
    draft_choice
        .and_then(|choice| concise_choice_tag(choice.tag_hint.as_deref()))
        .or_else(|| draft_choice.and_then(|choice| concise_choice_tag(Some(choice.label.as_str()))))
        .unwrap_or_else(|| fallback_choice_tag(affordance_kind))
}

fn concise_choice_tag(candidate: Option<&str>) -> Option<&str> {
    let candidate = candidate?.trim();
    if candidate.is_empty()
        || candidate.chars().count() > 14
        || candidate.contains('.')
        || candidate.contains(',')
        || candidate.contains('，')
        || candidate.contains('。')
        || candidate.contains("다.")
        || candidate.contains("한다")
    {
        None
    } else {
        Some(candidate)
    }
}

const fn fallback_choice_tag(affordance_kind: AffordanceKind) -> &'static str {
    match affordance_kind {
        AffordanceKind::Move => "움직임",
        AffordanceKind::Observe => "관찰",
        AffordanceKind::Contact => "대화",
        AffordanceKind::ResourceOrBody => "상태",
        AffordanceKind::PressureResponse => "대응",
    }
}

fn assemble_scene_director_proposal(
    pending: &PendingAgentTurn,
    prompt_context: &PromptContextPacket,
    draft: &WebgptTurnDraft,
    resolution_proposal: &ResolutionProposal,
) -> Option<SceneDirectorProposal> {
    let director: SceneDirectorPacket =
        serde_json::from_value(prompt_context.visible_context.active_scene_director.clone())
            .ok()?;
    let transition_needed = matches!(
        director.pacing_state.transition_pressure,
        TransitionPressure::High
    ) || matches!(
        director.current_scene.scene_phase,
        ScenePhase::TransitionReady
    ) || director.pacing_state.unresolved_probe_turns >= 3
        || director.pacing_state.repeated_choice_shape_count >= 3;
    if !transition_needed {
        return None;
    }
    let evidence_refs = scene_director_evidence_refs(&director, resolution_proposal);
    if evidence_refs.is_empty() {
        return None;
    }
    let to_scene_question = next_scene_question(draft);
    Some(SceneDirectorProposal {
        schema_version: singulari_world::SCENE_DIRECTOR_PROPOSAL_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        scene_id: director.current_scene.scene_id.clone(),
        beat_kind: DramaticBeatKind::Transition,
        turn_function: "반복된 조사 장면을 닫고, 이번 턴의 visible outcome이 만든 다음 문제로 넘긴다."
            .to_owned(),
        tension_before: director.current_scene.current_tension,
        tension_after: lowered_tension(director.current_scene.current_tension),
        scene_effect: SceneEffect::SceneQuestionTransformed,
        paragraph_strategy: ParagraphStrategy {
            opening_shape: "new_visible_change".to_owned(),
            middle_shape: "changed_problem".to_owned(),
            closure_shape: "new_scene_question".to_owned(),
        },
        choice_strategy: ChoiceStrategy {
            must_change_choice_shape: false,
            avoid_recent_choice_tags: Vec::new(),
        },
        transition: Some(SceneTransitionProposal {
            from_scene_id: director.current_scene.scene_id,
            to_scene_question,
            transition_reason: "active_scene_director가 반복 조사 한계에 도달했고, 이번 턴 결과가 다음 장면 질문으로 넘어갈 근거를 만들었다."
                .to_owned(),
            evidence_refs: evidence_refs.clone(),
        }),
        evidence_refs,
    })
}

fn scene_director_evidence_refs(
    director: &SceneDirectorPacket,
    resolution_proposal: &ResolutionProposal,
) -> Vec<String> {
    let mut refs = BTreeSet::new();
    refs.extend(
        director
            .current_scene
            .evidence_refs
            .iter()
            .filter(|reference| !reference.trim().is_empty())
            .cloned(),
    );
    refs.extend(
        resolution_proposal
            .outcome
            .evidence_refs
            .iter()
            .filter(|reference| !reference.trim().is_empty())
            .cloned(),
    );
    if refs.is_empty() {
        refs.insert("current_turn".to_owned());
    }
    refs.into_iter().collect()
}

fn lowered_tension(tension: TensionLevel) -> TensionLevel {
    match tension {
        TensionLevel::High => TensionLevel::Medium,
        other => other,
    }
}

fn next_scene_question(draft: &WebgptTurnDraft) -> String {
    let summary = draft.outcome.summary.trim();
    if summary.is_empty() {
        return "드러난 변화 뒤에 무엇을 감수하고 행동할 것인가?".to_owned();
    }
    let compact = summary.chars().take(48).collect::<String>();
    format!("{compact} 이후 무엇을 감수하고 행동할 것인가?")
}

fn assemble_choice_plan(
    prompt_context: &PromptContextPacket,
    draft: &WebgptTurnDraft,
) -> Vec<ChoicePlan> {
    let mut plan = prompt_context
        .pre_turn_simulation
        .available_affordances
        .iter()
        .filter(|affordance| (1..FREEFORM_CHOICE_SLOT).contains(&affordance.slot))
        .map(|affordance| {
            let draft_choice = draft
                .choices
                .iter()
                .find(|choice| choice.slot == affordance.slot);
            ChoicePlan {
                slot: affordance.slot,
                plan_kind: ChoicePlanKind::OrdinaryAffordance,
                grounding_ref: affordance.affordance_id.clone(),
                label_seed: draft_choice
                    .map(|choice| choice.label.trim())
                    .filter(|label| !label.is_empty())
                    .unwrap_or(affordance.action_contract.as_str())
                    .to_owned(),
                intent_seed: draft_choice
                    .map(|choice| choice.intent.trim())
                    .filter(|intent| !intent.is_empty())
                    .unwrap_or(affordance.action_contract.as_str())
                    .to_owned(),
                evidence_refs: vec![affordance.affordance_id.clone()],
            }
        })
        .collect::<Vec<_>>();
    plan.push(ChoicePlan {
        slot: FREEFORM_CHOICE_SLOT,
        plan_kind: ChoicePlanKind::Freeform,
        grounding_ref: "current_turn".to_owned(),
        label_seed: FREEFORM_CHOICE_TAG.to_owned(),
        intent_seed: "6 뒤에 직접 행동, 말, 내면 판단을 이어서 서술한다".to_owned(),
        evidence_refs: vec!["current_turn".to_owned()],
    });
    plan.push(ChoicePlan {
        slot: GUIDE_CHOICE_SLOT,
        plan_kind: ChoicePlanKind::DelegatedJudgment,
        grounding_ref: "current_turn".to_owned(),
        label_seed: GUIDE_CHOICE_TAG.to_owned(),
        intent_seed: GUIDE_CHOICE_REDACTED_INTENT.to_owned(),
        evidence_refs: vec!["current_turn".to_owned()],
    });
    plan
}

fn canonical_target_refs(hints: &[String], reference_index: &ReferenceIndex) -> Vec<String> {
    let mut refs = BTreeSet::new();
    for hint in hints {
        if let Some(reference) = reference_index.canonicalize(hint)
            && is_safe_resolution_target_ref(reference.as_str())
        {
            refs.insert(reference);
        }
    }
    refs.into_iter().collect()
}

fn is_safe_resolution_target_ref(reference: &str) -> bool {
    matches!(reference, "current_turn")
        || reference.starts_with("place:")
        || reference.starts_with("location:")
        || reference.starts_with("entities.places:")
        || (reference.starts_with("encounter:") && reference.ends_with(":affordance"))
}

fn evidence_refs(
    hints: &[String],
    reference_index: &ReferenceIndex,
    fallback_refs: &[&str],
) -> Vec<String> {
    let mut refs = BTreeSet::new();
    for hint in hints {
        if let Some(reference) = reference_index.canonicalize(hint) {
            refs.insert(reference);
        }
    }
    if refs.is_empty() {
        for fallback in fallback_refs {
            refs.insert((*fallback).to_owned());
        }
    }
    refs.into_iter().collect()
}

fn action_input_kind(input_kind: TurnInputKind) -> ActionInputKind {
    match input_kind {
        TurnInputKind::NumericChoice | TurnInputKind::MacroTimeFlow | TurnInputKind::CcCanvas => {
            ActionInputKind::PresentedChoice
        }
        TurnInputKind::GuideChoice => ActionInputKind::DelegatedJudgment,
        TurnInputKind::CodexQuery => ActionInputKind::CodexQuery,
        TurnInputKind::FreeformAction => ActionInputKind::Freeform,
    }
}

fn parse_ambiguity(raw: &str) -> Result<ActionAmbiguity> {
    parse_string_enum(raw, "interpreted_intent.ambiguity")
}

fn parse_outcome_kind(raw: &str) -> Result<ResolutionOutcomeKind> {
    parse_string_enum(raw, "outcome.kind")
}

fn parse_string_enum<T>(raw: &str, field: &str) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_value(Value::String(raw.trim().to_owned()))
        .with_context(|| format!("webgpt draft {field} has unsupported value: {raw}"))
}

fn required_text(raw: &str, field: &str) -> Result<String> {
    let text = raw.trim();
    if text.is_empty() {
        bail!("webgpt draft {field} must not be empty");
    }
    Ok(text.to_owned())
}

impl ReferenceIndex {
    fn from_prompt_context(prompt_context: &PromptContextPacket) -> Result<Self> {
        let value = serde_json::to_value(prompt_context)
            .context("failed to serialize prompt context for reference index")?;
        let mut index = Self::default();
        index.insert("current_turn");
        index.insert("player_input");
        index.collect_from_value(None, &value);
        Ok(index)
    }

    fn insert(&mut self, reference: &str) {
        if is_ref_atom(reference) {
            self.canonical_refs.insert(reference.to_owned());
            if let Some(alias) = entity_reference_alias(reference) {
                self.insert_unique_alias(alias, reference.to_owned());
            }
            for alias in affordance_reference_aliases(reference) {
                self.insert_unique_alias(alias, reference.to_owned());
            }
        }
    }

    fn insert_unique_alias(&mut self, alias: String, canonical: String) {
        match self.aliases.get(&alias) {
            Some(existing) if existing != &canonical => {
                self.aliases.remove(&alias);
            }
            Some(_) => {}
            None => {
                self.aliases.insert(alias, canonical);
            }
        }
    }

    fn collect_from_value(&mut self, key: Option<&str>, value: &Value) {
        match value {
            Value::Object(map) => {
                for (child_key, child_value) in map {
                    self.collect_from_value(Some(child_key.as_str()), child_value);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.collect_from_value(key, item);
                }
            }
            Value::String(text) if key.is_some_and(is_reference_key) => {
                self.insert(text);
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
    }

    fn canonicalize(&self, raw: &str) -> Option<String> {
        let trimmed = raw.trim();
        if self.canonical_refs.contains(trimmed) {
            return Some(trimmed.to_owned());
        }
        self.aliases.get(trimmed).cloned()
    }
}

fn is_reference_key(key: &str) -> bool {
    key.ends_with("_id")
        || key.ends_with("_ids")
        || key.ends_with("_ref")
        || key.ends_with("_refs")
        || matches!(
            key,
            "actor_ref" | "source_entity_id" | "target_entity_id" | "character_id"
        )
}

fn is_ref_atom(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed != value || trimmed.is_empty() || trimmed.len() > 180 {
        return false;
    }
    if trimmed.starts_with('/') || trimmed.starts_with("singulari.") {
        return false;
    }
    if trimmed
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
    {
        return false;
    }
    matches!(trimmed, "current_turn" | "player_input")
        || trimmed.starts_with("turn_")
        || trimmed.starts_with("stw_")
        || trimmed.contains(':')
        || trimmed.contains('.')
}

fn entity_reference_alias(canonical: &str) -> Option<String> {
    for prefix in [
        "entities.characters:",
        "entities.places:",
        "entities.items:",
        "entities.objects:",
    ] {
        let Some(alias) = canonical.strip_prefix(prefix) else {
            continue;
        };
        if alias.contains(':') {
            return Some(alias.to_owned());
        }
    }
    None
}

fn affordance_reference_aliases(canonical: &str) -> Vec<String> {
    if !canonical.starts_with("encounter:") || !canonical.ends_with(":affordance") {
        return Vec::new();
    }
    [
        canonical.replace(":___:", "::"),
        canonical.replace(":__:", "::"),
    ]
    .into_iter()
    .filter(|alias| alias != canonical)
    .collect()
}

fn default_clear_ambiguity() -> String {
    "clear".to_owned()
}

fn default_success_kind() -> String {
    "success".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use singulari_world::{
        AgentSubmitTurnOptions, CompilePromptContextPacketOptions, InitWorldOptions,
        compile_prompt_context_packet, enqueue_agent_turn, init_world,
    };
    use tempfile::tempdir;

    fn seed_body(world_id: &str) -> String {
        format!(
            r#"
schema_version: singulari.world_seed.v1
world_id: {world_id}
title: "draft assembly test"
premise:
  genre: "fantasy"
  protagonist: "archive apprentice"
"#
        )
    }

    fn pending_and_context() -> Result<(tempfile::TempDir, PendingAgentTurn, PromptContextPacket)> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body("stw_draft"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_draft".to_owned(),
            input: "1".to_owned(),
            narrative_level: None,
        })?;
        let context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
            store_root: Some(store.as_path()),
            pending: &pending,
            engine_session_kind: "webgpt_project_session",
        })?;
        Ok((temp, pending, context))
    }

    fn draft() -> WebgptTurnDraft {
        WebgptTurnDraft {
            schema_version: WEBGPT_TURN_DRAFT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_draft".to_owned(),
            turn_id: "turn_0001".to_owned(),
            interpreted_intent: WebgptDraftIntent {
                summary: "플레이어가 항구의 압력을 살핀다.".to_owned(),
                target_hints: vec![
                    "place:opening_location".to_owned(),
                    "char:anchor".to_owned(),
                    "entities.characters:char:anchor".to_owned(),
                    "player_input".to_owned(),
                ],
                pressure_hints: vec!["pressure:open_questions".to_owned()],
                evidence_hints: vec!["player_input".to_owned()],
                ambiguity: "clear".to_owned(),
            },
            outcome: WebgptDraftOutcome {
                kind: "success".to_owned(),
                summary: "항구의 미해결 압력이 더 또렷해졌다.".to_owned(),
                evidence_hints: vec!["player_input".to_owned()],
            },
            visible_scene: WebgptDraftScene {
                text_blocks: vec!["물안개 사이로 검문소의 줄이 낮게 흔들렸다.".to_owned()],
                tone_notes: vec!["감각 압력을 유지했다.".to_owned()],
            },
            choices: (1..=5)
                .map(|slot| WebgptDraftChoice {
                    slot,
                    label: format!("draft label {slot}"),
                    intent: format!("draft intent {slot}"),
                    tag_hint: Some(format!("태그{slot}")),
                })
                .collect(),
            pressure_movements: Vec::new(),
        }
    }

    #[test]
    fn parses_complete_turn_draft_and_rejects_partial_prefix() -> Result<()> {
        let complete = serde_json::to_string(&draft())?;
        assert!(parse_webgpt_turn_draft(complete.as_str()).is_ok());

        let partial = r#"{"schema_version":"singulari.webgpt_turn_draft.v1","world_id":"stw_draft","turn_id":"turn_0001","interpreted_intent":{"summary":"x"}}, "visible_scene":{"text_blocks":["x"]}"#;
        assert!(parse_webgpt_turn_draft(partial).is_err());
        Ok(())
    }

    #[test]
    fn assembler_keeps_prose_from_webgpt_and_owns_slots_and_refs() -> Result<()> {
        let (_temp, pending, context) = pending_and_context()?;
        let draft = draft();
        let response = assemble_agent_turn_response(&pending, &context, &draft)?;

        assert_eq!(
            response.visible_scene.text_blocks,
            vec!["물안개 사이로 검문소의 줄이 낮게 흔들렸다."]
        );
        assert_eq!(response.next_choices.len(), 7);
        assert_eq!(response.next_choices[5].slot, FREEFORM_CHOICE_SLOT);
        assert_eq!(response.next_choices[6].slot, GUIDE_CHOICE_SLOT);
        let Some(proposal) = response.resolution_proposal else {
            anyhow::bail!("assembler should create resolution proposal");
        };
        assert_eq!(
            proposal.next_choice_plan[0].grounding_ref,
            context.pre_turn_simulation.available_affordances[0].affordance_id
        );
        assert!(
            proposal
                .pressure_noop_reasons
                .iter()
                .any(|reason| reason.pressure_ref.starts_with("pressure:"))
        );
        assert!(
            !proposal
                .interpreted_intent
                .target_refs
                .contains(&"player_input".to_owned())
        );
        assert!(
            !proposal
                .interpreted_intent
                .target_refs
                .iter()
                .any(|target_ref| target_ref.contains("char:anchor"))
        );
        Ok(())
    }

    #[test]
    fn assembler_materializes_transition_when_scene_director_is_stuck() -> Result<()> {
        let (_temp, pending, mut context) = pending_and_context()?;
        let mut director = SceneDirectorPacket::default();
        director.world_id.clone_from(&pending.world_id);
        director.turn_id.clone_from(&pending.turn_id);
        director.current_scene.scene_id = "scene:loop".to_owned();
        director.current_scene.scene_phase = ScenePhase::TransitionReady;
        director.current_scene.current_tension = TensionLevel::Medium;
        director.current_scene.evidence_refs = vec!["current_turn".to_owned()];
        director.pacing_state.transition_pressure = TransitionPressure::High;
        director.pacing_state.repeated_choice_shape_count = 9;
        director.pacing_state.unresolved_probe_turns = 10;
        context.visible_context.active_scene_director = serde_json::to_value(&director)?;

        let mut draft = draft();
        for choice in &mut draft.choices {
            choice.tag_hint = None;
            choice.label = "생활 공간 안에서 이미 일이 벌어진 뒤의 공기가 드러나고, 깨진 손잡이를 중심으로 첫 선택의 압력이 생겼다.".to_owned();
        }
        let response = assemble_agent_turn_response(&pending, &context, &draft)?;

        let Some(proposal) = response.scene_director_proposal else {
            anyhow::bail!("stuck scene director should materialize a transition proposal");
        };
        assert_eq!(proposal.beat_kind, DramaticBeatKind::Transition);
        assert!(proposal.transition.is_some());
        assert!(
            !response.next_choices[0]
                .tag
                .contains("생활 공간 안에서 이미 일이 벌어진")
        );
        assert!(response.next_choices[0].tag.chars().count() <= 14);
        Ok(())
    }
}
