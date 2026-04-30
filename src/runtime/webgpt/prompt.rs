use anyhow::{Context, Result};
use serde_json::Value;
use singulari_world::{AgentOutputContract, PromptContextPacket};
use std::collections::BTreeSet;

const NARRATIVE_TURN_PACKET_SCHEMA_VERSION: &str = "singulari.narrative_turn_packet.v1";
const PROMPT_REFERENCE_ARRAY_CAP: usize = 6;

const COMPACT_AGENT_TURN_RESPONSE_SCHEMA_GUIDE: &str = r#"AgentTurnResponse JSON만 반환한다.
필수 top-level:
- schema_version="singulari.agent_turn_response.v1", world_id, turn_id
- resolution_proposal, visible_scene, next_choices
- 선택 필드(scene_director_proposal, consequence_proposal, social_exchange_proposal, encounter_proposal, hook_events, *_updates, *_events, actor_*_events)는 이번 턴에서 실제 변화가 있을 때만 쓴다. 없으면 생략하거나 []/null로 둔다.
- 선택 배열 이름: "plot_thread_events", "scene_pressure_events", "world_lore_updates", "character_text_design_updates", "body_resource_events", "location_events", "hidden_state_delta".

resolution_proposal 필수:
- schema_version="singulari.resolution_proposal.v1", world_id, turn_id
- interpreted_intent: input_kind, summary, target_refs, pressure_refs, evidence_refs, ambiguity
- interpreted_intent.input_kind는 narrative_turn_packet.response_contract.interpreted_intent_input_kind 값만 쓴다. pre_turn_simulation.input_kind를 그대로 복사하지 않는다.
- interpreted_intent.ambiguity는 clear/minor/high 중 하나다.
- outcome: kind, summary, evidence_refs. kind는 success/partial_success/blocked/costly_success/delayed/escalated 중 하나다.
- gate_results 항목: gate_kind, gate_ref, visibility, status, reason, evidence_refs. status는 passed/softened/blocked/cost_imposed/unknown_needs_probe 중 하나다.
- proposed_effects 항목: effect_kind, target_ref, visibility, summary, evidence_refs.
- process_ticks 항목: process_ref, cause, visibility, summary, evidence_refs.
- pressure_noop_reasons: narrative_turn_packet.pre_turn_simulation.pressure_obligations의 각 pressure_id를 움직이지 않으면 반드시 pressure_ref/evidence_refs로 설명
- narrative_brief: visible_summary, required_beats, forbidden_visible_details
- next_choice_plan 항목: slot, plan_kind, grounding_ref, label_seed, intent_seed, evidence_refs. slot 1..5 plan_kind=ordinary_affordance, slot 6 plan_kind=freeform, slot 7 plan_kind=delegated_judgment.

visible_scene:
- schema_version="singulari.narrative_scene.v1"
- text_blocks: 한국어 VN prose 문단 배열
- tone_notes: 문자열 배열. 예: ["전역 서사는 감각 압력을 유지했다."]

next_choices:
- slot 1..5는 이번 visible_scene에서 곧바로 이어지는 구체 행동
- slot 6 tag="자유서술"
- slot 7 tag="판단 위임", intent="맡긴다. 세부 내용은 선택 후 드러난다."

엄격 규칙:
- narrative_turn_packet 안에 실제 존재하는 ref만 쓴다. 새 ref나 설명용 JSON pointer를 만들지 않는다.
- hidden/adjudication-only 내용은 visible_scene, next_choices, canon_event, player-visible summary에 쓰지 않는다.
- needs_context는 기본 []다. 쓰면 host가 커밋하지 않는다."#;

const SEEDLESS_PROSE_CONTRACT: &str = r"- 이 계약은 seedless style contract다. 여기 있는 문체/작법 규칙은 소재, 사건, 인물, 장소, 장르 장치, 과거사, 상징을 새로 만들 권한이 없다.
- scene_fact_boundaries: 오직 narrative turn packet의 player-visible facts, current player_input, visible canon, selected memory items에서 허용된 사실만 쓴다. style contract, schema examples, previous WebGPT phrasing, UI labels는 장면 사실이 아니다.
- 캐릭터 voice_anchors는 캐릭터 텍스트 디자인이다. speech는 화법, endings는 어미/말끝, tone은 어투/거리감/어휘, gestures는 반복 제스처, habits는 행동 습관, drift는 변화 방향으로 적용한다.
- 문체와 서사 작법은 캐릭터에 귀속하지 말고 visible_scene의 전역 서사에만 적용한다. 문단 순서는 장면 압력과 player_input에 맞춰 달라져야 하며, 고정된 전개 템플릿을 반복하지 않는다.
- paragraph_grammar: 각 문단은 감각 변화, 몸의 반응, 외부 압력, 해석을 유보한 단서, 다음 행동을 압박하는 변화 중 최소 둘을 포함한다.
- 시작 문단은 배경 설명이 아니라 현재 장면에서 감각적으로 바뀐 것과 visible constraint를 연다.
- 상호작용 문단은 말 한 줄, 작은 몸짓, 끊긴 반응, 침묵, 거리 변화 중 하나를 중심으로 둔다. 대사 뒤에는 설명 대신 행동이나 사물 반응을 붙인다.
- 마감 문단은 요약이나 교훈으로 닫지 말고, 바로 다음 선택을 압박하는 미해결 상태로 닫는다.
- dialogue_contract: 대사는 설명문이 아니다. 인물이 자기 상태, 세계 규칙, 선택지 의도를 길게 해설하지 않게 하고, 말은 짧은 호흡·망설임·생략·상대와의 거리감으로 구분한다.
- style_vector: sentence_pressure=high, exposition_directness=low, sensory_density=medium_high, dialogue_explanation=low, paragraph_closure=unresolved, metaphor_density=low_medium, interior_monologue=restrained, scene_continuity=strict.
- anti_translation_rules: 한국어 문체는 자연스러운 구어 기반 서사다. 번역체/보고서체/만연체를 피하고, 긴 인과문은 감각·반응·판단으로 쪼갠다.
- 문장은 보통 25~55자 안팎으로 끊고, 90자를 넘는 문장은 드물게만 쓴다. 한 문장에 원인, 감각, 판단, 결과를 모두 넣지 마라.
- `해당`, `진행`, `확인`, `수행`, `위치하다`, `존재하다`, `~하는 것이 필요했다`, `~하는 것으로 보였다`, `~할 수 있었다` 같은 번역체/보고서체 표현을 남발하지 마라.
- prohibited_seed_leakage: Style source는 리듬, 생략, 문단 배열, 대사 압력, 금지 표현만 제어한다. Style source를 content source로 해석하지 마라.
- 유명 작품명, 작가명, 장르 관습, 예시 문장, 문체 설명에서 소재를 빌려오지 마라. seed/canon에 없는 사물·인물·사건·상징·시대감은 품질 향상 명목으로도 추가하지 않는다.
- 추상 감정 설명보다 몸, 시선, 호흡, 손, 거리, 소리, 냄새, 온도 같은 관찰 가능한 흔적으로 보여준다.
- 문단 끝에는 다음 행동을 압박하는 작은 불편함이나 미해결 감각을 남긴다. 다만 선택지 의도나 내부 판정을 본문에서 해설하지 않는다.
- 문체를 설명문으로 노출하지 마라. 대사 말끝, 행동 습관, 문단 박자, 장면 압력으로만 체감되게 써라.
- tone_notes에는 이번 턴에서 실제로 반영한 캐릭터 화법/어미/어투와 전역 서사 문체를 짧게 기록한다.";

pub(super) fn build_webgpt_turn_prompt(prompt_context: &PromptContextPacket) -> Result<String> {
    let output_contract =
        serde_json::from_value::<AgentOutputContract>(prompt_context.output_contract.clone())
            .context("webgpt prompt context output_contract was not an AgentOutputContract")?;
    let narrative_turn_packet_value = build_narrative_turn_packet(prompt_context)?;
    let allowed_reference_atoms =
        serde_json::to_string_pretty(&build_allowed_reference_atoms(&narrative_turn_packet_value))
            .context("failed to serialize webgpt allowed reference atoms")?;
    let narrative_turn_packet = serde_json::to_string(&narrative_turn_packet_value)
        .context("failed to serialize webgpt narrative turn packet")?;
    let narrative_budget = &output_contract.narrative_budget;
    Ok(format!(
        r#"Singulari World web frontend에서 pending turn 하나가 들어왔어. 너는 WebGPT narrative engine adapter다.

서사 출력 지시:
- 이번 턴 서사 목표: {level_label}. 기본 선택 턴이면 {standard_blocks}문단 / 약 {target_chars}자까지 충분히 써라. 큰 사건이면 {major_blocks}문단 / 약 {major_target_chars}자까지 확장해라.
- text_blocks는 한 항목을 너무 길게 뭉치지 말고, 장면 박자마다 별도 문단으로 나눠라.
- 짧은 로그나 요약이 아니라 한국어 VN prose로 써라. 장면, 감각, 행동, 반응, 여운을 각각 분리해서 쌓아라.

역할:
- 너는 Singulari World의 trusted narrative agent다.
- 플레이어에게 다시 묻지 말고, 아래 narrative turn packet만 보고 바로 서사 턴을 작성한다.
- hidden/private context는 판정에만 쓰고, visible_scene/canon_event/choice text에는 절대 누출하지 않는다.
- 출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고, 게임식 수치 계산처럼 보이게 쓰지 않는다.
{text_design_directive}
- 출력량은 narrative turn packet의 output_contract.narrative_level과 narrative_budget을 따른다. 레벨 간 차이는 확연해야 한다.
- 레벨 1은 표준 VN 밀도, 레벨 2는 장면 확장 밀도, 레벨 3은 장편 연재 밀도다. 레벨 2/3에서는 같은 사건도 감각, 행동, 반응, 여운, 압박을 더 길게 쌓는다.
- player_input이 "세계 개막"이면 그것은 선택지가 아니라 시드에서 첫 서사를 여는 bootstrap turn이다.
- narrative_turn_packet.opening_randomizer가 있으면 사용자의 시드에 덧붙은 player-visible 개막 seed로 취급한다. 그 안의 location_frame, protagonist_frame, immediate_pressure, first_visible_object, social_weather, opening_question을 첫 장면의 시작 조건으로 반영한다.
- opening_randomizer가 없으면 사용자 시드와 visible facts만으로 시작한다. 이전 conversation 문구나 일반적인 bootstrap 기본값을 재사용하지 마라.
- opening_randomizer는 반복 수렴을 피하기 위한 시작 조건이지, 시드에 없는 장르 장치·숨은 과거사·고정 인물 설정을 만드는 권한이 아니다.
- 시드나 visible facts에 명시되지 않은 장르 장치, 과거사, 외부 세계 대비, 게임 인터페이스식 능력 구조를 추론해서 주입하지 마라. 이런 장치는 explicit positive evidence가 있을 때만 쓴다.
- protagonist가 현재 정보를 모른다는 사실만으로 장면 밖 배경, 과거사, 시대 대비 독백, 정체성 상실 클리셰를 만들지 마라.
- 매 턴 survival/social/material/threat/mystery/desire/moral_cost/time_pressure 중 최소 하나의 장면 압력을 visible_scene과 next_choices에 반영한다. 편향을 지우더라도 무미건조한 로그로 쓰지 마라.
- `anchor_character` 저장 필드는 호환용이다. 시드나 visible canon이 명시하지 않으면 구체 인물, 배후 구조, 정해진 역할로 해석하지 마라. 장면 초점은 visible evidence가 만든다.
- slot 7은 항상 판단 위임이고 preview는 숨긴다: "맡긴다. 세부 내용은 선택 후 드러난다."
- slot 6은 항상 자유서술이며 inline prose를 요구하는 선택지로 둔다.
- 이 WebGPT conversation의 이전 turn들은 말맛, 직전 감정선, 장면 리듬을 잇는 working context다.
- ChatGPT Project의 새 세션이나 기존 conversation history는 세계 상태 저장소가 아니다. 세계 연속성은 narrative_turn_packet으로만 복원한다.
- narrative_turn_packet.visible_context.active_scene_pressure는 이번 턴 선택지와 문단 박자를 누르는 압력 계약이다.
- narrative_turn_packet.visible_context.affordance_graph와 pre_turn_simulation.available_affordances는 slot 1..5의 행동 허가표다.
- narrative_turn_packet.visible_context.active_hook_ledger는 Promise/Echo 후킹 장부다. 새 사실을 만들지 말고, due promise는 진전/상환 후보로, returning echo는 비처벌적 여운/선택 압력으로만 반영한다.
- narrative_turn_packet.visible_context.active_body_resource_state와 active_location_graph는 몸/자원/장소 제약의 최소 상태다.
- narrative_turn_packet.visible_context.active_character_text_design과 selected_memory_items는 말맛과 가까운 연속성에만 쓴다.
- narrative_turn_packet.adjudication_boundary는 판정 전용이다. hidden/adjudication-only 세부 내용을 visible_scene, next_choices, canon_event, image prompt에 복사하지 마라.
- 세계의 사실/상태/source of truth는 아래 narrative turn packet과 world store다. 웹 채팅 UI나 이전 MCP tool 결과를 source of truth로 쓰지 마라.
- conversation/project context가 compact 되었거나 narrative turn packet과 충돌하면 narrative turn packet을 우선한다.
- 웹 검색, 외부 사이트 탐색, repo 탐색, 소스 파일 읽기를 하지 마라. 필요한 스키마와 revival packet은 이 프롬프트 안에 있다.

참조 ref 계약:
- `target_refs`, `pressure_refs`, `evidence_refs`, `gate_ref`,
  `grounding_ref`, `process_ref`, `effect.target_ref`는 아래
  `allowed_reference_atoms`에 있는 문자열만 정확히 복사한다.
- 몸/자원/위치/지식 상태를 설명하고 싶어도 새 ref를 만들지 않는다.
  정확한 ref가 없으면 `current_turn`, `player_input`, 선택된
  affordance_id, pressure_id, process_id 중 하나를 쓴다.
- 사람이 읽는 설명 문구, 상태 요약, JSON pointer, 새 `mind:*`,
  `body:*`, `rel:*` 같은 임의 ref는 금지한다.

allowed_reference_atoms JSON:
```json
{allowed_reference_atoms}
```

{agent_schema}

narrative turn packet JSON:
```json
{narrative_turn_packet}
```

출력:
- AgentTurnResponse JSON 하나만 반환한다.
- Markdown fence, 설명문, 도입문 없이 JSON 본문만 반환한다.
- world_id는 "{world_id}", turn_id는 "{turn_id}"와 정확히 같아야 한다.
"#,
        level_label = narrative_budget.level_label,
        standard_blocks = narrative_budget.standard_choice_turn_blocks,
        target_chars = narrative_budget.target_chars,
        major_blocks = narrative_budget.major_turn_blocks,
        major_target_chars = narrative_budget.major_target_chars,
        text_design_directive = SEEDLESS_PROSE_CONTRACT,
        allowed_reference_atoms = allowed_reference_atoms,
        agent_schema = COMPACT_AGENT_TURN_RESPONSE_SCHEMA_GUIDE,
        narrative_turn_packet = narrative_turn_packet,
        world_id = prompt_context.world_id,
        turn_id = prompt_context.turn_id,
    ))
}

fn build_allowed_reference_atoms(narrative_turn_packet: &Value) -> Vec<String> {
    let mut refs = BTreeSet::new();
    refs.insert("current_turn".to_owned());
    refs.insert("player_input".to_owned());
    collect_allowed_reference_atoms(None, narrative_turn_packet, &mut refs);
    refs.into_iter().collect()
}

fn collect_allowed_reference_atoms(key: Option<&str>, value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (child_key, child_value) in map {
                collect_allowed_reference_atoms(Some(child_key.as_str()), child_value, refs);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_allowed_reference_atoms(key, item, refs);
            }
        }
        Value::String(text) if key.is_some_and(is_reference_key) && is_ref_atom(text) => {
            refs.insert(text.to_owned());
        }
        _ => {}
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

fn build_narrative_turn_packet(prompt_context: &PromptContextPacket) -> Result<Value> {
    let full = serde_json::to_value(prompt_context)
        .context("failed to serialize prompt context for narrative turn packet")?;
    let pre_turn = prompt_safe_projection(
        full.get("pre_turn_simulation")
            .context("prompt context missing pre_turn_simulation")?
            .clone(),
    );
    let visible = prompt_safe_projection(
        full.get("visible_context")
            .context("prompt context missing visible_context")?
            .clone(),
    );
    let hidden_boundary = json_field(&pre_turn, "hidden_visibility_boundary");
    Ok(serde_json::json!({
        "schema_version": NARRATIVE_TURN_PACKET_SCHEMA_VERSION,
        "world_id": prompt_context.world_id,
        "turn_id": prompt_context.turn_id,
        "current_turn": prompt_context.current_turn,
        "opening_randomizer": prompt_context.opening_randomizer,
        "output_contract": prompt_context.output_contract,
        "response_contract": {
            "interpreted_intent_input_kind": response_input_kind_from_pre_turn(&pre_turn),
            "input_kind_mapping": "copy this value into resolution_proposal.interpreted_intent.input_kind"
        },
        "pre_turn_simulation": {
            "schema_version": json_field(&pre_turn, "schema_version"),
            "world_id": prompt_context.world_id,
            "turn_id": prompt_context.turn_id,
            "player_input": json_field(&pre_turn, "player_input"),
            "input_kind": json_field(&pre_turn, "input_kind"),
            "selected_choice": json_field(&pre_turn, "selected_choice"),
            "available_affordances": json_array_field(&pre_turn, "available_affordances"),
            "pressure_obligations": json_array_field(&pre_turn, "pressure_obligations"),
            "due_processes": json_array_field(&pre_turn, "due_processes"),
            "required_resolution_fields": json_field(&pre_turn, "required_resolution_fields"),
            "hidden_visibility_boundary": hidden_boundary.clone(),
        },
        "visible_context": {
            "recent_scene_window": json_array_field(&visible, "recent_scene_window"),
            "active_scene_pressure": json_field(&visible, "active_scene_pressure"),
            "active_plot_threads": json_field(&visible, "active_plot_threads"),
            "active_body_resource_state": json_field(&visible, "active_body_resource_state"),
            "active_location_graph": json_field(&visible, "active_location_graph"),
            "affordance_graph": json_field(&visible, "affordance_graph"),
            "active_scene_director": json_field(&visible, "active_scene_director"),
            "active_hook_ledger": json_field(&visible, "active_hook_ledger"),
            "narrative_style_state": json_field(&visible, "narrative_style_state"),
            "active_character_text_design": json_field(&visible, "active_character_text_design"),
            "selected_memory_items": json_array_field(&visible, "selected_memory_items"),
        },
        "adjudication_boundary": {
            "hidden_visibility_boundary": hidden_boundary,
            "policy": "hidden/adjudication-only details may shape resolution but must not appear in player-visible fields"
        },
        "source_of_truth_policy": prompt_context.source_of_truth_policy,
    }))
}

fn response_input_kind_from_pre_turn(pre_turn: &Value) -> &'static str {
    match pre_turn.get("input_kind").and_then(Value::as_str) {
        Some("guide_choice") => "delegated_judgment",
        Some("codex_query") => "codex_query",
        Some("numeric_choice" | "macro_time_flow" | "cc_canvas") => "presented_choice",
        _ => "freeform",
    }
}

fn prompt_safe_projection(value: Value) -> Value {
    prompt_safe_projection_for_key(None, value)
}

fn prompt_safe_projection_for_key(key: Option<&str>, value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(child_key, child_value)| {
                    let projected =
                        prompt_safe_projection_for_key(Some(child_key.as_str()), child_value);
                    (child_key, projected)
                })
                .collect(),
        ),
        Value::Array(items) if key.is_some_and(is_compactable_prompt_ref_key) => {
            compact_prompt_reference_array(items)
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| prompt_safe_projection_for_key(None, item))
                .collect(),
        ),
        scalar => scalar,
    }
}

fn is_compactable_prompt_ref_key(key: &str) -> bool {
    key.ends_with("_refs")
        || matches!(
            key,
            "source_refs" | "evidence_refs" | "target_refs" | "pressure_refs"
        )
}

fn compact_prompt_reference_array(items: Vec<Value>) -> Value {
    let mut seen = BTreeSet::new();
    let mut refs = Vec::new();
    for item in items {
        if let Some(reference) = item.as_str().map(str::trim)
            && !reference.is_empty()
            && seen.insert(reference.to_owned())
        {
            refs.push(reference.to_owned());
        }
    }

    if refs.len() <= PROMPT_REFERENCE_ARRAY_CAP {
        return Value::Array(refs.into_iter().map(Value::String).collect());
    }

    let tail_ref = refs
        .iter()
        .rev()
        .find(|reference| is_event_reference(reference))
        .or_else(|| refs.last())
        .cloned();
    let front_cap = tail_ref.as_ref().map_or(PROMPT_REFERENCE_ARRAY_CAP, |_| {
        PROMPT_REFERENCE_ARRAY_CAP - 1
    });
    let mut compact = Vec::new();
    for reference in refs {
        if tail_ref.as_ref() == Some(&reference) {
            continue;
        }
        if compact.len() >= front_cap {
            break;
        }
        compact.push(Value::String(reference));
    }
    if let Some(tail_ref) = tail_ref {
        compact.push(Value::String(tail_ref));
    }
    Value::Array(compact)
}

fn is_event_reference(reference: &str) -> bool {
    reference.contains("_event:")
}

fn json_field(source: &Value, key: &str) -> Value {
    source.get(key).cloned().unwrap_or(Value::Null)
}

fn json_array_field(source: &Value, key: &str) -> Value {
    source.get(key).cloned().unwrap_or(Value::Array(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_safe_projection_caps_repeated_refs_and_keeps_latest_event() {
        let projected = prompt_safe_projection(serde_json::json!({
            "active_scene_pressure": [{
                "pressure_id": "pressure:gate",
                "source_refs": [
                    "visible_scene.text_blocks[0]",
                    "visible_scene.text_blocks[0]",
                    "scene_pressure_event:scene_pressure_event:turn_0001:00",
                    "scene_pressure_event:scene_pressure_event:turn_0001:01",
                    "scene_pressure_event:scene_pressure_event:turn_0001:02",
                    "scene_pressure_event:scene_pressure_event:turn_0001:03",
                    "scene_pressure_event:scene_pressure_event:turn_0001:04",
                    "scene_pressure_event:scene_pressure_event:turn_0001:05",
                    "scene_pressure_event:scene_pressure_event:turn_0001:06"
                ]
            }],
            "pressure_obligations": [{
                "pressure_ref": "pressure:gate",
                "evidence_refs": [
                    "pressure:gate",
                    "plot_thread_event:plot_thread_event:turn_0001:00",
                    "plot_thread_event:plot_thread_event:turn_0001:01",
                    "plot_thread_event:plot_thread_event:turn_0001:02",
                    "plot_thread_event:plot_thread_event:turn_0001:03",
                    "plot_thread_event:plot_thread_event:turn_0001:04",
                    "plot_thread_event:plot_thread_event:turn_0001:05",
                    "plot_thread_event:plot_thread_event:turn_0001:06"
                ]
            }]
        }));

        let Some(source_refs) = projected
            .pointer("/active_scene_pressure/0/source_refs")
            .and_then(Value::as_array)
        else {
            panic!("source refs should remain an array");
        };
        let Some(evidence_refs) = projected
            .pointer("/pressure_obligations/0/evidence_refs")
            .and_then(Value::as_array)
        else {
            panic!("evidence refs should remain an array");
        };

        assert_eq!(source_refs.len(), PROMPT_REFERENCE_ARRAY_CAP);
        assert_eq!(evidence_refs.len(), PROMPT_REFERENCE_ARRAY_CAP);
        assert!(source_refs.contains(&Value::String(
            "scene_pressure_event:scene_pressure_event:turn_0001:06".to_owned()
        )));
        assert!(evidence_refs.contains(&Value::String(
            "plot_thread_event:plot_thread_event:turn_0001:06".to_owned()
        )));
    }
}
