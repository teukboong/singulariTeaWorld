use serde::{Deserialize, Serialize};

pub const KNOWLEDGE_CLAIM_SCHEMA_VERSION: &str = "singulari.knowledge_claim.v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KnowledgeTier {
    WorldTrueHidden,
    #[default]
    PlayerObserved,
    PlayerInferred,
    Rumor,
    FalseBelief,
    Contradicted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TruthStatus {
    True,
    False,
    Unknown,
    Contested,
    Inferred,
    Rumored,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KnowledgeClaim {
    pub schema_version: String,
    pub claim_id: String,
    pub holder_ref: String,
    pub tier: KnowledgeTier,
    pub truth_status: TruthStatus,
    pub proposition: String,
    #[serde(default)]
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerRenderPermission {
    DirectFact,
    UncertainInference,
    SourcedRumor,
    BeliefOnly,
    BlockedHiddenTruth,
}

#[must_use]
pub const fn player_render_permission(tier: KnowledgeTier) -> PlayerRenderPermission {
    match tier {
        KnowledgeTier::WorldTrueHidden => PlayerRenderPermission::BlockedHiddenTruth,
        KnowledgeTier::PlayerObserved => PlayerRenderPermission::DirectFact,
        KnowledgeTier::PlayerInferred => PlayerRenderPermission::UncertainInference,
        KnowledgeTier::Rumor => PlayerRenderPermission::SourcedRumor,
        KnowledgeTier::FalseBelief | KnowledgeTier::Contradicted => {
            PlayerRenderPermission::BeliefOnly
        }
    }
}

#[must_use]
pub const fn can_render_knowledge_tier_to_player(tier: KnowledgeTier) -> bool {
    !matches!(
        player_render_permission(tier),
        PlayerRenderPermission::BlockedHiddenTruth
    )
}

#[must_use]
pub fn render_rule_for_player(tier: KnowledgeTier) -> &'static str {
    match player_render_permission(tier) {
        PlayerRenderPermission::DirectFact => "may render as observed fact",
        PlayerRenderPermission::UncertainInference => "must render with uncertainty language",
        PlayerRenderPermission::SourcedRumor => "must render with source/rumor framing",
        PlayerRenderPermission::BeliefOnly => "must render as a holder belief, not world fact",
        PlayerRenderPermission::BlockedHiddenTruth => "must not render to player-visible surfaces",
    }
}

#[must_use]
pub fn visible_knowledge_text_is_qualified(tier: KnowledgeTier, text: &str) -> bool {
    match player_render_permission(tier) {
        PlayerRenderPermission::DirectFact => true,
        PlayerRenderPermission::UncertainInference => contains_any(
            text,
            &[
                "추정",
                "가능성",
                "듯",
                "아마",
                "확실하지",
                "inferred",
                "seems",
                "may",
                "might",
                "uncertain",
            ],
        ),
        PlayerRenderPermission::SourcedRumor => contains_any(
            text,
            &[
                "소문", "전해", "들었", "출처", "rumor", "heard", "reported", "source",
            ],
        ),
        PlayerRenderPermission::BeliefOnly => contains_any(
            text,
            &["믿", "오해", "착각", "belief", "believes", "mistaken"],
        ),
        PlayerRenderPermission::BlockedHiddenTruth => false,
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    let lower = text.to_ascii_lowercase();
    needles
        .iter()
        .any(|needle| text.contains(needle) || lower.contains(&needle.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::{
        KnowledgeTier, PlayerRenderPermission, can_render_knowledge_tier_to_player,
        player_render_permission, render_rule_for_player,
    };

    #[test]
    fn hidden_truth_is_not_player_renderable() {
        assert!(!can_render_knowledge_tier_to_player(
            KnowledgeTier::WorldTrueHidden
        ));
        assert_eq!(
            player_render_permission(KnowledgeTier::WorldTrueHidden),
            PlayerRenderPermission::BlockedHiddenTruth
        );
    }

    #[test]
    fn inferred_and_rumor_tiers_require_qualified_rendering() {
        assert_eq!(
            player_render_permission(KnowledgeTier::PlayerInferred),
            PlayerRenderPermission::UncertainInference
        );
        assert_eq!(
            player_render_permission(KnowledgeTier::Rumor),
            PlayerRenderPermission::SourcedRumor
        );
        assert!(render_rule_for_player(KnowledgeTier::Rumor).contains("source"));
        assert!(super::visible_knowledge_text_is_qualified(
            KnowledgeTier::PlayerInferred,
            "북문 뒤쪽에 누군가 있을 가능성이 있다"
        ));
        assert!(!super::visible_knowledge_text_is_qualified(
            KnowledgeTier::PlayerInferred,
            "북문 뒤쪽에 암살자가 있다"
        ));
    }
}
