use crate::models::{
    CODEX_VIEW_SCHEMA_VERSION, CharacterRecord, CharacterVoiceAnchor, CodexAnalysisEntry,
    CodexEntityEntry, CodexFactEntry, CodexHiddenFilter, CodexRecommendation, CodexTimelineEntry,
    CodexView, CodexVoiceAnchorEntry, EntityRecords, HiddenState, PROTAGONIST_CHARACTER_ID,
    PlayerKnowledge, TurnSnapshot, WorldRecord, redact_guide_choice_public_hints,
};
use crate::store::{
    HIDDEN_STATE_FILENAME, LATEST_SNAPSHOT_FILENAME, PLAYER_KNOWLEDGE_FILENAME, WORLD_FILENAME,
    read_json, resolve_store_paths, world_file_paths,
};
use crate::world_db::{
    CanonEventRow, WorldSearchHit, recent_canon_events, search_world_db, visible_entity_records,
    visible_world_facts,
};
use anyhow::Result;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::str::FromStr;

const DEFAULT_CODEX_LIMIT: usize = 12;
const DEFAULT_RECOMMENDATION_LIMIT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodexViewSection {
    All,
    Timeline,
    Almanac,
    Blueprint,
    Analysis,
    Related,
}

impl FromStr for CodexViewSection {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "all" => Ok(Self::All),
            "timeline" | "protagonist" | "history" => Ok(Self::Timeline),
            "almanac" | "facts" => Ok(Self::Almanac),
            "blueprint" | "entities" => Ok(Self::Blueprint),
            "analysis" | "live" => Ok(Self::Analysis),
            "related" | "recommendations" => Ok(Self::Related),
            other => anyhow::bail!(
                "unknown Archive View section: {other}; expected all|timeline|almanac|blueprint|analysis|related"
            ),
        }
    }
}

impl std::fmt::Display for CodexViewSection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::All => "all",
            Self::Timeline => "timeline",
            Self::Almanac => "almanac",
            Self::Blueprint => "blueprint",
            Self::Analysis => "analysis",
            Self::Related => "related",
        })
    }
}

#[derive(Debug, Clone)]
pub struct BuildCodexViewOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub query: Option<String>,
    pub limit: usize,
}

impl BuildCodexViewOptions {
    #[must_use]
    pub fn new(world_id: String) -> Self {
        Self {
            store_root: None,
            world_id,
            query: None,
            limit: DEFAULT_CODEX_LIMIT,
        }
    }
}

/// Build the player-visible Archive View from database projections.
///
/// # Errors
///
/// Returns an error when world files or database projections cannot be read.
pub fn build_codex_view(options: &BuildCodexViewOptions) -> Result<CodexView> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let world: WorldRecord = read_json(&files.dir.join(WORLD_FILENAME))?;
    let snapshot: TurnSnapshot = read_json(&files.dir.join(LATEST_SNAPSHOT_FILENAME))?;
    let hidden_state: HiddenState = read_json(&files.dir.join(HIDDEN_STATE_FILENAME))?;
    let player_knowledge: PlayerKnowledge = read_json(&files.dir.join(PLAYER_KNOWLEDGE_FILENAME))?;
    let entity_records: EntityRecords = read_json(&files.entities)?;
    let limit = options.limit.max(1);
    let events = recent_canon_events(&files.dir, world.world_id.as_str(), limit)?
        .into_iter()
        .filter(|event| event.visibility == "player_visible")
        .map(codex_timeline_entry)
        .collect();
    let facts = visible_world_facts(&files.dir, world.world_id.as_str(), limit)?
        .into_iter()
        .map(|fact| CodexFactEntry {
            fact_id: fact.fact_id,
            category: fact.category,
            subject: fact.subject,
            predicate: fact.predicate,
            object: fact.object,
        })
        .collect();
    let entities = visible_entity_records(&files.dir, world.world_id.as_str(), limit)?
        .into_iter()
        .map(|entity| CodexEntityEntry {
            entity_id: entity.entity_id,
            entity_type: entity.entity_type,
            name: entity.name,
            status: entity.status,
        })
        .collect();
    let search_hits = match &options.query {
        Some(query) => search_world_db(
            &files.dir,
            world.world_id.as_str(),
            query.as_str(),
            DEFAULT_RECOMMENDATION_LIMIT,
        )?,
        None => Vec::new(),
    };
    Ok(CodexView {
        schema_version: CODEX_VIEW_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id.clone(),
        turn_id: snapshot.turn_id.clone(),
        title: world.title,
        protagonist_timeline: events,
        world_almanac: facts,
        world_blueprint: entities,
        voice_anchors: visible_voice_anchors(&entity_records, limit),
        realtime_analysis: realtime_analysis(&snapshot, &player_knowledge, search_hits.len()),
        related_recommendations: recommendations(&player_knowledge, &search_hits),
        hidden_filter: CodexHiddenFilter {
            hidden_secrets: hidden_state.secrets.len(),
            hidden_timers: hidden_state.timers.len(),
            policy: "hidden truth is counted but never rendered in Archive View".to_owned(),
        },
    })
}

#[must_use]
pub fn render_codex_view_markdown(view: &CodexView) -> String {
    let mut markdown = String::new();
    writeln!(markdown, "# World Archive").ok();
    writeln!(markdown).ok();
    writeln!(markdown, "- world: `{}`", view.world_id).ok();
    writeln!(markdown, "- turn: `{}`", view.turn_id).ok();
    writeln!(
        markdown,
        "- hidden filter: {} secrets / {} timers",
        view.hidden_filter.hidden_secrets, view.hidden_filter.hidden_timers
    )
    .ok();
    write_codex_section(&mut markdown, view, CodexViewSection::Timeline);
    write_codex_section(&mut markdown, view, CodexViewSection::Almanac);
    write_codex_section(&mut markdown, view, CodexViewSection::Blueprint);
    write_codex_section(&mut markdown, view, CodexViewSection::Analysis);
    write_codex_section(&mut markdown, view, CodexViewSection::Related);
    markdown
}

#[must_use]
pub fn render_codex_view_section_markdown(view: &CodexView, section: CodexViewSection) -> String {
    if section == CodexViewSection::All {
        return render_codex_view_markdown(view);
    }
    let mut markdown = String::new();
    writeln!(markdown, "# World Archive: {section}").ok();
    writeln!(markdown).ok();
    writeln!(markdown, "- world: `{}`", view.world_id).ok();
    writeln!(markdown, "- turn: `{}`", view.turn_id).ok();
    writeln!(
        markdown,
        "- hidden filter: {} secrets / {} timers",
        view.hidden_filter.hidden_secrets, view.hidden_filter.hidden_timers
    )
    .ok();
    write_codex_section(&mut markdown, view, section);
    markdown
}

fn write_codex_section(markdown: &mut String, view: &CodexView, section: CodexViewSection) {
    match section {
        CodexViewSection::All => {
            write_codex_section(markdown, view, CodexViewSection::Timeline);
            write_codex_section(markdown, view, CodexViewSection::Almanac);
            write_codex_section(markdown, view, CodexViewSection::Blueprint);
            write_codex_section(markdown, view, CodexViewSection::Analysis);
            write_codex_section(markdown, view, CodexViewSection::Related);
        }
        CodexViewSection::Timeline => write_timeline(markdown, view),
        CodexViewSection::Almanac => write_almanac(markdown, view),
        CodexViewSection::Blueprint => write_blueprint(markdown, view),
        CodexViewSection::Analysis => write_analysis(markdown, view),
        CodexViewSection::Related => write_related(markdown, view),
    }
}

fn write_timeline(markdown: &mut String, view: &CodexView) {
    writeln!(markdown).ok();
    writeln!(markdown, "## 주인공의 연대기").ok();
    if view.protagonist_timeline.is_empty() {
        writeln!(markdown, "- 아직 플레이어에게 공개된 사건 기록이 없다").ok();
    }
    for event in &view.protagonist_timeline {
        writeln!(
            markdown,
            "- `{}` `{}` [{}] {}",
            event.turn_id, event.event_id, event.kind, event.summary
        )
        .ok();
    }
}

fn write_almanac(markdown: &mut String, view: &CodexView) {
    writeln!(markdown).ok();
    writeln!(markdown, "## 세계 연감").ok();
    if view.world_almanac.is_empty() {
        writeln!(markdown, "- 아직 공개된 세계 사실이 적다").ok();
    }
    for fact in &view.world_almanac {
        writeln!(
            markdown,
            "- `{}` {}.{} = {}",
            fact.fact_id, fact.subject, fact.predicate, fact.object
        )
        .ok();
    }
}

fn write_blueprint(markdown: &mut String, view: &CodexView) {
    writeln!(markdown).ok();
    writeln!(markdown, "## 세계 청사진").ok();
    if view.world_blueprint.is_empty() {
        writeln!(markdown, "- 아직 주인공이 확실히 아는 항목이 적다").ok();
    }
    for entity in &view.world_blueprint {
        writeln!(
            markdown,
            "- `{}` [{}] {} — {}",
            entity.entity_id, entity.entity_type, entity.name, entity.status
        )
        .ok();
    }
    writeln!(markdown).ok();
    writeln!(markdown, "### 캐릭터 음성 앵커").ok();
    if view.voice_anchors.is_empty() {
        writeln!(markdown, "- 아직 플레이어에게 공개된 화법 앵커가 없다").ok();
    }
    for anchor in &view.voice_anchors {
        writeln!(markdown, "- `{}` {}", anchor.character_id, anchor.name).ok();
        write_anchor_values(markdown, "말투", &anchor.speech);
        write_anchor_values(markdown, "제스처", &anchor.gestures);
        write_anchor_values(markdown, "버릇", &anchor.habits);
        write_anchor_values(markdown, "변화", &anchor.drift);
    }
}

fn write_analysis(markdown: &mut String, view: &CodexView) {
    writeln!(markdown).ok();
    writeln!(markdown, "## 실시간 분석").ok();
    for line in &view.realtime_analysis {
        writeln!(markdown, "- {}: {}", line.label, line.value).ok();
    }
}

fn write_related(markdown: &mut String, view: &CodexView) {
    writeln!(markdown).ok();
    writeln!(markdown, "## 관련 항목 추천").ok();
    if view.related_recommendations.is_empty() {
        writeln!(markdown, "- 현재 추천할 검색 항목이 없다").ok();
    }
    for recommendation in &view.related_recommendations {
        writeln!(
            markdown,
            "- [{}] `{}` — {}",
            recommendation.source, recommendation.target, recommendation.reason
        )
        .ok();
    }
}

fn codex_timeline_entry(event: CanonEventRow) -> CodexTimelineEntry {
    CodexTimelineEntry {
        turn_id: event.turn_id,
        event_id: event.event_id,
        kind: event.kind,
        summary: redact_guide_choice_public_hints(&event.summary),
    }
}

fn visible_voice_anchors(entities: &EntityRecords, limit: usize) -> Vec<CodexVoiceAnchorEntry> {
    entities
        .characters
        .iter()
        .filter(|character| character_voice_anchor_is_visible(character))
        .filter_map(codex_voice_anchor_entry)
        .take(limit)
        .collect()
}

fn character_voice_anchor_is_visible(character: &CharacterRecord) -> bool {
    matches!(
        character.knowledge_state.as_str(),
        "self" | "known" | "player_visible" | "protagonist_known"
    )
}

fn codex_voice_anchor_entry(character: &CharacterRecord) -> Option<CodexVoiceAnchorEntry> {
    let anchor = effective_voice_anchor(character)?;
    Some(CodexVoiceAnchorEntry {
        character_id: character.id.clone(),
        name: character.name.visible.clone(),
        speech: anchor.speech,
        gestures: anchor.gestures,
        habits: anchor.habits,
        drift: anchor.drift,
    })
}

fn effective_voice_anchor(character: &CharacterRecord) -> Option<CharacterVoiceAnchor> {
    if !character.voice_anchor.is_empty() {
        return Some(character.voice_anchor.clone());
    }
    if character.id == PROTAGONIST_CHARACTER_ID {
        return Some(CharacterVoiceAnchor::protagonist_default());
    }
    None
}

fn write_anchor_values(markdown: &mut String, label: &str, values: &[String]) {
    if !values.is_empty() {
        writeln!(markdown, "  - {label}: {}", values.join(" / ")).ok();
    }
}

fn realtime_analysis(
    snapshot: &TurnSnapshot,
    player_knowledge: &PlayerKnowledge,
    search_hit_count: usize,
) -> Vec<CodexAnalysisEntry> {
    vec![
        CodexAnalysisEntry {
            label: "phase".to_owned(),
            value: snapshot.phase.clone(),
        },
        CodexAnalysisEntry {
            label: "location".to_owned(),
            value: snapshot.protagonist_state.location.clone(),
        },
        CodexAnalysisEntry {
            label: "open_questions".to_owned(),
            value: (snapshot.open_questions.len() + player_knowledge.open_questions.len())
                .to_string(),
        },
        CodexAnalysisEntry {
            label: "known_entities".to_owned(),
            value: player_knowledge.known_entities.len().to_string(),
        },
        CodexAnalysisEntry {
            label: "related_search_hits".to_owned(),
            value: search_hit_count.to_string(),
        },
    ]
}

fn recommendations(
    player_knowledge: &PlayerKnowledge,
    search_hits: &[WorldSearchHit],
) -> Vec<CodexRecommendation> {
    let mut recommendations = search_hits
        .iter()
        .map(|hit| CodexRecommendation {
            source: hit.source_table.clone(),
            target: hit.source_id.clone(),
            reason: redact_guide_choice_public_hints(&format!("{}: {}", hit.title, hit.snippet)),
        })
        .collect::<Vec<_>>();
    if recommendations.is_empty() {
        for question in player_knowledge
            .open_questions
            .iter()
            .take(DEFAULT_RECOMMENDATION_LIMIT)
        {
            recommendations.push(CodexRecommendation {
                source: "open_question".to_owned(),
                target: question.clone(),
                reason: "아직 닫히지 않은 플레이어-visible 질문".to_owned(),
            });
        }
    }
    recommendations
}

#[cfg(test)]
mod tests {
    use super::{BuildCodexViewOptions, build_codex_view, render_codex_view_markdown};
    use crate::store::{InitWorldOptions, init_world};
    use crate::turn::{AdvanceTurnOptions, advance_turn};
    use tempfile::tempdir;

    #[test]
    fn codex_view_filters_hidden_truth_and_lists_visible_records() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let seed_path = temp.path().join("seed.yaml");
        let store = temp.path().join("store");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_codex_view
title: "기록 세계"
premise:
  genre: "중세 판타지"
  protagonist: "현대인의 전생, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_codex_view".to_owned(),
            input: "4".to_owned(),
        })?;
        let mut options = BuildCodexViewOptions::new("stw_codex_view".to_owned());
        options.store_root = Some(store);
        options.query = Some("안내자".to_owned());
        let view = build_codex_view(&options)?;
        let rendered = render_codex_view_markdown(&view);
        assert!(rendered.contains("주인공의 연대기"));
        assert!(rendered.contains("세계 연감"));
        assert!(rendered.contains("캐릭터 음성 앵커"));
        assert!(!rendered.contains("manifestation rules are world-local"));
        Ok(())
    }
}
