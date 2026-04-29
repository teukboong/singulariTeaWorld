use crate::agent_bridge::AgentOutputContract;
use serde::{Deserialize, Serialize};

pub const NARRATIVE_STYLE_STATE_SCHEMA_VERSION: &str = "singulari.narrative_style_state.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NarrativeStyleState {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub narrative_level: u8,
    pub density_contract: String,
    #[serde(default)]
    pub paragraph_grammar: Vec<String>,
    pub dialogue_contract: String,
    pub style_vector: StyleVector,
    #[serde(default)]
    pub anti_translation_rules: Vec<String>,
    #[serde(default)]
    pub prohibited_seed_leakage: Vec<String>,
    pub compiler_policy: NarrativeStylePolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StyleVector {
    pub sentence_pressure: String,
    pub exposition_directness: String,
    pub sensory_density: String,
    pub dialogue_explanation: String,
    pub paragraph_closure: String,
    pub metaphor_density: String,
    pub interior_monologue: String,
    pub scene_continuity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NarrativeStylePolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for NarrativeStylePolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_output_contract_and_seedless_style_contract_v1".to_owned(),
            use_rules: vec![
                "Narrative style controls rhythm, density, paragraph shape, and Korean prose quality only.".to_owned(),
                "Style state must not create facts, genre devices, character backstory, symbols, or plot events.".to_owned(),
                "Character speech design remains separate from global narration style.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn compile_narrative_style_state(
    world_id: &str,
    turn_id: &str,
    output_contract: &AgentOutputContract,
) -> NarrativeStyleState {
    NarrativeStyleState {
        schema_version: NARRATIVE_STYLE_STATE_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        narrative_level: output_contract.narrative_level,
        density_contract: density_contract(output_contract),
        paragraph_grammar: vec![
            "각 문단은 감각 변화, 몸의 반응, 외부 압력, 해석을 유보한 단서, 다음 행동 압박 중 최소 둘을 포함한다.".to_owned(),
            "시작 문단은 배경 설명보다 현재 장면에서 바뀐 감각과 visible constraint를 먼저 연다.".to_owned(),
            "마감 문단은 요약이나 교훈이 아니라 다음 행동을 압박하는 미해결 상태로 닫는다.".to_owned(),
        ],
        dialogue_contract: "대사는 설명문이 아니다. 짧은 호흡, 망설임, 생략, 거리감, 말끝으로 인물을 구분한다.".to_owned(),
        style_vector: StyleVector {
            sentence_pressure: "high".to_owned(),
            exposition_directness: "low".to_owned(),
            sensory_density: "medium_high".to_owned(),
            dialogue_explanation: "low".to_owned(),
            paragraph_closure: "unresolved".to_owned(),
            metaphor_density: "low_medium".to_owned(),
            interior_monologue: "restrained".to_owned(),
            scene_continuity: "strict".to_owned(),
        },
        anti_translation_rules: vec![
            "한국어 문체는 자연스러운 구어 기반 서사로 쓴다.".to_owned(),
            "긴 인과문은 감각, 반응, 판단으로 쪼갠다.".to_owned(),
            "`해당`, `진행`, `확인`, `수행`, `위치하다`, `존재하다` 같은 보고서체 표현을 남발하지 않는다.".to_owned(),
        ],
        prohibited_seed_leakage: vec![
            "문체 규칙은 소재, 사건, 인물, 장소, 과거사, 상징을 만들 권한이 없다.".to_owned(),
            "스타일 참조를 content source로 해석하지 않는다.".to_owned(),
            "유명 작품명, 작가명, 예시 문장, 장르 관습에서 소재를 빌려오지 않는다.".to_owned(),
        ],
        compiler_policy: NarrativeStylePolicy::default(),
    }
}

fn density_contract(output_contract: &AgentOutputContract) -> String {
    let budget = &output_contract.narrative_budget;
    format!(
        "{}; standard={} blocks/~{} chars; major={} blocks/~{} chars",
        budget.level_label,
        budget.standard_choice_turn_blocks,
        budget.target_chars,
        budget.major_turn_blocks,
        budget.major_target_chars
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_bridge::{AgentNarrativeBudget, AgentOutputContract};

    #[test]
    fn compiles_style_as_seedless_contract_not_content_source() {
        let state = compile_narrative_style_state(
            "stw_style",
            "turn_0006",
            &AgentOutputContract {
                language: "ko".to_owned(),
                must_return_json: true,
                hidden_truth_must_not_appear_in_visible_text: true,
                narrative_level: 3,
                narrative_budget: AgentNarrativeBudget {
                    level_label: "레벨 3".to_owned(),
                    ordinary_turn_blocks: 8,
                    standard_choice_turn_blocks: 14,
                    major_turn_blocks: 20,
                    opening_or_climax_blocks: 24,
                    target_chars: 2400,
                    major_target_chars: 3600,
                    ordinary_turn: "8 blocks".to_owned(),
                    standard_choice_turn: "14 blocks".to_owned(),
                    major_turn: "20 blocks".to_owned(),
                    opening_or_climax: "24 blocks".to_owned(),
                    character_budget: "full".to_owned(),
                },
            },
        );

        assert_eq!(state.narrative_level, 3);
        assert!(
            state
                .compiler_policy
                .use_rules
                .iter()
                .any(|rule| rule.contains("must not create facts"))
        );
        assert!(
            state
                .prohibited_seed_leakage
                .iter()
                .any(|rule| rule.contains("content source"))
        );
    }
}
