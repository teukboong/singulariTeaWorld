use crate::job_ledger::{
    ReadWorldJobsOptions, WorldJob, WorldJobKind, WorldJobStatus, read_world_jobs,
};
use crate::projection_health::{
    ProjectionHealthReport, ProjectionHealthStatus, build_projection_health_report,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const HOST_SUPERVISOR_PLAN_SCHEMA_VERSION: &str = "singulari.host_supervisor_plan.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSupervisorStatus {
    Idle,
    Ready,
    Blocked,
}

impl std::fmt::Display for HostSupervisorStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle => formatter.write_str("idle"),
            Self::Ready => formatter.write_str("ready"),
            Self::Blocked => formatter.write_str("blocked"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostSupervisorLaneKind {
    Text,
    Image,
}

impl std::fmt::Display for HostSupervisorLaneKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => formatter.write_str("text"),
            Self::Image => formatter.write_str("image"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSupervisorLanePlan {
    pub lane: HostSupervisorLaneKind,
    pub status: HostSupervisorStatus,
    pub pending_jobs: Vec<WorldJob>,
    pub claimed_jobs: Vec<WorldJob>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostSupervisorPlan {
    pub schema_version: String,
    pub world_id: String,
    pub status: HostSupervisorStatus,
    pub lanes: Vec<HostSupervisorLanePlan>,
    pub projection_health: ProjectionHealthReport,
    pub recommended_action: String,
}

/// Compile the deterministic host-supervisor view for one world.
///
/// # Errors
///
/// Returns an error when store paths or job/projection reads fail before a
/// component-level health report can be produced.
pub fn build_host_supervisor_plan(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<HostSupervisorPlan> {
    let jobs = read_world_jobs(&ReadWorldJobsOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id: world_id.to_owned(),
        extra_visual_jobs: Vec::new(),
    })?;
    let projection_health = build_projection_health_report(store_root, world_id)?;
    let blocked = projection_health.status == ProjectionHealthStatus::Failed;
    let lanes = vec![
        supervisor_lane_plan(HostSupervisorLaneKind::Text, &jobs, blocked),
        supervisor_lane_plan(HostSupervisorLaneKind::Image, &jobs, blocked),
    ];
    let status = if blocked {
        HostSupervisorStatus::Blocked
    } else if lanes
        .iter()
        .any(|lane| lane.status == HostSupervisorStatus::Ready)
    {
        HostSupervisorStatus::Ready
    } else {
        HostSupervisorStatus::Idle
    };
    Ok(HostSupervisorPlan {
        schema_version: HOST_SUPERVISOR_PLAN_SCHEMA_VERSION.to_owned(),
        world_id: world_id.to_owned(),
        status,
        recommended_action: recommended_action(status, &lanes),
        lanes,
        projection_health,
    })
}

#[must_use]
pub fn render_host_supervisor_plan(plan: &HostSupervisorPlan) -> String {
    let mut lines = vec![
        format!("world: {}", plan.world_id),
        format!("status: {}", plan.status),
        format!("recommended_action: {}", plan.recommended_action),
        format!("projection_health: {}", plan.projection_health.status),
    ];
    for lane in &plan.lanes {
        lines.push(format!("{}: {} - {}", lane.lane, lane.status, lane.detail));
        for job in &lane.pending_jobs {
            lines.push(format!("  pending: {} {}", job.job_id, job.slot));
        }
        for job in &lane.claimed_jobs {
            lines.push(format!("  claimed: {} {}", job.job_id, job.slot));
        }
    }
    lines.join("\n")
}

fn supervisor_lane_plan(
    lane: HostSupervisorLaneKind,
    jobs: &[WorldJob],
    blocked: bool,
) -> HostSupervisorLanePlan {
    let pending_jobs: Vec<WorldJob> = jobs
        .iter()
        .filter(|job| lane_owns_job(lane, job) && job.status == WorldJobStatus::Pending)
        .cloned()
        .collect();
    let claimed_jobs: Vec<WorldJob> = jobs
        .iter()
        .filter(|job| lane_owns_job(lane, job) && job.status == WorldJobStatus::Claimed)
        .cloned()
        .collect();
    let status = if blocked {
        HostSupervisorStatus::Blocked
    } else if !pending_jobs.is_empty() {
        HostSupervisorStatus::Ready
    } else {
        HostSupervisorStatus::Idle
    };
    HostSupervisorLanePlan {
        lane,
        status,
        detail: lane_detail(status, pending_jobs.len(), claimed_jobs.len()),
        pending_jobs,
        claimed_jobs,
    }
}

fn lane_owns_job(lane: HostSupervisorLaneKind, job: &WorldJob) -> bool {
    match lane {
        HostSupervisorLaneKind::Text => job.kind == WorldJobKind::TextTurn,
        HostSupervisorLaneKind::Image => matches!(
            job.kind,
            WorldJobKind::SceneCg | WorldJobKind::ReferenceAsset | WorldJobKind::UiAsset
        ),
    }
}

fn lane_detail(status: HostSupervisorStatus, pending: usize, claimed: usize) -> String {
    match status {
        HostSupervisorStatus::Blocked => {
            format!("blocked by failed projection health; pending={pending}, claimed={claimed}")
        }
        HostSupervisorStatus::Ready => format!("ready jobs pending={pending}, claimed={claimed}"),
        HostSupervisorStatus::Idle => format!("no pending jobs; claimed={claimed}"),
    }
}

fn recommended_action(status: HostSupervisorStatus, lanes: &[HostSupervisorLanePlan]) -> String {
    match status {
        HostSupervisorStatus::Blocked => "repair_projection_before_dispatch".to_owned(),
        HostSupervisorStatus::Idle => "sleep_until_new_job".to_owned(),
        HostSupervisorStatus::Ready => {
            let ready_lanes: Vec<String> = lanes
                .iter()
                .filter(|lane| lane.status == HostSupervisorStatus::Ready)
                .map(|lane| lane.lane.to_string())
                .collect();
            format!("dispatch_lanes:{}", ready_lanes.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::host_supervisor::{
        HostSupervisorStatus, build_host_supervisor_plan, render_host_supervisor_plan,
    };
    use crate::store::{InitWorldOptions, init_world};
    use tempfile::tempdir;

    fn seed_body() -> &'static str {
        r#"
schema_version: singulari.world_seed.v1
world_id: stw_host_supervisor
title: "슈퍼바이저 세계"
premise:
  genre: "중세 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#
    }

    #[test]
    fn supervisor_plan_sees_initial_visual_jobs() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let plan = build_host_supervisor_plan(Some(&store), "stw_host_supervisor")?;

        assert_eq!(plan.status, HostSupervisorStatus::Ready);
        assert!(plan.recommended_action.contains("image"));
        assert!(render_host_supervisor_plan(&plan).contains("image: ready"));
        Ok(())
    }

    #[test]
    fn supervisor_plan_blocks_on_failed_projection_health() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body())?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        std::fs::remove_file(initialized.world_dir.join(crate::WORLD_DB_FILENAME))?;

        let plan = build_host_supervisor_plan(Some(&store), "stw_host_supervisor")?;

        assert_eq!(plan.status, HostSupervisorStatus::Blocked);
        assert_eq!(plan.recommended_action, "repair_projection_before_dispatch");
        Ok(())
    }
}
