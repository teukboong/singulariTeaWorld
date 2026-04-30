use anyhow::{Context, Result};
use serde_json::Value;
use singulari_world::{AgentOutputContract, PromptContextPacket};
use std::collections::BTreeSet;

const NARRATIVE_TURN_PACKET_SCHEMA_VERSION: &str = "singulari.narrative_turn_packet.v1";

#[allow(dead_code)]
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
      "target_refs": ["narrative_turn_packet 안의 장소/압력/믿음/자원/affordance/process ref"],
      "pressure_refs": ["narrative_turn_packet.visible_context.active_scene_pressure의 pressure_id"],
      "evidence_refs": ["current_turn 또는 narrative_turn_packet 안의 visible ref"],
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
        "gate_ref": "narrative_turn_packet 안의 ref",
        "visibility": "player_visible|adjudication_only",
        "status": "passed|softened|blocked|cost_imposed|unknown_needs_probe",
        "reason": "visible이면 플레이어-visible 이유, hidden이면 본문 복사 금지",
        "evidence_refs": ["visible ref 또는 adjudication ref"]
      }
    ],
    "proposed_effects": [],
    "process_ticks": [],
    "pressure_noop_reasons": [
      {
        "pressure_ref": "pressure id from narrative_turn_packet.pre_turn_simulation.pressure_obligations",
        "reason": "이 턴에서 압력이 직접 이동하지 않았다면 visible한 이유",
        "evidence_refs": ["same pressure id"]
      }
    ],
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
    "scene_id": "narrative_turn_packet.visible_context.active_scene_director.current_scene.scene_id",
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
    "evidence_refs": ["narrative_turn_packet 안의 visible ref"]
  },
  "consequence_proposal": {
    "schema_version": "singulari.consequence_proposal.v1",
    "world_id": "<world_id>",
    "turn_id": "<turn_id>",
    "introduced": [],
    "updated": [],
    "paid_off": [],
    "ephemeral_effects": [
      {
        "effect_ref": "visible_scene.text_blocks[0]",
        "reason": "장기 consequence로 남기지 않을 감각/분위기 효과만 여기에 적는다.",
        "evidence_refs": ["visible_scene.text_blocks[0]"]
      }
    ]
  },
  "social_exchange_proposal": {
    "schema_version": "singulari.social_exchange_proposal.v1",
    "world_id": "<world_id>",
    "turn_id": "<turn_id>",
    "exchanges": [
      {
        "actor_ref": "player|char:id|rel:id|scene:social_outcome",
        "target_ref": "player|char:id|rel:id",
        "act_kind": "ask|answer|evade|refuse|offer|accept|counter_offer|threaten|apologize|insult|promise|demand|reveal_conditionally|withhold|test|grant_permission|revoke_permission",
        "stance_after": "neutral_procedure|wary_testing|cooperative|guarded_helpful|offended|evasive|threatening|bargaining|indebted|pressuring|appeasing|withholding",
        "intensity_after": "trace|low|medium|high|crisis",
        "summary": "이번 대화에서 바뀐 사회적 계약",
        "player_visible_signal": "플레이어가 알 수 있는 대화 상태 신호",
        "source_refs": ["narrative_turn_packet 안의 player-visible ref"],
        "relationship_refs": [],
        "consequence_refs": [],
        "commitment_refs": [],
        "unresolved_ask_refs": []
      }
    ],
    "commitments": [],
    "unresolved_asks": [],
    "leverage_updates": [],
    "paid_off_or_closed": [],
    "ephemeral_social_notes": []
  },
  "encounter_proposal": {
    "schema_version": "singulari.encounter_proposal.v1",
    "world_id": "<world_id>",
    "turn_id": "<turn_id>",
    "mutations": [
      {
        "surface_id": "encounter:<turn_id>:scene-specific-id",
        "label": "플레이어가 알아볼 짧은 표면명",
        "kind": "barrier|access_controller|evidence_trace|movable_object|usable_tool|container|hazard|exit|hiding_place|social_handle|environmental_feature|time_sensitive_cue",
        "status": "available|blocked|locked|hidden_but_signaled|degraded|claimed_by_actor|moving|exhausted|resolved|gone",
        "salience": "background|useful|important|critical",
        "summary": "이번 장면에서 이 표면이 무엇인지",
        "player_visible_signal": "플레이어가 이미 볼 수 있는 신호",
        "location_ref": "현재 장소/ref",
        "holder_ref": null,
        "source_refs": ["visible_scene.text_blocks[0] 또는 next_choices[slot=2]"],
        "linked_entity_refs": [],
        "linked_pressure_refs": [],
        "linked_social_refs": [],
        "affordances": [
          {
            "schema_version": "singulari.encounter_affordance.v1",
            "affordance_id": "encounter:<turn_id>:scene-specific-id:inspect",
            "action_kind": "inspect|touch|move|open|close|force|repair|break|take|use|talk_about|trade_over|threaten_with|hide_behind|follow|wait|listen|smell|compare|mark|bypass",
            "label_seed": "선택지 씨앗",
            "intent_seed": "행동 의도 씨앗",
            "availability": "available|requires_condition|risky|blocked|unknown_needs_probe",
            "required_refs": [],
            "risk_tags": [],
            "evidence_refs": ["visible_scene.text_blocks[0]"]
          }
        ],
        "constraints": [],
        "change_potential": [],
        "persistence": "current_beat|current_scene|until_changed|search_only"
      }
    ],
    "closures": []
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
      "thread_id": "narrative_turn_packet.visible_context.active_plot_threads 중 이번 턴에서 실제로 변한 thread_id",
      "change": "advanced",
      "status_after": "active",
      "urgency_after": "soon",
      "summary": "이번 visible_scene에서 이 thread가 어떻게 변했는지 한 문장",
      "evidence_refs": ["visible_scene.text_blocks[0]"]
    }
  ],
  "scene_pressure_events": [
    {
      "pressure_id": "narrative_turn_packet.visible_context.active_scene_pressure 중 이번 턴에서 실제로 변한 pressure_id",
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
- resolution_proposal은 LLM 지능이 해석한 판정 제안이고, Rust가 commit 전에 audit한다. narrative_turn_packet.pre_turn_simulation.required_resolution_fields.resolution_proposal_required가 true인 normal turn에서는 반드시 작성한다. narrative_turn_packet에 없는 ref나 evidence_refs 없는 durable effect를 넣지 않는다.
- scene_director_proposal은 선택(optional)이다. 작성할 경우 narrative_turn_packet.visible_context.active_scene_director의 current_scene/recommended_next_beats/paragraph_budget_hint에 맞춰 이번 턴의 구조적 역할을 요약한다.
- scene_director_proposal은 resolution_proposal을 대체하지 않는다. 장면 박자와 선택지 모양만 설명하며, 새 canon 사실이나 hidden motive를 만들 권한이 없다.
- scene_director_proposal.evidence_refs는 narrative_turn_packet JSON 안의 player-visible 문자열 ref만 쓴다. hidden/adjudication-only ref는 금지한다.
- consequence_proposal은 선택(optional)이다. 작성할 경우 비용, 의심, 관계 변화, 지식 변화, 기회 상실처럼 다음 턴에도 돌아와야 하는 여파만 기록한다.
- consequence_proposal은 typed projection을 대체하지 않는다. body/resource/relationship/process/lore 변화는 기존 effect/event와 evidence로 연결하고, 사소한 색채 효과는 ephemeral_effects 객체(`effect_ref`, `reason`, `evidence_refs`)로 설명한다. 문자열 배열은 금지한다.
- social_exchange_proposal은 선택(optional)이다. 작성할 경우 이번 대화에서 교환/회피/거절/약속/조건/빚/미해결 질문으로 남은 것만 기록한다.
- social_exchange_proposal은 대화 트리나 호감도 미터가 아니다. active_social_exchange를 보고 다음 대화의 현재 태도, unresolved_asks 반복 방지, 조건부 약속만 compact하게 갱신한다.
- social_exchange_proposal의 source_refs/evidence_refs는 narrative_turn_packet JSON 안의 player-visible 문자열 ref만 쓴다. hidden motive, adjudication-only truth, 미래 route hint는 summary/signal/condition/ask에 쓰지 않는다.
- encounter_proposal은 선택(optional)이다. 작성할 경우 이번 턴 visible_scene/next_choices에서 실제로 조작 가능해진 표면만 기록한다.
- encounter_proposal은 퍼즐 정답이나 물리 엔진이 아니다. active_encounter_surface를 보고 조사/이동/도구/사회적 접근/위험 우회가 다음 턴 선택지에서 무엇을 건드리는지만 compact하게 갱신한다.
- encounter_proposal의 source_refs/evidence_refs는 narrative_turn_packet JSON 안의 player-visible 문자열 ref 또는 이번 응답의 visible_scene.text_blocks/next_choices ref만 쓴다. HiddenButSignaled 표면은 보이는 신호만 적고 숨은 내용은 쓰지 않는다.
- resolution_proposal의 모든 `target_refs`, `pressure_refs`, `gate_ref`, `grounding_ref`, `process_ref`, `effect.target_ref`는 narrative_turn_packet JSON 안에 실제 문자열로 존재하는 ref만 쓴다. 설명용 JSON pointer(`visible_context...`)나 새로 만든 관계/장소/인물 ref는 쓰지 않는다.
- selected_memory_items 안의 `source_id`, `edge_id`, `entity_id`, `location_id`, `pressure_id`, `affordance_id`처럼 실제 문자열로 들어 있는 ref는 evidence로 쓸 수 있다.
- narrative_turn_packet.pre_turn_simulation.pressure_obligations의 각 pressure_id는 resolution_proposal에서 실제 gate/effect/process tick으로 움직이거나, `pressure_noop_reasons`에 같은 pressure_ref와 evidence_refs를 넣어 왜 직접 이동하지 않았는지 visible reason을 적어야 한다. 단순히 pressure_refs나 outcome.evidence_refs에 언급만 하는 것은 부족하다.
- resolution_proposal의 visible field에는 hidden/adjudication-only 세부 내용을 절대 쓰지 않는다.
- resolution_proposal.next_choice_plan은 slot 1..7을 모두 포함한다. ordinary_affordance slot 1..5는 narrative_turn_packet.visible_context.affordance_graph의 같은 slot affordance_id를 grounding_ref/evidence_refs에 넣는다. slot 6은 freeform, slot 7은 delegated_judgment다.
- actor_goal_events/actor_move_events는 중요한 NPC가 실제 장면에서 보인 목표와 움직임만 기록한다. actor_ref는 기존 relationship/entity ref 또는 이번 응답의 entity_updates로 생성한 char:id여야 한다. hidden 목표는 visible_scene/next_choices에 동기로 해설하지 말고 관찰 가능한 행동으로만 암시한다.
- slot 1,2,3,4,5의 tag/intent는 템플릿 문구가 아니라 이번 visible_scene에서 바로 이어지는 구체 선택지여야 한다.
- next_choices 안에는 label/preview/choices 필드를 쓰지 않는다. 오직 slot/tag/intent만 쓴다.
- plot_thread_events는 이번 턴에서 실제로 진행/복잡화/차단/해결/실패/퇴장한 active_visible thread만 적는다. 변화가 없으면 빈 배열이다.
- plot_thread_events.thread_id는 narrative_turn_packet.visible_context.active_plot_threads에 있는 thread_id만 쓴다. 새 thread를 임의로 만들거나 hidden/dormant 상태로 바꾸지 않는다.
- scene_pressure_events는 이번 턴에서 실제로 강해지거나 약해진 visible_active pressure만 적는다. hidden_adjudication_only pressure_id는 절대 쓰지 않는다.
- entity_updates/relationship_updates/world_lore_updates/character_text_design_updates는 전부 typed schema다. 변화가 없으면 빈 배열이며, 임의 key/value JSON을 넣지 않는다.
- body_resource_events/location_events도 typed schema다. 장면에 실제 증거가 없으면 빈 배열로 둔다.
- needs_context는 기본적으로 빈 배열이다. narrative_turn_packet에 없는 맥락 없이는 응답을 닫을 수 없을 때만 request_id/capsule_kinds/query/reason/evidence_refs를 넣는다. needs_context가 비어 있지 않으면 host는 그 응답을 커밋하지 않는다.
- slot 번호가 기능 계약이다. tag는 UI 문구이므로 장면에 맞게 짧게 바꿔도 된다. 단 slot 7 tag는 "판단 위임"으로 유지한다.
- extra_contacts는 주변 인물이 플레이어와 직접 상호작용했거나, 의미 있는 목격/거래/도움/위협/감정 흔적을 남겼을 때만 쓴다.
- extra_contacts 항목을 쓸 때는 surface_label, contact_summary를 반드시 실제 장면 내용으로 채운다. 스키마 설명 문구나 예시 문구를 값으로 복사하지 않는다.
- 단순 배경 군중은 extra_contacts에 넣지 않는다. 한 번 스쳐간 인물은 memory_action "trace", 다시 떠올릴 이유가 분명하면 "remember"를 쓴다."#;

const COMPACT_AGENT_TURN_RESPONSE_SCHEMA_GUIDE: &str = r#"AgentTurnResponse JSON만 반환한다.
필수 top-level:
- schema_version="singulari.agent_turn_response.v1", world_id, turn_id
- resolution_proposal, visible_scene, next_choices
- 선택 필드(scene_director_proposal, consequence_proposal, social_exchange_proposal, encounter_proposal, *_updates, *_events, actor_*_events)는 이번 턴에서 실제 변화가 있을 때만 쓴다. 없으면 생략하거나 []/null로 둔다.
- 선택 배열 이름: "plot_thread_events", "scene_pressure_events", "world_lore_updates", "character_text_design_updates", "body_resource_events", "location_events", "hidden_state_delta".

resolution_proposal 필수:
- schema_version="singulari.resolution_proposal.v1", world_id, turn_id
- interpreted_intent: input_kind, summary, target_refs, pressure_refs, evidence_refs, ambiguity
- outcome: kind, summary, evidence_refs
- gate_results, proposed_effects, process_ticks
- pressure_noop_reasons: narrative_turn_packet.pre_turn_simulation.pressure_obligations의 각 pressure_id를 움직이지 않으면 반드시 pressure_ref/evidence_refs로 설명
- narrative_brief: visible_summary, required_beats, forbidden_visible_details
- next_choice_plan: slot 1..5는 narrative_turn_packet.pre_turn_simulation.available_affordances의 같은 slot affordance_id를 grounding_ref/evidence_refs로 사용. slot 6은 freeform/current_turn, slot 7은 delegated_judgment/current_turn.

visible_scene:
- schema_version="singulari.narrative_scene.v1"
- text_blocks: 한국어 VN prose 문단 배열
- tone_notes: 짧게

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
    let pre_turn = full
        .get("pre_turn_simulation")
        .context("prompt context missing pre_turn_simulation")?;
    let visible = full
        .get("visible_context")
        .context("prompt context missing visible_context")?;
    let hidden_boundary = json_field(pre_turn, "hidden_visibility_boundary");
    Ok(serde_json::json!({
        "schema_version": NARRATIVE_TURN_PACKET_SCHEMA_VERSION,
        "world_id": prompt_context.world_id,
        "turn_id": prompt_context.turn_id,
        "current_turn": prompt_context.current_turn,
        "opening_randomizer": prompt_context.opening_randomizer,
        "output_contract": prompt_context.output_contract,
        "pre_turn_simulation": {
            "schema_version": json_field(pre_turn, "schema_version"),
            "world_id": prompt_context.world_id,
            "turn_id": prompt_context.turn_id,
            "player_input": json_field(pre_turn, "player_input"),
            "input_kind": json_field(pre_turn, "input_kind"),
            "selected_choice": json_field(pre_turn, "selected_choice"),
            "available_affordances": json_array_field(pre_turn, "available_affordances"),
            "pressure_obligations": json_array_field(pre_turn, "pressure_obligations"),
            "due_processes": json_array_field(pre_turn, "due_processes"),
            "required_resolution_fields": json_field(pre_turn, "required_resolution_fields"),
            "hidden_visibility_boundary": hidden_boundary.clone(),
        },
        "visible_context": {
            "recent_scene_window": json_array_field(visible, "recent_scene_window"),
            "active_scene_pressure": json_field(visible, "active_scene_pressure"),
            "active_plot_threads": json_field(visible, "active_plot_threads"),
            "active_body_resource_state": json_field(visible, "active_body_resource_state"),
            "active_location_graph": json_field(visible, "active_location_graph"),
            "affordance_graph": json_field(visible, "affordance_graph"),
            "active_scene_director": json_field(visible, "active_scene_director"),
            "narrative_style_state": json_field(visible, "narrative_style_state"),
            "active_character_text_design": json_field(visible, "active_character_text_design"),
            "selected_memory_items": json_array_field(visible, "selected_memory_items"),
        },
        "adjudication_boundary": {
            "hidden_visibility_boundary": hidden_boundary,
            "policy": "hidden/adjudication-only details may shape resolution but must not appear in player-visible fields"
        },
        "source_of_truth_policy": prompt_context.source_of_truth_policy,
    }))
}

fn json_field(source: &Value, key: &str) -> Value {
    source.get(key).cloned().unwrap_or(Value::Null)
}

fn json_array_field(source: &Value, key: &str) -> Value {
    source.get(key).cloned().unwrap_or(Value::Array(Vec::new()))
}
