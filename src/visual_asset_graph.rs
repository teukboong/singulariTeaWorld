use crate::visual_assets::{VisualArtifactKind, WorldVisualAsset, WorldVisualAssets};
use serde::{Deserialize, Serialize};

pub const VISUAL_ASSET_GRAPH_PACKET_SCHEMA_VERSION: &str = "singulari.visual_asset_graph_packet.v1";
pub const VISUAL_ASSET_NODE_SCHEMA_VERSION: &str = "singulari.visual_asset_node.v1";

const ACCEPTED_REFERENCE_BUDGET: usize = 7;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualAssetGraphPacket {
    pub schema_version: String,
    pub world_id: String,
    #[serde(default)]
    pub display_assets: Vec<VisualAssetNode>,
    #[serde(default)]
    pub reference_assets: Vec<VisualAssetNode>,
    #[serde(default)]
    pub pending_jobs: Vec<VisualAssetJobNode>,
    pub compiler_policy: VisualAssetGraphPolicy,
}

impl Default for VisualAssetGraphPacket {
    fn default() -> Self {
        Self {
            schema_version: VISUAL_ASSET_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: String::new(),
            display_assets: Vec::new(),
            reference_assets: Vec::new(),
            pending_jobs: Vec::new(),
            compiler_policy: VisualAssetGraphPolicy::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualAssetNode {
    pub schema_version: String,
    pub asset_id: String,
    pub slot: String,
    pub artifact_kind: VisualArtifactKind,
    pub canonical_use: String,
    pub display_allowed: bool,
    pub reference_allowed: bool,
    pub exists: bool,
    pub asset_url: String,
    pub path: String,
    #[serde(default)]
    pub entity_refs: Vec<String>,
    pub boundary: VisualAssetBoundary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualAssetJobNode {
    pub slot: String,
    pub artifact_kind: VisualArtifactKind,
    pub canonical_use: String,
    pub display_allowed: bool,
    pub reference_allowed: bool,
    pub destination_path: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VisualAssetBoundary {
    Display,
    ReferenceOnly,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualAssetGraphPolicy {
    pub source: String,
    pub accepted_reference_budget: usize,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for VisualAssetGraphPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_world_visual_assets_v0".to_owned(),
            accepted_reference_budget: ACCEPTED_REFERENCE_BUDGET,
            use_rules: vec![
                "Display assets may appear in VN UI; reference-only assets may be attached to image generation but must not be shown as scene CG.".to_owned(),
                "Character and location design sheets are reference-only even when generated.".to_owned(),
                "Pending jobs describe work to do; they are not completed assets.".to_owned(),
            ],
        }
    }
}

#[must_use]
pub fn compile_visual_asset_graph_packet(manifest: &WorldVisualAssets) -> VisualAssetGraphPacket {
    let mut display_assets = Vec::new();
    let mut reference_assets = Vec::new();
    push_world_asset(
        &mut display_assets,
        &mut reference_assets,
        &manifest.menu_background,
    );
    push_world_asset(
        &mut display_assets,
        &mut reference_assets,
        &manifest.stage_background,
    );
    for entity in &manifest.visual_entities {
        let node = VisualAssetNode {
            schema_version: VISUAL_ASSET_NODE_SCHEMA_VERSION.to_owned(),
            asset_id: format!("asset:{}:{}", entity.artifact_kind.as_str(), entity.slot),
            slot: entity.slot.clone(),
            artifact_kind: entity.artifact_kind,
            canonical_use: entity.canonical_use.clone(),
            display_allowed: entity.display_allowed,
            reference_allowed: entity.reference_allowed,
            exists: entity.exists,
            asset_url: entity.asset_url.clone(),
            path: entity.recommended_path.clone(),
            entity_refs: vec![entity.entity_id.clone()],
            boundary: boundary_for(entity.display_allowed, entity.reference_allowed),
        };
        if node.reference_allowed {
            reference_assets.push(node);
        } else if node.display_allowed {
            display_assets.push(node);
        }
    }
    reference_assets.truncate(ACCEPTED_REFERENCE_BUDGET);
    VisualAssetGraphPacket {
        schema_version: VISUAL_ASSET_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
        world_id: manifest.world_id.clone(),
        display_assets,
        reference_assets,
        pending_jobs: manifest
            .image_generation_jobs
            .iter()
            .map(|job| VisualAssetJobNode {
                slot: job.slot.clone(),
                artifact_kind: job.artifact_kind,
                canonical_use: job.canonical_use.clone(),
                display_allowed: job.display_allowed,
                reference_allowed: job.reference_allowed,
                destination_path: job.destination_path.clone(),
            })
            .collect(),
        compiler_policy: VisualAssetGraphPolicy::default(),
    }
}

fn push_world_asset(
    display_assets: &mut Vec<VisualAssetNode>,
    reference_assets: &mut Vec<VisualAssetNode>,
    asset: &WorldVisualAsset,
) {
    let node = VisualAssetNode {
        schema_version: VISUAL_ASSET_NODE_SCHEMA_VERSION.to_owned(),
        asset_id: format!("asset:{}:{}", asset.artifact_kind.as_str(), asset.slot),
        slot: asset.slot.clone(),
        artifact_kind: asset.artifact_kind,
        canonical_use: asset.canonical_use.clone(),
        display_allowed: asset.display_allowed,
        reference_allowed: asset.reference_allowed,
        exists: asset.exists,
        asset_url: asset.asset_url.clone(),
        path: asset.recommended_path.clone(),
        entity_refs: Vec::new(),
        boundary: boundary_for(asset.display_allowed, asset.reference_allowed),
    };
    if node.display_allowed {
        display_assets.push(node);
    } else if node.reference_allowed {
        reference_assets.push(node);
    }
}

const fn boundary_for(display_allowed: bool, reference_allowed: bool) -> VisualAssetBoundary {
    if display_allowed {
        VisualAssetBoundary::Display
    } else if reference_allowed {
        VisualAssetBoundary::ReferenceOnly
    } else {
        VisualAssetBoundary::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual_assets::{
        VisualBudgetPolicy, VisualEntityAsset, WorldVisualAsset, WorldVisualAssets,
        WorldVisualStyleProfile,
    };

    #[test]
    fn splits_display_and_reference_assets() {
        let manifest = WorldVisualAssets {
            schema_version: "singulari.world_visual_assets.v1".to_owned(),
            world_id: "stw_visual".to_owned(),
            style_profile: WorldVisualStyleProfile {
                style_prompt: String::new(),
                palette_prompt: String::new(),
                camera_language: String::new(),
                negative_prompt: String::new(),
            },
            budget_policy: VisualBudgetPolicy::default(),
            menu_background: ui_asset("menu_background"),
            stage_background: ui_asset("stage_background"),
            visual_entities: vec![VisualEntityAsset {
                entity_id: "char:protagonist".to_owned(),
                entity_type: "character".to_owned(),
                display_name: "Protagonist".to_owned(),
                slot: "character_sheet:char:protagonist".to_owned(),
                artifact_kind: VisualArtifactKind::CharacterDesignSheet,
                canonical_use: VisualArtifactKind::CharacterDesignSheet
                    .canonical_use()
                    .to_owned(),
                display_allowed: false,
                reference_allowed: true,
                prompt: String::new(),
                recommended_path: "char.png".to_owned(),
                asset_url: "/assets/char.png".to_owned(),
                exists: true,
                generation_policy: String::new(),
                prompt_policy: String::new(),
            }],
            image_generation_jobs: Vec::new(),
            updated_at: "2026-04-29T00:00:00Z".to_owned(),
        };

        let packet = compile_visual_asset_graph_packet(&manifest);

        assert_eq!(packet.display_assets.len(), 2);
        assert_eq!(packet.reference_assets.len(), 1);
        assert_eq!(
            packet.reference_assets[0].boundary,
            VisualAssetBoundary::ReferenceOnly
        );
    }

    fn ui_asset(slot: &str) -> WorldVisualAsset {
        WorldVisualAsset {
            slot: slot.to_owned(),
            artifact_kind: VisualArtifactKind::UiBackground,
            canonical_use: VisualArtifactKind::UiBackground.canonical_use().to_owned(),
            display_allowed: true,
            reference_allowed: false,
            prompt: String::new(),
            recommended_path: format!("{slot}.png"),
            asset_url: format!("/assets/{slot}.png"),
            exists: true,
            prompt_policy: String::new(),
        }
    }
}
