use serde::{Deserialize, Serialize};

pub const RUNTIME_CAPABILITY_PROFILE_SCHEMA_VERSION: &str =
    "singulari.runtime_capability_profile.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapabilityProfile {
    VnPlayer,
    McpWebReadOnly,
    McpWebPlay,
    TrustedLocal,
    WebgptText,
    WebgptImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSurfaceKind {
    PlayerFacing,
    BackendAdapter,
    TrustedOperator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeCapability {
    PlayerVisibleRead,
    HiddenAdjudicationRead,
    PlayerInputWrite,
    AgentCommitWrite,
    VisualJobCompletionWrite,
    LocalFilesystemPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilityBoundary {
    pub schema_version: String,
    pub profile: RuntimeCapabilityProfile,
    pub surface_kind: RuntimeSurfaceKind,
    pub capabilities: Vec<RuntimeCapability>,
}

impl RuntimeCapabilityProfile {
    #[must_use]
    pub fn boundary(self) -> RuntimeCapabilityBoundary {
        let (surface_kind, capabilities) = match self {
            Self::VnPlayer => (
                RuntimeSurfaceKind::PlayerFacing,
                vec![
                    RuntimeCapability::PlayerVisibleRead,
                    RuntimeCapability::PlayerInputWrite,
                ],
            ),
            Self::McpWebReadOnly => (
                RuntimeSurfaceKind::PlayerFacing,
                vec![RuntimeCapability::PlayerVisibleRead],
            ),
            Self::McpWebPlay => (
                RuntimeSurfaceKind::PlayerFacing,
                vec![
                    RuntimeCapability::PlayerVisibleRead,
                    RuntimeCapability::PlayerInputWrite,
                    RuntimeCapability::VisualJobCompletionWrite,
                ],
            ),
            Self::TrustedLocal => (
                RuntimeSurfaceKind::TrustedOperator,
                vec![
                    RuntimeCapability::PlayerVisibleRead,
                    RuntimeCapability::HiddenAdjudicationRead,
                    RuntimeCapability::PlayerInputWrite,
                    RuntimeCapability::AgentCommitWrite,
                    RuntimeCapability::VisualJobCompletionWrite,
                    RuntimeCapability::LocalFilesystemPath,
                ],
            ),
            Self::WebgptText => (
                RuntimeSurfaceKind::BackendAdapter,
                vec![RuntimeCapability::PlayerVisibleRead],
            ),
            Self::WebgptImage => (
                RuntimeSurfaceKind::BackendAdapter,
                vec![
                    RuntimeCapability::PlayerVisibleRead,
                    RuntimeCapability::VisualJobCompletionWrite,
                    RuntimeCapability::LocalFilesystemPath,
                ],
            ),
        };
        RuntimeCapabilityBoundary {
            schema_version: RUNTIME_CAPABILITY_PROFILE_SCHEMA_VERSION.to_owned(),
            profile: self,
            surface_kind,
            capabilities,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VnPlayer => "vn_player",
            Self::McpWebReadOnly => "mcp_web_read_only",
            Self::McpWebPlay => "mcp_web_play",
            Self::TrustedLocal => "trusted_local",
            Self::WebgptText => "webgpt_text",
            Self::WebgptImage => "webgpt_image",
        }
    }
}

impl RuntimeCapabilityBoundary {
    #[must_use]
    pub fn allows(&self, capability: RuntimeCapability) -> bool {
        self.capabilities.contains(&capability)
    }

    #[must_use]
    pub fn allows_hidden_adjudication(&self) -> bool {
        self.allows(RuntimeCapability::HiddenAdjudicationRead)
    }

    #[must_use]
    pub fn allows_agent_commit(&self) -> bool {
        self.allows(RuntimeCapability::AgentCommitWrite)
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeCapability, RuntimeCapabilityProfile, RuntimeSurfaceKind};

    #[test]
    fn player_and_web_surfaces_do_not_read_hidden_adjudication() {
        for profile in [
            RuntimeCapabilityProfile::VnPlayer,
            RuntimeCapabilityProfile::McpWebReadOnly,
            RuntimeCapabilityProfile::McpWebPlay,
        ] {
            let boundary = profile.boundary();
            assert_eq!(boundary.surface_kind, RuntimeSurfaceKind::PlayerFacing);
            assert!(boundary.allows(RuntimeCapability::PlayerVisibleRead));
            assert!(!boundary.allows_hidden_adjudication());
            assert!(!boundary.allows_agent_commit());
        }
    }

    #[test]
    fn webgpt_lanes_are_backend_adapters_without_hidden_reads() {
        for profile in [
            RuntimeCapabilityProfile::WebgptText,
            RuntimeCapabilityProfile::WebgptImage,
        ] {
            let boundary = profile.boundary();
            assert_eq!(boundary.surface_kind, RuntimeSurfaceKind::BackendAdapter);
            assert!(boundary.allows(RuntimeCapability::PlayerVisibleRead));
            assert!(!boundary.allows_hidden_adjudication());
            assert!(!boundary.allows_agent_commit());
        }
    }

    #[test]
    fn only_trusted_local_can_commit_agent_turns() {
        for profile in [
            RuntimeCapabilityProfile::VnPlayer,
            RuntimeCapabilityProfile::McpWebReadOnly,
            RuntimeCapabilityProfile::McpWebPlay,
            RuntimeCapabilityProfile::WebgptText,
            RuntimeCapabilityProfile::WebgptImage,
        ] {
            assert!(!profile.boundary().allows_agent_commit());
        }
        assert!(
            RuntimeCapabilityProfile::TrustedLocal
                .boundary()
                .allows_agent_commit()
        );
    }
}
