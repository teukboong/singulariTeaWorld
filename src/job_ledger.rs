use crate::agent_bridge::load_pending_agent_turn;
use crate::visual_assets::{
    BuildWorldVisualAssetsOptions, ImageGenerationJob, VisualArtifactKind, VisualJobClaim,
    build_world_visual_assets, load_visual_job_claim,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const WORLD_JOB_LEDGER_SCHEMA_VERSION: &str = "singulari.world_job_ledger.v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldJobKind {
    TextTurn,
    SceneCg,
    ReferenceAsset,
    UiAsset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorldJobStatus {
    Pending,
    Claimed,
    Running,
    Completed,
    FailedRetryable,
    FailedTerminal,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldJob {
    pub schema_version: String,
    pub world_id: String,
    pub job_id: String,
    pub kind: WorldJobKind,
    pub status: WorldJobStatus,
    pub slot: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_kind: Option<VisualArtifactKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ReadWorldJobsOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub extra_visual_jobs: Vec<ImageGenerationJob>,
}

/// Read the current job-like state without migrating existing files.
///
/// # Errors
///
/// Returns an error when the world store, visual manifest, or visual claims
/// cannot be read.
pub fn read_world_jobs(options: &ReadWorldJobsOptions) -> Result<Vec<WorldJob>> {
    let mut jobs = Vec::new();
    if let Ok(pending) =
        load_pending_agent_turn(options.store_root.as_deref(), options.world_id.as_str())
    {
        jobs.push(WorldJob {
            schema_version: WORLD_JOB_LEDGER_SCHEMA_VERSION.to_owned(),
            world_id: pending.world_id,
            job_id: format!("text_turn:{}", pending.turn_id),
            kind: WorldJobKind::TextTurn,
            status: WorldJobStatus::Pending,
            slot: pending.turn_id.clone(),
            turn_id: Some(pending.turn_id),
            artifact_kind: None,
            path: Some(pending.pending_ref.clone()),
            input_ref: Some(pending.pending_ref),
            output_ref: None,
            claim_id: None,
            claim_owner: None,
            claimed_at: None,
            claim_path: None,
            attempt_id: None,
            last_error: None,
        });
    }

    let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
    })?;
    let mut visual_jobs = manifest.image_generation_jobs.clone();
    for extra_job in &options.extra_visual_jobs {
        if !visual_jobs
            .iter()
            .any(|candidate| candidate.slot == extra_job.slot)
        {
            visual_jobs.push(extra_job.clone());
        }
    }
    for job in visual_jobs {
        let claim = load_visual_job_claim(
            options.store_root.as_deref(),
            options.world_id.as_str(),
            job.slot.as_str(),
        )?;
        let status = if claim.is_some() {
            WorldJobStatus::Claimed
        } else {
            WorldJobStatus::Pending
        };
        jobs.push(world_job_from_image_job(
            options.world_id.as_str(),
            &job,
            status,
            claim.as_ref(),
        ));
    }

    if manifest.menu_background.exists {
        jobs.push(completed_visual_job(
            options.world_id.as_str(),
            manifest.menu_background.slot.as_str(),
            manifest.menu_background.artifact_kind,
            Path::new(manifest.menu_background.recommended_path.as_str()),
        ));
    }
    if manifest.stage_background.exists {
        jobs.push(completed_visual_job(
            options.world_id.as_str(),
            manifest.stage_background.slot.as_str(),
            manifest.stage_background.artifact_kind,
            Path::new(manifest.stage_background.recommended_path.as_str()),
        ));
    }
    for asset in manifest.visual_entities.iter().filter(|asset| asset.exists) {
        jobs.push(completed_visual_job(
            options.world_id.as_str(),
            asset.slot.as_str(),
            asset.artifact_kind,
            Path::new(asset.recommended_path.as_str()),
        ));
    }
    jobs.sort_by(|left, right| {
        job_status_rank(left.status)
            .cmp(&job_status_rank(right.status))
            .then_with(|| left.kind_label().cmp(right.kind_label()))
            .then_with(|| left.slot.cmp(&right.slot))
    });
    Ok(jobs)
}

impl WorldJob {
    fn kind_label(&self) -> &'static str {
        match self.kind {
            WorldJobKind::TextTurn => "text_turn",
            WorldJobKind::SceneCg => "scene_cg",
            WorldJobKind::ReferenceAsset => "reference_asset",
            WorldJobKind::UiAsset => "ui_asset",
        }
    }

    #[must_use]
    pub const fn is_dispatchable(&self) -> bool {
        self.status.is_dispatchable()
    }

    #[must_use]
    pub const fn is_in_flight(&self) -> bool {
        self.status.is_in_flight()
    }

    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

fn world_job_from_image_job(
    world_id: &str,
    job: &ImageGenerationJob,
    status: WorldJobStatus,
    claim: Option<&VisualJobClaim>,
) -> WorldJob {
    WorldJob {
        schema_version: WORLD_JOB_LEDGER_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        job_id: format!(
            "{}:{}",
            visual_job_kind(job.artifact_kind).kind_label(),
            job.slot
        ),
        kind: visual_job_kind(job.artifact_kind),
        status,
        slot: job.slot.clone(),
        turn_id: turn_id_from_slot(job.slot.as_str()),
        artifact_kind: Some(job.artifact_kind),
        path: Some(job.destination_path.clone()),
        input_ref: None,
        output_ref: Some(job.destination_path.clone()),
        claim_id: claim.map(|value| value.claim_id.clone()),
        claim_owner: claim.map(|value| value.claimed_by.clone()),
        claimed_at: claim.map(|value| value.claimed_at.clone()),
        claim_path: claim.map(|value| value.claim_path.clone()),
        attempt_id: claim.map(|value| value.claim_id.clone()),
        last_error: None,
    }
}

fn completed_visual_job(
    world_id: &str,
    slot: &str,
    artifact_kind: VisualArtifactKind,
    path: &Path,
) -> WorldJob {
    WorldJob {
        schema_version: WORLD_JOB_LEDGER_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        job_id: format!("{}:{slot}", visual_job_kind(artifact_kind).kind_label()),
        kind: visual_job_kind(artifact_kind),
        status: WorldJobStatus::Completed,
        slot: slot.to_owned(),
        turn_id: turn_id_from_slot(slot),
        artifact_kind: Some(artifact_kind),
        path: Some(path.display().to_string()),
        input_ref: None,
        output_ref: Some(path.display().to_string()),
        claim_id: None,
        claim_owner: None,
        claimed_at: None,
        claim_path: None,
        attempt_id: None,
        last_error: None,
    }
}

fn visual_job_kind(artifact_kind: VisualArtifactKind) -> WorldJobKind {
    match artifact_kind {
        VisualArtifactKind::SceneCg => WorldJobKind::SceneCg,
        VisualArtifactKind::CharacterDesignSheet | VisualArtifactKind::LocationDesignSheet => {
            WorldJobKind::ReferenceAsset
        }
        VisualArtifactKind::UiBackground => WorldJobKind::UiAsset,
    }
}

fn turn_id_from_slot(slot: &str) -> Option<String> {
    slot.strip_prefix("turn_cg:").map(str::to_owned)
}

fn job_status_rank(status: WorldJobStatus) -> u8 {
    match status {
        WorldJobStatus::Running => 0,
        WorldJobStatus::Claimed => 1,
        WorldJobStatus::Pending => 2,
        WorldJobStatus::FailedRetryable => 3,
        WorldJobStatus::Completed => 4,
        WorldJobStatus::FailedTerminal | WorldJobStatus::Cancelled => 5,
    }
}

impl WorldJobStatus {
    #[must_use]
    pub const fn is_dispatchable(self) -> bool {
        matches!(self, Self::Pending | Self::FailedRetryable)
    }

    #[must_use]
    pub const fn is_in_flight(self) -> bool {
        matches!(self, Self::Claimed | Self::Running)
    }

    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::FailedTerminal | Self::Cancelled
        )
    }
}

trait WorldJobKindLabel {
    fn kind_label(self) -> &'static str;
}

impl WorldJobKindLabel for WorldJobKind {
    fn kind_label(self) -> &'static str {
        match self {
            Self::TextTurn => "text_turn",
            Self::SceneCg => "scene_cg",
            Self::ReferenceAsset => "reference_asset",
            Self::UiAsset => "ui_asset",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{InitWorldOptions, init_world};
    use crate::visual_assets::{
        ClaimVisualJobOptions, VN_ASSETS_DIR, VisualArtifactKind, claim_visual_job,
        visual_generation_job,
    };
    use tempfile::tempdir;

    const MINIMAL_PNG: &[u8] = b"\x89PNG\r\n\x1a\nminimal-test-png";

    #[test]
    fn reads_visual_jobs_across_pending_claimed_and_completed_files() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_job_ledger
title: "잡 원장"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let menu_path = initialized
            .world_dir
            .join(VN_ASSETS_DIR)
            .join("menu_background.png");
        let Some(menu_parent) = menu_path.parent() else {
            anyhow::bail!("menu path has no parent: {}", menu_path.display());
        };
        std::fs::create_dir_all(menu_parent)?;
        std::fs::write(&menu_path, MINIMAL_PNG)?;
        let claimed = claim_visual_job(&ClaimVisualJobOptions {
            store_root: Some(store.clone()),
            world_id: "stw_job_ledger".to_owned(),
            slot: Some("stage_background".to_owned()),
            claimed_by: "test-worker".to_owned(),
            force: false,
            extra_jobs: Vec::new(),
        })?;
        assert!(matches!(
            claimed,
            crate::visual_assets::VisualJobClaimOutcome::Claimed { .. }
        ));

        let turn_cg_path = initialized
            .world_dir
            .join(VN_ASSETS_DIR)
            .join("turn_cg")
            .join("turn_0001.png");
        let scene_job = visual_generation_job(
            "turn_cg:turn_0001".to_owned(),
            VisualArtifactKind::SceneCg,
            "scene prompt".to_owned(),
            turn_cg_path.display().to_string(),
            Vec::new(),
            Vec::new(),
            "test",
        );

        let jobs = read_world_jobs(&ReadWorldJobsOptions {
            store_root: Some(store),
            world_id: "stw_job_ledger".to_owned(),
            extra_visual_jobs: vec![scene_job],
        })?;
        let turn_cg_output = turn_cg_path.display().to_string();

        assert!(jobs.iter().any(|job| {
            job.slot == "menu_background"
                && job.kind == WorldJobKind::UiAsset
                && job.status == WorldJobStatus::Completed
        }));
        assert!(jobs.iter().any(|job| {
            job.slot == "stage_background"
                && job.kind == WorldJobKind::UiAsset
                && job.status == WorldJobStatus::Claimed
                && job.claim_owner.as_deref() == Some("test-worker")
                && job.claim_id.is_some()
                && job.claim_path.is_some()
        }));
        assert!(jobs.iter().any(|job| {
            job.slot == "turn_cg:turn_0001"
                && job.kind == WorldJobKind::SceneCg
                && job.status == WorldJobStatus::Pending
                && job.turn_id.as_deref() == Some("turn_0001")
                && job.output_ref.as_deref() == Some(turn_cg_output.as_str())
        }));
        Ok(())
    }

    #[test]
    fn job_status_lifecycle_groups_dispatch_and_terminal_states() {
        assert!(WorldJobStatus::Pending.is_dispatchable());
        assert!(WorldJobStatus::FailedRetryable.is_dispatchable());
        assert!(WorldJobStatus::Claimed.is_in_flight());
        assert!(WorldJobStatus::Running.is_in_flight());
        assert!(WorldJobStatus::Completed.is_terminal());
        assert!(WorldJobStatus::FailedTerminal.is_terminal());
        assert!(WorldJobStatus::Cancelled.is_terminal());
    }
}
