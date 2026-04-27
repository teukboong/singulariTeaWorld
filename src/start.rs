use crate::models::{
    AnchorCharacter, LanguagePolicy, RuntimeContract, WORLD_SEED_SCHEMA_VERSION, WorldLaws,
    WorldPremise, WorldSeed,
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
    "앵커 인물을 자동 구조나 만능 해결 장치로 쓰지 않는다",
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
    "현대인",
    "modern person",
    "전생",
    "reincarnated",
    "환생",
    "reincarnation",
    "빙의",
    "possessed",
    "회귀",
    "regressor",
    "플레이어",
    "player",
    "용사",
    "hero",
    "마법사",
    "mage",
    "wizard",
    "기사",
    "knight",
    "상인",
    "merchant",
];
const SPECIAL_CONDITION_HINTS: &[&str] = &[
    "치트",
    "cheat",
    "능력",
    "ability",
    "스킬",
    "skill",
    "가호",
    "blessing",
    "저주",
    "curse",
    "시스템",
    "system",
    "성좌",
    "constellation",
    "특전",
    "boon",
    "각성",
    "awakening",
    "권능",
    "authority",
    "기억",
    "memory",
    "마법",
    "magic",
    "회귀",
    "regression",
    "talent",
    "gifted",
];

#[derive(Debug, Clone)]
pub struct StartWorldOptions {
    pub seed_text: String,
    pub world_id: Option<String>,
    pub title: Option<String>,
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
    Ok(WorldSeed {
        schema_version: WORLD_SEED_SCHEMA_VERSION.to_owned(),
        world_id: normalized_world_id(world_id),
        title: normalized_title(title, seed_text),
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
        non_goals: DEFAULT_NON_GOALS
            .iter()
            .map(|non_goal| (*non_goal).to_owned())
            .collect(),
    })
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
        "next: singulari-world turn --input \"4\" --render".to_owned(),
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
    use tempfile::tempdir;

    #[test]
    fn compact_seed_sets_anchor_character_invariant() -> anyhow::Result<()> {
        let seed = world_seed_from_compact_text(
            "중세 판타지, 현대인 전생 남주, 치트 보유",
            Some("stw_seed_test"),
            Some("테스트 세계"),
        )?;
        assert_eq!(seed.world_id, "stw_seed_test");
        assert_eq!(seed.title, "테스트 세계");
        assert_eq!(seed.anchor_character.invariant, ANCHOR_CHARACTER_INVARIANT);
        assert_eq!(seed.premise.genre, "중세 판타지");
        assert_eq!(seed.premise.protagonist, "현대인 전생 남주");
        assert_eq!(seed.premise.special_condition.as_deref(), Some("치트 보유"));
        Ok(())
    }

    #[test]
    fn start_world_persists_compact_seed() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let started = start_world(&StartWorldOptions {
            seed_text: "중세 판타지, 현대인 전생 남주, 치트 보유".to_owned(),
            world_id: Some("stw_start_test".to_owned()),
            title: None,
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
}
