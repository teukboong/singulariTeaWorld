use crate::models::{
    ANCHOR_CHARACTER_ID, AdjudicationReport, CanonEvent, EntityRecords, EntityUpdateRecord,
    OPENING_LOCATION_ID, PROTAGONIST_CHARACTER_ID, RelationshipUpdateRecord,
    StructuredEntityUpdates, TurnInputKind,
};

const PLAYER_VISIBLE: &str = "player_visible";
const UPDATE_KIND_HISTORY: &str = "personal_history";
const UPDATE_KIND_PLACE_STATE: &str = "place_state";
const UPDATE_KIND_FACTION_PRESSURE: &str = "faction_pressure";
const RELATION_KIND_STORY_TENSION: &str = "story_tension";

#[derive(Debug)]
pub struct EntityUpdateInput<'a> {
    pub entities: &'a mut EntityRecords,
    pub event: &'a CanonEvent,
    pub adjudication: &'a AdjudicationReport,
    pub input_kind: TurnInputKind,
    pub created_at: &'a str,
}

/// Apply the deterministic V1 entity/relationship update helper.
///
/// The helper updates `entities.json`-backed records and returns the exact
/// projection rows that should be mirrored into `world.db`.
#[must_use]
pub fn apply_structured_entity_updates(
    mut input: EntityUpdateInput<'_>,
) -> StructuredEntityUpdates {
    let mut updates =
        StructuredEntityUpdates::empty(input.event.world_id.as_str(), input.event.turn_id.as_str());
    apply_protagonist_history(&mut updates, &mut input);
    apply_location_state(&mut updates, &mut input);
    apply_guide_relationship(&mut updates, &mut input);
    if matches!(input.input_kind, TurnInputKind::MacroTimeFlow) {
        apply_faction_pressure(&mut updates, &input);
    }
    updates
}

fn apply_protagonist_history(
    updates: &mut StructuredEntityUpdates,
    input: &mut EntityUpdateInput<'_>,
) {
    let summary = format!(
        "{}: {} ({})",
        input.event.turn_id, input.event.summary, input.adjudication.summary
    );
    if let Some(protagonist) = input
        .entities
        .characters
        .iter_mut()
        .find(|character| character.id == PROTAGONIST_CHARACTER_ID)
    {
        push_limited_unique(&mut protagonist.history, &summary, 24);
    }
    updates.entity_updates.push(EntityUpdateRecord {
        update_id: format!("{}:{}", input.event.event_id, UPDATE_KIND_HISTORY),
        world_id: input.event.world_id.clone(),
        turn_id: input.event.turn_id.clone(),
        entity_id: PROTAGONIST_CHARACTER_ID.to_owned(),
        update_kind: UPDATE_KIND_HISTORY.to_owned(),
        visibility: PLAYER_VISIBLE.to_owned(),
        summary,
        source_event_id: input.event.event_id.clone(),
        created_at: input.created_at.to_owned(),
    });
}

fn apply_location_state(updates: &mut StructuredEntityUpdates, input: &mut EntityUpdateInput<'_>) {
    let location = input
        .event
        .location
        .as_deref()
        .unwrap_or(OPENING_LOCATION_ID);
    let summary = format!(
        "{} 이후 장소 상태: {}",
        input.event.turn_id, input.adjudication.outcome
    );
    if let Some(place) = input
        .entities
        .places
        .iter_mut()
        .find(|place| place.id == location)
    {
        push_limited_unique(&mut place.notes, &summary, 16);
    }
    updates.entity_updates.push(EntityUpdateRecord {
        update_id: format!("{}:{}", input.event.event_id, UPDATE_KIND_PLACE_STATE),
        world_id: input.event.world_id.clone(),
        turn_id: input.event.turn_id.clone(),
        entity_id: location.to_owned(),
        update_kind: UPDATE_KIND_PLACE_STATE.to_owned(),
        visibility: PLAYER_VISIBLE.to_owned(),
        summary,
        source_event_id: input.event.event_id.clone(),
        created_at: input.created_at.to_owned(),
    });
}

fn apply_guide_relationship(
    updates: &mut StructuredEntityUpdates,
    input: &mut EntityUpdateInput<'_>,
) {
    let relation_summary = match input.input_kind {
        TurnInputKind::GuideChoice => "주인공이 안내자의 숨은 안내에 맡긴 기록",
        TurnInputKind::CodexQuery => "주인공이 세계 기록을 통해 안내자의 기록자 권한에 닿은 기록",
        TurnInputKind::FreeformAction | TurnInputKind::NumericChoice => {
            "주인공과 앵커 인물 사이의 이야기 장력이 한 박자 누적됨"
        }
        TurnInputKind::MacroTimeFlow => "거시 흐름 속에서 앵커 인물 가능성이 다시 고정됨",
        TurnInputKind::CcCanvas => "장면 표면 전환 중에도 앵커 인물 상수가 유지됨",
    };
    if let Some(anchor_character) = input
        .entities
        .characters
        .iter_mut()
        .find(|character| character.id == ANCHOR_CHARACTER_ID)
    {
        push_limited_unique(
            &mut anchor_character.relationships,
            &format!("protagonist: {relation_summary}"),
            24,
        );
    }
    updates.relationship_updates.push(RelationshipUpdateRecord {
        update_id: format!("{}:{}", input.event.event_id, RELATION_KIND_STORY_TENSION),
        world_id: input.event.world_id.clone(),
        turn_id: input.event.turn_id.clone(),
        source_entity_id: PROTAGONIST_CHARACTER_ID.to_owned(),
        target_entity_id: ANCHOR_CHARACTER_ID.to_owned(),
        relation_kind: RELATION_KIND_STORY_TENSION.to_owned(),
        visibility: PLAYER_VISIBLE.to_owned(),
        summary: relation_summary.to_owned(),
        source_event_id: input.event.event_id.clone(),
        created_at: input.created_at.to_owned(),
    });
}

fn apply_faction_pressure(updates: &mut StructuredEntityUpdates, input: &EntityUpdateInput<'_>) {
    let summary = "아직 이름 없는 세력/도시 압력이 거시 흐름 후보로 기록됨".to_owned();
    updates.entity_updates.push(EntityUpdateRecord {
        update_id: format!("{}:{}", input.event.event_id, UPDATE_KIND_FACTION_PRESSURE),
        world_id: input.event.world_id.clone(),
        turn_id: input.event.turn_id.clone(),
        entity_id: "concept:unresolved_faction_pressure".to_owned(),
        update_kind: UPDATE_KIND_FACTION_PRESSURE.to_owned(),
        visibility: PLAYER_VISIBLE.to_owned(),
        summary,
        source_event_id: input.event.event_id.clone(),
        created_at: input.created_at.to_owned(),
    });
}

fn push_limited_unique(values: &mut Vec<String>, value: &str, limit: usize) {
    if values.iter().any(|existing| existing == value) {
        return;
    }
    values.push(value.to_owned());
    if values.len() > limit {
        let overflow = values.len() - limit;
        values.drain(0..overflow);
    }
}

#[cfg(test)]
mod tests {
    use super::{EntityUpdateInput, apply_structured_entity_updates};
    use crate::adjudication::{AdjudicationInput, adjudicate_turn};
    use crate::models::{
        AnchorCharacter, EntityRecords, HiddenState, LanguagePolicy, PROTAGONIST_CHARACTER_ID,
        RuntimeContract, TurnInputKind, TurnSnapshot, WorldLaws, WorldPremise, WorldRecord,
        WorldSeed, initial_canon_event,
    };

    fn world() -> WorldRecord {
        WorldRecord::from_seed(
            WorldSeed {
                schema_version: crate::models::WORLD_SEED_SCHEMA_VERSION.to_owned(),
                world_id: "stw_entities".to_owned(),
                title: "엔티티 세계".to_owned(),
                created_by: "local_user".to_owned(),
                runtime_contract: RuntimeContract::default(),
                premise: WorldPremise {
                    genre: "중세 판타지".to_owned(),
                    protagonist: "현대인 전생".to_owned(),
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
    fn helper_updates_protagonist_history_and_relationships() {
        let world = world();
        let snapshot = TurnSnapshot::initial(&world, "session".to_owned());
        let hidden = HiddenState::initial(world.world_id.as_str());
        let mut event = initial_canon_event(&world);
        event.turn_id = "turn_0001".to_owned();
        event.event_id = "evt_000001".to_owned();
        event.summary = "4번 [안내자의 선택] 선택이 접수됐다".to_owned();
        let adjudication = adjudicate_turn(&AdjudicationInput {
            world: &world,
            snapshot: &snapshot,
            hidden_state: &hidden,
            turn_id: "turn_0001",
            input_kind: TurnInputKind::GuideChoice,
            selected_choice: None,
            effective_input: "4",
        });
        let mut entities = EntityRecords::initial(&world);
        let updates = apply_structured_entity_updates(EntityUpdateInput {
            entities: &mut entities,
            event: &event,
            adjudication: &adjudication,
            input_kind: TurnInputKind::GuideChoice,
            created_at: "2026-04-27T00:00:00Z",
        });
        assert_eq!(updates.entity_updates.len(), 2);
        assert_eq!(updates.relationship_updates.len(), 1);
        assert!(
            entities
                .characters
                .iter()
                .find(|character| character.id == PROTAGONIST_CHARACTER_ID)
                .is_some_and(|character| !character.history.is_empty())
        );
    }
}
