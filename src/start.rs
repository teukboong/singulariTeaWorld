use crate::models::{
    AnchorCharacter, LanguagePolicy, OPENING_RANDOMIZER_SCHEMA_VERSION, OpeningRandomizer,
    RuntimeContract, WORLD_SEED_SCHEMA_VERSION, WorldLaws, WorldPremise, WorldSeed,
};
use crate::store::{ActiveWorldBinding, InitializedWorld, init_world_from_seed, save_active_world};
use anyhow::{Result, bail};
use chrono::Utc;
use std::path::PathBuf;

const DEFAULT_TITLE: &str = "Singulari World World";
const DEFAULT_PROTAGONIST: &str = "미정의 주인공";
const DEFAULT_GENRE: &str = "미정";
const DEFAULT_OPENING_STATE: &str = "Interlude";
const GENERATED_WORLD_ID_PREFIX: &str = "stw";
const MAX_TITLE_CHARS: usize = 32;
const DEFAULT_NON_GOALS: &[&str] = &[
    "초반부터 세계관 백과를 전부 공개하지 않는다",
    "시드가 명시하지 않은 장르 장치나 숨은 구조를 자동으로 만들지 않는다",
    "매 턴 최소 하나의 세계 압력을 움직여 건조한 로그가 되지 않게 한다",
];
const GENRE_HINTS: &[&str] = &[
    "판타지",
    "fantasy",
    "무협",
    "martial",
    "SF",
    "sci-fi",
    "science fiction",
    "현대",
    "modern",
    "중세",
    "medieval",
    "근세",
    "early modern",
    "미래",
    "future",
    "아포칼립스",
    "apocalypse",
    "학원",
    "academy",
    "로맨스",
    "romance",
    "스팀펑크",
    "steampunk",
    "사이버펑크",
    "cyberpunk",
    "던전",
    "dungeon",
    "왕국",
    "kingdom",
    "제국",
    "empire",
];
const PROTAGONIST_HINTS: &[&str] = &[
    "주인공",
    "protagonist",
    "남주",
    "male protagonist",
    "여주",
    "female protagonist",
    "플레이어",
    "player",
    "용사",
    "hero",
    "마법사",
    "mage",
    "wizard",
    "기사",
    "knight",
    "순찰자",
    "patrol",
    "상인",
    "merchant",
];
const SPECIAL_CONDITION_HINTS: &[&str] = &[
    "능력",
    "ability",
    "스킬",
    "skill",
    "가호",
    "blessing",
    "저주",
    "curse",
    "성좌",
    "constellation",
    "특전",
    "boon",
    "각성",
    "awakening",
    "권능",
    "authority",
    "마법",
    "magic",
    "talent",
    "gifted",
];
const OPENING_LOCATION_FRAMES: &[&str] = &[
    "사람이 오가는 경계 지점에서 시작한다",
    "일이 이미 진행 중인 생활 공간에서 시작한다",
    "거래나 통과 절차가 막힌 장소에서 시작한다",
    "이동 중 잠깐 멈춘 길목에서 시작한다",
    "서로를 완전히 믿지 않는 사람들이 모인 곳에서 시작한다",
    "공개된 작업 현장에서 작은 이상이 보이는 순간 시작한다",
    "안쪽보다 바깥의 소리와 시선이 먼저 닿는 장소에서 시작한다",
    "물건이 오가고 책임 소재가 흐린 장소에서 시작한다",
];
const OPENING_PROTAGONIST_FRAMES: &[&str] = &[
    "주인공의 신분보다 지금 맡은 일이 먼저 드러난다",
    "주인공은 설명보다 행동 압력 속에서 현재 위치를 파악한다",
    "주인공은 타인의 시선 속에서 첫 결정을 해야 한다",
    "주인공은 가지고 있는 것보다 잃으면 곤란한 것을 먼저 의식한다",
    "주인공의 말보다 몸의 반응과 주변의 기대가 먼저 보인다",
    "주인공은 상황을 전부 알기 전에 작은 책임을 떠안는다",
];
const OPENING_IMMEDIATE_PRESSURES: &[&str] = &[
    "누군가 지금 당장 답을 요구한다",
    "평범한 절차 하나가 예상 밖으로 멈춘다",
    "작은 물건 하나 때문에 주변 반응이 달라진다",
    "소리나 침묵이 사람들의 움직임을 바꾼다",
    "시간이 짧고, 설명을 기다릴 여유가 없다",
    "사소해 보이는 손상이 곧 책임 문제로 번진다",
    "지금 움직이지 않으면 다른 사람이 먼저 결론을 낸다",
];
const OPENING_VISIBLE_OBJECTS: &[&str] = &[
    "젖은 천",
    "닫히지 않는 문",
    "깨진 손잡이",
    "비어 있는 자루",
    "먼지가 덜 앉은 발자국",
    "묶다 만 끈",
    "잠시 멈춘 수레",
    "손때 묻은 도구",
    "잉크가 마르지 않은 문서",
];
const OPENING_SOCIAL_WEATHERS: &[&str] = &[
    "서로 책임을 미루는 분위기",
    "겉으로는 조용하지만 시선이 날카로운 분위기",
    "누군가를 기다리는 듯한 정체감",
    "작은 실수가 곧 소문이 될 것 같은 긴장",
    "친절과 경계가 섞인 응대",
    "말을 아끼는 사람들 사이의 압박",
];
const OPENING_QUESTIONS: &[&str] = &[
    "무엇을 먼저 확인해야 하는가",
    "누구에게 말을 걸어야 하는가",
    "지금 보이는 물건을 건드려도 되는가",
    "이 장소에 더 머물러야 하는가",
    "책임을 피할지, 받아들일지 정해야 하는가",
    "소리가 난 쪽과 사람들의 시선 중 어느 쪽을 따라야 하는가",
];

#[derive(Debug, Clone)]
pub struct StartWorldOptions {
    pub seed_text: String,
    pub world_id: Option<String>,
    pub title: Option<String>,
    pub randomize_opening_seed: bool,
    pub store_root: Option<PathBuf>,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct StartedWorld {
    pub seed: WorldSeed,
    pub initialized: InitializedWorld,
    pub active_binding: ActiveWorldBinding,
}

/// Start a world from the compact seed text user gives in worldsim chat.
///
/// # Errors
///
/// Returns an error when the compact seed is empty, the generated seed violates
/// world invariants, or the file-backed world initialization fails.
pub fn start_world(options: &StartWorldOptions) -> Result<StartedWorld> {
    let seed = world_seed_from_compact_text(
        options.seed_text.as_str(),
        options.world_id.as_deref(),
        options.title.as_deref(),
        options.randomize_opening_seed,
    )?;
    let initialized = init_world_from_seed(
        seed.clone(),
        options.store_root.as_deref(),
        options.session_id.clone(),
    )?;
    let active_binding = save_active_world(
        options.store_root.as_deref(),
        initialized.world.world_id.as_str(),
        initialized.session_id.as_str(),
    )?;
    Ok(StartedWorld {
        seed,
        initialized,
        active_binding,
    })
}

/// Convert terse worldsim chat seed text into the minimum structured world seed.
///
/// # Errors
///
/// Returns an error when `seed_text` is empty after trimming.
pub fn world_seed_from_compact_text(
    seed_text: &str,
    world_id: Option<&str>,
    title: Option<&str>,
    randomize_opening_seed: bool,
) -> Result<WorldSeed> {
    let seed_text = seed_text.trim();
    if seed_text.is_empty() {
        bail!("singulari-world start requires non-empty seed text");
    }
    let fragments = compact_seed_fragments(seed_text);
    let genre = select_fragment(&fragments, GENRE_HINTS).unwrap_or_else(|| {
        fragments
            .first()
            .cloned()
            .unwrap_or_else(|| DEFAULT_GENRE.to_owned())
    });
    let protagonist = collect_fragments(&fragments, PROTAGONIST_HINTS)
        .unwrap_or_else(|| DEFAULT_PROTAGONIST.to_owned());
    let special_condition = collect_fragments(&fragments, SPECIAL_CONDITION_HINTS);
    let normalized_world_id = normalized_world_id(world_id);
    let normalized_title = normalized_title(title, seed_text);
    Ok(WorldSeed {
        schema_version: WORLD_SEED_SCHEMA_VERSION.to_owned(),
        world_id: normalized_world_id.clone(),
        title: normalized_title.clone(),
        created_by: "local_user".to_owned(),
        runtime_contract: RuntimeContract::default(),
        premise: WorldPremise {
            genre,
            protagonist,
            special_condition,
            opening_state: DEFAULT_OPENING_STATE.to_owned(),
        },
        anchor_character: AnchorCharacter::default(),
        language: LanguagePolicy::default(),
        laws: WorldLaws::default(),
        opening_randomizer: randomize_opening_seed.then(|| {
            opening_randomizer_from_user_seed(
                seed_text,
                normalized_world_id.as_str(),
                normalized_title.as_str(),
            )
        }),
        non_goals: DEFAULT_NON_GOALS
            .iter()
            .map(|non_goal| (*non_goal).to_owned())
            .collect(),
    })
}

fn opening_randomizer_from_user_seed(
    seed_text: &str,
    world_id: &str,
    title: &str,
) -> OpeningRandomizer {
    let variation_key = opening_variation_key(seed_text, world_id, title);
    let mut cursor = variation_key;
    OpeningRandomizer {
        schema_version: OPENING_RANDOMIZER_SCHEMA_VERSION.to_owned(),
        enabled_by_user: true,
        variation_key: format!("{variation_key:016x}"),
        user_seed_policy: "사용자의 시드에 아래 시작 조건을 덧붙인다. 이 조건은 첫 장면 다양화를 위한 player-visible 개막 seed이며, 장르 장치나 숨은 과거사를 대신 만들지 않는다.".to_owned(),
        location_frame: pick_opening_axis(&mut cursor, OPENING_LOCATION_FRAMES).to_owned(),
        protagonist_frame: pick_opening_axis(&mut cursor, OPENING_PROTAGONIST_FRAMES).to_owned(),
        immediate_pressure: pick_opening_axis(&mut cursor, OPENING_IMMEDIATE_PRESSURES).to_owned(),
        first_visible_object: pick_opening_axis(&mut cursor, OPENING_VISIBLE_OBJECTS).to_owned(),
        social_weather: pick_opening_axis(&mut cursor, OPENING_SOCIAL_WEATHERS).to_owned(),
        opening_question: pick_opening_axis(&mut cursor, OPENING_QUESTIONS).to_owned(),
    }
}

fn opening_variation_key(seed_text: &str, world_id: &str, title: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in [seed_text, world_id, title].join("\n").bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn pick_opening_axis<'a>(cursor: &mut u64, options: &'a [&'a str]) -> &'a str {
    debug_assert!(!options.is_empty());
    *cursor = cursor
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let options_len = options.len() as u64;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "modulo result is strictly smaller than options.len(), which already fits usize"
    )]
    let index = (*cursor % options_len) as usize;
    options[index]
}

#[must_use]
pub fn render_started_world_report(started: &StartedWorld) -> String {
    [
        format!("world: {}", started.initialized.world.world_id),
        format!("title: {}", started.initialized.world.title),
        format!("session: {}", started.initialized.session_id),
        format!(
            "anchor_invariant: {}",
            started.initialized.world.anchor_character.invariant
        ),
        format!(
            "premise: {} / {}",
            started.seed.premise.genre, started.seed.premise.protagonist
        ),
        format!("world_dir: {}", started.initialized.world_dir.display()),
        format!("snapshot: {}", started.initialized.snapshot_path.display()),
        format!("active_world: {}", started.active_binding.world_id),
        "next: singulari-world turn --input \"1\" --render".to_owned(),
    ]
    .join("\n")
}

fn compact_seed_fragments(seed_text: &str) -> Vec<String> {
    seed_text
        .split(|ch| [',', '，', '、', '/', '|', '\n', ';', '；'].contains(&ch))
        .map(str::trim)
        .filter(|fragment| !fragment.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn select_fragment(fragments: &[String], hints: &[&str]) -> Option<String> {
    fragments
        .iter()
        .find(|fragment| contains_any(fragment, hints))
        .cloned()
}

fn collect_fragments(fragments: &[String], hints: &[&str]) -> Option<String> {
    let matching = fragments
        .iter()
        .filter(|fragment| contains_any(fragment, hints))
        .cloned()
        .collect::<Vec<_>>();
    if matching.is_empty() {
        None
    } else {
        Some(matching.join(", "))
    }
}

fn contains_any(value: &str, hints: &[&str]) -> bool {
    hints.iter().any(|hint| value.contains(hint))
}

fn normalized_world_id(world_id: Option<&str>) -> String {
    world_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(generated_world_id, ToOwned::to_owned)
}

fn generated_world_id() -> String {
    let now = Utc::now();
    format!(
        "{}_{}_{}",
        GENERATED_WORLD_ID_PREFIX,
        now.format("%Y%m%d_%H%M%S"),
        now.timestamp_subsec_millis()
    )
}

fn normalized_title(title: Option<&str>, seed_text: &str) -> String {
    title
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map_or_else(|| compact_title_from_seed(seed_text), ToOwned::to_owned)
}

fn compact_title_from_seed(seed_text: &str) -> String {
    let collapsed = seed_text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return DEFAULT_TITLE.to_owned();
    }
    collapsed.chars().take(MAX_TITLE_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use super::{StartWorldOptions, start_world, world_seed_from_compact_text};
    use crate::models::ANCHOR_CHARACTER_INVARIANT;
    use anyhow::bail;
    use tempfile::tempdir;

    #[test]
    fn compact_seed_sets_anchor_character_invariant() -> anyhow::Result<()> {
        let seed = world_seed_from_compact_text(
            "중세 변경 마을, 남자 순찰자, 마법 길표식",
            Some("stw_seed_test"),
            Some("테스트 세계"),
            false,
        )?;
        assert_eq!(seed.world_id, "stw_seed_test");
        assert_eq!(seed.title, "테스트 세계");
        assert_eq!(seed.anchor_character.invariant, ANCHOR_CHARACTER_INVARIANT);
        assert_eq!(seed.premise.genre, "중세 변경 마을");
        assert_eq!(seed.premise.protagonist, "남자 순찰자");
        assert_eq!(
            seed.premise.special_condition.as_deref(),
            Some("마법 길표식")
        );
        Ok(())
    }

    #[test]
    fn sparse_medieval_male_seed_does_not_inject_isekai_tropes() -> anyhow::Result<()> {
        let seed = world_seed_from_compact_text("중세 남자주인공", None, None, false)?;
        let joined = [
            seed.premise.genre,
            seed.premise.protagonist,
            seed.premise.special_condition.unwrap_or_default(),
        ]
        .join(" ");
        for forbidden in ["현대", "전생", "환생", "빙의", "회귀", "치트", "시스템"] {
            assert!(
                !joined.contains(forbidden),
                "sparse seed injected forbidden trope {forbidden}: {joined}"
            );
        }
        Ok(())
    }

    #[test]
    fn start_world_persists_compact_seed() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let started = start_world(&StartWorldOptions {
            seed_text: "중세 변경 마을, 남자 순찰자, 마법 길표식".to_owned(),
            world_id: Some("stw_start_test".to_owned()),
            title: None,
            randomize_opening_seed: false,
            store_root: Some(temp.path().join("store")),
            session_id: Some("session_start_test".to_owned()),
        })?;
        assert_eq!(started.initialized.world.world_id, "stw_start_test");
        assert_eq!(
            started.initialized.world.anchor_character.invariant,
            ANCHOR_CHARACTER_INVARIANT
        );
        assert_eq!(started.active_binding.world_id, "stw_start_test");
        assert_eq!(started.active_binding.session_id, "session_start_test");
        assert!(started.initialized.snapshot_path.exists());
        Ok(())
    }

    #[test]
    fn opening_randomizer_is_user_enabled_and_deterministic() -> anyhow::Result<()> {
        let first = world_seed_from_compact_text(
            "중세판타지",
            Some("stw_opening_randomizer"),
            Some("다양화 테스트"),
            true,
        )?;
        let second = world_seed_from_compact_text(
            "중세판타지",
            Some("stw_opening_randomizer"),
            Some("다양화 테스트"),
            true,
        )?;
        let Some(first_randomizer) = first.opening_randomizer.as_ref() else {
            bail!("first seed did not attach opening randomizer");
        };
        let Some(second_randomizer) = second.opening_randomizer.as_ref() else {
            bail!("second seed did not attach opening randomizer");
        };
        assert_eq!(
            first_randomizer.variation_key,
            second_randomizer.variation_key
        );
        assert!(first_randomizer.enabled_by_user);
        assert_eq!(first.premise.genre, "중세판타지");
        assert!(
            first_randomizer
                .user_seed_policy
                .contains("사용자의 시드에")
        );
        Ok(())
    }
}
