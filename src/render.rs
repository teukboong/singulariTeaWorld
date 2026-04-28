use crate::codex_view::render_codex_view_markdown;
use crate::models::{
    AdjudicationReport, DashboardSummary, FREEFORM_CHOICE_TAG, RenderPacket, TurnChoice,
    is_guide_choice_tag, normalize_turn_choices,
};
use crate::store::{read_json, resolve_store_paths, world_file_paths};
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RenderPacketLoadOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub turn_id: Option<String>,
}

/// Load a stored render packet for a world.
///
/// # Errors
///
/// Returns an error when the world snapshot or render packet cannot be read.
pub fn load_render_packet(options: &RenderPacketLoadOptions) -> Result<RenderPacket> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let latest_snapshot: crate::models::TurnSnapshot = read_json(&files.latest_snapshot)?;
    let turn_id = options
        .turn_id
        .clone()
        .unwrap_or_else(|| latest_snapshot.turn_id.clone());
    let path = files
        .dir
        .join("sessions")
        .join(&latest_snapshot.session_id)
        .join("render_packets")
        .join(format!("{turn_id}.json"));
    read_json(&path).with_context(|| {
        format!(
            "render packet not found for world_id={}, session_id={}, turn_id={turn_id}",
            options.world_id, latest_snapshot.session_id
        )
    })
}

#[must_use]
pub fn render_packet_markdown(packet: &RenderPacket) -> String {
    match packet.mode.as_str() {
        "codex" => render_codex(packet),
        "macro_time_flow" => render_macro_time_flow(packet),
        "cc" => render_canvas_note(packet),
        _ => render_normal_turn(packet),
    }
}

fn render_normal_turn(packet: &RenderPacket) -> String {
    let mut sections = Vec::new();
    sections.push(format!("## {}", packet.turn_id));
    sections.push(normal_story(packet));
    if let Some(adjudication) = &packet.adjudication {
        sections.push(render_adjudication(adjudication));
    }
    sections.push(render_dashboard(&packet.visible_state.dashboard));
    sections.push(render_scan(packet));
    sections.push(render_choices(&packet.visible_state.choices));
    sections.join("\n\n")
}

fn render_codex(packet: &RenderPacket) -> String {
    if let Some(view) = &packet.codex_view {
        let mut sections = vec![render_codex_view_markdown(view)];
        sections.push(render_dashboard(&packet.visible_state.dashboard));
        sections.push(render_choices(&packet.visible_state.choices));
        return sections.join("\n\n");
    }
    let mut sections = Vec::new();
    sections.push("# World Archive".to_owned());
    sections.push(format!(
        "세계 기록이 열린다. 지금 보이는 것은 `{}` 이후 플레이어가 알아도 되는 표면뿐이다.",
        packet.turn_id
    ));
    sections.push(
        [
            "- 주인공의 연대기",
            "- 세계 연감",
            "- 세계 청사진",
            "- 실시간 분석",
            "- 관련 항목 추천",
            "- 닫고 재개하기",
        ]
        .join("\n"),
    );
    sections.push(render_dashboard(&packet.visible_state.dashboard));
    sections.push(render_choices(&packet.visible_state.choices));
    sections.join("\n\n")
}

fn render_adjudication(adjudication: &AdjudicationReport) -> String {
    let mut lines = vec![
        "### 판정".to_owned(),
        format!("- outcome: `{}`", adjudication.outcome),
        format!("- summary: {}", adjudication.summary),
    ];
    for gate in &adjudication.gates {
        lines.push(format!(
            "- {}: `{}` — {}",
            gate.gate, gate.status, gate.reason
        ));
    }
    lines.join("\n")
}

fn render_macro_time_flow(packet: &RenderPacket) -> String {
    let mut sections = Vec::new();
    sections.push(format!("## {} — 시간의 흐름", packet.turn_id));
    sections.push(
        "시점이 한 걸음 물러난다. 세계는 아직 확정되지 않은 갈피들을 천천히 펼쳐 보인다."
            .to_owned(),
    );
    sections.push(
        [
            "### 다가오는 운명의 갈피들",
            "1. 아직 이름 붙지 않은 첫 사건의 문턱",
            "2. 몸, 자원, 시간 중 하나가 비용으로 떠오를 가능성",
            "3. 가까운 장소가 구체적인 위험이나 기회로 변하는 순간",
            "4. 아직 정해지지 않은 인물, 장소, 물건, 세력 중 하나가 극점이 되는 순간",
            "5. 다음 사건 압력이 선택지로 좁혀지는 순간",
        ]
        .join("\n"),
    );
    sections.push(render_dashboard(&packet.visible_state.dashboard));
    sections.push(render_choices(&packet.visible_state.choices));
    sections.join("\n\n")
}

fn render_canvas_note(packet: &RenderPacket) -> String {
    [
        format!("## {} — `.cc` 렌더 준비", packet.turn_id),
        "이 턴은 캔버스 변환 요청으로 기록됐다. V1 CLI는 HTML 캔버스를 직접 만들지 않고, 장면 렌더 패킷과 금지 노출 목록을 보존한다.".to_owned(),
        render_dashboard(&packet.visible_state.dashboard),
        render_choices(&packet.visible_state.choices),
    ]
    .join("\n\n")
}

fn normal_story(packet: &RenderPacket) -> String {
    if let Some(scene) = packet
        .narrative_scene
        .as_ref()
        .filter(|scene| !scene.text_blocks.is_empty())
    {
        return scene.text_blocks.join("\n\n");
    }
    let dashboard = &packet.visible_state.dashboard;
    let paragraphs = [
        format!(
            "세계는 `{}`의 표면에서 천천히 숨을 고른다. 아직 문장은 완전한 장편 서사로 부풀지 않았지만, 방금 일어난 변화는 세계의 장부에 남았다.",
            dashboard.location
        ),
        format!(
            "확정된 변화는 이것이다. {} 지금 화면에는 플레이어가 확인한 사실만 남긴다.",
            punctuated(dashboard.status.as_str())
        ),
    ];
    paragraphs.join("\n\n")
}

fn render_dashboard(dashboard: &DashboardSummary) -> String {
    [
        "### 상태",
        &format!(
            "* [🫀 건강: 큰 이상은 아직 드러나지 않음] | [🧠 정신: {}]",
            dashboard.status
        ),
        "* [🍞 허기: 아직 견딜 만함] | [💧 갈증: 아직 견딜 만함] | [💤 피로: 낮게 깔림]",
        "* [🌡️ 체온: 정상에 가까움] | [🌬️ 기온: 아직 미정]",
        "* [☁️ 날씨: 아직 미정] | [🌙 달의 위상: 아직 미정]",
        &format!(
            "* [📍 위치: {}] | [🧭 진행 중 사건: {}]",
            dashboard.location, dashboard.current_event
        ),
        &format!(
            "* [🧵 진행도: {}] | [⚑ 상태: {}]",
            dashboard.phase, dashboard.status
        ),
        "* [✨ 희망: 아직 꺼지지 않음] | [☠️ 치명 상태: 없음]",
        "* [🎒 소지품: 아직 구체화 전]",
        &format!(
            "* [🪪 이름: 미정] | [🌱 나이: 미정] | [🔢 턴: {}]",
            dashboard.phase
        ),
        "* [⏰ 시간: 전조] | [👂 감각: 주변을 더 살펴야 함] | [🍃 바람: 아직 미정] | [⌛ 경과: 한 박자]",
    ]
    .join("\n")
}

fn render_scan(packet: &RenderPacket) -> String {
    let mut lines = vec![
        "### 감각 스캔".to_owned(),
        "| 대상 (Target) | 분류 (Class) | 거리/방향 (Distance) | 주관적 인식 (Thought) |"
            .to_owned(),
        "| --- | --- | --- | --- |".to_owned(),
        "| ⚖️ 관찰 범위 | visible_scope | 🧭 현재 | 지금 확인된 표면 단서만 다룬다 |".to_owned(),
    ];
    for target in &packet.visible_state.scan_targets {
        let target = player_visible_scan_target(target);
        lines.push(format!(
            "| 🔎 {} | {} | 📍 {} | {} |",
            target.target, target.class, target.distance, target.thought
        ));
    }
    while lines.len() < 8 {
        lines.push(
            "| 🌫️ 미확정 단서 | unknown | 🧭 주변 | 아직 이름 붙일 증거가 부족하다 |".to_owned(),
        );
    }
    lines.join("\n")
}

fn player_visible_scan_target(target: &crate::models::ScanTarget) -> crate::models::ScanTarget {
    if leaks_internal_anchor_or_hidden_text([
        target.target.as_str(),
        target.class.as_str(),
        target.distance.as_str(),
        target.thought.as_str(),
    ]) {
        return crate::models::ScanTarget {
            target: "미확정 단서".to_owned(),
            class: "unknown".to_owned(),
            distance: "주변".to_owned(),
            thought: "아직 이름 붙일 증거가 부족하다".to_owned(),
        };
    }
    target.clone()
}

fn leaks_internal_anchor_or_hidden_text<'a>(parts: impl IntoIterator<Item = &'a str>) -> bool {
    parts.into_iter().any(|part| {
        [
            "숨겨진 진실",
            "숨겨져",
            "hidden",
            "secret",
            "anchor_character",
            "앵커 인물",
            "시드가 정한",
            "정체와 역할",
            "seed-defined",
        ]
        .iter()
        .any(|needle| part.contains(needle))
    })
}

fn render_choices(choices: &[TurnChoice]) -> String {
    let mut lines = vec!["### 선택".to_owned()];
    let choices = normalize_turn_choices(choices);
    for choice in &choices {
        lines.push(format!(
            "### {}. {} {}",
            choice.slot,
            choice_icon(choice.tag.as_str()),
            choice.tag
        ));
        lines.push(format!(">> {}", choice.player_visible_intent()));
        lines.push(String::new());
    }
    lines.join("\n")
}

fn choice_icon(tag: &str) -> &'static str {
    match tag {
        "정로" => "🛤️",
        "관찰" => "🔎",
        "관계" => "🕯️",
        tag if is_guide_choice_tag(tag) => "✦",
        "기록" => "📖",
        "흐름" => "⏳",
        tag if tag == FREEFORM_CHOICE_TAG => "✍",
        _ => "•",
    }
}

fn punctuated(text: &str) -> String {
    if text.ends_with(['.', '!', '?']) {
        return text.to_owned();
    }
    format!("{text}.")
}

#[cfg(test)]
mod tests {
    use super::render_packet_markdown;
    use crate::store::{InitWorldOptions, init_world};
    use crate::turn::{AdvanceTurnOptions, advance_turn};
    use tempfile::tempdir;

    #[test]
    fn renders_normal_turn_with_dashboard_scan_and_guide_choice() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_render
title: "렌더 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let turn = advance_turn(&AdvanceTurnOptions {
            store_root: Some(store),
            world_id: "stw_render".to_owned(),
            input: "7".to_owned(),
        })?;
        let rendered = render_packet_markdown(&turn.render_packet);
        assert!(rendered.contains("### 상태"));
        assert!(rendered.contains("대상 (Target)"));
        assert!(rendered.contains("판단 위임"));
        assert!(rendered.contains("맡긴다. 세부 내용은 선택 후 드러난다."));
        assert!(rendered.contains("자유서술"));
        assert!(rendered.contains("6 뒤에 직접 행동"));
        Ok(())
    }
}
