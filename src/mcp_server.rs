use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::Utc;
use rmcp::model::{
    Annotated, CallToolRequestParams, CallToolResult, Content, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, Meta, PaginatedRequestParams, RawResource,
    RawResourceTemplate, ReadResourceRequestParams, ReadResourceResult, Resource, ResourceContents,
    ResourceTemplate, ServerCapabilities, ServerInfo, Tool, ToolAnnotations,
};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, handler::server::tool,
    service::RequestContext,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use singulari_world::{
    AgentCommitTurnOptions, AgentSubmitTurnOptions, AgentTurnResponse, BuildCodexViewOptions,
    BuildResumePackOptions, BuildVnPacketOptions, BuildWorldVisualAssetsOptions,
    ClaimVisualJobOptions, CompilePromptContextPacketOptions, CompleteVisualJobOptions,
    ReleaseVisualJobClaimOptions, StartWorldOptions, TurnFormRejection, TurnFormSubmission,
    advance_turn, assemble_agent_turn_response_from_form, build_codex_view, build_resume_pack,
    build_turn_form_spec, build_vn_packet, build_world_visual_assets, claim_visual_job,
    commit_agent_turn, compile_prompt_context_packet, complete_visual_job, enqueue_agent_turn,
    load_pending_agent_turn, refresh_world_docs, release_visual_job_claim, repair_world_db,
    resolve_store_paths, resolve_world_id, search_world_db, start_world, validate_world,
};
use std::{
    fs,
    net::{IpAddr, ToSocketAddrs},
    path::PathBuf,
    time::Duration,
};

const MCP_INSTRUCTIONS: &str = "Singulari World MCP server. It exposes standalone world-simulator tools for worldsim-agent play: player input can be queued as a pending agent turn, the trusted agent can read hidden adjudication context, and committed visible narrative is projected back into the VN packet. Browser-visible routes remain redacted.";
const WEB_IMAGE_COMPLETION_MAX_BASE64_CHARS: usize = 32 * 1024 * 1024;
const WEB_IMAGE_COMPLETION_MAX_URL_BYTES: u64 = 16 * 1024 * 1024;
const WEB_IMAGE_FETCH_TIMEOUT_SECS: u64 = 30;
const WEB_IMAGE_COMPLETION_STAGING_DIR: &str = "web_mcp_image_ingest";
const CHATGPT_VN_WIDGET_MIME_TYPE: &str = "text/html+skybridge";
const CHATGPT_WIDGET_PROBE_URI: &str = "ui://singulari-world/widget-probe.html";
const CHATGPT_WIDGET_PROBE_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <style>
    body { margin: 0; font: 14px system-ui, sans-serif; background: #10141f; color: #f8fafc; }
    main { padding: 18px; border: 1px solid rgba(148, 163, 184, .35); border-radius: 12px; }
    h1 { margin: 0 0 8px; font-size: 18px; }
    p { margin: 0; color: #cbd5e1; }
  </style>
</head>
<body>
  <main>
    <h1>Singulari Widget Probe</h1>
    <p id="message">If this card is visible, ChatGPT mounted the Skybridge widget resource.</p>
  </main>
  <script>
    const output = window.openai?.toolOutput || window.openai?.toolResponseMetadata || {};
    const message = output.message || output.structuredContent?.message;
    if (message) document.getElementById("message").textContent = message;
    window.openai?.notifyIntrinsicHeight?.();
  </script>
</body>
</html>"#;

#[derive(Clone)]
pub struct WorldsimMcpServer {
    tools: Vec<Tool>,
    profile: WorldsimMcpToolProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "Each MCP binary uses a different subset of tool profiles from this shared module"
)]
pub enum WorldsimMcpToolProfile {
    Full,
    WebPlay,
    WebAuthoring,
    WebReadOnly,
}

impl WorldsimMcpServer {
    #[allow(
        dead_code,
        reason = "Used by the stdio MCP binary; the web MCP binary selects an explicit profile instead"
    )]
    pub fn new() -> Self {
        Self::with_profile(WorldsimMcpToolProfile::Full)
    }

    pub fn with_profile(profile: WorldsimMcpToolProfile) -> Self {
        let mut tools = worldsim_mcp_tools();
        if profile.chatgpt_app_enabled() {
            attach_chatgpt_app_tool_metadata(&mut tools);
        }
        Self { tools, profile }
    }

    fn tool_allowed(&self, name: &str) -> bool {
        match self.profile {
            WorldsimMcpToolProfile::Full => true,
            WorldsimMcpToolProfile::WebPlay => matches!(
                name,
                "worldsim_current"
                    | "worldsim_submit_player_input"
                    | "worldsim_visual_assets"
                    | "worldsim_current_cg_image"
                    | "worldsim_widget_probe"
                    | "worldsim_probe_image_ingest"
                    | "worldsim_complete_visual_job_from_base64"
                    | "worldsim_complete_visual_job_from_url"
                    | "worldsim_resume_pack"
                    | "worldsim_search"
                    | "worldsim_codex_view"
                    | "worldsim_validate"
            ),
            WorldsimMcpToolProfile::WebAuthoring => matches!(
                name,
                "worldsim_current"
                    | "worldsim_next_turn_form"
                    | "worldsim_submit_turn_form"
                    | "worldsim_resume_pack"
                    | "worldsim_search"
                    | "worldsim_codex_view"
                    | "worldsim_validate"
            ),
            WorldsimMcpToolProfile::WebReadOnly => matches!(
                name,
                "worldsim_current"
                    | "worldsim_visual_assets"
                    | "worldsim_current_cg_image"
                    | "worldsim_widget_probe"
                    | "worldsim_resume_pack"
                    | "worldsim_search"
                    | "worldsim_codex_view"
                    | "worldsim_validate"
            ),
        }
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "MCP tool registry is an explicit dispatch surface"
)]
fn worldsim_mcp_tools() -> Vec<Tool> {
    vec![
        Tool::new(
            "worldsim_start_world",
            "Create and activate a world from compact seed text. Returns the world record, active binding, and initial packet refs.",
            tool::schema_for_type::<WorldsimStartWorldParams>(),
        ),
        Tool::new(
            "worldsim_current",
            "Return the current player-visible VN packet for the active or explicit world.",
            tool::schema_for_type::<WorldsimWorldParams>(),
        ),
        Tool::new(
            "worldsim_submit_player_input",
            "Submit a player choice or freeform action. Defaults to agent-authored mode and returns a pending turn.",
            tool::schema_for_type::<WorldsimSubmitPlayerInputParams>(),
        ),
        Tool::new(
            "worldsim_next_pending_turn",
            "Return the trusted local-agent pending turn packet, including private adjudication context.",
            tool::schema_for_type::<WorldsimWorldParams>(),
        ),
        Tool::new(
            "worldsim_next_turn_form",
            "Return a bounded form spec for the current pending agent turn. The text backend fills this form instead of authoring AgentTurnResponse JSON.",
            tool::schema_for_type::<WorldsimWorldParams>(),
        ),
        Tool::new(
            "worldsim_submit_turn_form",
            "Submit a bounded form for the current pending agent turn. The backend validates fields, assembles AgentTurnResponse, commits it, and returns the VN packet or field errors.",
            tool::schema_for_type::<WorldsimSubmitTurnFormParams>(),
        ),
        Tool::new(
            "worldsim_commit_agent_turn",
            "Commit an agent-authored response object, validate hidden-truth redaction, and return the updated VN packet.",
            tool::schema_for_type::<WorldsimCommitAgentTurnParams>(),
        ),
        Tool::new(
            "worldsim_visual_assets",
            "Return player-visible visual asset manifest and host image generation jobs. The MCP server does not call image providers; the host runs image_generation_call and saves to destination_path.",
            tool::schema_for_type::<WorldsimWorldParams>(),
        ),
        Tool::new(
            "worldsim_current_cg_image",
            "Return the current turn CG as MCP image content when a generated PNG already exists.",
            tool::schema_for_type::<WorldsimCurrentCgImageParams>(),
        ),
        Tool::new(
            "worldsim_widget_probe",
            "Diagnostic Apps SDK widget probe. Returns a tiny Skybridge HTML widget with no world-state dependency.",
            tool::schema_for_type::<WorldsimWidgetProbeParams>(),
        ),
        Tool::new(
            "worldsim_probe_image_ingest",
            "Record the exact image reference shape ChatGPT/App hosts can pass to this MCP server. This probe does not fetch remote URLs or complete visual jobs.",
            tool::schema_for_type::<WorldsimProbeImageIngestParams>(),
        ),
        Tool::new(
            "worldsim_complete_visual_job_from_base64",
            "Complete a pending visual job from a host-provided PNG base64 payload or data URL. This is the narrow ChatGPT web image-ingest path.",
            tool::schema_for_type::<WorldsimCompleteVisualJobFromBase64Params>(),
        ),
        Tool::new(
            "worldsim_complete_visual_job_from_url",
            "Complete a pending visual job from an HTTPS image/png URL. The URL is fetched with size, redirect, and host-shape limits before the normal PNG completion verifier runs.",
            tool::schema_for_type::<WorldsimCompleteVisualJobFromUrlParams>(),
        ),
        Tool::new(
            "worldsim_claim_visual_job",
            "Atomically claim one pending player-visible host image generation job. The image host should generate from the returned prompt and save to destination_path.",
            tool::schema_for_type::<WorldsimClaimVisualJobParams>(),
        ),
        Tool::new(
            "worldsim_complete_visual_job",
            "Mark a visual generation job complete after the host has saved a PNG to the returned destination_path, or copy a generated PNG into that destination.",
            tool::schema_for_type::<WorldsimCompleteVisualJobParams>(),
        ),
        Tool::new(
            "worldsim_release_visual_job",
            "Release a claimed visual generation job without accepting an asset, for host worker failure or user retry recovery.",
            tool::schema_for_type::<WorldsimReleaseVisualJobParams>(),
        ),
        Tool::new(
            "worldsim_resume_pack",
            "Return compact world continuity for context recovery.",
            tool::schema_for_type::<WorldsimResumePackParams>(),
        ),
        Tool::new(
            "worldsim_search",
            "Search player-visible world memory and DB projections.",
            tool::schema_for_type::<WorldsimSearchParams>(),
        ),
        Tool::new(
            "worldsim_codex_view",
            "Return the DB-backed player-visible Archive View with hidden truth filtered.",
            tool::schema_for_type::<WorldsimCodexViewParams>(),
        ),
        Tool::new(
            "worldsim_validate",
            "Validate JSON/JSONL/world.db consistency for a world.",
            tool::schema_for_type::<WorldsimWorldParams>(),
        ),
        Tool::new(
            "worldsim_repair_db",
            "Rebuild world.db projections from persisted JSON/JSONL evidence files.",
            tool::schema_for_type::<WorldsimWorldParams>(),
        ),
    ]
}

impl ServerHandler for WorldsimMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(MCP_INSTRUCTIONS.to_owned()),
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        let tools = self
            .tools
            .iter()
            .filter(|tool| self.tool_allowed(tool.name.as_ref()))
            .cloned()
            .collect();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    #[expect(
        clippy::too_many_lines,
        reason = "MCP tool dispatch keeps tool names and parameter parsers auditable"
    )]
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        if !self.tool_allowed(request.name.as_ref()) {
            return Err(McpError::invalid_params(
                format!(
                    "tool disabled by singulari-world MCP profile: {}",
                    request.name
                ),
                None,
            ));
        }
        let arguments = request.arguments.unwrap_or_default();
        match request.name.as_ref() {
            "worldsim_start_world" => {
                let params: WorldsimStartWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_start_world(params)).await
            }
            "worldsim_current" => {
                let params: WorldsimWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_current(params)).await
            }
            "worldsim_submit_player_input" => {
                let params: WorldsimSubmitPlayerInputParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_submit_player_input(params)).await
            }
            "worldsim_next_pending_turn" => {
                let params: WorldsimWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_next_pending_turn(params)).await
            }
            "worldsim_next_turn_form" => {
                let params: WorldsimWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_next_turn_form(params)).await
            }
            "worldsim_submit_turn_form" => {
                let params: WorldsimSubmitTurnFormParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_submit_turn_form(params)).await
            }
            "worldsim_commit_agent_turn" => {
                let params: WorldsimCommitAgentTurnParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_commit_agent_turn(params)).await
            }
            "worldsim_visual_assets" => {
                let params: WorldsimWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_visual_assets(params)).await
            }
            "worldsim_current_cg_image" => {
                let params: WorldsimCurrentCgImageParams = tool::parse_json_object(arguments)?;
                blocking_tool_result(move || worldsim_current_cg_image(params)).await
            }
            "worldsim_widget_probe" => {
                let params: WorldsimWidgetProbeParams = tool::parse_json_object(arguments)?;
                std::future::ready(worldsim_widget_probe(&params)).await
            }
            "worldsim_probe_image_ingest" => {
                let params: WorldsimProbeImageIngestParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_probe_image_ingest(params)).await
            }
            "worldsim_complete_visual_job_from_base64" => {
                let params: WorldsimCompleteVisualJobFromBase64Params =
                    tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_complete_visual_job_from_base64(params)).await
            }
            "worldsim_complete_visual_job_from_url" => {
                let params: WorldsimCompleteVisualJobFromUrlParams =
                    tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_complete_visual_job_from_url(params)).await
            }
            "worldsim_claim_visual_job" => {
                let params: WorldsimClaimVisualJobParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_claim_visual_job(params)).await
            }
            "worldsim_complete_visual_job" => {
                let params: WorldsimCompleteVisualJobParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_complete_visual_job(params)).await
            }
            "worldsim_release_visual_job" => {
                let params: WorldsimReleaseVisualJobParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_release_visual_job(params)).await
            }
            "worldsim_resume_pack" => {
                let params: WorldsimResumePackParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_resume_pack(params)).await
            }
            "worldsim_search" => {
                let params: WorldsimSearchParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_search(params)).await
            }
            "worldsim_codex_view" => {
                let params: WorldsimCodexViewParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_codex_view(params)).await
            }
            "worldsim_validate" => {
                let params: WorldsimWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_validate(params)).await
            }
            "worldsim_repair_db" => {
                let params: WorldsimWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_repair_db(params)).await
            }
            other => Err(McpError::invalid_params(
                format!("unknown tool: {other}"),
                None,
            )),
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        if !self.tool_allowed(name) {
            return None;
        }
        self.tools
            .iter()
            .find(|tool| tool.name.as_ref() == name)
            .cloned()
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        let resources = if self.profile.chatgpt_app_enabled() {
            vec![chatgpt_widget_probe_resource()]
        } else {
            Vec::new()
        };
        std::future::ready(Ok(ListResourcesResult::with_all_items(resources)))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        let templates = if self.profile.chatgpt_app_enabled() {
            vec![chatgpt_widget_probe_resource_template()]
        } else {
            Vec::new()
        };
        std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(templates)))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResult, McpError>> + Send + '_ {
        let result = match (self.profile.chatgpt_app_enabled(), request.uri.as_str()) {
            (true, CHATGPT_WIDGET_PROBE_URI) => Ok(chatgpt_widget_probe_contents()),
            _ => Err(McpError::invalid_params(
                format!("unknown singulari-world resource: {}", request.uri),
                None,
            )),
        };
        std::future::ready(result)
    }
}

impl WorldsimMcpToolProfile {
    const fn chatgpt_app_enabled(self) -> bool {
        matches!(
            self,
            Self::Full | Self::WebPlay | Self::WebAuthoring | Self::WebReadOnly
        )
    }
}

async fn blocking_tool<F, T>(operation: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Result<T> + Send + 'static,
    T: Serialize + Send + 'static,
{
    let value = tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            McpError::internal_error(format!("worldsim task join failed: {error}"), None)
        })?
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    json_tool_result(&value)
}

async fn blocking_tool_result<F>(operation: F) -> Result<CallToolResult, McpError>
where
    F: FnOnce() -> Result<CallToolResult> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| {
            McpError::internal_error(format!("worldsim task join failed: {error}"), None)
        })?
        .map_err(|error| McpError::internal_error(error.to_string(), None))
}

fn json_tool_result<T>(value: &T) -> Result<CallToolResult, McpError>
where
    T: Serialize,
{
    let value = serde_json::to_value(value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::structured(value))
}

fn attach_chatgpt_app_tool_metadata(tools: &mut [Tool]) {
    for tool in tools {
        match tool.name.as_ref() {
            "worldsim_current" => {
                tool.annotations = Some(ToolAnnotations::new().read_only(true));
                tool.meta = Some(chatgpt_backend_tool_meta("Loading world…", "World ready"));
            }
            "worldsim_submit_player_input" => {
                tool.annotations = Some(
                    ToolAnnotations::new()
                        .read_only(false)
                        .destructive(false)
                        .idempotent(false)
                        .open_world(false),
                );
                tool.meta = Some(chatgpt_backend_tool_meta("Sending choice…", "Choice sent"));
            }
            "worldsim_next_turn_form" => {
                tool.annotations = Some(ToolAnnotations::new().read_only(true));
                tool.meta = Some(chatgpt_backend_tool_meta(
                    "Loading turn form…",
                    "Turn form ready",
                ));
            }
            "worldsim_submit_turn_form" => {
                tool.annotations = Some(
                    ToolAnnotations::new()
                        .read_only(false)
                        .destructive(false)
                        .idempotent(false)
                        .open_world(false),
                );
                tool.meta = Some(chatgpt_backend_tool_meta(
                    "Submitting turn…",
                    "Turn committed",
                ));
            }
            "worldsim_widget_probe" => {
                tool.annotations = Some(ToolAnnotations::new().read_only(true));
                tool.meta = Some(chatgpt_app_tool_meta(
                    "Rendering widget probe…",
                    "Widget probe rendered",
                    true,
                    true,
                    CHATGPT_WIDGET_PROBE_URI,
                ));
            }
            "worldsim_visual_assets"
            | "worldsim_current_cg_image"
            | "worldsim_resume_pack"
            | "worldsim_search"
            | "worldsim_codex_view"
            | "worldsim_validate" => {
                tool.annotations = Some(ToolAnnotations::new().read_only(true));
                tool.meta = Some(chatgpt_backend_tool_meta(
                    "Checking world…",
                    "World data ready",
                ));
            }
            "worldsim_complete_visual_job_from_base64"
            | "worldsim_complete_visual_job_from_url" => {
                tool.annotations = Some(
                    ToolAnnotations::new()
                        .read_only(false)
                        .destructive(false)
                        .idempotent(false)
                        .open_world(false),
                );
                tool.meta = Some(chatgpt_backend_tool_meta("Saving CG…", "CG saved"));
            }
            _ => {}
        }
    }
}

fn chatgpt_app_tool_meta(
    invoking: &str,
    invoked: &str,
    widget_accessible: bool,
    renders_widget: bool,
    resource_uri: &str,
) -> Meta {
    let mut meta = Meta::new();
    meta.0.insert(
        "securitySchemes".to_owned(),
        serde_json::json!([{ "type": "noauth" }]),
    );
    meta.0.insert(
        "ui".to_owned(),
        serde_json::json!({
            "visibility": ["model", "app"],
            "resourceUri": resource_uri,
        }),
    );
    meta.0.insert(
        "openai/widgetAccessible".to_owned(),
        serde_json::json!(widget_accessible),
    );
    meta.0.insert(
        "openai/toolInvocation/invoking".to_owned(),
        serde_json::json!(invoking),
    );
    meta.0.insert(
        "openai/toolInvocation/invoked".to_owned(),
        serde_json::json!(invoked),
    );
    if renders_widget {
        meta.0.insert(
            "openai/outputTemplate".to_owned(),
            serde_json::json!(resource_uri),
        );
    }
    meta
}

fn chatgpt_backend_tool_meta(invoking: &str, invoked: &str) -> Meta {
    let mut meta = Meta::new();
    meta.0.insert(
        "securitySchemes".to_owned(),
        serde_json::json!([{ "type": "noauth" }]),
    );
    meta.0.insert(
        "openai/widgetAccessible".to_owned(),
        serde_json::json!(false),
    );
    meta.0.insert(
        "openai/toolInvocation/invoking".to_owned(),
        serde_json::json!(invoking),
    );
    meta.0.insert(
        "openai/toolInvocation/invoked".to_owned(),
        serde_json::json!(invoked),
    );
    meta
}

fn chatgpt_app_tool_result_meta(resource_uri: &str) -> Meta {
    let mut meta = Meta::new();
    meta.0.insert(
        "openai/outputTemplate".to_owned(),
        serde_json::json!(resource_uri),
    );
    meta.0.insert(
        "ui".to_owned(),
        serde_json::json!({
            "resourceUri": resource_uri,
        }),
    );
    meta
}

fn chatgpt_widget_probe_resource() -> Resource {
    let mut raw = RawResource::new(CHATGPT_WIDGET_PROBE_URI, "Singulari Widget Probe");
    raw.title = Some("Singulari Widget Probe".to_owned());
    raw.description =
        Some("Minimal Skybridge probe for verifying ChatGPT Apps SDK widget mounting.".to_owned());
    raw.mime_type = Some(CHATGPT_VN_WIDGET_MIME_TYPE.to_owned());
    raw.size = u32::try_from(CHATGPT_WIDGET_PROBE_HTML.len()).ok();
    raw.meta = Some(chatgpt_widget_probe_resource_meta());
    Annotated::new(raw, None)
}

fn chatgpt_widget_probe_resource_template() -> ResourceTemplate {
    Annotated::new(
        RawResourceTemplate {
            uri_template: CHATGPT_WIDGET_PROBE_URI.to_owned(),
            name: "Singulari Widget Probe".to_owned(),
            title: Some("Singulari Widget Probe".to_owned()),
            description: Some(
                "Minimal Skybridge probe for verifying ChatGPT Apps SDK widget mounting."
                    .to_owned(),
            ),
            mime_type: Some(CHATGPT_VN_WIDGET_MIME_TYPE.to_owned()),
            icons: None,
        },
        None,
    )
}

fn chatgpt_widget_probe_contents() -> ReadResourceResult {
    ReadResourceResult {
        contents: vec![ResourceContents::TextResourceContents {
            uri: CHATGPT_WIDGET_PROBE_URI.to_owned(),
            mime_type: Some(CHATGPT_VN_WIDGET_MIME_TYPE.to_owned()),
            text: CHATGPT_WIDGET_PROBE_HTML.to_owned(),
            meta: Some(chatgpt_widget_probe_resource_meta()),
        }],
    }
}

fn chatgpt_widget_probe_resource_meta() -> Meta {
    let mut meta = Meta::new();
    meta.0.insert(
        "openai/widgetDescription".to_owned(),
        serde_json::json!("A tiny diagnostic widget that proves whether ChatGPT mounts Skybridge UI resources from this MCP connector."),
    );
    meta.0.insert(
        "openai/widgetPrefersBorder".to_owned(),
        serde_json::json!(true),
    );
    meta.0.insert(
        "openai/widgetCSP".to_owned(),
        serde_json::json!({
            "connect_domains": [],
            "resource_domains": []
        }),
    );
    meta
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimWorldParams {
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimWidgetProbeParams {
    #[serde(default = "default_widget_probe_message")]
    message: String,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimStartWorldParams {
    seed_text: String,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
}

fn default_widget_probe_message() -> String {
    "Skybridge widget probe returned from Singulari World MCP.".to_owned()
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimSubmitPlayerInputParams {
    input: String,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    agent_authored: Option<bool>,
    #[serde(default)]
    narrative_level: Option<u8>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimCommitAgentTurnParams {
    response: Value,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimSubmitTurnFormParams {
    submission: TurnFormSubmission,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimClaimVisualJobParams {
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    slot: Option<String>,
    #[serde(default = "default_visual_job_claimed_by")]
    claimed_by: String,
    #[serde(default)]
    force: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimCurrentCgImageParams {
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimProbeImageIngestParams {
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    slot: Option<String>,
    #[serde(default)]
    image_base64: Option<String>,
    #[serde(default)]
    image_url: Option<String>,
    #[serde(default)]
    resource_uri: Option<String>,
    #[serde(default)]
    file_id: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    source_note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimCompleteVisualJobFromBase64Params {
    slot: String,
    image_base64: String,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    claim_id: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    #[serde(default)]
    source_note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimCompleteVisualJobFromUrlParams {
    slot: String,
    image_url: String,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    claim_id: Option<String>,
    #[serde(default)]
    source_note: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimCompleteVisualJobParams {
    slot: String,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    claim_id: Option<String>,
    #[serde(default)]
    generated_path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimReleaseVisualJobParams {
    slot: String,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimResumePackParams {
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    recent_events: Option<usize>,
    #[serde(default)]
    recent_memories: Option<usize>,
    #[serde(default)]
    chapters: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimSearchParams {
    query: String,
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimCodexViewParams {
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct WorldsimStartedWorldResponse {
    status: &'static str,
    seed: singulari_world::WorldSeed,
    world: singulari_world::WorldRecord,
    active: singulari_world::ActiveWorldBinding,
    session_id: String,
    world_dir: String,
    snapshot: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WorldsimSubmitPlayerInputResponse {
    WaitingAgent {
        pending: Box<singulari_world::PendingAgentTurn>,
    },
    Committed {
        packet: Box<singulari_world::VnPacket>,
        turn_id: String,
    },
}

#[derive(Debug, Serialize)]
struct WorldsimTurnFormResponse {
    world_id: String,
    turn_id: String,
    pending_ref: String,
    form: singulari_world::TurnFormSpec,
}

#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WorldsimSubmitTurnFormResponse {
    Committed {
        world_id: String,
        turn_id: String,
        committed: Box<singulari_world::CommittedAgentTurn>,
    },
    FieldErrors {
        rejection: TurnFormRejection,
    },
    AuditRejected {
        world_id: String,
        turn_id: String,
        field_errors: Vec<singulari_world::TurnFormFieldError>,
        retryable: bool,
    },
}

#[derive(Debug, Serialize)]
struct WorldsimSearchResponse {
    world_id: String,
    query: String,
    hits: Vec<singulari_world::WorldSearchHit>,
}

#[derive(Debug, Serialize)]
struct WorldsimCurrentCgImageMetadata {
    world_id: String,
    turn_id: String,
    asset_path: String,
    asset_url: String,
    mime_type: String,
    bytes: usize,
}

#[derive(Debug, Serialize)]
struct WorldsimProbeImageIngestResponse {
    status: &'static str,
    world_id: String,
    probe_path: String,
    received: WorldsimProbeImageIngestSummary,
}

#[derive(Debug, Serialize)]
struct WorldsimProbeImageIngestSummary {
    slot: Option<String>,
    has_image_base64: bool,
    image_base64_bytes: Option<usize>,
    image_url: Option<String>,
    resource_uri: Option<String>,
    file_id: Option<String>,
    mime_type: Option<String>,
    source_note: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorldsimCompleteVisualJobFromBase64Response {
    status: &'static str,
    world_id: String,
    slot: String,
    completion: singulari_world::VisualJobCompletion,
    staged_path: String,
    staged_bytes: usize,
    staged_file_removed: bool,
    source_note: Option<String>,
}

#[derive(Debug, Serialize)]
struct WorldsimCompleteVisualJobFromUrlResponse {
    status: &'static str,
    world_id: String,
    slot: String,
    completion: singulari_world::VisualJobCompletion,
    fetched_url: String,
    final_url: String,
    fetched_bytes: usize,
    staged_path: String,
    staged_file_removed: bool,
    source_note: Option<String>,
}

fn worldsim_start_world(params: WorldsimStartWorldParams) -> Result<WorldsimStartedWorldResponse> {
    let started = start_world(&StartWorldOptions {
        seed_text: params.seed_text,
        world_id: params.world_id,
        title: params.title,
        randomize_opening_seed: false,
        store_root: store_root(params.store_root),
        session_id: params.session_id,
    })?;
    Ok(WorldsimStartedWorldResponse {
        status: "created",
        seed: started.seed,
        world: started.initialized.world,
        active: started.active_binding,
        session_id: started.initialized.session_id,
        world_dir: started.initialized.world_dir.display().to_string(),
        snapshot: started.initialized.snapshot_path.display().to_string(),
    })
}

fn worldsim_current(params: WorldsimWorldParams) -> Result<singulari_world::VnPacket> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let mut options = BuildVnPacketOptions::new(world_id);
    options.store_root = store_root;
    build_vn_packet(&options)
}

fn worldsim_widget_probe(params: &WorldsimWidgetProbeParams) -> Result<CallToolResult, McpError> {
    let payload = serde_json::json!({
        "message": params.message,
        "fromTool": "worldsim_widget_probe",
    });
    let mut result = json_tool_result(&payload)?;
    result.content = vec![Content::text(format!(
        "Widget probe ready with message: {}",
        payload["message"].as_str().unwrap_or("probe")
    ))];
    result.meta = Some(chatgpt_app_tool_result_meta(CHATGPT_WIDGET_PROBE_URI));
    Ok(result)
}

fn worldsim_submit_player_input(
    params: WorldsimSubmitPlayerInputParams,
) -> Result<WorldsimSubmitPlayerInputResponse> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    if params.agent_authored.unwrap_or(true) {
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root,
            world_id,
            input: params.input,
            narrative_level: params.narrative_level,
        })?;
        return Ok(WorldsimSubmitPlayerInputResponse::WaitingAgent {
            pending: Box::new(pending),
        });
    }

    let advanced = advance_turn(&singulari_world::AdvanceTurnOptions {
        store_root: store_root.clone(),
        world_id: world_id.clone(),
        input: params.input,
    })?;
    let mut packet_options = BuildVnPacketOptions::new(world_id);
    packet_options.store_root = store_root;
    let packet = build_vn_packet(&packet_options)?;
    Ok(WorldsimSubmitPlayerInputResponse::Committed {
        packet: Box::new(packet),
        turn_id: advanced.snapshot.turn_id,
    })
}

fn worldsim_next_pending_turn(
    params: WorldsimWorldParams,
) -> Result<singulari_world::PendingAgentTurn> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    load_pending_agent_turn(store_root.as_deref(), world_id.as_str())
}

fn worldsim_next_turn_form(params: WorldsimWorldParams) -> Result<WorldsimTurnFormResponse> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let pending = load_pending_agent_turn(store_root.as_deref(), world_id.as_str())?;
    let context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root: store_root.as_deref(),
        pending: &pending,
        engine_session_kind: "webgpt_tool_form",
    })?;
    let form = build_turn_form_spec(&pending, &context);
    Ok(WorldsimTurnFormResponse {
        world_id,
        turn_id: pending.turn_id,
        pending_ref: pending.pending_ref,
        form,
    })
}

fn worldsim_submit_turn_form(
    params: WorldsimSubmitTurnFormParams,
) -> Result<WorldsimSubmitTurnFormResponse> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(
        store_root.as_deref(),
        params
            .world_id
            .as_deref()
            .or(Some(params.submission.world_id.as_str())),
    )?;
    if params.submission.world_id != world_id {
        bail!(
            "worldsim_submit_turn_form world_id mismatch: argument={world_id}, submission={}",
            params.submission.world_id
        );
    }
    let pending = load_pending_agent_turn(store_root.as_deref(), world_id.as_str())?;
    let context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root: store_root.as_deref(),
        pending: &pending,
        engine_session_kind: "webgpt_tool_form",
    })?;
    let response =
        match assemble_agent_turn_response_from_form(&pending, &context, params.submission) {
            Ok(response) => response,
            Err(rejection) => {
                return Ok(WorldsimSubmitTurnFormResponse::FieldErrors { rejection });
            }
        };
    match commit_agent_turn(&AgentCommitTurnOptions {
        store_root,
        world_id: world_id.clone(),
        response,
    }) {
        Ok(committed) => Ok(WorldsimSubmitTurnFormResponse::Committed {
            world_id,
            turn_id: committed.turn_id.clone(),
            committed: Box::new(committed),
        }),
        Err(error) => Ok(WorldsimSubmitTurnFormResponse::AuditRejected {
            world_id,
            turn_id: pending.turn_id,
            field_errors: vec![singulari_world::TurnFormFieldError {
                field_path: "submission".to_owned(),
                message: format!("assembled turn response was rejected: {error:#}"),
                allowed_values: Vec::new(),
            }],
            retryable: true,
        }),
    }
}

fn worldsim_commit_agent_turn(
    params: WorldsimCommitAgentTurnParams,
) -> Result<singulari_world::CommittedAgentTurn> {
    let store_root = store_root(params.store_root);
    let response: AgentTurnResponse = serde_json::from_value(params.response)
        .context("worldsim_commit_agent_turn response object is not a valid agent turn response")?;
    let world_id = resolve_world_id(store_root.as_deref(), Some(response.world_id.as_str()))?;
    if let Some(requested_world_id) = params.world_id.as_deref()
        && requested_world_id != world_id
    {
        bail!(
            "worldsim_commit_agent_turn world_id mismatch: argument={requested_world_id}, response={world_id}"
        );
    }
    commit_agent_turn(&AgentCommitTurnOptions {
        store_root,
        world_id,
        response,
    })
}

fn worldsim_visual_assets(
    params: WorldsimWorldParams,
) -> Result<singulari_world::WorldVisualAssets> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root,
        world_id,
    })
}

fn worldsim_current_cg_image(params: WorldsimCurrentCgImageParams) -> Result<CallToolResult> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let mut packet_options = BuildVnPacketOptions::new(world_id.clone());
    packet_options.store_root = store_root;
    let packet = build_vn_packet(&packet_options)?;
    if !packet.image.exists {
        let value = serde_json::json!({
            "status": "missing",
            "world_id": world_id,
            "turn_id": packet.turn_id,
            "asset_path": packet.image.recommended_path,
            "asset_url": packet.image.asset_url,
            "reason": "current turn CG PNG does not exist yet"
        });
        return json_tool_result(&value).map_err(|error| anyhow::anyhow!(error.to_string()));
    }

    let asset_path = PathBuf::from(packet.image.recommended_path.as_str());
    let bytes = fs::read(asset_path.as_path())
        .with_context(|| format!("failed to read current CG {}", asset_path.display()))?;
    let encoded = STANDARD.encode(bytes.as_slice());
    let metadata = WorldsimCurrentCgImageMetadata {
        world_id,
        turn_id: packet.turn_id,
        asset_path: asset_path.display().to_string(),
        asset_url: packet.image.asset_url,
        mime_type: "image/png".to_owned(),
        bytes: bytes.len(),
    };
    let structured = serde_json::to_value(&metadata)?;
    Ok(CallToolResult {
        content: vec![
            Content::text(serde_json::to_string(&metadata)?),
            Content::image(encoded, "image/png"),
        ],
        structured_content: Some(structured),
        is_error: Some(false),
        meta: None,
    })
}

fn worldsim_probe_image_ingest(
    params: WorldsimProbeImageIngestParams,
) -> Result<WorldsimProbeImageIngestResponse> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let paths = resolve_store_paths(store_root.as_deref())?;
    let probe_dir = paths
        .worlds_dir
        .join(world_id.as_str())
        .join("visual_jobs")
        .join("ingest_probes");
    fs::create_dir_all(probe_dir.as_path())
        .with_context(|| format!("failed to create {}", probe_dir.display()))?;

    let image_base64_bytes = params.image_base64.as_ref().map(String::len);
    let summary = WorldsimProbeImageIngestSummary {
        slot: params.slot,
        has_image_base64: params.image_base64.is_some(),
        image_base64_bytes,
        image_url: params.image_url,
        resource_uri: params.resource_uri,
        file_id: params.file_id,
        mime_type: params.mime_type,
        source_note: params.source_note,
    };
    let generated_at = Utc::now().to_rfc3339();
    let probe_path = probe_dir.join(format!(
        "{}-image-ingest-probe.json",
        generated_at.replace([':', '.'], "-")
    ));
    let record = serde_json::json!({
        "schema_version": "singulari.web_mcp_image_ingest_probe.v1",
        "generated_at": generated_at,
        "world_id": world_id,
        "received": &summary,
        "retention_policy": "image payload bytes are not persisted by this probe; only shape and reference metadata are recorded"
    });
    fs::write(&probe_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to write {}", probe_path.display()))?;
    Ok(WorldsimProbeImageIngestResponse {
        status: "recorded",
        world_id,
        probe_path: probe_path.display().to_string(),
        received: summary,
    })
}

fn worldsim_complete_visual_job_from_base64(
    params: WorldsimCompleteVisualJobFromBase64Params,
) -> Result<WorldsimCompleteVisualJobFromBase64Response> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let image_payload =
        parse_png_base64_payload(params.image_base64.as_str(), params.mime_type.as_deref())?;
    let image_bytes = STANDARD
        .decode(image_payload.base64.as_bytes())
        .with_context(|| {
            format!(
                "worldsim_complete_visual_job_from_base64 invalid base64: slot={}",
                params.slot
            )
        })?;
    let staged = complete_visual_job_from_web_image_bytes(WebImageCompletionBytesOptions {
        store_root,
        world_id: world_id.clone(),
        slot: params.slot.clone(),
        claim_id: params.claim_id,
        image_bytes: image_bytes.as_slice(),
        staging_reason: "base64",
    })?;
    Ok(WorldsimCompleteVisualJobFromBase64Response {
        status: "completed",
        world_id,
        slot: params.slot,
        completion: staged.completion,
        staged_path: staged.staged_path,
        staged_bytes: staged.staged_bytes,
        staged_file_removed: staged.staged_file_removed,
        source_note: params.source_note,
    })
}

fn worldsim_complete_visual_job_from_url(
    params: WorldsimCompleteVisualJobFromUrlParams,
) -> Result<WorldsimCompleteVisualJobFromUrlResponse> {
    validate_fetchable_image_url(params.image_url.as_str())?;
    let response = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(WEB_IMAGE_FETCH_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .context("failed to build image URL fetch client")?
        .get(params.image_url.as_str())
        .header(reqwest::header::ACCEPT, "image/png")
        .send()
        .with_context(|| {
            format!(
                "worldsim_complete_visual_job_from_url fetch failed: slot={}, url={}",
                params.slot, params.image_url
            )
        })?
        .error_for_status()
        .with_context(|| {
            format!(
                "worldsim_complete_visual_job_from_url non-success status: slot={}, url={}",
                params.slot, params.image_url
            )
        })?;
    let final_url = response.url().to_string();
    validate_fetchable_image_url(final_url.as_str())?;
    if let Some(content_type) = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        && !content_type
            .split(';')
            .next()
            .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("image/png"))
    {
        bail!("worldsim_complete_visual_job_from_url only accepts image/png, got {content_type}");
    }
    if let Some(content_length) = response.content_length()
        && content_length > WEB_IMAGE_COMPLETION_MAX_URL_BYTES
    {
        bail!(
            "worldsim_complete_visual_job_from_url payload too large: bytes={content_length}, max={WEB_IMAGE_COMPLETION_MAX_URL_BYTES}"
        );
    }
    let image_bytes = response
        .bytes()
        .context("failed to read fetched image body")?;
    if image_bytes.len() as u64 > WEB_IMAGE_COMPLETION_MAX_URL_BYTES {
        bail!(
            "worldsim_complete_visual_job_from_url payload too large: bytes={}, max={WEB_IMAGE_COMPLETION_MAX_URL_BYTES}",
            image_bytes.len()
        );
    }
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let staged = complete_visual_job_from_web_image_bytes(WebImageCompletionBytesOptions {
        store_root,
        world_id: world_id.clone(),
        slot: params.slot.clone(),
        claim_id: params.claim_id,
        image_bytes: image_bytes.as_ref(),
        staging_reason: "url",
    })?;
    Ok(WorldsimCompleteVisualJobFromUrlResponse {
        status: "completed",
        world_id,
        slot: params.slot,
        completion: staged.completion,
        fetched_url: params.image_url,
        final_url,
        fetched_bytes: image_bytes.len(),
        staged_path: staged.staged_path,
        staged_file_removed: staged.staged_file_removed,
        source_note: params.source_note,
    })
}

struct WebImageCompletionBytesOptions<'a> {
    store_root: Option<PathBuf>,
    world_id: String,
    slot: String,
    claim_id: Option<String>,
    image_bytes: &'a [u8],
    staging_reason: &'static str,
}

struct WebImageCompletionStagedResult {
    completion: singulari_world::VisualJobCompletion,
    staged_path: String,
    staged_bytes: usize,
    staged_file_removed: bool,
}

fn complete_visual_job_from_web_image_bytes(
    options: WebImageCompletionBytesOptions<'_>,
) -> Result<WebImageCompletionStagedResult> {
    let paths = resolve_store_paths(options.store_root.as_deref())?;
    let staged_dir = paths
        .worlds_dir
        .join(options.world_id.as_str())
        .join("visual_jobs")
        .join(WEB_IMAGE_COMPLETION_STAGING_DIR);
    fs::create_dir_all(staged_dir.as_path())
        .with_context(|| format!("failed to create {}", staged_dir.display()))?;
    let staged_path = staged_dir.join(format!(
        "{}-{}-{}.png",
        Utc::now().to_rfc3339().replace([':', '.'], "-"),
        options.staging_reason,
        safe_probe_component(options.slot.as_str())
    ));
    let staged_bytes = options.image_bytes.len();
    fs::write(staged_path.as_path(), options.image_bytes)
        .with_context(|| format!("failed to write {}", staged_path.display()))?;
    let completion = complete_visual_job(&CompleteVisualJobOptions {
        store_root: options.store_root,
        world_id: options.world_id,
        slot: options.slot,
        claim_id: options.claim_id,
        generated_path: Some(staged_path.clone()),
    })?;
    let staged_file_removed = fs::remove_file(staged_path.as_path()).is_ok();
    Ok(WebImageCompletionStagedResult {
        completion,
        staged_path: staged_path.display().to_string(),
        staged_bytes,
        staged_file_removed,
    })
}

fn validate_fetchable_image_url(raw: &str) -> Result<()> {
    let normalized_host = validate_fetchable_image_url_shape(raw)?;
    validate_fetchable_image_url_dns(raw, normalized_host.as_str())?;
    Ok(())
}

fn validate_fetchable_image_url_shape(raw: &str) -> Result<String> {
    let url = reqwest::Url::parse(raw)
        .with_context(|| format!("worldsim_complete_visual_job_from_url invalid URL: {raw}"))?;
    if url.scheme() != "https" {
        bail!("worldsim_complete_visual_job_from_url only accepts https URLs");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("worldsim_complete_visual_job_from_url rejects URLs with embedded credentials");
    }
    let Some(host) = url.host_str() else {
        bail!("worldsim_complete_visual_job_from_url URL host is missing");
    };
    let normalized_host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if normalized_host == "localhost" || normalized_host.ends_with(".localhost") {
        bail!("worldsim_complete_visual_job_from_url rejects localhost URLs");
    }
    if let Ok(ip) = normalized_host.parse::<IpAddr>()
        && is_private_or_local_ip(ip)
    {
        bail!("worldsim_complete_visual_job_from_url rejects private or local IP URLs");
    }
    Ok(normalized_host)
}

fn validate_fetchable_image_url_dns(raw: &str, normalized_host: &str) -> Result<()> {
    if normalized_host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let url = reqwest::Url::parse(raw)
        .with_context(|| format!("worldsim_complete_visual_job_from_url invalid URL: {raw}"))?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs = (normalized_host, port).to_socket_addrs().with_context(|| {
        format!(
            "worldsim_complete_visual_job_from_url DNS resolution failed: host={normalized_host}"
        )
    })?;
    let mut saw_addr = false;
    for addr in addrs {
        saw_addr = true;
        if is_private_or_local_ip(addr.ip()) {
            bail!(
                "worldsim_complete_visual_job_from_url rejects host resolving to private or local IP: host={normalized_host}, ip={}",
                addr.ip()
            );
        }
    }
    if !saw_addr {
        bail!(
            "worldsim_complete_visual_job_from_url DNS resolution returned no addresses: host={normalized_host}"
        );
    }
    Ok(())
}

fn is_private_or_local_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_broadcast()
                || ip.is_documentation()
                || ip.is_unspecified()
        }
        IpAddr::V6(ip) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.segments()[0] & 0xfe00 == 0xfc00
                || ip.segments()[0] & 0xffc0 == 0xfe80
        }
    }
}

struct ParsedPngBase64Payload<'a> {
    base64: String,
    _raw: &'a str,
}

fn parse_png_base64_payload<'a>(
    raw: &'a str,
    explicit_mime_type: Option<&str>,
) -> Result<ParsedPngBase64Payload<'a>> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        bail!("worldsim_complete_visual_job_from_base64 image_base64 is empty");
    }
    if trimmed.len() > WEB_IMAGE_COMPLETION_MAX_BASE64_CHARS {
        bail!(
            "worldsim_complete_visual_job_from_base64 payload too large: chars={}, max={WEB_IMAGE_COMPLETION_MAX_BASE64_CHARS}",
            trimmed.len()
        );
    }

    let (mime_type, base64_part) = if let Some(data_url) = trimmed.strip_prefix("data:") {
        let Some((header, body)) = data_url.split_once(',') else {
            bail!("worldsim_complete_visual_job_from_base64 malformed data URL");
        };
        let mime = header.split(';').next().unwrap_or_default();
        (Some(mime), body)
    } else {
        (explicit_mime_type, trimmed)
    };
    if let Some(mime) = mime_type
        && mime != "image/png"
    {
        bail!("worldsim_complete_visual_job_from_base64 only accepts image/png, got {mime}");
    }
    let base64 = base64_part
        .chars()
        .filter(|character| !character.is_ascii_whitespace())
        .collect::<String>();
    Ok(ParsedPngBase64Payload { base64, _raw: raw })
}

fn safe_probe_component(raw: &str) -> String {
    raw.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn worldsim_claim_visual_job(
    params: WorldsimClaimVisualJobParams,
) -> Result<singulari_world::VisualJobClaimOutcome> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let extra_jobs = current_turn_visual_jobs(store_root.as_ref(), world_id.as_str())?;
    claim_visual_job(&ClaimVisualJobOptions {
        store_root,
        world_id,
        slot: params.slot,
        claimed_by: params.claimed_by,
        force: params.force,
        extra_jobs,
    })
}

fn current_turn_visual_jobs(
    store_root: Option<&PathBuf>,
    world_id: &str,
) -> Result<Vec<singulari_world::ImageGenerationJob>> {
    let mut packet_options = BuildVnPacketOptions::new(world_id.to_owned());
    packet_options.store_root = store_root.cloned();
    let packet = build_vn_packet(&packet_options)?;
    Ok(packet.image.image_generation_job.into_iter().collect())
}

fn worldsim_complete_visual_job(
    params: WorldsimCompleteVisualJobParams,
) -> Result<singulari_world::VisualJobCompletion> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    complete_visual_job(&CompleteVisualJobOptions {
        store_root,
        world_id,
        slot: params.slot,
        claim_id: params.claim_id,
        generated_path: params.generated_path.map(PathBuf::from),
    })
}

fn worldsim_release_visual_job(
    params: WorldsimReleaseVisualJobParams,
) -> Result<singulari_world::VisualJobClaimRelease> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    release_visual_job_claim(&ReleaseVisualJobClaimOptions {
        store_root,
        world_id,
        slot: params.slot,
    })
}

fn worldsim_resume_pack(params: WorldsimResumePackParams) -> Result<singulari_world::ResumePack> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let mut options = BuildResumePackOptions::new(world_id);
    options.store_root = store_root;
    options.recent_events = params.recent_events.unwrap_or(8);
    options.recent_memories = params.recent_memories.unwrap_or(8);
    options.chapter_limit = params.chapters.unwrap_or(3);
    build_resume_pack(&options)
}

fn worldsim_search(params: WorldsimSearchParams) -> Result<WorldsimSearchResponse> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let paths = resolve_store_paths(store_root.as_deref())?;
    let world_dir = paths.worlds_dir.join(world_id.as_str());
    let hits = search_world_db(
        &world_dir,
        world_id.as_str(),
        params.query.as_str(),
        params.limit.unwrap_or(10),
    )?;
    Ok(WorldsimSearchResponse {
        world_id,
        query: params.query,
        hits,
    })
}

fn worldsim_codex_view(params: WorldsimCodexViewParams) -> Result<singulari_world::CodexView> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let mut options = BuildCodexViewOptions::new(world_id);
    options.store_root = store_root;
    options.query = params.query;
    options.limit = params.limit.unwrap_or(12);
    build_codex_view(&options)
}

fn worldsim_validate(params: WorldsimWorldParams) -> Result<singulari_world::ValidationReport> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    validate_world(store_root.as_deref(), world_id.as_str())
}

fn worldsim_repair_db(params: WorldsimWorldParams) -> Result<singulari_world::WorldDbRepairReport> {
    let store_root = store_root(params.store_root);
    let world_id = resolve_world_id(store_root.as_deref(), params.world_id.as_deref())?;
    let paths = resolve_store_paths(store_root.as_deref())?;
    let world_dir = paths.worlds_dir.join(world_id.as_str());
    let report = repair_world_db(&world_dir, world_id.as_str())?;
    refresh_world_docs(&world_dir)?;
    Ok(report)
}

fn store_root(raw: Option<String>) -> Option<PathBuf> {
    raw.map(PathBuf::from)
}

fn default_visual_job_claimed_by() -> String {
    "webgpt_image_worker".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn json_tool_result_exposes_structured_content() -> anyhow::Result<()> {
        let result = json_tool_result(&serde_json::json!({
                "job": {
                "image_generation_call": {
                    "capability": "image_generation",
                    "destination_path": "/tmp/example.png"
                }
            }
        }))?;
        let structured = result
            .structured_content
            .as_ref()
            .context("structured content missing")?;
        assert_eq!(
            structured["job"]["image_generation_call"]["capability"],
            "image_generation"
        );
        Ok(())
    }

    #[test]
    fn mcp_visual_claim_includes_current_turn_cg_job() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store_root = temp.path().join("store");
        let claimed = worldsim_claim_visual_job(WorldsimClaimVisualJobParams {
            store_root: Some(store_root.display().to_string()),
            world_id: Some("stw_mcp_turn_cg".to_owned()),
            slot: Some("turn_cg:turn_0005".to_owned()),
            claimed_by: "mcp-test".to_owned(),
            force: false,
        });
        assert!(claimed.is_err(), "world should not exist before creation");
        worldsim_start_world(WorldsimStartWorldParams {
            seed_text: "mcp turn cg fantasy smoke".to_owned(),
            store_root: Some(store_root.display().to_string()),
            world_id: Some("stw_mcp_turn_cg".to_owned()),
            title: None,
            session_id: None,
        })?;
        for _ in 0..5 {
            advance_turn(&singulari_world::AdvanceTurnOptions {
                store_root: Some(store_root.clone()),
                world_id: "stw_mcp_turn_cg".to_owned(),
                input: "1".to_owned(),
            })?;
        }
        let claimed = worldsim_claim_visual_job(WorldsimClaimVisualJobParams {
            store_root: Some(store_root.display().to_string()),
            world_id: Some("stw_mcp_turn_cg".to_owned()),
            slot: Some("turn_cg:turn_0005".to_owned()),
            claimed_by: "mcp-test".to_owned(),
            force: false,
        })?;
        let singulari_world::VisualJobClaimOutcome::Claimed { claim } = claimed else {
            anyhow::bail!("turn CG job should be claimable through MCP");
        };
        assert_eq!(claim.slot, "turn_cg:turn_0005");
        assert_eq!(
            claim.job.image_generation_call.capability,
            "image_generation"
        );
        Ok(())
    }

    #[test]
    fn web_play_profile_excludes_trusted_agent_tools() {
        let server = WorldsimMcpServer::with_profile(WorldsimMcpToolProfile::WebPlay);
        assert!(server.tool_allowed("worldsim_submit_player_input"));
        assert!(server.tool_allowed("worldsim_probe_image_ingest"));
        assert!(server.tool_allowed("worldsim_complete_visual_job_from_base64"));
        assert!(server.tool_allowed("worldsim_complete_visual_job_from_url"));
        assert!(!server.tool_allowed("worldsim_next_pending_turn"));
        assert!(!server.tool_allowed("worldsim_next_turn_form"));
        assert!(!server.tool_allowed("worldsim_submit_turn_form"));
        assert!(!server.tool_allowed("worldsim_commit_agent_turn"));
        assert!(!server.tool_allowed("worldsim_repair_db"));
    }

    #[test]
    fn web_authoring_profile_exposes_bounded_turn_form_tools_only() {
        let server = WorldsimMcpServer::with_profile(WorldsimMcpToolProfile::WebAuthoring);
        assert!(server.tool_allowed("worldsim_current"));
        assert!(server.tool_allowed("worldsim_next_turn_form"));
        assert!(server.tool_allowed("worldsim_submit_turn_form"));
        assert!(server.tool_allowed("worldsim_resume_pack"));
        assert!(!server.tool_allowed("worldsim_submit_player_input"));
        assert!(!server.tool_allowed("worldsim_next_pending_turn"));
        assert!(!server.tool_allowed("worldsim_commit_agent_turn"));
        assert!(!server.tool_allowed("worldsim_claim_visual_job"));
        assert!(!server.tool_allowed("worldsim_complete_visual_job"));
        assert!(!server.tool_allowed("worldsim_repair_db"));
    }

    #[test]
    fn web_play_profile_does_not_advertise_chatgpt_vn_widget() -> anyhow::Result<()> {
        let server = WorldsimMcpServer::with_profile(WorldsimMcpToolProfile::WebPlay);
        let current_tool = server
            .get_tool("worldsim_current")
            .context("worldsim_current tool missing")?;
        let meta = current_tool.meta.context("worldsim_current meta missing")?;
        assert!(!meta.0.contains_key("openai/outputTemplate"));
        assert!(!meta.0.contains_key("ui"));
        assert_eq!(meta.0["openai/widgetAccessible"], serde_json::json!(false));

        let submit_tool = server
            .get_tool("worldsim_submit_player_input")
            .context("worldsim_submit_player_input tool missing")?;
        let submit_meta = submit_tool.meta.context("submit meta missing")?;
        assert!(!submit_meta.0.contains_key("openai/outputTemplate"));
        assert_eq!(
            submit_meta.0["openai/widgetAccessible"],
            serde_json::json!(false)
        );
        let probe_tool = server
            .get_tool("worldsim_widget_probe")
            .context("worldsim_widget_probe tool missing")?;
        let probe_meta = probe_tool.meta.context("probe meta missing")?;
        assert_eq!(
            probe_meta.0["openai/outputTemplate"],
            serde_json::json!(CHATGPT_WIDGET_PROBE_URI)
        );
        assert_eq!(
            probe_meta.0["ui"]["resourceUri"],
            serde_json::json!(CHATGPT_WIDGET_PROBE_URI)
        );
        Ok(())
    }

    #[test]
    fn chatgpt_widget_probe_result_meta_points_to_probe_resource() {
        let probe_meta = chatgpt_app_tool_result_meta(CHATGPT_WIDGET_PROBE_URI);
        assert_eq!(
            probe_meta.0["openai/outputTemplate"],
            serde_json::json!(CHATGPT_WIDGET_PROBE_URI)
        );
        assert_eq!(
            probe_meta.0["ui"]["resourceUri"],
            serde_json::json!(CHATGPT_WIDGET_PROBE_URI)
        );
    }

    #[test]
    fn chatgpt_widget_probe_resource_matches_apps_sdk_shape() -> anyhow::Result<()> {
        let resource = chatgpt_widget_probe_resource();
        assert_eq!(resource.raw.uri, CHATGPT_WIDGET_PROBE_URI);
        assert_eq!(
            resource.raw.mime_type.as_deref(),
            Some(CHATGPT_VN_WIDGET_MIME_TYPE)
        );

        let template = chatgpt_widget_probe_resource_template();
        assert_eq!(template.raw.uri_template, CHATGPT_WIDGET_PROBE_URI);
        assert_eq!(
            template.raw.mime_type.as_deref(),
            Some(CHATGPT_VN_WIDGET_MIME_TYPE)
        );

        let result = worldsim_widget_probe(&WorldsimWidgetProbeParams {
            message: "probe ok".to_owned(),
        })?;
        assert_eq!(
            result.meta.context("probe result meta missing")?.0["openai/outputTemplate"],
            serde_json::json!(CHATGPT_WIDGET_PROBE_URI)
        );
        assert_eq!(
            result
                .structured_content
                .context("probe structured missing")?["message"],
            serde_json::json!("probe ok")
        );
        Ok(())
    }

    #[test]
    fn url_completion_accepts_only_https_nonlocal_hosts() {
        assert!(validate_fetchable_image_url_shape("https://example.com/image.png").is_ok());
        assert!(validate_fetchable_image_url_shape("http://example.com/image.png").is_err());
        assert!(validate_fetchable_image_url_shape("https://localhost/image.png").is_err());
        assert!(validate_fetchable_image_url_shape("https://127.0.0.1/image.png").is_err());
        assert!(validate_fetchable_image_url_shape("https://10.0.0.4/image.png").is_err());
        assert!(validate_fetchable_image_url_shape("https://[::1]/image.png").is_err());
        assert!(
            validate_fetchable_image_url_shape("https://user:pass@example.com/image.png").is_err()
        );
    }

    #[test]
    fn image_ingest_probe_records_shape_without_payload_bytes() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store_root = temp.path().join("store");
        worldsim_start_world(WorldsimStartWorldParams {
            seed_text: "web mcp image ingest probe fantasy smoke".to_owned(),
            store_root: Some(store_root.display().to_string()),
            world_id: Some("stw_mcp_probe".to_owned()),
            title: None,
            session_id: None,
        })?;
        let response = worldsim_probe_image_ingest(WorldsimProbeImageIngestParams {
            store_root: Some(store_root.display().to_string()),
            world_id: Some("stw_mcp_probe".to_owned()),
            slot: Some("turn_cg:turn_0001".to_owned()),
            image_base64: Some("abcdef".to_owned()),
            image_url: Some("https://example.com/image.png".to_owned()),
            resource_uri: Some("resource://image/1".to_owned()),
            file_id: Some("file_123".to_owned()),
            mime_type: Some("image/png".to_owned()),
            source_note: Some("probe".to_owned()),
        })?;
        assert_eq!(response.status, "recorded");
        assert_eq!(response.received.image_base64_bytes, Some(6));
        let record = fs::read_to_string(response.probe_path)?;
        assert!(record.contains("image_base64_bytes"));
        assert!(!record.contains("abcdef"));
        Ok(())
    }

    #[test]
    fn base64_completion_reuses_visual_job_completion_contract() -> anyhow::Result<()> {
        const MINIMAL_PNG: &[u8] = b"\x89PNG\r\n\x1a\nminimal-test-png";

        let temp = tempdir()?;
        let store_root = temp.path().join("store");
        worldsim_start_world(WorldsimStartWorldParams {
            seed_text: "web mcp base64 completion fantasy smoke".to_owned(),
            store_root: Some(store_root.display().to_string()),
            world_id: Some("stw_mcp_base64_complete".to_owned()),
            title: None,
            session_id: None,
        })?;
        for _ in 0..5 {
            advance_turn(&singulari_world::AdvanceTurnOptions {
                store_root: Some(store_root.clone()),
                world_id: "stw_mcp_base64_complete".to_owned(),
                input: "1".to_owned(),
            })?;
        }
        let claimed = worldsim_claim_visual_job(WorldsimClaimVisualJobParams {
            store_root: Some(store_root.display().to_string()),
            world_id: Some("stw_mcp_base64_complete".to_owned()),
            slot: Some("turn_cg:turn_0005".to_owned()),
            claimed_by: "mcp-web-test".to_owned(),
            force: false,
        })?;
        let singulari_world::VisualJobClaimOutcome::Claimed { claim } = claimed else {
            anyhow::bail!("turn CG job should be claimable before base64 completion");
        };
        let data_url = format!("data:image/png;base64,{}", STANDARD.encode(MINIMAL_PNG));
        let response =
            worldsim_complete_visual_job_from_base64(WorldsimCompleteVisualJobFromBase64Params {
                store_root: Some(store_root.display().to_string()),
                world_id: Some("stw_mcp_base64_complete".to_owned()),
                slot: "turn_cg:turn_0005".to_owned(),
                claim_id: Some(claim.claim_id),
                image_base64: data_url,
                mime_type: None,
                source_note: Some("unit-test".to_owned()),
            })?;
        assert_eq!(response.status, "completed");
        assert!(response.staged_file_removed);
        assert!(
            response
                .completion
                .destination_path
                .ends_with("turn_0005.png")
        );
        assert!(std::path::Path::new(&response.completion.destination_path).is_file());
        assert!(!std::path::Path::new(&response.staged_path).exists());
        Ok(())
    }
}
