use anyhow::{Context, Result, bail};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, handler::server::tool,
    service::RequestContext, transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use singulari_world::{
    AgentCommitTurnOptions, AgentSubmitTurnOptions, AgentTurnResponse, BuildCodexViewOptions,
    BuildResumePackOptions, BuildVnPacketOptions, BuildWorldVisualAssetsOptions,
    ClaimVisualJobOptions, CompleteVisualJobOptions, ReleaseVisualJobClaimOptions,
    StartWorldOptions, advance_turn, build_codex_view, build_resume_pack, build_vn_packet,
    build_world_visual_assets, claim_visual_job, commit_agent_turn, complete_visual_job,
    enqueue_agent_turn, load_pending_agent_turn, refresh_world_docs, release_visual_job_claim,
    repair_world_db, resolve_store_paths, resolve_world_id, search_world_db, start_world,
    validate_world,
};
use std::path::PathBuf;

const MCP_INSTRUCTIONS: &str = "Singulari World MCP server. It exposes standalone world-simulator tools for worldsim-agent play: player input can be queued as a pending agent turn, the trusted agent can read hidden adjudication context, and committed visible narrative is projected back into the VN packet. Browser-visible routes remain redacted.";

#[tokio::main]
async fn main() {
    if let Err(error) = run_main().await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run_main() -> Result<()> {
    if let Some(command) = std::env::args().nth(1) {
        bail!("unknown singulari-world-mcp command: {command}");
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("singulari_world_mcp=info")
        .init();

    tracing::info!("Singulari World MCP server starting");
    let service = WorldsimMcpServer::new()
        .serve(stdio())
        .await
        .inspect_err(|error| tracing::error!("MCP server error: {error}"))?;
    service.waiting().await?;
    Ok(())
}

#[derive(Clone)]
struct WorldsimMcpServer {
    tools: Vec<Tool>,
}

impl WorldsimMcpServer {
    fn new() -> Self {
        Self {
            tools: vec![
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
                    "worldsim_commit_agent_turn",
                    "Commit an agent-authored response object, validate hidden-truth redaction, and return the updated VN packet.",
                    tool::schema_for_type::<WorldsimCommitAgentTurnParams>(),
                ),
                Tool::new(
                    "worldsim_visual_assets",
                    "Return player-visible visual asset manifest and Codex App image generation jobs. The MCP server does not call image providers; Codex App runs the codex_app_call and saves to destination_path.",
                    tool::schema_for_type::<WorldsimWorldParams>(),
                ),
                Tool::new(
                    "worldsim_claim_visual_job",
                    "Atomically claim one pending player-visible Codex App image generation job. Codex App should call its image generation host capability with the returned prompt and save to destination_path.",
                    tool::schema_for_type::<WorldsimClaimVisualJobParams>(),
                ),
                Tool::new(
                    "worldsim_complete_visual_job",
                    "Mark a visual generation job complete after Codex App has saved a PNG to the returned destination_path, or copy a generated PNG into that destination.",
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
            ],
        }
    }
}

impl ServerHandler for WorldsimMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(MCP_INSTRUCTIONS.to_owned()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(self.tools.clone())))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
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
            "worldsim_commit_agent_turn" => {
                let params: WorldsimCommitAgentTurnParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_commit_agent_turn(params)).await
            }
            "worldsim_visual_assets" => {
                let params: WorldsimWorldParams = tool::parse_json_object(arguments)?;
                blocking_tool(move || worldsim_visual_assets(params)).await
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
        self.tools
            .iter()
            .find(|tool| tool.name.as_ref() == name)
            .cloned()
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

fn json_tool_result<T>(value: &T) -> Result<CallToolResult, McpError>
where
    T: Serialize,
{
    let value = serde_json::to_value(value)
        .map_err(|error| McpError::internal_error(error.to_string(), None))?;
    Ok(CallToolResult::structured(value))
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
struct WorldsimWorldParams {
    #[serde(default)]
    store_root: Option<String>,
    #[serde(default)]
    world_id: Option<String>,
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
struct WorldsimSearchResponse {
    world_id: String,
    query: String,
    hits: Vec<singulari_world::WorldSearchHit>,
}

fn worldsim_start_world(params: WorldsimStartWorldParams) -> Result<WorldsimStartedWorldResponse> {
    let started = start_world(&StartWorldOptions {
        seed_text: params.seed_text,
        world_id: params.world_id,
        title: params.title,
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
    "codex_app_image_worker".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn json_tool_result_exposes_structured_content() -> anyhow::Result<()> {
        let result = json_tool_result(&serde_json::json!({
            "job": {
                "codex_app_call": {
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
            structured["job"]["codex_app_call"]["capability"],
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
        assert_eq!(claim.job.codex_app_call.capability, "image_generation");
        Ok(())
    }
}
