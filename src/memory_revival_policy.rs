use serde::{Deserialize, Serialize};

pub const MEMORY_REVIVAL_POLICY_SCHEMA_VERSION: &str = "singulari.memory_revival_policy.v1";

const CODEX_RECENT_EVENTS: usize = 12;
const CODEX_RECENT_MEMORIES: usize = 12;
const CODEX_CHAPTER_LIMIT: usize = 4;
const CODEX_ARCHIVE_LIMIT: usize = 12;
const CODEX_UPDATE_LIMIT: usize = 8;
const CODEX_SEARCH_LIMIT: usize = 4;

const WEBGPT_RECENT_EVENTS: usize = 24;
const WEBGPT_RECENT_MEMORIES: usize = 24;
const WEBGPT_CHAPTER_LIMIT: usize = 6;
const WEBGPT_ARCHIVE_LIMIT: usize = 24;
const WEBGPT_UPDATE_LIMIT: usize = 16;
const WEBGPT_SEARCH_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRevivalPolicy {
    pub schema_version: String,
    pub profile_name: String,
    pub engine_session_kind: String,
    pub purpose: String,
    pub recent_events: usize,
    pub recent_character_memories: usize,
    pub chapter_summaries: usize,
    pub archive_limit: usize,
    pub update_limit: usize,
    pub query_recall_limit: usize,
    #[serde(default)]
    pub anti_repetition_rules: Vec<String>,
}

impl MemoryRevivalPolicy {
    #[must_use]
    pub fn for_engine_session(engine_session_kind: &str) -> Self {
        if engine_session_kind.starts_with("webgpt") {
            return Self::webgpt(engine_session_kind);
        }
        Self::codex(engine_session_kind)
    }

    fn webgpt(engine_session_kind: &str) -> Self {
        Self {
            schema_version: MEMORY_REVIVAL_POLICY_SCHEMA_VERSION.to_owned(),
            profile_name: "webgpt_active_memory".to_owned(),
            engine_session_kind: engine_session_kind.to_owned(),
            purpose: "WebGPT context-window and compaction behavior are not the world source of truth, so host-worker proactively surfaces more player-visible continuity from world.db before each turn.".to_owned(),
            recent_events: WEBGPT_RECENT_EVENTS,
            recent_character_memories: WEBGPT_RECENT_MEMORIES,
            chapter_summaries: WEBGPT_CHAPTER_LIMIT,
            archive_limit: WEBGPT_ARCHIVE_LIMIT,
            update_limit: WEBGPT_UPDATE_LIMIT,
            query_recall_limit: WEBGPT_SEARCH_LIMIT,
            anti_repetition_rules: vec![
                "Prefer unresolved or recently changed continuity over repeating the same archive facts every turn.".to_owned(),
                "Use query recall only when it connects to the current player input.".to_owned(),
            ],
        }
    }

    fn codex(engine_session_kind: &str) -> Self {
        Self {
            schema_version: MEMORY_REVIVAL_POLICY_SCHEMA_VERSION.to_owned(),
            profile_name: "codex_balanced_memory".to_owned(),
            engine_session_kind: engine_session_kind.to_owned(),
            purpose: "Codex-style engines already carry clearer session context, so revival should stay balanced and avoid bloating prompt context.".to_owned(),
            recent_events: CODEX_RECENT_EVENTS,
            recent_character_memories: CODEX_RECENT_MEMORIES,
            chapter_summaries: CODEX_CHAPTER_LIMIT,
            archive_limit: CODEX_ARCHIVE_LIMIT,
            update_limit: CODEX_UPDATE_LIMIT,
            query_recall_limit: CODEX_SEARCH_LIMIT,
            anti_repetition_rules: vec![
                "Surface only continuity that materially changes the next turn.".to_owned(),
                "Do not include decorative archive context when recent scene state is enough.".to_owned(),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webgpt_policy_is_more_aggressive_than_codex_policy() {
        let webgpt = MemoryRevivalPolicy::for_engine_session("webgpt-text");
        let codex = MemoryRevivalPolicy::for_engine_session("codex-text");

        assert_eq!(webgpt.profile_name, "webgpt_active_memory");
        assert_eq!(codex.profile_name, "codex_balanced_memory");
        assert!(webgpt.recent_events > codex.recent_events);
        assert!(webgpt.archive_limit > codex.archive_limit);
        assert!(!webgpt.anti_repetition_rules.is_empty());
    }
}
