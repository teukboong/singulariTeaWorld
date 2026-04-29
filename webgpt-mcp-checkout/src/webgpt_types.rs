use serde::{Deserialize, Serialize};

/// Browser-session health vocabulary for the `WebGPT` MCP worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebGptHealthState {
    Ready,
    Degraded,
    ChallengePage,
    ExpiredSession,
    SelectorDrift,
    RateLimited,
    Busy,
    #[default]
    Unavailable,
}

impl WebGptHealthState {
    #[must_use]
    pub fn from_wire(value: &str) -> Self {
        match value {
            "ready" => Self::Ready,
            "degraded" => Self::Degraded,
            "challenge_page" => Self::ChallengePage,
            "expired_session" => Self::ExpiredSession,
            "selector_drift" => Self::SelectorDrift,
            "rate_limited" => Self::RateLimited,
            "busy" => Self::Busy,
            _ => Self::Unavailable,
        }
    }

    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::ChallengePage => "challenge_page",
            Self::ExpiredSession => "expired_session",
            Self::SelectorDrift => "selector_drift",
            Self::RateLimited => "rate_limited",
            Self::Busy => "busy",
            Self::Unavailable => "unavailable",
        }
    }

    #[must_use]
    pub const fn is_sendable(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// `WebGPT` MCP currently owns the research session shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebGptSessionKind {
    BridgeInteractive,
    #[default]
    McpResearch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebGptCitation {
    pub title: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebGptAnswer {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub session_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub answer_markdown: String,
    #[serde(default)]
    pub citations: Vec<WebGptCitation>,
    #[serde(default)]
    pub search_used: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<WebGptHealthState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_reasoning_level: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebGptProfileBootstrap {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_profile_dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WebGptHealthReport {
    pub state: WebGptHealthState,
    pub session_kind: WebGptSessionKind,
    #[serde(default)]
    pub live_probe: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub worker_entry: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub profile_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<WebGptProfileBootstrap>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_reasoning_level: Option<String>,
}
