use serde::{Deserialize, Serialize};

pub const SINGULARI_WORLD_SCHEMA_VERSION: &str = "singulari.world.v1";
pub const WORLD_SEED_SCHEMA_VERSION: &str = "singulari.world_seed.v1";
pub const CANON_EVENT_SCHEMA_VERSION: &str = "singulari.canon_event.v1";
pub const HIDDEN_STATE_SCHEMA_VERSION: &str = "singulari.hidden_state.v1";
pub const ENTITY_RECORDS_SCHEMA_VERSION: &str = "singulari.entities.v1";
pub const TURN_SNAPSHOT_SCHEMA_VERSION: &str = "singulari.turn_snapshot.v1";
pub const PLAYER_KNOWLEDGE_SCHEMA_VERSION: &str = "singulari.player_knowledge.v1";
pub const RENDER_PACKET_SCHEMA_VERSION: &str = "singulari.render_packet.v1";
pub const TURN_LOG_ENTRY_SCHEMA_VERSION: &str = "singulari.turn_log_entry.v1";
pub const ADJUDICATION_SCHEMA_VERSION: &str = "singulari.adjudication.v1";
pub const CODEX_VIEW_SCHEMA_VERSION: &str = "singulari.codex_view.v1";
pub const ENTITY_UPDATE_SCHEMA_VERSION: &str = "singulari.entity_update.v1";
pub const NARRATIVE_SCENE_SCHEMA_VERSION: &str = "singulari.narrative_scene.v1";

pub const ANCHOR_CHARACTER_INVARIANT: &str = "anchor_character";
pub const ANCHOR_CHARACTER_ID: &str = "char:anchor";
pub const PROTAGONIST_CHARACTER_ID: &str = "char:protagonist";
pub const OPENING_LOCATION_ID: &str = "place:opening_location";
pub const INITIAL_TURN_ID: &str = "turn_0000";
pub const INITIAL_EVENT_ID: &str = "evt_000000";
pub const DEFAULT_CHOICE_COUNT: usize = 7;
pub const FREEFORM_CHOICE_SLOT: u8 = 6;
pub const FREEFORM_CHOICE_TAG: &str = "자유서술";
pub const GUIDE_CHOICE_SLOT: u8 = 7;
pub const GUIDE_CHOICE_TAG: &str = "판단 위임";
pub const LEGACY_GUIDE_CHOICE_TAG: &str = "안내자의 선택";
pub const GUIDE_CHOICE_REDACTED_INTENT: &str = "맡긴다. 세부 내용은 선택 후 드러난다.";

const DEPRECATED_GUIDE_CHOICE_HINTS: &[(&str, &str)] = &[
    (
        "안내자가 보기에 가장 덜 무모하고 가장 의미 있는 길을 따른다",
        GUIDE_CHOICE_REDACTED_INTENT,
    ),
    (
        "안내자의 선택은 현재 상태에서 가장 덜 무모한 길을 가리킨다",
        "판단 위임은 선택 전에는 세부 내용이 드러나지 않는다",
    ),
    (
        "주인공이 안내자의 최선 후보를 따른 기록",
        "주인공이 판단 위임에 맡긴 기록",
    ),
    ("4번 [안내자의 선택]", "7번 [판단 위임]"),
    ("4번 [판단 위임]", "7번 [판단 위임]"),
    ("안내자의 최선 후보", "판단 위임"),
    ("안내자의 선택", GUIDE_CHOICE_TAG),
    ("앵커 인물", "아직 정해지지 않은 극점"),
];

/// Remove pre-redaction Guide-choice hints from player-facing projections.
///
/// Old worlds may already contain earlier wording in canon/history rows. Keep
/// the evidence files intact, but never re-render those hints as guidance.
#[must_use]
pub fn redact_guide_choice_public_hints(text: &str) -> String {
    let mut redacted = text.to_owned();
    for (needle, replacement) in DEPRECATED_GUIDE_CHOICE_HINTS {
        if redacted.contains(needle) {
            redacted = redacted.replace(needle, replacement);
        }
    }
    redacted
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldSeed {
    #[serde(default = "default_world_seed_schema")]
    pub schema_version: String,
    pub world_id: String,
    pub title: String,
    #[serde(default = "default_created_by")]
    pub created_by: String,
    #[serde(default)]
    pub runtime_contract: RuntimeContract,
    pub premise: WorldPremise,
    #[serde(default)]
    pub anchor_character: AnchorCharacter,
    #[serde(default)]
    pub language: LanguagePolicy,
    #[serde(default)]
    pub laws: WorldLaws,
    #[serde(default)]
    pub non_goals: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeContract {
    #[serde(default = "default_codex_source")]
    pub codex_source: String,
    #[serde(default = "default_runtime_mode")]
    pub mode: String,
}

impl Default for RuntimeContract {
    fn default() -> Self {
        Self {
            codex_source: default_codex_source(),
            mode: default_runtime_mode(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldPremise {
    pub genre: String,
    pub protagonist: String,
    #[serde(default)]
    pub special_condition: Option<String>,
    #[serde(default = "default_opening_state")]
    pub opening_state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorCharacter {
    #[serde(default = "default_anchor_invariant")]
    pub invariant: String,
    #[serde(default = "default_anchor_display_role")]
    pub display_role: String,
    #[serde(default = "default_anchor_world_relation")]
    pub relationship_to_world: String,
    #[serde(default = "default_anchor_relation")]
    pub relationship_to_guide: String,
}

impl Default for AnchorCharacter {
    fn default() -> Self {
        Self {
            invariant: default_anchor_invariant(),
            display_role: default_anchor_display_role(),
            relationship_to_world: default_anchor_world_relation(),
            relationship_to_guide: default_anchor_relation(),
        }
    }
}

impl AnchorCharacter {
    #[must_use]
    pub fn normalized(mut self) -> Self {
        apply_default_when_empty(&mut self.invariant, ANCHOR_CHARACTER_INVARIANT);
        apply_default_when_empty(&mut self.display_role, "미정 극점");
        apply_default_when_empty(
            &mut self.relationship_to_world,
            "플레이어-visible 사건에서 정해지는 극점 후보",
        );
        apply_default_when_empty(&mut self.relationship_to_guide, "unresolved dramatic focus");
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LanguagePolicy {
    #[serde(default = "default_user_language")]
    pub user_language: String,
    #[serde(default = "default_output_mode")]
    pub default_output_mode: String,
}

impl Default for LanguagePolicy {
    fn default() -> Self {
        Self {
            user_language: default_user_language(),
            default_output_mode: default_output_mode(),
        }
    }
}

// Why: these fields mirror the Singulari World seed law panel; using booleans keeps seed files readable.
#[allow(
    clippy::struct_excessive_bools,
    reason = "world law records intentionally preserve user-facing boolean law toggles"
)]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldLaws {
    #[serde(default = "default_true")]
    pub death_is_final: bool,
    #[serde(default = "default_true")]
    pub discovery_required: bool,
    #[serde(default = "default_true")]
    pub bodily_needs_active: bool,
    #[serde(default = "default_true")]
    pub miracles_forbidden: bool,
}

impl Default for WorldLaws {
    fn default() -> Self {
        Self {
            death_is_final: true,
            discovery_required: true,
            bodily_needs_active: true,
            miracles_forbidden: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldRecord {
    pub schema_version: String,
    pub world_id: String,
    pub title: String,
    pub created_by: String,
    pub runtime_contract: RuntimeContract,
    pub premise: WorldPremise,
    pub anchor_character: AnchorCharacter,
    pub language: LanguagePolicy,
    pub laws: WorldLaws,
    pub non_goals: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl WorldRecord {
    #[must_use]
    pub fn from_seed(seed: WorldSeed, created_at: String) -> Self {
        let anchor_character = seed.anchor_character.normalized();
        Self {
            schema_version: SINGULARI_WORLD_SCHEMA_VERSION.to_owned(),
            world_id: seed.world_id,
            title: seed.title,
            created_by: seed.created_by,
            runtime_contract: seed.runtime_contract,
            premise: seed.premise,
            anchor_character,
            language: seed.language,
            laws: seed.laws,
            non_goals: seed.non_goals,
            created_at: created_at.clone(),
            updated_at: created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonEvent {
    pub schema_version: String,
    pub event_id: String,
    pub world_id: String,
    pub turn_id: String,
    pub occurred_at_world_time: String,
    pub visibility: String,
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub evidence: EventEvidence,
    #[serde(default)]
    pub consequences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEvidence {
    pub source: String,
    pub user_input: String,
    pub narrative_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiddenState {
    pub schema_version: String,
    pub world_id: String,
    #[serde(default)]
    pub secrets: Vec<HiddenStateSecret>,
    #[serde(default)]
    pub timers: Vec<HiddenStateTimer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiddenStateSecret {
    pub secret_id: String,
    pub status: String,
    pub truth: String,
    #[serde(default)]
    pub reveal_conditions: Vec<String>,
    #[serde(default)]
    pub forbidden_leaks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HiddenStateTimer {
    pub timer_id: String,
    pub kind: String,
    pub remaining_turns: u32,
    pub effect: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecords {
    pub schema_version: String,
    pub world_id: String,
    #[serde(default)]
    pub characters: Vec<CharacterRecord>,
    #[serde(default)]
    pub places: Vec<PlaceRecord>,
    #[serde(default)]
    pub factions: Vec<NamedEntity>,
    #[serde(default)]
    pub items: Vec<NamedEntity>,
    #[serde(default)]
    pub concepts: Vec<NamedEntity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterRecord {
    pub id: String,
    pub name: EntityName,
    pub role: String,
    pub knowledge_state: String,
    pub traits: TraitSet,
    #[serde(default, skip_serializing_if = "CharacterVoiceAnchor::is_empty")]
    pub voice_anchor: CharacterVoiceAnchor,
    pub body: CharacterBody,
    #[serde(default)]
    pub history: Vec<String>,
    #[serde(default)]
    pub relationships: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityName {
    pub visible: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraitSet {
    #[serde(default)]
    pub confirmed: Vec<String>,
    #[serde(default)]
    pub rumored: Vec<String>,
    #[serde(default)]
    pub hidden: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterBody {
    #[serde(default)]
    pub injuries: Vec<String>,
    pub needs: BodyNeeds,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CharacterVoiceAnchor {
    #[serde(default)]
    pub speech: Vec<String>,
    #[serde(default)]
    pub endings: Vec<String>,
    #[serde(default)]
    pub tone: Vec<String>,
    #[serde(default)]
    pub gestures: Vec<String>,
    #[serde(default)]
    pub habits: Vec<String>,
    #[serde(default)]
    pub drift: Vec<String>,
}

impl CharacterVoiceAnchor {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.speech.is_empty()
            && self.endings.is_empty()
            && self.tone.is_empty()
            && self.gestures.is_empty()
            && self.habits.is_empty()
            && self.drift.is_empty()
    }

    #[must_use]
    pub fn protagonist_default() -> Self {
        Self {
            speech: vec![
                "모르는 세계 지식은 단정하지 않고 확인한 단서부터 말한다".to_owned(),
                "모르는 세계 지식은 단정하지 않고 조건을 먼저 세운다".to_owned(),
            ],
            endings: vec![
                "긴장할수록 짧은 평서문과 낮은 의문문으로 끊는다".to_owned(),
                "결론을 단정하기보다 '~같다', '~부터 보자'처럼 확인의 여지를 남긴다".to_owned(),
            ],
            tone: vec![
                "자기 확신보다 관찰 순서를 앞세우는 절제된 어투를 쓴다".to_owned(),
                "모르는 일을 아는 척하지 않고, 필요한 말만 낮게 덧붙인다".to_owned(),
            ],
            gestures: vec![
                "이름, 기록, 동의가 얽힐 때 왼손목을 누르거나 가린다".to_owned(),
                "위험한 선택 전 주변 단서와 사람의 반응을 먼저 본다".to_owned(),
            ],
            habits: vec![
                "능력이나 선택의 대가와 흔적을 먼저 의식한다".to_owned(),
                "확신보다 관찰을 앞세우고 모르는 부분은 유보한다".to_owned(),
            ],
            drift: vec!["상황 파악에서 자기 이름과 선택에 책임지는 선언으로 이동한다".to_owned()],
        }
    }

    #[must_use]
    pub fn anchor_default() -> Self {
        Self {
            speech: vec![
                "짧고 담담하게 말하며 감정 설명보다 판단을 먼저 둔다".to_owned(),
                "숨겨진 진실은 과잉 설명하지 않고 필요한 만큼만 연다".to_owned(),
            ],
            endings: vec![
                "짧은 평서와 여백 있는 명령형을 쓰되, 해설형 장광설을 피한다".to_owned(),
                "상대가 선택해야 할 순간에는 말끝을 끊어 직접 판단할 공간을 남긴다".to_owned(),
            ],
            tone: vec![
                "보호와 경고가 섞인 낮고 담담한 어투를 유지한다".to_owned(),
                "친밀함은 설명보다 거리 조절과 짧은 확인으로 드러낸다".to_owned(),
            ],
            gestures: vec![
                "위험할수록 손목이나 표식을 감추고, 보호할 때 한 걸음 앞으로 선다".to_owned(),
                "가까이 두되 함부로 닿지 않는 거리감으로 신뢰를 표현한다".to_owned(),
            ],
            habits: vec![
                "대답 전 짧은 침묵으로 말해도 되는 경계를 잰다".to_owned(),
                "상대의 선택권을 지키는 방식으로 도움을 준다".to_owned(),
            ],
            drift: vec![
                "신뢰가 쌓이면 설명이 길어지는 대신 말하지 않을 권리를 더 솔직히 드러낸다"
                    .to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BodyNeeds {
    pub hunger: String,
    pub thirst: String,
    pub fatigue: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceRecord {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<Coordinates>,
    pub known_to_protagonist: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Coordinates {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamedEntity {
    pub id: String,
    pub name: String,
    pub known_to_protagonist: bool,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerKnowledge {
    pub schema_version: String,
    pub world_id: String,
    #[serde(default)]
    pub known_entities: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSnapshot {
    pub schema_version: String,
    pub world_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_event: Option<CurrentEvent>,
    pub protagonist_state: ProtagonistState,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub last_choices: Vec<TurnChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentEvent {
    pub event_id: String,
    pub progress: String,
    pub rail_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtagonistState {
    pub location: String,
    #[serde(default)]
    pub inventory: Vec<String>,
    #[serde(default)]
    pub body: Vec<String>,
    #[serde(default)]
    pub mind: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnChoice {
    pub slot: u8,
    pub tag: String,
    pub intent: String,
}

impl TurnChoice {
    #[must_use]
    pub fn player_visible_intent(&self) -> &str {
        if is_guide_choice_tag(self.tag.as_str()) {
            GUIDE_CHOICE_REDACTED_INTENT
        } else {
            self.intent.as_str()
        }
    }
}

#[must_use]
pub fn is_guide_choice_tag(tag: &str) -> bool {
    tag == GUIDE_CHOICE_TAG || tag == LEGACY_GUIDE_CHOICE_TAG
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnInputKind {
    NumericChoice,
    GuideChoice,
    FreeformAction,
    CodexQuery,
    MacroTimeFlow,
    CcCanvas,
}

impl TurnInputKind {
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::NumericChoice => "numeric_choice",
            Self::GuideChoice => "guide_choice",
            Self::FreeformAction => "freeform_action",
            Self::CodexQuery => "codex_query",
            Self::MacroTimeFlow => "macro_time_flow",
            Self::CcCanvas => "cc_canvas",
        }
    }
}

impl std::fmt::Display for TurnInputKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_wire())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderPacket {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub mode: String,
    pub narrative_contract: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub narrative_scene: Option<NarrativeScene>,
    pub visible_state: VisibleState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adjudication: Option<AdjudicationReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codex_view: Option<CodexView>,
    pub canon_delta_refs: Vec<String>,
    pub forbidden_reveals: Vec<String>,
    pub style_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeScene {
    #[serde(default = "default_narrative_scene_schema")]
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default)]
    pub text_blocks: Vec<String>,
    #[serde(default)]
    pub tone_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisibleState {
    pub dashboard: DashboardSummary,
    pub scan_targets: Vec<ScanTarget>,
    pub choices: Vec<TurnChoice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSummary {
    pub phase: String,
    pub location: String,
    pub anchor_invariant: String,
    pub current_event: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanTarget {
    pub target: String,
    pub class: String,
    pub distance: String,
    pub thought: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnLogEntry {
    pub schema_version: String,
    pub world_id: String,
    pub session_id: String,
    pub turn_id: String,
    pub input: String,
    pub input_kind: TurnInputKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_choice: Option<TurnChoice>,
    pub canon_event_id: String,
    pub snapshot_ref: String,
    pub render_packet_ref: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjudicationReport {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub outcome: String,
    pub summary: String,
    #[serde(default)]
    pub gates: Vec<AdjudicationGate>,
    #[serde(default)]
    pub visible_constraints: Vec<String>,
    #[serde(default)]
    pub consequences: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjudicationGate {
    pub gate: String,
    pub status: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexView {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub title: String,
    #[serde(default)]
    pub protagonist_timeline: Vec<CodexTimelineEntry>,
    #[serde(default)]
    pub world_almanac: Vec<CodexFactEntry>,
    #[serde(default)]
    pub world_blueprint: Vec<CodexEntityEntry>,
    #[serde(default)]
    pub voice_anchors: Vec<CodexVoiceAnchorEntry>,
    #[serde(default)]
    pub local_faces: Vec<CodexLocalFaceEntry>,
    #[serde(default)]
    pub realtime_analysis: Vec<CodexAnalysisEntry>,
    #[serde(default)]
    pub related_recommendations: Vec<CodexRecommendation>,
    pub hidden_filter: CodexHiddenFilter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexTimelineEntry {
    pub turn_id: String,
    pub event_id: String,
    pub kind: String,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexFactEntry {
    pub fact_id: String,
    pub category: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexEntityEntry {
    pub entity_id: String,
    pub entity_type: String,
    pub name: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexVoiceAnchorEntry {
    pub character_id: String,
    pub name: String,
    #[serde(default)]
    pub speech: Vec<String>,
    #[serde(default)]
    pub endings: Vec<String>,
    #[serde(default)]
    pub tone: Vec<String>,
    #[serde(default)]
    pub gestures: Vec<String>,
    #[serde(default)]
    pub habits: Vec<String>,
    #[serde(default)]
    pub drift: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexLocalFaceEntry {
    pub extra_id: String,
    pub display_name: String,
    pub role: String,
    pub home_location_id: String,
    pub last_seen_turn: String,
    pub disposition: String,
    pub last_contact: String,
    #[serde(default)]
    pub open_hooks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAnalysisEntry {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexRecommendation {
    pub source: String,
    pub target: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexHiddenFilter {
    pub hidden_secrets: usize,
    pub hidden_timers: usize,
    pub policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredEntityUpdates {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub entity_updates: Vec<EntityUpdateRecord>,
    #[serde(default)]
    pub relationship_updates: Vec<RelationshipUpdateRecord>,
}

impl StructuredEntityUpdates {
    #[must_use]
    pub fn empty(world_id: &str, turn_id: &str) -> Self {
        Self {
            schema_version: ENTITY_UPDATE_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            turn_id: turn_id.to_owned(),
            entity_updates: Vec::new(),
            relationship_updates: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityUpdateRecord {
    pub update_id: String,
    pub world_id: String,
    pub turn_id: String,
    pub entity_id: String,
    pub update_kind: String,
    pub visibility: String,
    pub summary: String,
    pub source_event_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelationshipUpdateRecord {
    pub update_id: String,
    pub world_id: String,
    pub turn_id: String,
    pub source_entity_id: String,
    pub target_entity_id: String,
    pub relation_kind: String,
    pub visibility: String,
    pub summary: String,
    pub source_event_id: String,
    pub created_at: String,
}

impl EntityRecords {
    #[must_use]
    pub fn initial(world: &WorldRecord) -> Self {
        Self {
            schema_version: ENTITY_RECORDS_SCHEMA_VERSION.to_owned(),
            world_id: world.world_id.clone(),
            characters: vec![initial_protagonist(world), initial_anchor_character(world)],
            places: vec![PlaceRecord {
                id: OPENING_LOCATION_ID.to_owned(),
                name: "미정".to_owned(),
                coordinates: None,
                known_to_protagonist: true,
                notes: vec!["첫 장면이 시작되면 구체화된다".to_owned()],
            }],
            factions: Vec::new(),
            items: Vec::new(),
            concepts: vec![NamedEntity {
                id: "concept:dramatic_focus".to_owned(),
                name: "Unresolved dramatic focus".to_owned(),
                known_to_protagonist: false,
                notes: vec![
                    "첫 극점은 인물, 장소, 물건, 세력, 맹세, 위협, 질문 중 플레이어-visible 사건에서 떠오른다".to_owned(),
                    "시드가 명시하지 않은 숨은 인물이나 운명적 안내자를 자동 생성하지 않는다".to_owned(),
                ],
            }],
        }
    }
}

impl HiddenState {
    #[must_use]
    pub fn initial(world_id: &str) -> Self {
        Self {
            schema_version: HIDDEN_STATE_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            secrets: vec![HiddenStateSecret {
                secret_id: "sec_dramatic_focus_unresolved_001".to_owned(),
                status: "veiled".to_owned(),
                truth: "The initial dramatic focus is unresolved. It may become a character, place, object, faction, oath, threat, or question only after player-visible evidence establishes it.".to_owned(),
                reveal_conditions: vec![
                    "the seed explicitly declares a focus".to_owned(),
                    "player-visible scenes establish a repeated pressure center".to_owned(),
                ],
                forbidden_leaks: vec![
                    "do not assume the focus is a hidden character".to_owned(),
                    "do not turn sparse seeds into reincarnation, system, cheat, or destined-guide stories".to_owned(),
                ],
            }],
            timers: Vec::new(),
        }
    }
}

impl PlayerKnowledge {
    #[must_use]
    pub fn initial(world_id: &str) -> Self {
        Self {
            schema_version: PLAYER_KNOWLEDGE_SCHEMA_VERSION.to_owned(),
            world_id: world_id.to_owned(),
            known_entities: vec![
                PROTAGONIST_CHARACTER_ID.to_owned(),
                OPENING_LOCATION_ID.to_owned(),
            ],
            open_questions: vec![
                "첫 장면의 즉시 압력은 아직 구체화되지 않았다".to_owned(),
                "중요한 인물, 장소, 물건, 세력, 위협은 플레이어-visible 사건에서 정해진다"
                    .to_owned(),
            ],
        }
    }
}

impl TurnSnapshot {
    #[must_use]
    pub fn initial(world: &WorldRecord, session_id: String) -> Self {
        Self {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: world.world_id.clone(),
            session_id,
            turn_id: INITIAL_TURN_ID.to_owned(),
            phase: world.premise.opening_state.to_lowercase(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: OPENING_LOCATION_ID.to_owned(),
                inventory: Vec::new(),
                body: Vec::new(),
                mind: vec!["pre-event calm".to_owned()],
            },
            open_questions: vec![
                "첫 사건은 아직 시작되지 않았다".to_owned(),
                "이 세계의 극점은 아직 인물, 장소, 물건, 세력, 위협 중 어디에도 고정되지 않았다"
                    .to_owned(),
            ],
            last_choices: default_turn_choices(),
        }
    }
}

#[must_use]
pub fn default_turn_choices() -> Vec<TurnChoice> {
    vec![
        default_presented_choice(1),
        default_presented_choice(2),
        default_presented_choice(3),
        default_presented_choice(4),
        default_presented_choice(5),
        default_freeform_choice(),
        default_guide_choice(),
    ]
}

#[must_use]
fn default_presented_choice(slot: u8) -> TurnChoice {
    match slot {
        1 => TurnChoice {
            slot,
            tag: "움직임".to_owned(),
            intent: "현재 장소에서 가장 직접적으로 가능한 이동이나 접근을 시도한다".to_owned(),
        },
        2 => TurnChoice {
            slot,
            tag: "살핌".to_owned(),
            intent: "몸, 장소, 물건, 흔적 중 지금 실제로 보이는 것을 살핀다".to_owned(),
        },
        3 => TurnChoice {
            slot,
            tag: "접촉".to_owned(),
            intent: "현장에 있는 사람, 기척, 사회적 신호에 조심스럽게 반응한다".to_owned(),
        },
        4 => TurnChoice {
            slot,
            tag: "기록".to_owned(),
            intent: "현재 알려진 세계 기록을 연다".to_owned(),
        },
        _ => TurnChoice {
            slot,
            tag: "흐름".to_owned(),
            intent: "시간, 위험, 주변 움직임이 한 박자 뒤 어떻게 밀려오는지 본다".to_owned(),
        },
    }
}

#[must_use]
pub fn default_freeform_choice() -> TurnChoice {
    TurnChoice {
        slot: FREEFORM_CHOICE_SLOT,
        tag: FREEFORM_CHOICE_TAG.to_owned(),
        intent: "6 뒤에 직접 행동, 말, 내면 판단을 이어서 서술한다".to_owned(),
    }
}

#[must_use]
pub fn default_guide_choice() -> TurnChoice {
    TurnChoice {
        slot: GUIDE_CHOICE_SLOT,
        tag: GUIDE_CHOICE_TAG.to_owned(),
        intent: GUIDE_CHOICE_REDACTED_INTENT.to_owned(),
    }
}

#[must_use]
pub fn normalize_turn_choices(choices: &[TurnChoice]) -> Vec<TurnChoice> {
    let mut ordinary = choices
        .iter()
        .filter(|choice| {
            choice.tag != FREEFORM_CHOICE_TAG && !is_guide_choice_tag(choice.tag.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    ordinary.sort_by_key(|choice| choice.slot);

    let mut normalized = Vec::with_capacity(DEFAULT_CHOICE_COUNT);
    for slot in 1..FREEFORM_CHOICE_SLOT {
        let mut choice = ordinary
            .get(usize::from(slot - 1))
            .cloned()
            .unwrap_or_else(|| default_presented_choice(slot));
        choice.slot = slot;
        normalized.push(choice);
    }

    let mut freeform = choices
        .iter()
        .find(|choice| choice.tag == FREEFORM_CHOICE_TAG)
        .cloned()
        .unwrap_or_else(default_freeform_choice);
    freeform.slot = FREEFORM_CHOICE_SLOT;
    if freeform.intent.contains("7 뒤에") {
        freeform.intent = freeform.intent.replace("7 뒤에", "6 뒤에");
    }
    normalized.push(freeform);

    let mut guide = choices
        .iter()
        .find(|choice| is_guide_choice_tag(choice.tag.as_str()))
        .cloned()
        .unwrap_or_else(default_guide_choice);
    guide.slot = GUIDE_CHOICE_SLOT;
    GUIDE_CHOICE_TAG.clone_into(&mut guide.tag);
    GUIDE_CHOICE_REDACTED_INTENT.clone_into(&mut guide.intent);
    normalized.push(guide);

    normalized
}

#[must_use]
pub fn initial_canon_event(world: &WorldRecord) -> CanonEvent {
    CanonEvent {
        schema_version: CANON_EVENT_SCHEMA_VERSION.to_owned(),
        event_id: INITIAL_EVENT_ID.to_owned(),
        world_id: world.world_id.clone(),
        turn_id: INITIAL_TURN_ID.to_owned(),
        occurred_at_world_time: "prelude".to_owned(),
        visibility: "system".to_owned(),
        kind: "note".to_owned(),
        summary: "World initialized from seed. Dramatic focus starts unresolved.".to_owned(),
        entities: vec![PROTAGONIST_CHARACTER_ID.to_owned()],
        location: Some(OPENING_LOCATION_ID.to_owned()),
        evidence: EventEvidence {
            source: "world_init".to_owned(),
            user_input: "seed".to_owned(),
            narrative_ref: "sessions/*/snapshots/turn_0000.json".to_owned(),
        },
        consequences: vec!["world_created".to_owned()],
    }
}

fn initial_protagonist(world: &WorldRecord) -> CharacterRecord {
    CharacterRecord {
        id: PROTAGONIST_CHARACTER_ID.to_owned(),
        name: EntityName {
            visible: "미정".to_owned(),
            native: None,
        },
        role: format!("주인공 ({})", world.premise.protagonist),
        knowledge_state: "self".to_owned(),
        traits: TraitSet {
            confirmed: vec![world.premise.protagonist.clone()],
            rumored: Vec::new(),
            hidden: Vec::new(),
        },
        voice_anchor: CharacterVoiceAnchor::protagonist_default(),
        body: CharacterBody {
            injuries: Vec::new(),
            needs: BodyNeeds {
                hunger: "humanly sensed".to_owned(),
                thirst: "humanly sensed".to_owned(),
                fatigue: "humanly sensed".to_owned(),
            },
        },
        history: vec!["세계에 막 던져진 상태".to_owned()],
        relationships: Vec::new(),
    }
}

fn initial_anchor_character(world: &WorldRecord) -> CharacterRecord {
    CharacterRecord {
        id: ANCHOR_CHARACTER_ID.to_owned(),
        name: EntityName {
            visible: "미정".to_owned(),
            native: None,
        },
        role: format!(
            "{} / unresolved dramatic focus",
            world.anchor_character.display_role
        ),
        knowledge_state: "veiled".to_owned(),
        traits: TraitSet {
            confirmed: vec![
                "초기 극점은 아직 인물로 고정되지 않았다".to_owned(),
                "플레이어-visible 사건이 반복 압력 중심을 만들 때만 구체화된다".to_owned(),
                format!("anchor invariant: {}", world.anchor_character.invariant),
            ],
            rumored: Vec::new(),
            hidden: vec![
                "극점 종류".to_owned(),
                "반복 압력의 원인".to_owned(),
                "세계와 주인공에게 생기는 대가".to_owned(),
            ],
        },
        voice_anchor: CharacterVoiceAnchor::anchor_default(),
        body: CharacterBody {
            injuries: Vec::new(),
            needs: BodyNeeds {
                hunger: "world-dependent".to_owned(),
                thirst: "world-dependent".to_owned(),
                fatigue: "world-dependent".to_owned(),
            },
        },
        history: vec!["극점 후보 슬롯으로 비활성 대기한다".to_owned()],
        relationships: Vec::new(),
    }
}

fn apply_default_when_empty(value: &mut String, default_value: &str) {
    if value.trim().is_empty() {
        default_value.clone_into(value);
    }
}

fn default_world_seed_schema() -> String {
    WORLD_SEED_SCHEMA_VERSION.to_owned()
}

fn default_created_by() -> String {
    "local_user".to_owned()
}

fn default_codex_source() -> String {
    "runtime_profile".to_owned()
}

fn default_runtime_mode() -> String {
    "worldsim_text_runtime".to_owned()
}

fn default_narrative_scene_schema() -> String {
    NARRATIVE_SCENE_SCHEMA_VERSION.to_owned()
}

fn default_opening_state() -> String {
    "Interlude".to_owned()
}

fn default_anchor_invariant() -> String {
    ANCHOR_CHARACTER_INVARIANT.to_owned()
}

fn default_anchor_display_role() -> String {
    "미정 극점".to_owned()
}

fn default_anchor_world_relation() -> String {
    "플레이어-visible 사건에서 정해지는 극점 후보".to_owned()
}

fn default_anchor_relation() -> String {
    "unresolved dramatic focus".to_owned()
}

fn default_user_language() -> String {
    "ko".to_owned()
}

fn default_output_mode() -> String {
    "authentic_bilingual".to_owned()
}

const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::{
        CharacterVoiceAnchor, FREEFORM_CHOICE_SLOT, FREEFORM_CHOICE_TAG,
        GUIDE_CHOICE_REDACTED_INTENT, GUIDE_CHOICE_SLOT, GUIDE_CHOICE_TAG, LEGACY_GUIDE_CHOICE_TAG,
        TurnChoice, normalize_turn_choices, redact_guide_choice_public_hints,
    };

    #[test]
    fn protagonist_default_voice_anchor_does_not_imply_modern_reincarnation() {
        let anchor = CharacterVoiceAnchor::protagonist_default();
        let joined = [
            anchor.speech.join(" "),
            anchor.endings.join(" "),
            anchor.tone.join(" "),
            anchor.gestures.join(" "),
            anchor.habits.join(" "),
            anchor.drift.join(" "),
        ]
        .join(" ");

        for forbidden in ["현대", "전생", "환생", "빙의", "회귀", "치트"] {
            assert!(
                !joined.contains(forbidden),
                "default protagonist anchor should not inject {forbidden}: {joined}"
            );
        }
    }

    #[test]
    fn redacts_deprecated_guide_choice_hints_from_player_text() {
        let redacted = redact_guide_choice_public_hints(
            "turn_0001: 안내자가 보기에 가장 덜 무모하고 가장 의미 있는 길을 따른다 / 주인공이 안내자의 최선 후보를 따른 기록",
        );

        assert!(redacted.contains(GUIDE_CHOICE_REDACTED_INTENT));
        assert!(redacted.contains("주인공이 판단 위임에 맡긴 기록"));
        assert!(!redacted.contains("가장 덜 무모"));
        assert!(!redacted.contains("최선 후보"));
    }

    #[test]
    fn normalizes_legacy_guide_and_freeform_slots_to_current_contract() {
        let normalized = normalize_turn_choices(&[
            TurnChoice {
                slot: 1,
                tag: "움직임".to_owned(),
                intent: "다가간다".to_owned(),
            },
            TurnChoice {
                slot: 4,
                tag: LEGACY_GUIDE_CHOICE_TAG.to_owned(),
                intent: "안내자가 보기에 가장 덜 무모하고 가장 의미 있는 길을 따른다".to_owned(),
            },
            TurnChoice {
                slot: 5,
                tag: "기록".to_owned(),
                intent: "기록을 본다".to_owned(),
            },
            TurnChoice {
                slot: 6,
                tag: "흐름".to_owned(),
                intent: "흐름을 본다".to_owned(),
            },
            TurnChoice {
                slot: 7,
                tag: FREEFORM_CHOICE_TAG.to_owned(),
                intent: "7 뒤에 직접 행동을 쓴다".to_owned(),
            },
        ]);

        assert_eq!(normalized.len(), 7);
        assert_eq!(normalized[5].slot, FREEFORM_CHOICE_SLOT);
        assert_eq!(normalized[5].tag, FREEFORM_CHOICE_TAG);
        assert!(normalized[5].intent.contains("6 뒤에"));
        assert_eq!(normalized[6].slot, GUIDE_CHOICE_SLOT);
        assert_eq!(normalized[6].tag, GUIDE_CHOICE_TAG);
        assert_eq!(normalized[6].intent, GUIDE_CHOICE_REDACTED_INTENT);
    }
}
