use crate::adjudication::{AdjudicationInput, adjudicate_turn};
use crate::codex_view::{BuildCodexViewOptions, build_codex_view};
use crate::entity_update::{EntityUpdateInput, apply_structured_entity_updates};
use crate::models::{
    ANCHOR_CHARACTER_ID, CANON_EVENT_SCHEMA_VERSION, CanonEvent, CurrentEvent, DashboardSummary,
    EntityRecords, EventAuthority, EventEvidence, FREEFORM_CHOICE_SLOT, FREEFORM_CHOICE_TAG,
    HiddenState, PROTAGONIST_CHARACTER_ID, RENDER_PACKET_SCHEMA_VERSION, RenderPacket, ScanTarget,
    TURN_LOG_ENTRY_SCHEMA_VERSION, TurnChoice, TurnInputKind, TurnLogEntry, TurnSnapshot,
    VisibleState, WorldEventKind, WorldRecord, default_freeform_choice, default_turn_choices,
    is_guide_choice_tag, normalize_turn_choices,
};
use crate::store::{
    StorePaths, TURN_LOG_FILENAME, WorldFilePaths, acquire_world_commit_lock, append_canon_event,
    append_jsonl, read_json, resolve_store_paths, world_file_paths, write_json,
};
use crate::world_db::RecordTurnDbInput;
use crate::world_db::record_turn_in_world_db;
use crate::world_docs::refresh_world_docs;
use anyhow::{Context, Result, bail};
use chrono::Utc;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AdvanceTurnOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct AdvancedTurn {
    pub world: WorldRecord,
    pub previous_snapshot: TurnSnapshot,
    pub snapshot: TurnSnapshot,
    pub canon_event: CanonEvent,
    pub render_packet: RenderPacket,
    pub snapshot_path: PathBuf,
    pub render_packet_path: PathBuf,
    pub turn_log_path: PathBuf,
}

struct ClassifiedInput {
    kind: TurnInputKind,
    mode: String,
    selected_choice: Option<TurnChoice>,
    effective_input: String,
    style_notes: Vec<String>,
}

struct TurnArtifactPaths {
    snapshot: PathBuf,
    render_packet: PathBuf,
    turn_log: PathBuf,
}

struct PersistTurnInput<'a> {
    files: &'a WorldFilePaths,
    world: &'a WorldRecord,
    entities: &'a EntityRecords,
    snapshot: &'a TurnSnapshot,
    canon_event: &'a CanonEvent,
    render_packet: &'a RenderPacket,
    turn_log_entry: &'a TurnLogEntry,
    structured_updates: &'a crate::models::StructuredEntityUpdates,
    paths: &'a TurnArtifactPaths,
}

/// Advance one simulator turn and persist every resulting artifact.
///
/// # Errors
///
/// Returns an error when world files cannot be loaded, the current snapshot is
/// terminal, input is empty, or any turn artifact cannot be written.
pub fn advance_turn(options: &AdvanceTurnOptions) -> Result<AdvancedTurn> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let _world_lock = acquire_world_commit_lock(&files.dir, "advance_turn")?;
    advance_turn_without_world_lock(options, &store_paths, &files)
}

pub(crate) fn advance_turn_without_world_lock(
    options: &AdvanceTurnOptions,
    store_paths: &StorePaths,
    files: &WorldFilePaths,
) -> Result<AdvancedTurn> {
    advance_turn_without_world_lock_with_authority(
        options,
        store_paths,
        files,
        EventAuthority::TurnReducer,
    )
}

pub(crate) fn advance_turn_without_world_lock_with_authority(
    options: &AdvanceTurnOptions,
    store_paths: &StorePaths,
    files: &WorldFilePaths,
    event_authority: EventAuthority,
) -> Result<AdvancedTurn> {
    let input = options.input.trim();
    if input.is_empty() {
        bail!("turn input must not be empty");
    }
    let world: WorldRecord = read_json(&files.world)?;
    let previous_snapshot: TurnSnapshot = read_json(&files.latest_snapshot)?;
    ensure_world_is_not_terminal(&world, &previous_snapshot)?;
    let hidden_state: HiddenState = read_json(&files.hidden_state)?;
    let mut entities: EntityRecords = read_json(&files.entities)?;
    let classified = classify_input(input, &previous_snapshot)?;
    let turn_number = next_turn_number(previous_snapshot.turn_id.as_str())?;
    let turn_id = format!("turn_{turn_number:04}");
    let event_id = format!("evt_{turn_number:06}");
    let created_at = Utc::now().to_rfc3339();
    let artifact_paths = build_turn_artifact_paths(files, &previous_snapshot, &turn_id);

    let snapshot = build_next_snapshot(&world, &previous_snapshot, &turn_id, &classified);
    let adjudication = adjudicate_turn(&AdjudicationInput {
        world: &world,
        snapshot: &previous_snapshot,
        hidden_state: &hidden_state,
        turn_id: turn_id.as_str(),
        input_kind: classified.kind,
        selected_choice: classified.selected_choice.as_ref(),
        effective_input: classified.effective_input.as_str(),
    });
    let canon_event = build_canon_event(
        &world,
        &previous_snapshot,
        &event_id,
        &turn_id,
        &classified,
        &adjudication,
        event_authority,
    );
    let structured_updates = apply_structured_entity_updates(EntityUpdateInput {
        entities: &mut entities,
        event: &canon_event,
        adjudication: &adjudication,
        input_kind: classified.kind,
        created_at: created_at.as_str(),
    });
    let codex_view = if matches!(classified.kind, TurnInputKind::CodexQuery) {
        let mut options = BuildCodexViewOptions::new(world.world_id.clone());
        options.store_root = Some(store_paths.root.clone());
        let mut view = build_codex_view(&options)?;
        view.turn_id.clone_from(&turn_id);
        Some(view)
    } else {
        None
    };
    let render_packet = build_render_packet(
        &world,
        &snapshot,
        &canon_event,
        &hidden_state,
        &classified,
        adjudication.clone(),
        codex_view,
    );
    let turn_log_entry = TurnLogEntry {
        schema_version: TURN_LOG_ENTRY_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id.clone(),
        session_id: snapshot.session_id.clone(),
        turn_id: turn_id.clone(),
        input: input.to_owned(),
        input_kind: classified.kind,
        selected_choice: classified.selected_choice.clone(),
        canon_event_id: canon_event.event_id.clone(),
        snapshot_ref: artifact_paths.snapshot.display().to_string(),
        render_packet_ref: artifact_paths.render_packet.display().to_string(),
        created_at,
    };

    let canon_event = persist_turn_artifacts(&PersistTurnInput {
        files,
        world: &world,
        entities: &entities,
        snapshot: &snapshot,
        canon_event: &canon_event,
        render_packet: &render_packet,
        turn_log_entry: &turn_log_entry,
        structured_updates: &structured_updates,
        paths: &artifact_paths,
    })?;
    refresh_world_docs(&files.dir)?;

    Ok(AdvancedTurn {
        world,
        previous_snapshot,
        snapshot,
        canon_event,
        render_packet,
        snapshot_path: artifact_paths.snapshot,
        render_packet_path: artifact_paths.render_packet,
        turn_log_path: artifact_paths.turn_log,
    })
}

fn ensure_world_is_not_terminal(world: &WorldRecord, snapshot: &TurnSnapshot) -> Result<()> {
    if snapshot.phase == "terminal" {
        bail!(
            "cannot advance terminal world: world_id={}, turn_id={}",
            world.world_id,
            snapshot.turn_id
        );
    }
    Ok(())
}

fn build_turn_artifact_paths(
    files: &WorldFilePaths,
    previous_snapshot: &TurnSnapshot,
    turn_id: &str,
) -> TurnArtifactPaths {
    let session_dir = files
        .dir
        .join("sessions")
        .join(&previous_snapshot.session_id);
    TurnArtifactPaths {
        snapshot: session_dir
            .join("snapshots")
            .join(format!("{turn_id}.json")),
        render_packet: session_dir
            .join("render_packets")
            .join(format!("{turn_id}.json")),
        turn_log: session_dir.join(TURN_LOG_FILENAME),
    }
}

fn persist_turn_artifacts(input: &PersistTurnInput<'_>) -> Result<CanonEvent> {
    ensure_parent_dir(&input.paths.snapshot)?;
    ensure_parent_dir(&input.paths.render_packet)?;
    let canon_event = append_canon_event(&input.files.canon_events, input.canon_event)?;
    append_jsonl(&input.files.entity_updates, input.structured_updates)?;
    write_json(&input.files.entities, input.entities)?;
    write_json(&input.paths.snapshot, input.snapshot)?;
    write_json(&input.files.latest_snapshot, input.snapshot)?;
    write_json(&input.paths.render_packet, input.render_packet)?;
    append_jsonl(&input.paths.turn_log, input.turn_log_entry)?;
    record_turn_in_world_db(&RecordTurnDbInput {
        world_dir: &input.files.dir,
        world: input.world,
        entities: input.entities,
        snapshot: input.snapshot,
        canon_event: &canon_event,
        render_packet: input.render_packet,
        turn_log_entry: input.turn_log_entry,
        structured_updates: input.structured_updates,
    })?;
    Ok(canon_event)
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

#[must_use]
pub fn render_advanced_turn_report(turn: &AdvancedTurn) -> String {
    let selected = turn
        .render_packet
        .visible_state
        .choices
        .iter()
        .find(|choice| {
            turn.canon_event
                .summary
                .contains(format!("{}번", choice.slot).as_str())
        })
        .map_or("none".to_owned(), |choice| {
            format!("{} {}", choice.slot, choice.tag)
        });
    [
        format!("world: {}", turn.world.world_id),
        format!("turn: {}", turn.snapshot.turn_id),
        format!("previous_turn: {}", turn.previous_snapshot.turn_id),
        format!("mode: {}", turn.render_packet.mode),
        format!("event: {}", turn.canon_event.event_id),
        format!("selected_choice: {selected}"),
        format!("snapshot: {}", turn.snapshot_path.display()),
        format!("render_packet: {}", turn.render_packet_path.display()),
        format!("turn_log: {}", turn.turn_log_path.display()),
    ]
    .join("\n")
}

fn classify_input(input: &str, snapshot: &TurnSnapshot) -> Result<ClassifiedInput> {
    let cc_mode = input.contains(".cc");
    let effective_input = input.replace(".cc", "").trim().to_owned();
    let choices = normalize_turn_choices(&snapshot.last_choices);
    let inline_freeform = inline_freeform_choice(&effective_input, &choices);
    let selection = effective_input
        .parse::<u8>()
        .ok()
        .and_then(|slot| choices.iter().find(|choice| choice.slot == slot))
        .cloned();
    let mut style_notes = vec!["Korean UI".to_owned()];
    if input.starts_with("월뮬") {
        style_notes.push("authentic bilingual mode reinforced".to_owned());
    }
    if input.contains("뮬월") {
        style_notes.push("pure Korean output for this turn".to_owned());
    }
    if cc_mode {
        style_notes.push(".cc canvas transform requested".to_owned());
        return Ok(ClassifiedInput {
            kind: TurnInputKind::CcCanvas,
            mode: "cc".to_owned(),
            selected_choice: selection,
            effective_input,
            style_notes,
        });
    }
    if let Some((choice, action)) = inline_freeform {
        if action.is_empty() {
            bail!("자유서술은 6 뒤에 행동 서술을 같이 써야 한다: 예) 6 세아에게 낮게 묻는다");
        }
        return Ok(ClassifiedInput {
            kind: TurnInputKind::FreeformAction,
            mode: "normal".to_owned(),
            selected_choice: Some(choice),
            effective_input: action,
            style_notes,
        });
    }
    if let Some(choice) = &selection {
        if choice.tag == FREEFORM_CHOICE_TAG {
            bail!(
                "자유서술은 6 뒤에 행동 서술을 같이 써야 한다: 예) 6 문 아래의 흙을 손끝으로 문질러 본다"
            );
        }
        if is_guide_choice_tag(choice.tag.as_str()) {
            return Ok(ClassifiedInput {
                kind: TurnInputKind::GuideChoice,
                mode: "normal".to_owned(),
                selected_choice: selection,
                effective_input,
                style_notes,
            });
        }
        if choice.tag == "기록" {
            return Ok(ClassifiedInput {
                kind: TurnInputKind::CodexQuery,
                mode: "codex".to_owned(),
                selected_choice: selection,
                effective_input,
                style_notes,
            });
        }
        if choice.tag == "흐름" {
            return Ok(ClassifiedInput {
                kind: TurnInputKind::MacroTimeFlow,
                mode: "macro_time_flow".to_owned(),
                selected_choice: selection,
                effective_input,
                style_notes,
            });
        }
        return Ok(ClassifiedInput {
            kind: TurnInputKind::NumericChoice,
            mode: "normal".to_owned(),
            selected_choice: selection,
            effective_input,
            style_notes,
        });
    }
    Ok(ClassifiedInput {
        kind: TurnInputKind::FreeformAction,
        mode: "normal".to_owned(),
        selected_choice: None,
        effective_input,
        style_notes,
    })
}

fn inline_freeform_choice(
    effective_input: &str,
    choices: &[TurnChoice],
) -> Option<(TurnChoice, String)> {
    let choice = choices
        .iter()
        .find(|choice| choice.slot == FREEFORM_CHOICE_SLOT)
        .cloned()
        .unwrap_or_else(default_freeform_choice);
    let slot_digit = char::from_digit(u32::from(FREEFORM_CHOICE_SLOT), 10)?;
    let after_slot = effective_input.strip_prefix(slot_digit)?;
    let action = freeform_action_after_slot_prefix(after_slot)?;
    Some((choice, action))
}

fn freeform_action_after_slot_prefix(after_slot: &str) -> Option<String> {
    if after_slot.is_empty() {
        return Some(String::new());
    }
    let rest = if let Some(rest) = after_slot.strip_prefix("번") {
        rest
    } else if after_slot.starts_with(char::is_whitespace) {
        after_slot
    } else if after_slot
        .chars()
        .next()
        .is_some_and(is_freeform_slot_delimiter)
    {
        &after_slot[after_slot.chars().next()?.len_utf8()..]
    } else {
        return None;
    };
    let trimmed = rest
        .trim_start()
        .trim_start_matches(is_freeform_slot_delimiter)
        .trim_start();
    Some(trimmed.to_owned())
}

fn is_freeform_slot_delimiter(ch: char) -> bool {
    matches!(ch, '.' | ')' | ':' | '-' | '—')
}

fn build_next_snapshot(
    world: &WorldRecord,
    previous: &TurnSnapshot,
    turn_id: &str,
    input: &ClassifiedInput,
) -> TurnSnapshot {
    let phase = match input.kind {
        TurnInputKind::CodexQuery => "codex",
        TurnInputKind::MacroTimeFlow => "macro_time_flow",
        TurnInputKind::CcCanvas => previous.phase.as_str(),
        TurnInputKind::NumericChoice
        | TurnInputKind::GuideChoice
        | TurnInputKind::FreeformAction => {
            if previous.phase == "interlude" {
                "event"
            } else {
                previous.phase.as_str()
            }
        }
    };
    let mut mind = previous.protagonist_state.mind.clone();
    push_limited(&mut mind, mind_note(input), 5);
    let open_questions = previous.open_questions.clone();
    TurnSnapshot {
        schema_version: previous.schema_version.clone(),
        world_id: world.world_id.clone(),
        session_id: previous.session_id.clone(),
        turn_id: turn_id.to_owned(),
        phase: phase.to_owned(),
        current_event: next_current_event(previous, input),
        protagonist_state: crate::models::ProtagonistState {
            location: previous.protagonist_state.location.clone(),
            inventory: previous.protagonist_state.inventory.clone(),
            body: previous.protagonist_state.body.clone(),
            mind,
        },
        open_questions,
        last_choices: default_turn_choices(),
    }
}

fn next_current_event(previous: &TurnSnapshot, input: &ClassifiedInput) -> Option<CurrentEvent> {
    if matches!(
        input.kind,
        TurnInputKind::CodexQuery | TurnInputKind::MacroTimeFlow | TurnInputKind::CcCanvas
    ) {
        return previous.current_event.clone();
    }
    let selected_tag = input
        .selected_choice
        .as_ref()
        .map(|choice| choice.tag.as_str());
    if previous.current_event.is_none()
        && (matches!(selected_tag, Some("움직임" | FREEFORM_CHOICE_TAG))
            || input.kind == TurnInputKind::GuideChoice)
    {
        return Some(CurrentEvent {
            event_id: "event_opening_prelude".to_owned(),
            progress: "첫 변화가 기록되기 시작함".to_owned(),
            rail_required: true,
        });
    }
    previous.current_event.as_ref().map(|event| CurrentEvent {
        event_id: event.event_id.clone(),
        progress: if matches!(selected_tag, Some("움직임" | FREEFORM_CHOICE_TAG))
            || input.kind == TurnInputKind::GuideChoice
        {
            "다음 의미 있는 박자로 전진함".to_owned()
        } else {
            event.progress.clone()
        },
        rail_required: event.rail_required,
    })
}

fn build_canon_event(
    world: &WorldRecord,
    previous: &TurnSnapshot,
    event_id: &str,
    turn_id: &str,
    input: &ClassifiedInput,
    adjudication: &crate::models::AdjudicationReport,
    authority: EventAuthority,
) -> CanonEvent {
    CanonEvent {
        schema_version: CANON_EVENT_SCHEMA_VERSION.to_owned(),
        event_id: event_id.to_owned(),
        world_id: world.world_id.clone(),
        turn_id: turn_id.to_owned(),
        occurred_at_world_time: format!("after {}", previous.turn_id),
        visibility: "player_visible".to_owned(),
        kind: input.kind.as_wire().to_owned(),
        event_kind: Some(WorldEventKind::from_turn_input_kind(input.kind)),
        authority: Some(authority),
        previous_event_hash: None,
        event_hash: None,
        summary: event_summary(input),
        entities: vec![
            PROTAGONIST_CHARACTER_ID.to_owned(),
            ANCHOR_CHARACTER_ID.to_owned(),
        ],
        location: Some(previous.protagonist_state.location.clone()),
        evidence: EventEvidence {
            source: "turn_reducer".to_owned(),
            user_input: input.effective_input.clone(),
            narrative_ref: format!("sessions/{}/snapshots/{turn_id}.json", previous.session_id),
        },
        consequences: event_consequences(input, adjudication),
    }
}

fn build_render_packet(
    world: &WorldRecord,
    snapshot: &TurnSnapshot,
    event: &CanonEvent,
    hidden_state: &HiddenState,
    input: &ClassifiedInput,
    adjudication: crate::models::AdjudicationReport,
    codex_view: Option<crate::models::CodexView>,
) -> RenderPacket {
    let forbidden_reveals = hidden_state
        .secrets
        .iter()
        .flat_map(|secret| {
            let mut refs = vec![format!("{}.truth", secret.secret_id)];
            refs.extend(secret.forbidden_leaks.iter().cloned());
            refs
        })
        .collect();
    let current_event = snapshot
        .current_event
        .as_ref()
        .map_or_else(|| "none".to_owned(), |event| event.event_id.clone());
    RenderPacket {
        schema_version: RENDER_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id.clone(),
        turn_id: snapshot.turn_id.clone(),
        mode: input.mode.clone(),
        narrative_contract: world.runtime_contract.mode.clone(),
        narrative_scene: None,
        visible_state: VisibleState {
            dashboard: DashboardSummary {
                phase: snapshot.phase.clone(),
                location: snapshot.protagonist_state.location.clone(),
                anchor_invariant: world.anchor_character.invariant.clone(),
                current_event,
                status: event.summary.clone(),
            },
            scan_targets: vec![
                ScanTarget {
                    target: "현재 행위자".to_owned(),
                    class: "self".to_owned(),
                    distance: "현재 몸".to_owned(),
                    thought: "아직 장면 압력이 구체화되지 않았다".to_owned(),
                },
                ScanTarget {
                    target: "현재 장면".to_owned(),
                    class: "environment".to_owned(),
                    distance: "관찰 범위".to_owned(),
                    thought: "아직 원인을 단정하기에는 단서가 부족하다".to_owned(),
                },
            ],
            choices: normalize_turn_choices(&snapshot.last_choices),
        },
        adjudication: Some(adjudication),
        codex_view,
        canon_delta_refs: vec![event.event_id.clone()],
        forbidden_reveals,
        style_notes: input.style_notes.clone(),
    }
}

fn event_summary(input: &ClassifiedInput) -> String {
    match &input.selected_choice {
        Some(choice) if choice.tag == FREEFORM_CHOICE_TAG => format!(
            "{}번 [{}] 서술이 접수됐다: {}",
            choice.slot, choice.tag, input.effective_input
        ),
        Some(choice) => format!(
            "{}번 [{}] 선택이 접수됐다. {}",
            choice.slot,
            choice.tag,
            choice.player_visible_intent()
        ),
        None => format!("자유 입력이 시도됐다: {}", input.effective_input),
    }
}

fn event_consequences(
    input: &ClassifiedInput,
    adjudication: &crate::models::AdjudicationReport,
) -> Vec<String> {
    let mut consequences = match input.kind {
        TurnInputKind::GuideChoice => vec![
            "delegated_judgment_selected".to_owned(),
            "advance_without_overriding_world_law".to_owned(),
        ],
        TurnInputKind::CodexQuery => vec!["codex_view_requested".to_owned()],
        TurnInputKind::MacroTimeFlow => vec!["macro_time_flow_requested".to_owned()],
        TurnInputKind::CcCanvas => vec!["canvas_transform_requested".to_owned()],
        TurnInputKind::NumericChoice => vec!["choice_selected".to_owned()],
        TurnInputKind::FreeformAction => vec!["freeform_attempt_recorded".to_owned()],
    };
    consequences.extend(adjudication.consequences.iter().cloned());
    consequences
}

fn mind_note(input: &ClassifiedInput) -> String {
    match input.kind {
        TurnInputKind::GuideChoice => "위임 선택이 접수됐다".to_owned(),
        TurnInputKind::CodexQuery => "공개 기록 조회가 접수됐다".to_owned(),
        TurnInputKind::MacroTimeFlow => "시간 흐름 조회가 접수됐다".to_owned(),
        TurnInputKind::CcCanvas => "장면 변환 요청이 접수됐다".to_owned(),
        TurnInputKind::NumericChoice => "선택한 행동이 접수됐다".to_owned(),
        TurnInputKind::FreeformAction => "직접 서술한 행동이 접수됐다".to_owned(),
    }
}

fn push_limited(values: &mut Vec<String>, value: String, limit: usize) {
    values.push(value);
    if values.len() > limit {
        let overflow = values.len() - limit;
        values.drain(0..overflow);
    }
}

fn next_turn_number(turn_id: &str) -> Result<u32> {
    let number = turn_id
        .strip_prefix("turn_")
        .context("turn_id must start with turn_")?
        .parse::<u32>()
        .with_context(|| format!("turn_id has invalid numeric suffix: {turn_id}"))?;
    Ok(number + 1)
}

#[cfg(test)]
mod tests {
    use super::{AdvanceTurnOptions, advance_turn};
    use crate::models::{
        EventAuthority, FREEFORM_CHOICE_SLOT, GUIDE_CHOICE_SLOT, GUIDE_CHOICE_TAG, TurnChoice,
        TurnInputKind, WorldEventKind,
    };
    use crate::store::{
        InitWorldOptions, acquire_world_commit_lock, init_world, read_json, resolve_store_paths,
        world_file_paths, write_json,
    };
    use crate::validate::{ValidationStatus, validate_world};
    use tempfile::tempdir;

    fn seed_body() -> &'static str {
        r#"
schema_version: singulari.world_seed.v1
world_id: stw_turn
title: "턴 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#
    }

    fn legacy_slot_contract_choices() -> Vec<TurnChoice> {
        vec![
            TurnChoice {
                slot: 1,
                tag: "발소리".to_owned(),
                intent: "발소리의 방향으로 접근한다".to_owned(),
            },
            TurnChoice {
                slot: 2,
                tag: "주변".to_owned(),
                intent: "주변 단서를 확인한다".to_owned(),
            },
            TurnChoice {
                slot: 3,
                tag: "접촉".to_owned(),
                intent: "가까운 기척에 말을 건다".to_owned(),
            },
            TurnChoice {
                slot: 4,
                tag: GUIDE_CHOICE_TAG.to_owned(),
                intent: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
            },
            TurnChoice {
                slot: 5,
                tag: "기록".to_owned(),
                intent: "현재 알려진 세계 기록을 연다".to_owned(),
            },
            TurnChoice {
                slot: 6,
                tag: "흐름".to_owned(),
                intent: "시간의 관찰자 시점으로 다음 흐름을 본다".to_owned(),
            },
            TurnChoice {
                slot: 7,
                tag: "자유서술".to_owned(),
                intent: "7 뒤에 직접 행동, 말, 내면 독백을 서술한다".to_owned(),
            },
        ]
    }

    #[test]
    fn turn_seven_records_guide_choice_and_render_packet() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let turn = advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_turn".to_owned(),
            input: "7".to_owned(),
        })?;
        assert_eq!(turn.snapshot.turn_id, "turn_0001");
        assert_eq!(turn.canon_event.kind, TurnInputKind::GuideChoice.as_wire());
        assert_eq!(
            turn.canon_event.event_kind,
            Some(WorldEventKind::GuideChoice)
        );
        assert_eq!(
            turn.canon_event.authority,
            Some(EventAuthority::TurnReducer)
        );
        assert_eq!(turn.render_packet.visible_state.choices.len(), 7);
        assert!(
            turn.render_packet
                .visible_state
                .choices
                .iter()
                .any(|choice| {
                    choice.slot == GUIDE_CHOICE_SLOT && choice.tag == GUIDE_CHOICE_TAG
                })
        );
        let report = validate_world(Some(&store), "stw_turn")?;
        assert_eq!(report.status, ValidationStatus::Passed);
        Ok(())
    }

    #[test]
    fn advance_turn_refuses_existing_world_commit_lock() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_turn");
        let _lock = acquire_world_commit_lock(&files.dir, "test_holder")?;

        let result = advance_turn(&AdvanceTurnOptions {
            store_root: Some(store),
            world_id: "stw_turn".to_owned(),
            input: "1".to_owned(),
        });

        let Err(error) = result else {
            anyhow::bail!("advance turn should fail when a world commit lock is already held");
        };
        let rendered = format!("{error:#}");
        assert!(rendered.contains("world commit lock already held"));
        assert!(rendered.contains("test_holder"));
        Ok(())
    }

    #[test]
    fn legacy_snapshot_turn_seven_still_records_guide_choice() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            seed_body().replace("stw_turn", "stw_legacy_turn"),
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let store_paths = resolve_store_paths(Some(store.as_path()))?;
        let files = world_file_paths(&store_paths, "stw_legacy_turn");
        let mut snapshot: crate::models::TurnSnapshot = read_json(&files.latest_snapshot)?;
        snapshot.last_choices = legacy_slot_contract_choices();
        write_json(&files.latest_snapshot, &snapshot)?;

        let turn = advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_legacy_turn".to_owned(),
            input: "7".to_owned(),
        })?;

        assert_eq!(turn.canon_event.kind, TurnInputKind::GuideChoice.as_wire());
        assert!(
            turn.render_packet
                .visible_state
                .choices
                .iter()
                .any(|choice| choice.slot == GUIDE_CHOICE_SLOT && choice.tag == GUIDE_CHOICE_TAG)
        );
        Ok(())
    }

    #[test]
    fn turn_six_inline_freeform_records_direct_action() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body().replace("stw_turn", "stw_freeform"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let turn = advance_turn(&AdvanceTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_freeform".to_owned(),
            input: "6 세아에게 낮게 묻는다".to_owned(),
        })?;
        assert_eq!(
            turn.canon_event.kind,
            TurnInputKind::FreeformAction.as_wire()
        );
        assert!(
            turn.canon_event
                .summary
                .contains("6번 [자유서술] 서술이 접수됐다: 세아에게 낮게 묻는다")
        );
        assert_eq!(
            turn.render_packet
                .adjudication
                .as_ref()
                .map(|report| report.outcome.as_str()),
            Some("constrained")
        );
        assert!(
            turn.render_packet
                .visible_state
                .choices
                .iter()
                .any(|choice| choice.slot == FREEFORM_CHOICE_SLOT && choice.tag == "자유서술")
        );
        let report = validate_world(Some(&store), "stw_freeform")?;
        assert_eq!(report.status, ValidationStatus::Passed);
        Ok(())
    }

    #[test]
    fn bare_turn_six_requires_inline_description() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body().replace("stw_turn", "stw_bare_six"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let result = advance_turn(&AdvanceTurnOptions {
            store_root: Some(store),
            world_id: "stw_bare_six".to_owned(),
            input: "6".to_owned(),
        });
        let Err(error) = result else {
            anyhow::bail!("bare slot 6 should require inline description");
        };
        assert!(error.to_string().contains("6 뒤에 행동 서술"));
        Ok(())
    }

    #[test]
    fn turn_four_enters_codex_mode() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body().replace("stw_turn", "stw_codex"))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let turn = advance_turn(&AdvanceTurnOptions {
            store_root: Some(store),
            world_id: "stw_codex".to_owned(),
            input: "4".to_owned(),
        })?;
        assert_eq!(turn.render_packet.mode, "codex");
        assert_eq!(turn.snapshot.phase, "codex");
        Ok(())
    }
}
