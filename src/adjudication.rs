use crate::models::{
    ADJUDICATION_SCHEMA_VERSION, AdjudicationGate, AdjudicationReport, FREEFORM_CHOICE_TAG,
    HiddenState, TurnChoice, TurnInputKind, TurnSnapshot, WorldRecord,
};

const GATE_ALLOWED: &str = "allowed";
const GATE_CONSTRAINED: &str = "constrained";
const GATE_BLOCKED: &str = "blocked";
const OUTCOME_ALLOWED: &str = "allowed";
const OUTCOME_CONSTRAINED: &str = "constrained";
const OUTCOME_BLOCKED: &str = "blocked";

#[derive(Debug, Clone)]
pub struct AdjudicationInput<'a> {
    pub world: &'a WorldRecord,
    pub snapshot: &'a TurnSnapshot,
    pub hidden_state: &'a HiddenState,
    pub turn_id: &'a str,
    pub input_kind: TurnInputKind,
    pub selected_choice: Option<&'a TurnChoice>,
    pub effective_input: &'a str,
}

/// Build the deterministic V1 adjudication report for a turn.
///
/// # Errors
///
/// This function is infallible today; it returns a value directly so turn
/// reduction can keep the world-law gate visible without adding fake failure
/// handling.
#[must_use]
pub fn adjudicate_turn(input: &AdjudicationInput<'_>) -> AdjudicationReport {
    let mut gates = vec![
        body_gate(input),
        resource_gate(input),
        time_gate(input),
        social_permission_gate(input),
        knowledge_gate(input),
    ];
    if matches!(input.input_kind, TurnInputKind::GuideChoice) {
        gates.push(AdjudicationGate {
            gate: "delegated_judgment".to_owned(),
            status: GATE_CONSTRAINED.to_owned(),
            reason:
                "판단 위임은 보이는 증거 안에서 장면 압력, 존엄, 장기 흥미를 함께 보지만 세계 법칙을 우회하지 않는다"
                    .to_owned(),
        });
    }
    let outcome = adjudication_outcome(&gates);
    let visible_constraints = gates
        .iter()
        .filter(|gate| gate.status != GATE_ALLOWED)
        .map(|gate| format!("{}: {}", gate.gate, gate.reason))
        .collect::<Vec<_>>();
    let consequences = gates
        .iter()
        .filter(|gate| gate.status != GATE_ALLOWED)
        .map(|gate| format!("{}_{}", gate.gate, gate.status))
        .collect::<Vec<_>>();
    AdjudicationReport {
        schema_version: ADJUDICATION_SCHEMA_VERSION.to_owned(),
        world_id: input.world.world_id.clone(),
        turn_id: input.turn_id.to_owned(),
        outcome: outcome.to_owned(),
        summary: adjudication_summary(outcome, input),
        gates,
        visible_constraints,
        consequences,
    }
}

fn body_gate(input: &AdjudicationInput<'_>) -> AdjudicationGate {
    if !input.world.laws.bodily_needs_active {
        return allowed_gate("body", "몸 상태 판정이 이 세계 법칙에서 비활성화되어 있다");
    }
    if matches!(
        input.input_kind,
        TurnInputKind::CodexQuery | TurnInputKind::CcCanvas
    ) {
        return allowed_gate("body", "정보 조회나 렌더 전환은 몸을 크게 소모하지 않는다");
    }
    let effortful = contains_any(
        input.effective_input,
        &["달려", "전력", "싸움", "공격", "도망", "운반"],
    );
    if effortful {
        return constrained_gate("body", "큰 움직임은 허기, 피로, 부상 위험을 남긴다");
    }
    allowed_gate("body", "현재 몸 상태로는 시도 가능한 행동이다")
}

fn resource_gate(input: &AdjudicationInput<'_>) -> AdjudicationGate {
    let asks_for_missing_tool = contains_any(
        input.effective_input,
        &["검", "돈", "말", "지도", "약", "치료", "도구", "식량"],
    ) && input.snapshot.protagonist_state.inventory.is_empty();
    if asks_for_missing_tool {
        return blocked_gate("resource", "소지품에 없는 자원은 즉시 생기지 않는다");
    }
    allowed_gate("resource", "현재 기록된 자원 안에서 처리한다")
}

fn time_gate(input: &AdjudicationInput<'_>) -> AdjudicationGate {
    match input.input_kind {
        TurnInputKind::CodexQuery | TurnInputKind::CcCanvas => {
            allowed_gate("time", "조회/렌더 전환은 사건 시간을 크게 밀지 않는다")
        }
        TurnInputKind::MacroTimeFlow => constrained_gate(
            "time",
            "흐름 보기는 가능성을 보여주지만 확정 사건은 플레이어-visible 원인만큼만 전진한다",
        ),
        TurnInputKind::NumericChoice
        | TurnInputKind::GuideChoice
        | TurnInputKind::FreeformAction => {
            constrained_gate("time", "행동은 최소 한 박자의 시간을 소비한다")
        }
    }
}

fn social_permission_gate(input: &AdjudicationInput<'_>) -> AdjudicationGate {
    let social_push = contains_any(
        input.effective_input,
        &["명령", "설득", "협박", "거짓말", "유혹", "거래"],
    );
    if social_push {
        return constrained_gate(
            "social_permission",
            "타인의 마음과 권한은 자동으로 넘어오지 않는다",
        );
    }
    if input
        .selected_choice
        .is_some_and(|choice| choice.tag.contains("접촉") || choice.tag.contains("관계"))
    {
        return constrained_gate(
            "social_permission",
            "접촉 선택은 기회를 열지만 동의와 반응은 별도 판정한다",
        );
    }
    allowed_gate("social_permission", "직접적인 사회적 강제는 없다")
}

fn knowledge_gate(input: &AdjudicationInput<'_>) -> AdjudicationGate {
    if !input.world.laws.discovery_required {
        return allowed_gate("knowledge", "발견 요구 법칙이 비활성화되어 있다");
    }
    if input.hidden_state.secrets.is_empty() {
        return allowed_gate("knowledge", "현재 숨겨진 진실 장부가 비어 있다");
    }
    if matches!(input.input_kind, TurnInputKind::CodexQuery) {
        return constrained_gate("knowledge", "기록는 플레이어가 알아도 되는 기록만 연다");
    }
    let probes_hidden = contains_any(
        input.effective_input,
        &["정체", "비밀", "미래", "흑막", "앵커", "진실", "운명"],
    );
    if probes_hidden {
        return blocked_gate("knowledge", "증거 없이 숨겨진 진실을 바로 확정할 수 없다");
    }
    allowed_gate("knowledge", "보이는 증거 안에서 판정한다")
}

fn adjudication_outcome(gates: &[AdjudicationGate]) -> &'static str {
    if gates.iter().any(|gate| gate.status == GATE_BLOCKED) {
        return OUTCOME_BLOCKED;
    }
    if gates.iter().any(|gate| gate.status == GATE_CONSTRAINED) {
        return OUTCOME_CONSTRAINED;
    }
    OUTCOME_ALLOWED
}

fn adjudication_summary(outcome: &str, input: &AdjudicationInput<'_>) -> String {
    let action = match input.selected_choice {
        Some(choice)
            if input.input_kind == TurnInputKind::FreeformAction
                && choice.tag == FREEFORM_CHOICE_TAG =>
        {
            format!(
                "{}번 [{}]: {}",
                choice.slot, choice.tag, input.effective_input
            )
        }
        Some(choice) => format!("{}번 [{}]", choice.slot, choice.tag),
        None => input.effective_input.to_owned(),
    };
    match outcome {
        OUTCOME_BLOCKED => format!("{action}: 세계 법칙 때문에 그대로 성립하지 않는다"),
        OUTCOME_CONSTRAINED => format!("{action}: 성립하지만 비용, 시간, 권한, 지식 제한을 남긴다"),
        _ => format!("{action}: 현재 기록 안에서는 그대로 시도 가능하다"),
    }
}

fn allowed_gate(gate: &str, reason: &str) -> AdjudicationGate {
    AdjudicationGate {
        gate: gate.to_owned(),
        status: GATE_ALLOWED.to_owned(),
        reason: reason.to_owned(),
    }
}

fn constrained_gate(gate: &str, reason: &str) -> AdjudicationGate {
    AdjudicationGate {
        gate: gate.to_owned(),
        status: GATE_CONSTRAINED.to_owned(),
        reason: reason.to_owned(),
    }
}

fn blocked_gate(gate: &str, reason: &str) -> AdjudicationGate {
    AdjudicationGate {
        gate: gate.to_owned(),
        status: GATE_BLOCKED.to_owned(),
        reason: reason.to_owned(),
    }
}

fn contains_any(input: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| input.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{AdjudicationInput, adjudicate_turn};
    use crate::models::{
        AnchorCharacter, HiddenState, LanguagePolicy, RuntimeContract, TurnInputKind, TurnSnapshot,
        WorldLaws, WorldPremise, WorldRecord, WorldSeed,
    };

    fn test_world() -> WorldRecord {
        WorldRecord::from_seed(
            WorldSeed {
                schema_version: crate::models::WORLD_SEED_SCHEMA_VERSION.to_owned(),
                world_id: "stw_adj".to_owned(),
                title: "판정 세계".to_owned(),
                created_by: "local_user".to_owned(),
                runtime_contract: RuntimeContract::default(),
                premise: WorldPremise {
                    genre: "중세 판타지".to_owned(),
                    protagonist: "변경 순찰자".to_owned(),
                    special_condition: None,
                    opening_state: "interlude".to_owned(),
                },
                anchor_character: AnchorCharacter::default(),
                language: LanguagePolicy::default(),
                laws: WorldLaws::default(),
                non_goals: Vec::new(),
            },
            "2026-04-27T00:00:00Z".to_owned(),
        )
    }

    #[test]
    fn hidden_probe_is_blocked_by_knowledge_gate() {
        let world = test_world();
        let snapshot = TurnSnapshot::initial(&world, "session".to_owned());
        let hidden_state = HiddenState::initial(world.world_id.as_str());
        let report = adjudicate_turn(&AdjudicationInput {
            world: &world,
            snapshot: &snapshot,
            hidden_state: &hidden_state,
            turn_id: "turn_0001",
            input_kind: TurnInputKind::FreeformAction,
            selected_choice: None,
            effective_input: "흑막의 정체와 비밀을 바로 알아낸다",
        });
        assert_eq!(report.outcome, "blocked");
        assert!(
            report
                .gates
                .iter()
                .any(|gate| gate.gate == "knowledge" && gate.status == "blocked")
        );
    }
}
