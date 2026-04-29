#![allow(clippy::missing_errors_doc)]

use crate::agent_bridge::{AgentOutputContract, AgentTurnResponse};
use crate::store::{append_jsonl, read_json, write_json};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const NARRATIVE_STYLE_STATE_SCHEMA_VERSION: &str = "singulari.narrative_style_state.v1";
pub const NARRATIVE_STYLE_EVENT_SCHEMA_VERSION: &str = "singulari.narrative_style_event.v1";
pub const NARRATIVE_STYLE_STATE_FILENAME: &str = "narrative_style_state.json";
pub const NARRATIVE_STYLE_EVENTS_FILENAME: &str = "narrative_style_events.jsonl";

const ACTIVE_STYLE_EVENT_BUDGET: usize = 8;

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
    #[serde(default)]
    pub active_style_events: Vec<NarrativeStyleEventRecord>,
    pub compiler_policy: NarrativeStylePolicy,
}

impl Default for NarrativeStyleState {
    fn default() -> Self {
        Self {
            schema_version: NARRATIVE_STYLE_STATE_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            turn_id: String::new(),
            narrative_level: 1,
            density_contract: String::new(),
            paragraph_grammar: Vec::new(),
            dialogue_contract: String::new(),
            style_vector: StyleVector::default(),
            anti_translation_rules: Vec::new(),
            prohibited_seed_leakage: Vec::new(),
            active_style_events: Vec::new(),
            compiler_policy: NarrativeStylePolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
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
    pub active_style_event_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for NarrativeStylePolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_output_contract_and_seedless_style_contract_v1".to_owned(),
            active_style_event_budget: ACTIVE_STYLE_EVENT_BUDGET,
            use_rules: vec![
                "Narrative style controls rhythm, density, paragraph shape, and Korean prose quality only.".to_owned(),
                "Style state must not create facts, genre devices, character backstory, symbols, or plot events.".to_owned(),
                "Character speech design remains separate from global narration style.".to_owned(),
                "Style events must come from accepted prose evidence or explicit user feedback."
                    .to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NarrativeStyleEventPlan {
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub records: Vec<NarrativeStyleEventRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NarrativeStyleEventRecord {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub evidence_ref: String,
    pub observed_quality: String,
    pub correction_pressure: String,
    #[serde(default)]
    pub prohibited_content_sources: Vec<String>,
    pub recorded_at: String,
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
        active_style_events: Vec::new(),
        compiler_policy: NarrativeStylePolicy::default(),
    }
}

#[must_use]
pub fn prepare_narrative_style_event_plan(
    world_id: &str,
    turn_id: &str,
    response: &AgentTurnResponse,
) -> NarrativeStyleEventPlan {
    let mut records = Vec::new();
    let recorded_at = Utc::now().to_rfc3339();
    for (index, note) in response.visible_scene.tone_notes.iter().enumerate() {
        let trimmed = note.trim();
        if trimmed.is_empty() {
            continue;
        }
        records.push(style_event_record(
            world_id,
            turn_id,
            records.len(),
            format!("visible_scene.tone_notes[{index}]"),
            trimmed.to_owned(),
            "다음 턴의 문단 박자와 한국어 질감에만 반영한다.".to_owned(),
            recorded_at.as_str(),
        ));
    }
    if let Some(correction) = prose_correction_pressure(&response.visible_scene.text_blocks) {
        records.push(style_event_record(
            world_id,
            turn_id,
            records.len(),
            "visible_scene.text_blocks".to_owned(),
            "accepted_prose_surface".to_owned(),
            correction,
            recorded_at.as_str(),
        ));
    }
    NarrativeStyleEventPlan {
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        records,
    }
}

pub fn append_narrative_style_event_plan(
    world_dir: &Path,
    plan: &NarrativeStyleEventPlan,
) -> Result<()> {
    for record in &plan.records {
        append_jsonl(&world_dir.join(NARRATIVE_STYLE_EVENTS_FILENAME), record)?;
    }
    Ok(())
}

pub fn rebuild_narrative_style_state(
    world_dir: &Path,
    base_state: &NarrativeStyleState,
) -> Result<NarrativeStyleState> {
    let mut active_style_events = load_narrative_style_event_records(world_dir)?;
    active_style_events.reverse();
    active_style_events.truncate(ACTIVE_STYLE_EVENT_BUDGET);
    active_style_events.reverse();
    let state = NarrativeStyleState {
        active_style_events,
        ..base_state.clone()
    };
    write_json(&world_dir.join(NARRATIVE_STYLE_STATE_FILENAME), &state)?;
    Ok(state)
}

pub fn load_narrative_style_state(
    world_dir: &Path,
    base_state: NarrativeStyleState,
) -> Result<NarrativeStyleState> {
    let path = world_dir.join(NARRATIVE_STYLE_STATE_FILENAME);
    if path.is_file() {
        return read_json(&path);
    }
    Ok(base_state)
}

fn load_narrative_style_event_records(world_dir: &Path) -> Result<Vec<NarrativeStyleEventRecord>> {
    let path = world_dir.join(NARRATIVE_STYLE_EVENTS_FILENAME);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)?;
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<NarrativeStyleEventRecord>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Into::into)
}

fn style_event_record(
    world_id: &str,
    turn_id: &str,
    index: usize,
    evidence_ref: String,
    observed_quality: String,
    correction_pressure: String,
    recorded_at: &str,
) -> NarrativeStyleEventRecord {
    NarrativeStyleEventRecord {
        schema_version: NARRATIVE_STYLE_EVENT_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        event_id: format!("narrative_style_event:{turn_id}:{index:02}"),
        evidence_ref,
        observed_quality,
        correction_pressure,
        prohibited_content_sources: vec![
            "no new people".to_owned(),
            "no new places".to_owned(),
            "no new objects".to_owned(),
            "no new symbols".to_owned(),
            "no genre-device borrowing".to_owned(),
        ],
        recorded_at: recorded_at.to_owned(),
    }
}

fn prose_correction_pressure(text_blocks: &[String]) -> Option<String> {
    let joined = text_blocks.join("\n");
    let translationese_hits = ["해당", "진행", "수행", "위치", "존재"]
        .into_iter()
        .filter(|needle| joined.contains(needle))
        .count();
    let long_sentence_count = joined
        .split(['.', '!', '?', '다', '요'])
        .filter(|sentence| sentence.chars().count() > 90)
        .count();
    if translationese_hits == 0 && long_sentence_count == 0 {
        return None;
    }
    Some(format!(
        "번역체 히트 {translationese_hits}개, 긴 문장 {long_sentence_count}개. 다음 턴은 감각-반응-판단 단위로 쪼개고 보고서체 어휘를 줄인다."
    ))
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

    #[test]
    fn materializes_style_events_without_content_seed() -> anyhow::Result<()> {
        use crate::agent_bridge::AGENT_TURN_RESPONSE_SCHEMA_VERSION;
        use crate::models::{NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene};

        let temp = tempfile::tempdir()?;
        let response = AgentTurnResponse {
            schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
            world_id: "stw_style_events".to_owned(),
            turn_id: "turn_0002".to_owned(),
            visible_scene: NarrativeScene {
                schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
                speaker: None,
                text_blocks: vec![
                    "해당 위치에 존재하는 긴 문장이 계속 이어지며 진행된다".to_owned(),
                ],
                tone_notes: vec!["대사를 짧게 끊었다".to_owned()],
            },
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
            next_choices: Vec::new(),
        };
        let plan = prepare_narrative_style_event_plan("stw_style_events", "turn_0002", &response);
        append_narrative_style_event_plan(temp.path(), &plan)?;
        let state = rebuild_narrative_style_state(
            temp.path(),
            &NarrativeStyleState {
                world_id: "stw_style_events".to_owned(),
                turn_id: "turn_0002".to_owned(),
                ..NarrativeStyleState::default()
            },
        )?;

        assert!(!state.active_style_events.is_empty());
        assert!(
            state.active_style_events[0]
                .prohibited_content_sources
                .contains(&"no new places".to_owned())
        );
        Ok(())
    }
}
