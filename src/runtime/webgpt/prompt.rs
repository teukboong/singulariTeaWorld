use anyhow::{Context, Result};
use singulari_world::{AgentOutputContract, PromptContextPacket};

const AGENT_TURN_RESPONSE_SCHEMA_GUIDE: &str = r#"AgentTurnResponse 스키마:
```json
{
  "schema_version": "singulari.agent_turn_response.v1",
  "world_id": "<world_id>",
  "turn_id": "<turn_id>",
  "resolution_proposal": {
    "schema_version": "singulari.resolution_proposal.v1",
    "world_id": "<world_id>",
    "turn_id": "<turn_id>",
    "interpreted_intent": {
      "input_kind": "presented_choice|freeform|delegated_judgment|codex_query",
      "summary": "플레이어 입력을 장면 상태 안에서 어떻게 해석했는지",
      "target_refs": ["prompt_context 안의 장소/압력/믿음/자원/affordance/process ref"],
      "pressure_refs": ["prompt_context.visible_context.active_scene_pressure의 pressure_id"],
      "evidence_refs": ["current_turn 또는 prompt_context 안의 visible ref"],
      "ambiguity": "clear|minor|high"
    },
    "outcome": {
      "kind": "success|partial_success|blocked|costly_success|delayed|escalated",
      "summary": "플레이어-visible 결과 요약",
      "evidence_refs": ["visible ref"]
    },
    "gate_results": [
      {
        "gate_kind": "body|resource|location|social_permission|knowledge|time_pressure|hidden_constraint|world_law|affordance",
        "gate_ref": "prompt_context 안의 ref",
        "visibility": "player_visible|adjudication_only",
        "status": "passed|softened|blocked|cost_imposed|unknown_needs_probe",
        "reason": "visible이면 플레이어-visible 이유, hidden이면 본문 복사 금지",
        "evidence_refs": ["visible ref 또는 adjudication ref"]
      }
    ],
    "proposed_effects": [],
    "process_ticks": [],
    "narrative_brief": {
      "visible_summary": "visible_scene을 쓰기 전 결과/박자 요약",
      "required_beats": ["visible prose에 반드시 반영할 박자"],
      "forbidden_visible_details": ["visible text에 쓰면 안 되는 hidden/detail"]
    },
    "next_choice_plan": [
      {
        "slot": 1,
        "plan_kind": "ordinary_affordance",
        "grounding_ref": "affordance:slot:1:move",
        "label_seed": "선택지 tag seed",
        "intent_seed": "선택지 intent seed",
        "evidence_refs": ["affordance:slot:1:move"]
      },
      {
        "slot": 2,
        "plan_kind": "ordinary_affordance",
        "grounding_ref": "affordance:slot:2:observe",
        "label_seed": "선택지 tag seed",
        "intent_seed": "선택지 intent seed",
        "evidence_refs": ["affordance:slot:2:observe"]
      },
      {
        "slot": 3,
        "plan_kind": "ordinary_affordance",
        "grounding_ref": "affordance:slot:3:contact",
        "label_seed": "선택지 tag seed",
        "intent_seed": "선택지 intent seed",
        "evidence_refs": ["affordance:slot:3:contact"]
      },
      {
        "slot": 4,
        "plan_kind": "ordinary_affordance",
        "grounding_ref": "affordance:slot:4:body_resource",
        "label_seed": "선택지 tag seed",
        "intent_seed": "선택지 intent seed",
        "evidence_refs": ["affordance:slot:4:body_resource"]
      },
      {
        "slot": 5,
        "plan_kind": "ordinary_affordance",
        "grounding_ref": "affordance:slot:5:pressure_response",
        "label_seed": "선택지 tag seed",
        "intent_seed": "선택지 intent seed",
        "evidence_refs": ["affordance:slot:5:pressure_response"]
      },
      {
        "slot": 6,
        "plan_kind": "freeform",
        "grounding_ref": "current_turn",
        "label_seed": "자유서술",
        "intent_seed": "직접 행동을 입력한다",
        "evidence_refs": ["current_turn"]
      },
      {
        "slot": 7,
        "plan_kind": "delegated_judgment",
        "grounding_ref": "current_turn",
        "label_seed": "판단 위임",
        "intent_seed": "맡긴다. 세부 내용은 선택 후 드러난다.",
        "evidence_refs": ["current_turn"]
      }
    ]
  },
  "scene_director_proposal": {
    "schema_version": "singulari.scene_director_proposal.v1",
    "world_id": "<world_id>",
    "turn_id": "<turn_id>",
    "scene_id": "prompt_context.visible_context.active_scene_director.current_scene.scene_id",
    "beat_kind": "establish|probe|escalate|complicate|reveal|cost|choice_pressure|decompress|transition|cliffhanger",
    "turn_function": "이번 턴이 장면 안에서 수행한 구조적 역할",
    "tension_before": "low|medium|high",
    "tension_after": "low|medium|high",
    "scene_effect": "established|scene_question_narrowed|pressure_increased|pressure_softened|cost_imposed|visible_fact_revealed|choice_surface_changed|scene_question_transformed|necessary_stall",
    "paragraph_strategy": {
      "opening_shape": "observable_detail|action_consequence|dialogue_pressure|new_visible_change",
      "middle_shape": "blocked_relation|material_constraint|signal_interpretation|cost_tradeoff",
      "closure_shape": "forced_next_decision|concrete_unresolved_pressure|transition_handoff"
    },
    "choice_strategy": {
      "must_change_choice_shape": true,
      "avoid_recent_choice_tags": ["최근 반복된 선택지 태그"]
    },
    "transition": null,
    "evidence_refs": ["prompt_context 안의 visible ref"]
  },
  "consequence_proposal": {
    "schema_version": "singulari.consequence_proposal.v1",
    "world_id": "<world_id>",
    "turn_id": "<turn_id>",
    "introduced": [],
    "updated": [],
    "paid_off": [],
    "ephemeral_effects": []
  },
  "visible_scene": {
    "schema_version": "singulari.narrative_scene.v1",
    "text_blocks": ["위 서사 출력 지시와 pending.output_contract.narrative_budget에 맞춘 한국어 VN 본문"],
    "tone_notes": ["짧은 톤 메모"]
  },
  "adjudication": {
    "outcome": "accepted",
    "summary": "플레이어-visible 한 줄 요약",
    "gates": [
      {"gate":"body","status":"pass","reason":"..."},
      {"gate":"resource","status":"pass","reason":"..."},
      {"gate":"time","status":"pass","reason":"..."},
      {"gate":"social_permission","status":"pass","reason":"..."},
      {"gate":"knowledge","status":"pass","reason":"..."}
    ],
    "visible_constraints": ["아직 확인되지 않은 플레이어-visible 제약"],
    "consequences": ["이번 턴의 플레이어-visible 결과"]
  },
  "canon_event": {
    "visibility": "player_visible",
    "kind": "guided_choice",
    "summary": "플레이어-visible 사건 요약"
  },
  "entity_updates": [
    {
      "entity_id": "char_or_place_or_item_id",
      "update_kind": "seen_action",
      "visibility": "player_visible",
      "summary": "이번 턴에서 player-visible entity state가 어떻게 변했는지",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "relationship_updates": [
    {
      "source_entity_id": "char:a",
      "target_entity_id": "char:b",
      "relation_kind": "suspicion|trust|debt|fear|distance",
      "visibility": "player_visible",
      "summary": "이번 턴에서 대사 거리감/협조/의심에 영향을 주는 관계 변화",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "plot_thread_events": [
    {
      "thread_id": "prompt_context.visible_context.active_plot_threads 중 이번 턴에서 실제로 변한 thread_id",
      "change": "advanced",
      "status_after": "active",
      "urgency_after": "soon",
      "summary": "이번 visible_scene에서 이 thread가 어떻게 변했는지 한 문장",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "scene_pressure_events": [
    {
      "pressure_id": "prompt_context.visible_context.active_scene_pressure 중 이번 턴에서 실제로 변한 pressure_id",
      "change": "increased",
      "intensity_after": 3,
      "urgency_after": "soon",
      "summary": "이번 visible_scene에서 압력이 어떻게 변했는지",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "world_lore_updates": [
    {
      "subject": "player-visible subject",
      "predicate": "is|has|requires|forbids",
      "object": "이번 턴에서 확정된 세계 규칙/관습/장소 사실",
      "category": "customs|geography|social_order|danger_model|language_register",
      "visibility": "player_visible",
      "summary": "세계관에 남길 player-visible 사실",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "character_text_design_updates": [
    {
      "character_id": "char:id",
      "speech_pattern": "화법/어미/말버릇",
      "gesture_pattern": "습관적 제스처",
      "drift_note": "이번 턴 이후 조정할 말맛",
      "visibility": "player_visible",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "body_resource_events": [
    {
      "event_kind": "resource_gained",
      "target_id": "resource:scene:item",
      "visibility": "player_visible",
      "summary": "이번 턴에서 얻거나 잃은 visible body/resource 변화",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "location_events": [
    {
      "event_kind": "discovered",
      "location_id": "place:id",
      "name": "플레이어-visible 장소 이름",
      "knowledge_state": "known",
      "summary": "이번 턴에서 열린 장소/동선 변화",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "extra_contacts": [],
  "hidden_state_delta": [
    {
      "delta_kind": "secret_status",
      "target_id": "secret_or_timer_id",
      "summary": "판정 전용 hidden delta. visible text에 복사 금지",
      "evidence_refs": ["private_adjudication_context"]
    }
  ],
  "needs_context": [],
  "actor_goal_events": [
    {
      "actor_ref": "char:id",
      "goal_id": "goal:char:id:local_goal",
      "visibility": "player_visible|hidden_adjudication_only",
      "desire": "이 인물이 현재 장면에서 밀고 있는 국소 목표",
      "fear_or_constraint": "목표를 제한하는 visible 또는 hidden 제약",
      "current_leverage": ["관찰 가능한 수단/우위"],
      "pressure_refs": ["pressure:id"],
      "evidence_refs": ["visible_scene.text_blocks[0]"],
      "retired": false
    }
  ],
  "actor_move_events": [
    {
      "actor_ref": "char:id",
      "move_id": "move:char:id:observable_move",
      "visibility": "player_visible|hidden_adjudication_only",
      "action_summary": "이번 턴에 관찰된 인물 행동",
      "produced_pressure_refs": ["pressure:id"],
      "relationship_refs": ["rel:char:id->player:stance"],
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "next_choices": [
    {"slot":1,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"현재 장면 단서와 player_input에서 이어지는 구체 행동"},
    {"slot":2,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"몸, 장소, 물건, 흔적 중 이번 장면에 실제로 나온 단서를 살핀다"},
    {"slot":3,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"이번 장면에 실제로 있는 인물, 기척, 관계 신호에 반응한다"},
    {"slot":4,"tag":"현재 장면에 맞춘 기록 선택명","intent":"이번 장면에서 드러난 기록/단서/세계 지식을 확인한다"},
    {"slot":5,"tag":"현재 장면에 맞춘 흐름 선택명","intent":"이번 사건의 변화 압력이 다음 행동에 어떤 영향을 주는지 본다"},
    {"slot":6,"tag":"자유서술","intent":"플레이어가 원하는 행동과 말, 내면 독백을 직접 서술한다"},
    {"slot":7,"tag":"판단 위임","intent":"맡긴다. 세부 내용은 선택 후 드러난다."}
  ]
}
```
- next_choices는 서사 생성과 같은 응답에서 반드시 함께 작성한다. 별도 선택지 재생성 턴을 만들지 않는다.
- resolution_proposal은 LLM 지능이 해석한 판정 제안이고, Rust가 commit 전에 audit한다. 가능한 한 작성하되, prompt_context에 없는 ref나 evidence_refs 없는 durable effect를 넣지 않는다.
- scene_director_proposal은 선택(optional)이다. 작성할 경우 prompt_context.visible_context.active_scene_director의 current_scene/recommended_next_beats/paragraph_budget_hint에 맞춰 이번 턴의 구조적 역할을 요약한다.
- scene_director_proposal은 resolution_proposal을 대체하지 않는다. 장면 박자와 선택지 모양만 설명하며, 새 canon 사실이나 hidden motive를 만들 권한이 없다.
- scene_director_proposal.evidence_refs는 prompt_context JSON 안의 player-visible 문자열 ref만 쓴다. hidden/adjudication-only ref는 금지한다.
- consequence_proposal은 선택(optional)이다. 작성할 경우 비용, 의심, 관계 변화, 지식 변화, 기회 상실처럼 다음 턴에도 돌아와야 하는 여파만 기록한다.
- consequence_proposal은 typed projection을 대체하지 않는다. body/resource/relationship/process/lore 변화는 기존 effect/event와 evidence로 연결하고, 사소한 색채 효과는 ephemeral_effects로 설명한다.
- resolution_proposal의 모든 `target_refs`, `pressure_refs`, `gate_ref`, `grounding_ref`, `process_ref`, `effect.target_ref`는 prompt_context JSON 안에 실제 문자열로 존재하는 ref만 쓴다. 설명용 JSON pointer(`visible_context...`)나 새로 만든 관계/장소/인물 ref는 쓰지 않는다.
- selected_context_capsules와 selected_memory_items 안의 `source_id`, `edge_id`, `capsule_id`, `entity_id`, `location_id`, `pressure_id`, `affordance_id`처럼 실제 문자열로 들어 있는 ref는 evidence로 쓸 수 있다. 단 rejected_capsules에만 있는 ref는 쓰지 않는다.
- resolution_proposal의 visible field에는 hidden/adjudication-only 세부 내용을 절대 쓰지 않는다.
- resolution_proposal.next_choice_plan은 slot 1..7을 모두 포함한다. ordinary_affordance slot 1..5는 prompt_context.visible_context.affordance_graph의 같은 slot affordance_id를 grounding_ref/evidence_refs에 넣는다. slot 6은 freeform, slot 7은 delegated_judgment다.
- actor_goal_events/actor_move_events는 중요한 NPC가 실제 장면에서 보인 목표와 움직임만 기록한다. actor_ref는 기존 relationship/entity ref 또는 이번 응답의 entity_updates로 생성한 char:id여야 한다. hidden 목표는 visible_scene/next_choices에 동기로 해설하지 말고 관찰 가능한 행동으로만 암시한다.
- slot 1,2,3,4,5의 tag/intent는 템플릿 문구가 아니라 이번 visible_scene에서 바로 이어지는 구체 선택지여야 한다.
- next_choices 안에는 label/preview/choices 필드를 쓰지 않는다. 오직 slot/tag/intent만 쓴다.
- plot_thread_events는 이번 턴에서 실제로 진행/복잡화/차단/해결/실패/퇴장한 active_visible thread만 적는다. 변화가 없으면 빈 배열이다.
- plot_thread_events.thread_id는 prompt_context.visible_context.active_plot_threads에 있는 thread_id만 쓴다. 새 thread를 임의로 만들거나 hidden/dormant 상태로 바꾸지 않는다.
- scene_pressure_events는 이번 턴에서 실제로 강해지거나 약해진 visible_active pressure만 적는다. hidden_adjudication_only pressure_id는 절대 쓰지 않는다.
- entity_updates/relationship_updates/world_lore_updates/character_text_design_updates는 전부 typed schema다. 변화가 없으면 빈 배열이며, 임의 key/value JSON을 넣지 않는다.
- body_resource_events/location_events도 typed schema다. 장면에 실제 증거가 없으면 빈 배열로 둔다.
- needs_context는 기본적으로 빈 배열이다. 현재 prompt_context.selected_context_capsules에 없는 맥락 없이는 응답을 닫을 수 없을 때만 request_id/capsule_kinds/query/reason/evidence_refs를 넣는다. needs_context가 비어 있지 않으면 host는 그 응답을 커밋하지 않는다.
- slot 번호가 기능 계약이다. tag는 UI 문구이므로 장면에 맞게 짧게 바꿔도 된다. 단 slot 7 tag는 "판단 위임"으로 유지한다.
- extra_contacts는 주변 인물이 플레이어와 직접 상호작용했거나, 의미 있는 목격/거래/도움/위협/감정 흔적을 남겼을 때만 쓴다.
- extra_contacts 항목을 쓸 때는 surface_label, contact_summary를 반드시 실제 장면 내용으로 채운다. 스키마 설명 문구나 예시 문구를 값으로 복사하지 않는다.
- 단순 배경 군중은 extra_contacts에 넣지 않는다. 한 번 스쳐간 인물은 memory_action "trace", 다시 떠올릴 이유가 분명하면 "remember"를 쓴다."#;

const SEEDLESS_PROSE_CONTRACT: &str = r"- 이 계약은 seedless style contract다. 여기 있는 문체/작법 규칙은 소재, 사건, 인물, 장소, 장르 장치, 과거사, 상징을 새로 만들 권한이 없다.
- scene_fact_boundaries: 오직 prompt context packet의 player-visible facts, current player_input, visible canon, selected memory items에서 허용된 사실만 쓴다. style contract, schema examples, previous WebGPT phrasing, UI labels는 장면 사실이 아니다.
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
    let prompt_context_packet = serde_json::to_string(prompt_context)
        .context("failed to serialize webgpt prompt context packet")?;
    let narrative_budget = &output_contract.narrative_budget;
    Ok(format!(
        r#"Singulari World web frontend에서 pending turn 하나가 들어왔어. 너는 WebGPT narrative engine adapter다.

서사 출력 지시:
- 이번 턴 서사 목표: {level_label}. 기본 선택 턴이면 {standard_blocks}문단 / 약 {target_chars}자까지 충분히 써라. 큰 사건이면 {major_blocks}문단 / 약 {major_target_chars}자까지 확장해라.
- text_blocks는 한 항목을 너무 길게 뭉치지 말고, 장면 박자마다 별도 문단으로 나눠라.
- 짧은 로그나 요약이 아니라 한국어 VN prose로 써라. 장면, 감각, 행동, 반응, 여운을 각각 분리해서 쌓아라.

역할:
- 너는 Singulari World의 trusted narrative agent다.
- 플레이어에게 다시 묻지 말고, 아래 prompt context packet만 보고 바로 서사 턴을 작성한다.
- hidden/private context는 판정에만 쓰고, visible_scene/canon_event/choice text에는 절대 누출하지 않는다.
- 출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고, 게임식 수치 계산처럼 보이게 쓰지 않는다.
{text_design_directive}
- 출력량은 prompt context packet의 output_contract.narrative_level과 narrative_budget을 따른다. 레벨 간 차이는 확연해야 한다.
- 레벨 1은 표준 VN 밀도, 레벨 2는 장면 확장 밀도, 레벨 3은 장편 연재 밀도다. 레벨 2/3에서는 같은 사건도 감각, 행동, 반응, 여운, 압박을 더 길게 쌓는다.
- player_input이 "세계 개막"이면 그것은 선택지가 아니라 시드에서 첫 서사를 여는 bootstrap turn이다.
- prompt_context.opening_randomizer가 있으면 사용자의 시드에 덧붙은 player-visible 개막 seed로 취급한다. 그 안의 location_frame, protagonist_frame, immediate_pressure, first_visible_object, social_weather, opening_question을 첫 장면의 시작 조건으로 반영한다.
- opening_randomizer가 없으면 사용자 시드와 visible facts만으로 시작한다. 이전 conversation 문구나 일반적인 bootstrap 기본값을 재사용하지 마라.
- opening_randomizer는 반복 수렴을 피하기 위한 시작 조건이지, 시드에 없는 장르 장치·숨은 과거사·고정 인물 설정을 만드는 권한이 아니다.
- 시드나 visible facts에 명시되지 않은 장르 장치, 과거사, 외부 세계 대비, 게임 인터페이스식 능력 구조를 추론해서 주입하지 마라. 이런 장치는 explicit positive evidence가 있을 때만 쓴다.
- protagonist가 현재 정보를 모른다는 사실만으로 장면 밖 배경, 과거사, 시대 대비 독백, 정체성 상실 클리셰를 만들지 마라.
- 매 턴 survival/social/material/threat/mystery/desire/moral_cost/time_pressure 중 최소 하나의 장면 압력을 visible_scene과 next_choices에 반영한다. 편향을 지우더라도 무미건조한 로그로 쓰지 마라.
- `anchor_character` 저장 필드는 호환용이다. 시드나 visible canon이 명시하지 않으면 구체 인물, 배후 구조, 정해진 역할로 해석하지 마라. 장면 초점은 visible evidence가 만든다.
- slot 7은 항상 판단 위임이고 preview는 숨긴다: "맡긴다. 세부 내용은 선택 후 드러난다."
- slot 6은 항상 자유서술이며 inline prose를 요구하는 선택지로 둔다.
- 이 WebGPT conversation의 이전 turn들은 말맛, 직전 감정선, 장면 리듬을 잇는 working context다.
- ChatGPT Project의 새 세션이나 기존 conversation history는 세계 상태 저장소가 아니다. 세계 연속성은 prompt_context_packet으로만 복원한다.
- prompt_context.visible_context.active_scene_pressure는 이번 턴 선택지와 문단 박자를 누르는 압력 계약이다. visible_scene/next_choices에 반영한다.
- prompt_context.visible_context.active_plot_threads는 현재 열린 문제와 미해결 질문이다. quest-log처럼 설명하지 말고, 이번 장면이 자연스럽게 건드리는 thread만 선택지와 장면 압력으로 이어라.
- prompt_context.visible_context.active_body_resource_state는 주인공의 몸 상태와 실제 보유 자원이다. 없는 물건을 선택지 해결책으로 만들지 말고, 몸 제약은 행동/감각/타인 반응에 필요한 만큼만 반영해라.
- prompt_context.visible_context.active_location_graph는 현재 장소와 알려진 주변 장소다. 이동 선택지는 이 표면의 known/visited 장소 또는 visible_scene에서 정당화된 탐색 행동으로만 열어라.
- prompt_context.visible_context.affordance_graph는 slot 1..5의 행동 허가표다. next_choices 1..5는 각 slot의 affordance_kind/action_contract/source_refs/forbidden_shortcuts를 지켜 장면별 문구로 다시 써라. affordance_id나 source_refs 자체를 선택지에 노출하지 마라.
- prompt_context.visible_context.belief_graph는 주인공과 player-visible narrator가 확정적으로 아는 것의 경계다. belief node가 없는 원인, 정체, 배후, 과거사, 세계 규칙은 확정 서술하지 말고 단서나 불확실성으로만 남겨라.
- prompt_context.visible_context.world_process_clock는 보이는 세계 진행 압력이다. 다음 턴으로 넘기면 악화, 완화, 전환, 해소 중 하나가 일어날 수 있음을 문단 압력과 선택지 비용에 반영해라.
- prompt_context.visible_context.active_scene_director는 장면 박자 권고다. recommended_next_beats/forbidden_repetition/paragraph_budget_hint를 사용해 이번 턴 기능을 바꾸되, beat taxonomy나 Scene Director 용어를 player-visible text에 노출하지 마라. 이 packet은 canon source가 아니며 새 사실, hidden motive, 장소, 결과를 만들 권한이 없다.
- prompt_context.visible_context.narrative_style_state는 서사 문체와 문단 박자 계약이다. 소재나 설정을 만들지 말고 밀도, 문장 압력, 대사 호흡, 번역체 방지에만 적용해라.
- prompt_context.visible_context.active_character_text_design은 캐릭터별 화법/어미/어투/제스처/습관/drift 계약이다. 전역 문체와 섞지 말고, 인물이 말하거나 행동할 때만 자연스럽게 반영해라.
- prompt_context.visible_context.active_change_ledger는 플레이어 행동으로 변한 세계/관계/압력의 요약 장부다. 오래된 원시 사건보다 active_changes의 before/after/cause_turns를 우선해서 현재 장면의 여파로 반영해라.
- prompt_context.visible_context.active_pattern_debt는 반복 방지 압력이다. canon 사실로 쓰지 말고 replacement_pressure를 선택지 모양, 장면 박자, 문단 마감의 변화로만 반영해라.
- prompt_context.visible_context.active_belief_graph는 장기 누적된 믿음/오해/추론 경계다. 세계의 객관 진실이 아니라 holder/confidence가 붙은 인식 상태로만 써라.
- prompt_context.visible_context.active_world_process_clock는 장기 진행 압력이다. 매 턴 자동 진행하지 말고 player action, time passage, pressure evidence가 닿을 때만 결과에 반영해라.
- prompt_context.visible_context.active_player_intent_trace는 최근 플레이어 행동 모양이다. 플레이어 성격으로 고정하지 말고 다음 선택지 affordance를 미세 조정하는 scene-scoped 압력으로만 써라.
- prompt_context.visible_context.active_turn_retrieval_controller는 이번 턴의 검색 목표와 cue다. canon이나 인물 심리가 아니라 selected_context_capsules를 왜 읽었는지 설명하는 selector brain으로만 써라.
- prompt_context.visible_context.selected_context_capsules는 이번 턴 compiler가 실제로 로드한 context capsule 본문이다. rejected_capsules는 이번 턴에 의도적으로 배제된 맥락이므로 복원하거나 우회하지 마라. broad projection보다 selected capsule을 우선하되, capsule은 source of truth가 아니라 prompt transport unit임을 유지해라.
- prompt_context.visible_context.selected_memory_items는 이번 턴에 물리적으로 선택된 장기기억이다. 각 item의 reason/evidence_refs/source_id를 따라 필요한 항목만 쓰고, selected_memory_items 전체를 다시 요약하지 마라.
- prompt_context.adjudication_context는 판정 전용이다. hidden_world_process_clock를 포함해 visible_scene, next_choices, canon_event, image prompt에 복사하지 마라.
- prompt_context.prompt_policy.omitted_debug_sections는 의도적으로 프롬프트에서 뺀 debug/source 섹션이다. 비어 있는 사실을 임의로 복원하지 마라.
- prompt_context.budget_report는 이번 프롬프트에 포함/제외된 장기맥락의 감사표다. included에 없는 source를 기억으로 취급하지 말고, excluded는 필요하면 다음 턴 compiler가 다시 선별할 수 있는 후보일 뿐 현재 사실로 쓰지 마라.
- 세계의 사실/상태/source of truth는 아래 prompt context packet과 world store다. 웹 채팅 UI나 이전 MCP tool 결과를 source of truth로 쓰지 마라.
- conversation/project context가 compact 되었거나 prompt context packet과 충돌하면 prompt context packet을 우선한다.
- 웹 검색, 외부 사이트 탐색, repo 탐색, 소스 파일 읽기를 하지 마라. 필요한 스키마와 revival packet은 이 프롬프트 안에 있다.

{agent_schema}

prompt context packet JSON:
```json
{prompt_context_packet}
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
        agent_schema = AGENT_TURN_RESPONSE_SCHEMA_GUIDE,
        prompt_context_packet = prompt_context_packet,
        world_id = prompt_context.world_id,
        turn_id = prompt_context.turn_id,
    ))
}
