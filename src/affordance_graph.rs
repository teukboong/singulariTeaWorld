use crate::body_resource::{BodyResourcePacket, ResourceItem};
use crate::encounter_surface::{
    AffordanceAvailability, EncounterActionKind, EncounterSurface, EncounterSurfaceKind,
    EncounterSurfacePacket, EncounterSurfaceStatus,
};
use crate::location_graph::{LocationGraphPacket, LocationNode};
use crate::scene_pressure::{ScenePressure, ScenePressureKind, ScenePressurePacket};
use serde::{Deserialize, Serialize};

pub const AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.affordance_graph_packet.v1";
pub const AFFORDANCE_NODE_SCHEMA_VERSION: &str = "singulari.affordance_node.v1";
pub const ORDINARY_AFFORDANCE_SLOT_COUNT: u8 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AffordanceGraphPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub ordinary_choice_slots: Vec<AffordanceNode>,
    pub compiler_policy: AffordanceGraphPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AffordanceNode {
    pub schema_version: String,
    pub slot: u8,
    pub affordance_id: String,
    pub affordance_kind: AffordanceKind,
    pub label_contract: String,
    pub action_contract: String,
    #[serde(default)]
    pub source_refs: Vec<String>,
    #[serde(default)]
    pub pressure_refs: Vec<String>,
    #[serde(default)]
    pub forbidden_shortcuts: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AffordanceKind {
    Move,
    Observe,
    Contact,
    ResourceOrBody,
    PressureResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AffordanceGraphPolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for AffordanceGraphPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_visible_scene_pressure_body_resource_location_v1".to_owned(),
            use_rules: vec![
                "Slots 1..5 must be rewritten as scene-specific choices grounded in these affordances.".to_owned(),
                "Affordances are action permissions, not hidden lore or plot suggestions.".to_owned(),
                "Do not copy affordance_id/source_refs into player-visible choice text.".to_owned(),
                "Slot 6 remains freeform and slot 7 remains delegated judgment; this graph only covers ordinary slots 1..5.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn compile_affordance_graph_packet(
    world_id: &str,
    turn_id: &str,
    scene_pressure: &ScenePressurePacket,
    body_resource: &BodyResourcePacket,
    location_graph: &LocationGraphPacket,
) -> AffordanceGraphPacket {
    compile_affordance_graph_packet_with_encounter(
        world_id,
        turn_id,
        scene_pressure,
        body_resource,
        location_graph,
        None,
    )
}

#[must_use]
pub fn compile_affordance_graph_packet_with_encounter(
    world_id: &str,
    turn_id: &str,
    scene_pressure: &ScenePressurePacket,
    body_resource: &BodyResourcePacket,
    location_graph: &LocationGraphPacket,
    encounter_surface: Option<&EncounterSurfacePacket>,
) -> AffordanceGraphPacket {
    let pressure_refs = visible_pressure_refs(scene_pressure);
    if let Some(encounter_surface) = encounter_surface {
        let encounter_nodes = encounter_affordance_nodes(encounter_surface, &pressure_refs);
        if encounter_nodes.len() == usize::from(ORDINARY_AFFORDANCE_SLOT_COUNT) {
            return AffordanceGraphPacket {
                schema_version: AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
                world_id: world_id.to_owned(),
                turn_id: turn_id.to_owned(),
                ordinary_choice_slots: encounter_nodes,
                compiler_policy: AffordanceGraphPolicy {
                    source: "compiled_from_active_encounter_surface_with_visible_fallbacks_v1"
                        .to_owned(),
                    ..AffordanceGraphPolicy::default()
                },
            };
        }
    }
    AffordanceGraphPacket {
        schema_version: AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        turn_id: turn_id.to_owned(),
        ordinary_choice_slots: vec![
            movement_affordance(location_graph, &pressure_refs),
            observation_affordance(scene_pressure, location_graph, &pressure_refs),
            contact_affordance(scene_pressure, &pressure_refs),
            body_resource_affordance(body_resource, &pressure_refs),
            pressure_response_affordance(scene_pressure, &pressure_refs),
        ],
        compiler_policy: AffordanceGraphPolicy::default(),
    }
}

fn encounter_affordance_nodes(
    encounter_surface: &EncounterSurfacePacket,
    pressure_refs: &[String],
) -> Vec<AffordanceNode> {
    encounter_surface
        .active_surfaces
        .iter()
        .take(usize::from(ORDINARY_AFFORDANCE_SLOT_COUNT))
        .enumerate()
        .map(|(index, surface)| encounter_affordance_node(index, surface, pressure_refs))
        .collect()
}

fn encounter_affordance_node(
    index: usize,
    surface: &EncounterSurface,
    pressure_refs: &[String],
) -> AffordanceNode {
    let slot = u8::try_from(index + 1).unwrap_or(ORDINARY_AFFORDANCE_SLOT_COUNT);
    let affordance = surface.affordances.first();
    let mut source_refs = surface.source_refs.clone();
    if let Some(affordance) = affordance {
        source_refs.extend(affordance.evidence_refs.clone());
    }
    source_refs.sort();
    source_refs.dedup();
    let mut linked_pressure_refs = surface.linked_pressure_refs.clone();
    linked_pressure_refs.extend(pressure_refs.iter().cloned());
    linked_pressure_refs.sort();
    linked_pressure_refs.dedup();
    AffordanceNode {
        schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
        slot,
        affordance_id: affordance.map_or_else(
            || format!("affordance:encounter:slot:{slot}"),
            |affordance| affordance.affordance_id.clone(),
        ),
        affordance_kind: affordance_kind_for_encounter(surface, affordance.map(|a| a.action_kind)),
        label_contract: surface.label.clone(),
        action_contract: affordance.map_or_else(
            || surface.summary.clone(),
            |affordance| affordance.intent_seed.clone(),
        ),
        source_refs,
        pressure_refs: linked_pressure_refs,
        forbidden_shortcuts: forbidden_shortcuts_for_encounter(
            surface,
            affordance.map(|a| a.availability),
        ),
    }
}

fn affordance_kind_for_encounter(
    surface: &EncounterSurface,
    action_kind: Option<EncounterActionKind>,
) -> AffordanceKind {
    match action_kind {
        Some(
            EncounterActionKind::Move | EncounterActionKind::Follow | EncounterActionKind::Bypass,
        ) => AffordanceKind::Move,
        Some(
            EncounterActionKind::Inspect
            | EncounterActionKind::Listen
            | EncounterActionKind::Smell
            | EncounterActionKind::Compare
            | EncounterActionKind::Mark,
        ) => AffordanceKind::Observe,
        Some(
            EncounterActionKind::TalkAbout
            | EncounterActionKind::TradeOver
            | EncounterActionKind::ThreatenWith,
        ) => AffordanceKind::Contact,
        Some(
            EncounterActionKind::Take
            | EncounterActionKind::Use
            | EncounterActionKind::Repair
            | EncounterActionKind::Touch,
        ) => AffordanceKind::ResourceOrBody,
        _ => match surface.kind {
            EncounterSurfaceKind::Exit => AffordanceKind::Move,
            EncounterSurfaceKind::EvidenceTrace | EncounterSurfaceKind::TimeSensitiveCue => {
                AffordanceKind::Observe
            }
            EncounterSurfaceKind::SocialHandle | EncounterSurfaceKind::AccessController => {
                AffordanceKind::Contact
            }
            EncounterSurfaceKind::UsableTool | EncounterSurfaceKind::Container => {
                AffordanceKind::ResourceOrBody
            }
            _ => AffordanceKind::PressureResponse,
        },
    }
}

fn forbidden_shortcuts_for_encounter(
    surface: &EncounterSurface,
    availability: Option<AffordanceAvailability>,
) -> Vec<String> {
    let mut shortcuts = vec![
        "encounter surface 없이 새 해결 대상 발명".to_owned(),
        "hidden/adjudication-only 원인 노출".to_owned(),
    ];
    if matches!(
        surface.status,
        EncounterSurfaceStatus::Blocked | EncounterSurfaceStatus::Locked
    ) || matches!(
        availability,
        Some(AffordanceAvailability::Blocked | AffordanceAvailability::RequiresCondition)
    ) {
        shortcuts.push("조건을 바꾸지 않고 막힌 표면 통과".to_owned());
    }
    shortcuts
}

fn movement_affordance(
    location_graph: &LocationGraphPacket,
    pressure_refs: &[String],
) -> AffordanceNode {
    let source_refs = location_graph.known_nearby_locations.first().map_or_else(
        || {
            location_graph
                .current_location
                .as_ref()
                .map(location_source_refs)
                .unwrap_or_default()
        },
        location_source_refs,
    );
    AffordanceNode {
        schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
        slot: 1,
        affordance_id: "affordance:slot:1:move".to_owned(),
        affordance_kind: AffordanceKind::Move,
        label_contract: "이동/접근".to_owned(),
        action_contract:
            "현재 위치나 알려진 인접 장소가 허용하는 이동, 접근, 거리 조절만 제시한다.".to_owned(),
        source_refs,
        pressure_refs: pressure_refs.to_vec(),
        forbidden_shortcuts: vec![
            "지도에 없는 지름길 확정".to_owned(),
            "보이지 않는 안전지대 발명".to_owned(),
        ],
    }
}

fn observation_affordance(
    scene_pressure: &ScenePressurePacket,
    location_graph: &LocationGraphPacket,
    pressure_refs: &[String],
) -> AffordanceNode {
    let mut source_refs = scene_pressure_signal_refs(scene_pressure);
    if source_refs.is_empty() {
        source_refs = location_graph
            .current_location
            .as_ref()
            .map(location_source_refs)
            .unwrap_or_default();
    }
    AffordanceNode {
        schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
        slot: 2,
        affordance_id: "affordance:slot:2:observe".to_owned(),
        affordance_kind: AffordanceKind::Observe,
        label_contract: "관찰/확인".to_owned(),
        action_contract:
            "이미 보이는 신호, 흔적, 몸 감각, 장소 메모를 더 자세히 읽는 행동만 제시한다."
                .to_owned(),
        source_refs,
        pressure_refs: pressure_refs.to_vec(),
        forbidden_shortcuts: vec![
            "장면 밖 배경지식 획득".to_owned(),
            "정답 해설식 추리 확정".to_owned(),
        ],
    }
}

fn contact_affordance(
    scene_pressure: &ScenePressurePacket,
    pressure_refs: &[String],
) -> AffordanceNode {
    let social_refs = scene_pressure
        .visible_active
        .iter()
        .filter(|pressure| {
            matches!(
                pressure.kind,
                ScenePressureKind::SocialPermission
                    | ScenePressureKind::Threat
                    | ScenePressureKind::Desire
                    | ScenePressureKind::MoralCost
            )
        })
        .map(|pressure| pressure.pressure_id.clone())
        .collect::<Vec<_>>();
    AffordanceNode {
        schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
        slot: 3,
        affordance_id: "affordance:slot:3:contact".to_owned(),
        affordance_kind: AffordanceKind::Contact,
        label_contract: "접촉/반응".to_owned(),
        action_contract: "현장에 있는 사람, 기척, 시선, 사회적 위험에 반응하는 행동만 제시한다."
            .to_owned(),
        source_refs: social_refs,
        pressure_refs: pressure_refs.to_vec(),
        forbidden_shortcuts: vec![
            "존재하지 않는 조력자 호출".to_owned(),
            "사회적 허가 없는 친밀 행동".to_owned(),
        ],
    }
}

fn body_resource_affordance(
    body_resource: &BodyResourcePacket,
    pressure_refs: &[String],
) -> AffordanceNode {
    let mut source_refs = body_resource
        .body_constraints
        .iter()
        .flat_map(|constraint| constraint.source_refs.clone())
        .collect::<Vec<_>>();
    source_refs.extend(
        body_resource
            .resources
            .iter()
            .flat_map(resource_source_refs),
    );
    AffordanceNode {
        schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
        slot: 4,
        affordance_id: "affordance:slot:4:body_resource".to_owned(),
        affordance_kind: AffordanceKind::ResourceOrBody,
        label_contract: "몸/자원".to_owned(),
        action_contract: "현재 몸 상태와 실제 보유 자원으로 가능한 행동만 제시한다.".to_owned(),
        source_refs,
        pressure_refs: pressure_refs.to_vec(),
        forbidden_shortcuts: vec!["없는 도구 사용".to_owned(), "몸 제약 무시".to_owned()],
    }
}

fn pressure_response_affordance(
    scene_pressure: &ScenePressurePacket,
    pressure_refs: &[String],
) -> AffordanceNode {
    let source_refs = strongest_pressure(scene_pressure)
        .map(|pressure| vec![pressure.pressure_id.clone()])
        .unwrap_or_default();
    AffordanceNode {
        schema_version: AFFORDANCE_NODE_SCHEMA_VERSION.to_owned(),
        slot: 5,
        affordance_id: "affordance:slot:5:pressure_response".to_owned(),
        affordance_kind: AffordanceKind::PressureResponse,
        label_contract: "압력 대응".to_owned(),
        action_contract:
            "가장 강한 현재 압력에 대응해 위험, 시간, 욕망, 비용을 조정하는 행동만 제시한다."
                .to_owned(),
        source_refs,
        pressure_refs: pressure_refs.to_vec(),
        forbidden_shortcuts: vec![
            "압력 무시 후 안전한 관망".to_owned(),
            "숨은 판정 조건 노출".to_owned(),
        ],
    }
}

fn visible_pressure_refs(scene_pressure: &ScenePressurePacket) -> Vec<String> {
    scene_pressure
        .visible_active
        .iter()
        .map(|pressure| pressure.pressure_id.clone())
        .collect()
}

fn scene_pressure_signal_refs(scene_pressure: &ScenePressurePacket) -> Vec<String> {
    scene_pressure
        .visible_active
        .iter()
        .flat_map(|pressure| pressure.observable_signals.clone())
        .collect()
}

fn strongest_pressure(scene_pressure: &ScenePressurePacket) -> Option<&ScenePressure> {
    scene_pressure
        .visible_active
        .iter()
        .max_by_key(|pressure| pressure.intensity)
}

fn location_source_refs(location: &LocationNode) -> Vec<String> {
    let mut refs = location.source_refs.clone();
    refs.push(location.location_id.clone());
    refs
}

fn resource_source_refs(resource: &ResourceItem) -> Vec<String> {
    let mut refs = resource.source_refs.clone();
    refs.push(resource.resource_id.clone());
    refs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body_resource::{
        BODY_CONSTRAINT_SCHEMA_VERSION, BODY_RESOURCE_PACKET_SCHEMA_VERSION, BodyConstraint,
        BodyResourcePolicy, BodyResourceVisibility, RESOURCE_ITEM_SCHEMA_VERSION, ResourceItem,
        ResourceKind,
    };
    use crate::location_graph::{
        LOCATION_GRAPH_PACKET_SCHEMA_VERSION, LOCATION_NODE_SCHEMA_VERSION, LocationGraphPolicy,
        LocationKnowledgeState, LocationNode,
    };
    use crate::scene_pressure::{
        SCENE_PRESSURE_PACKET_SCHEMA_VERSION, SCENE_PRESSURE_SCHEMA_VERSION, ScenePressurePolicy,
        ScenePressureProseEffect, ScenePressureUrgency, ScenePressureVisibility,
    };

    #[test]
    fn compiles_five_ordinary_affordances_from_visible_state() {
        let graph = compile_affordance_graph_packet(
            "stw_affordance",
            "turn_0002",
            &ScenePressurePacket {
                schema_version: SCENE_PRESSURE_PACKET_SCHEMA_VERSION.to_owned(),
                world_id: "stw_affordance".to_owned(),
                turn_id: "turn_0002".to_owned(),
                visible_active: vec![ScenePressure {
                    schema_version: SCENE_PRESSURE_SCHEMA_VERSION.to_owned(),
                    pressure_id: "pressure:threat:noise".to_owned(),
                    kind: ScenePressureKind::Threat,
                    visibility: ScenePressureVisibility::PlayerVisible,
                    intensity: 4,
                    urgency: ScenePressureUrgency::Immediate,
                    source_refs: vec!["turn:0001".to_owned()],
                    provenance: None,
                    observable_signals: vec!["문밖 발소리".to_owned()],
                    choice_affordances: vec!["listen".to_owned()],
                    prose_effect: ScenePressureProseEffect {
                        paragraph_pressure: "short".to_owned(),
                        sensory_focus: vec!["sound".to_owned()],
                        dialogue_style: "low".to_owned(),
                    },
                }],
                hidden_adjudication_only: Vec::new(),
                compiler_policy: ScenePressurePolicy::default(),
            },
            &BodyResourcePacket {
                schema_version: BODY_RESOURCE_PACKET_SCHEMA_VERSION.to_owned(),
                world_id: "stw_affordance".to_owned(),
                turn_id: "turn_0002".to_owned(),
                body_constraints: vec![BodyConstraint {
                    schema_version: BODY_CONSTRAINT_SCHEMA_VERSION.to_owned(),
                    constraint_id: "body:constraint:00".to_owned(),
                    visibility: BodyResourceVisibility::PlayerVisible,
                    summary: "왼손이 저리다".to_owned(),
                    severity: 2,
                    source_refs: vec!["body:0".to_owned()],
                    scene_pressure_kinds: vec!["body".to_owned()],
                }],
                resources: vec![ResourceItem {
                    schema_version: RESOURCE_ITEM_SCHEMA_VERSION.to_owned(),
                    resource_id: "resource:inventory:00".to_owned(),
                    visibility: BodyResourceVisibility::PlayerVisible,
                    summary: "짧은 끈".to_owned(),
                    resource_kind: ResourceKind::Tool,
                    source_refs: vec!["inventory:0".to_owned()],
                }],
                compiler_policy: BodyResourcePolicy::default(),
            },
            &LocationGraphPacket {
                schema_version: LOCATION_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
                world_id: "stw_affordance".to_owned(),
                turn_id: "turn_0002".to_owned(),
                current_location: Some(LocationNode {
                    schema_version: LOCATION_NODE_SCHEMA_VERSION.to_owned(),
                    location_id: "place:room".to_owned(),
                    name: "방".to_owned(),
                    knowledge_state: LocationKnowledgeState::Visited,
                    notes: Vec::new(),
                    source_refs: vec!["place:room:source".to_owned()],
                }),
                known_nearby_locations: Vec::new(),
                compiler_policy: LocationGraphPolicy::default(),
            },
        );

        assert_eq!(graph.ordinary_choice_slots.len(), 5);
        assert_eq!(graph.ordinary_choice_slots[0].slot, 1);
        assert_eq!(
            graph.ordinary_choice_slots[3].affordance_kind,
            AffordanceKind::ResourceOrBody
        );
        assert!(
            graph.ordinary_choice_slots[3]
                .source_refs
                .contains(&"body:0".to_owned())
        );
        assert!(
            graph.ordinary_choice_slots[4]
                .source_refs
                .contains(&"pressure:threat:noise".to_owned())
        );
    }

    #[test]
    fn encounter_surface_overrides_generic_affordance_slots() {
        let encounter = EncounterSurfacePacket {
            schema_version: crate::encounter_surface::ENCOUNTER_SURFACE_PACKET_SCHEMA_VERSION
                .to_owned(),
            world_id: "stw_affordance".to_owned(),
            turn_id: "turn_0002".to_owned(),
            scene_id: "place:room".to_owned(),
            active_surfaces:
                (1..=5)
                    .map(|slot| EncounterSurface {
                        schema_version: crate::encounter_surface::ENCOUNTER_SURFACE_SCHEMA_VERSION
                            .to_owned(),
                        surface_id: format!("encounter:surface:{slot}"),
                        label: format!("표면 {slot}"),
                        kind: if slot == 2 {
                            EncounterSurfaceKind::EvidenceTrace
                        } else {
                            EncounterSurfaceKind::EnvironmentalFeature
                        },
                        status: EncounterSurfaceStatus::Available,
                        salience: crate::encounter_surface::EncounterSalience::Useful,
                        summary: format!("표면 {slot}을 건드린다"),
                        player_visible_signal: format!("표면 {slot}"),
                        location_ref: Some("place:room".to_owned()),
                        holder_ref: None,
                        source_refs: vec![format!("next_choices[slot={slot}]")],
                        linked_entity_refs: Vec::new(),
                        linked_pressure_refs: Vec::new(),
                        linked_social_refs: Vec::new(),
                        affordances: vec![crate::encounter_surface::EncounterAffordance {
                            schema_version:
                                crate::encounter_surface::ENCOUNTER_AFFORDANCE_SCHEMA_VERSION
                                    .to_owned(),
                            affordance_id: format!("encounter:surface:{slot}:inspect"),
                            action_kind: EncounterActionKind::Inspect,
                            label_seed: format!("표면 {slot}"),
                            intent_seed: format!("표면 {slot}을 확인한다"),
                            availability: AffordanceAvailability::Available,
                            required_refs: Vec::new(),
                            risk_tags: Vec::new(),
                            evidence_refs: vec![format!("next_choices[slot={slot}]")],
                        }],
                        constraints: Vec::new(),
                        change_potential: Vec::new(),
                        lifecycle: crate::encounter_surface::EncounterSurfaceLifecycle {
                            opened_turn_id: "turn_0002".to_owned(),
                            last_changed_turn_id: "turn_0002".to_owned(),
                            persistence:
                                crate::encounter_surface::EncounterPersistence::CurrentScene,
                        },
                    })
                    .collect(),
            recent_surface_changes: Vec::new(),
            blocked_interactions: Vec::new(),
            required_followups: Vec::new(),
            compiler_policy: crate::encounter_surface::EncounterSurfacePolicy::default(),
        };
        let graph = compile_affordance_graph_packet_with_encounter(
            "stw_affordance",
            "turn_0002",
            &ScenePressurePacket::default(),
            &BodyResourcePacket::default(),
            &LocationGraphPacket::default(),
            Some(&encounter),
        );

        assert_eq!(
            graph.compiler_policy.source,
            "compiled_from_active_encounter_surface_with_visible_fallbacks_v1"
        );
        assert_eq!(
            graph.ordinary_choice_slots[1].affordance_kind,
            AffordanceKind::Observe
        );
    }
}
