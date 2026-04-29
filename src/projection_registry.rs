// Projection registry pilot. Keep this small until more families move behind
// the same lifecycle contract.
#![allow(
    clippy::missing_errors_doc,
    reason = "Projection APIs return anyhow::Result with per-call path/context details."
)]

use crate::body_resource::{
    BODY_RESOURCE_STATE_FILENAME, BodyResourcePacket, compile_body_resource_packet,
    load_body_resource_state,
};
use crate::models::TurnSnapshot;
use anyhow::Result;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectionFamilyDescriptor {
    pub family_id: &'static str,
    pub state_filename: &'static str,
    pub prompt_packet_kind: &'static str,
}

pub trait ProjectionFamily {
    type Packet;

    const DESCRIPTOR: ProjectionFamilyDescriptor;
}

pub trait SnapshotProjectionFamily: ProjectionFamily {
    fn compile_from_snapshot(snapshot: &TurnSnapshot) -> Self::Packet;
    fn load_state(world_dir: &Path, fallback: Self::Packet) -> Result<Self::Packet>;

    fn prompt_packet(world_dir: &Path, snapshot: &TurnSnapshot) -> Result<Self::Packet> {
        Self::load_state(world_dir, Self::compile_from_snapshot(snapshot))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BodyResourceProjectionFamily;

impl ProjectionFamily for BodyResourceProjectionFamily {
    type Packet = BodyResourcePacket;

    const DESCRIPTOR: ProjectionFamilyDescriptor = ProjectionFamilyDescriptor {
        family_id: "body_resource",
        state_filename: BODY_RESOURCE_STATE_FILENAME,
        prompt_packet_kind: "active_body_resource_state",
    };
}

impl SnapshotProjectionFamily for BodyResourceProjectionFamily {
    fn compile_from_snapshot(snapshot: &TurnSnapshot) -> Self::Packet {
        compile_body_resource_packet(snapshot)
    }

    fn load_state(world_dir: &Path, fallback: Self::Packet) -> Result<Self::Packet> {
        load_body_resource_state(world_dir, fallback)
    }
}

pub const PROJECTION_FAMILY_REGISTRY: &[ProjectionFamilyDescriptor] =
    &[BodyResourceProjectionFamily::DESCRIPTOR];

pub fn load_body_resource_prompt_packet(
    world_dir: &Path,
    snapshot: &TurnSnapshot,
) -> Result<BodyResourcePacket> {
    BodyResourceProjectionFamily::prompt_packet(world_dir, snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::body_resource::{
        BODY_CONSTRAINT_SCHEMA_VERSION, BodyConstraint, BodyResourcePolicy, BodyResourceVisibility,
    };
    use crate::models::{ProtagonistState, TURN_SNAPSHOT_SCHEMA_VERSION};
    use crate::store::write_json;
    use tempfile::tempdir;

    fn snapshot() -> TurnSnapshot {
        TurnSnapshot {
            schema_version: TURN_SNAPSHOT_SCHEMA_VERSION.to_owned(),
            world_id: "stw_projection_family".to_owned(),
            session_id: "session".to_owned(),
            turn_id: "turn_0001".to_owned(),
            phase: "choice".to_owned(),
            current_event: None,
            protagonist_state: ProtagonistState {
                location: "place:gate".to_owned(),
                inventory: vec!["entry token".to_owned()],
                body: vec!["left wrist aches".to_owned()],
                mind: Vec::new(),
            },
            open_questions: Vec::new(),
            last_choices: Vec::new(),
        }
    }

    #[test]
    fn body_resource_family_loads_materialized_prompt_packet() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let mut materialized = BodyResourcePacket {
            world_id: "stw_projection_family".to_owned(),
            turn_id: "turn_0002".to_owned(),
            compiler_policy: BodyResourcePolicy {
                source: "test_materialized_state".to_owned(),
                ..BodyResourcePolicy::default()
            },
            ..BodyResourcePacket::default()
        };
        materialized.body_constraints.push(BodyConstraint {
            schema_version: BODY_CONSTRAINT_SCHEMA_VERSION.to_owned(),
            constraint_id: "body:constraint:test".to_owned(),
            visibility: BodyResourceVisibility::PlayerVisible,
            summary: "materialized fatigue".to_owned(),
            severity: 3,
            source_refs: vec!["test".to_owned()],
            scene_pressure_kinds: vec!["body".to_owned()],
        });
        write_json(
            &temp.path().join(BODY_RESOURCE_STATE_FILENAME),
            &materialized,
        )?;

        let packet = load_body_resource_prompt_packet(temp.path(), &snapshot())?;

        assert_eq!(packet.compiler_policy.source, "test_materialized_state");
        assert_eq!(packet.body_constraints[0].summary, "materialized fatigue");
        Ok(())
    }

    #[test]
    fn body_resource_family_falls_back_to_snapshot_compile() -> anyhow::Result<()> {
        let temp = tempdir()?;

        let packet = load_body_resource_prompt_packet(temp.path(), &snapshot())?;

        assert_eq!(packet.turn_id, "turn_0001");
        assert_eq!(packet.body_constraints[0].summary, "left wrist aches");
        assert_eq!(packet.resources[0].summary, "entry token");
        Ok(())
    }

    #[test]
    fn registry_lists_body_resource_family() {
        assert_eq!(
            PROJECTION_FAMILY_REGISTRY,
            &[ProjectionFamilyDescriptor {
                family_id: "body_resource",
                state_filename: BODY_RESOURCE_STATE_FILENAME,
                prompt_packet_kind: "active_body_resource_state",
            }]
        );
    }
}
