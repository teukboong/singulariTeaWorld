use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine as _;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, ListToolsResult, PaginatedRequestParams,
    ServerCapabilities, ServerInfo, Tool,
};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt, handler::server::tool,
    service::RequestContext, transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

mod webgpt_types;

use crate::webgpt_types::{
    WebGptAnswer, WebGptCitation, WebGptHealthReport, WebGptHealthState, WebGptProfileBootstrap,
    WebGptSessionKind,
};

const DEFAULT_NODE_BIN: &str = "node";
const DEFAULT_WORKER_ENTRY_RELATIVE: &str = "chatgpt-worker/dist/index.js";
const DEFAULT_SELECTORS_RELATIVE: &str = "chatgpt-worker/selectors.toml";
const DEFAULT_INTERACTIVE_PROFILE_SUBDIR: &str = ".hesperides/chatgpt-chrome-profile";
const DEFAULT_MCP_PROFILE_SUBDIR: &str = ".hesperides/chatgpt-chrome-profile-mcp";
const DEFAULT_MANUAL_PROFILE_SUBDIR: &str = ".hesperides/chatgpt-chrome-profile-manual";
const DEFAULT_CHATGPT_URL: &str = "https://chatgpt.com/";
const DEFAULT_CONTROL_TIMEOUT_SECS: u64 = 60;
const DEFAULT_RESEARCH_TIMEOUT_SECS: u64 = 180;
const DEFAULT_IMAGE_EXTRACT_TIMEOUT_SECS: u64 = 90;
const DEFAULT_RECOVERY_ATTEMPTS: u8 = 1;
const MAX_RECOVERY_ATTEMPTS: u8 = 3;
const MAX_IMAGE_REFERENCE_ATTACHMENTS: usize = 4;
const DEFAULT_REQUEST_ID: u64 = 1;
const MCP_PROTOCOL_VERSION: &str = "2025-03-26";
const DEFAULT_IMAGE_OUTPUT_SUBDIR: &str = ".cache/hesperides/webgpt-mcp/generated-images";
const MCP_CLIENT_VERSION: &str = "0.1.0";
const MCP_CLIENT_INIT_ID: i64 = 1;
const MCP_CLIENT_LIST_TOOLS_ID: i64 = 2;
const MCP_CLIENT_CALL_TOOL_ID: i64 = 3;
const ENV_WORKER_CHATGPT_URL: &str = "WEBGPT_MCP_CHATGPT_URL";
const ENV_WORKER_CHROME_BIN: &str = "WEBGPT_MCP_CHROME_BIN";
const ENV_WORKER_CDP_URL: &str = "WEBGPT_MCP_CDP_URL";
const ENV_WORKER_PROFILE_DIR: &str = "WEBGPT_MCP_PROFILE_DIR";
const ENV_WORKER_SELECTORS: &str = "WEBGPT_MCP_SELECTORS";
const ENV_WEBGPT_MCP_CDP_URL: &str = "WEBGPT_MCP_CDP_URL";
const ENV_WEBGPT_MCP_CHATGPT_URL: &str = "WEBGPT_MCP_CHATGPT_URL";
const ENV_WEBGPT_MCP_CHROME_BIN: &str = "WEBGPT_MCP_CHROME_BIN";
const ENV_WEBGPT_MCP_NODE_BIN: &str = "WEBGPT_MCP_NODE_BIN";
const ENV_WEBGPT_MCP_BOOTSTRAP_SNAPSHOT_DIR: &str = "WEBGPT_MCP_BOOTSTRAP_SNAPSHOT_DIR";
const ENV_WEBGPT_MCP_MANUAL_PROFILE_DIR: &str = "WEBGPT_MCP_MANUAL_PROFILE_DIR";
const ENV_WEBGPT_MCP_PROFILE_DIR: &str = "WEBGPT_MCP_PROFILE_DIR";
const ENV_WEBGPT_MCP_SELECTORS: &str = "WEBGPT_MCP_SELECTORS";
const ENV_WEBGPT_MCP_EVENT_LOG: &str = "WEBGPT_MCP_EVENT_LOG";
const ENV_WEBGPT_INTERACTIVE_PROFILE_DIR: &str = "WEBGPT_INTERACTIVE_PROFILE_DIR";
const BOOTSTRAP_MARKER_FILENAME: &str = ".webgpt-mcp-bootstrap.json";

#[tokio::main]
async fn main() {
    if let Err(error) = run_main().await {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}

async fn run_main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    if let Some(command) = args.next() {
        match command.as_str() {
            "client-call" => return run_client_call_cli(args.collect()),
            other => bail!("unknown webgpt-mcp command: {other}"),
        }
    }

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter("webgpt_mcp=info")
        .init();

    let service = WebGptMcpServer::new();
    tracing::info!("WebGPT MCP server starting");
    let service = service.serve(stdio()).await.inspect_err(|error| {
        tracing::error!("MCP server error: {error}");
    })?;
    service.waiting().await?;
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum ClientCallOutput {
    SmokeJson,
    TextBlocks,
    FirstText,
    RawCall,
}

impl ClientCallOutput {
    fn parse(raw: &str) -> Result<Self> {
        match raw {
            "smoke-json" => Ok(Self::SmokeJson),
            "text-blocks" => Ok(Self::TextBlocks),
            "first-text" => Ok(Self::FirstText),
            "raw-call" => Ok(Self::RawCall),
            other => bail!("unsupported client-call output mode: {other}"),
        }
    }
}

#[derive(Debug)]
struct ClientCallOptions {
    wrapper: PathBuf,
    client_name: String,
    tool_name: String,
    arguments: Value,
    require_tool: bool,
    output: ClientCallOutput,
}

#[derive(Debug)]
struct McpClientCallResult {
    text_blocks: Vec<String>,
    call_response: Value,
}

struct McpStdioClient {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
}

impl McpStdioClient {
    fn spawn(wrapper: &Path) -> Result<Self> {
        let mut child = Command::new(wrapper)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to start {}", wrapper.display()))?;
        let stdin = child.stdin.take().context("webgpt-mcp stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("webgpt-mcp stdout unavailable")?;
        Ok(Self {
            child,
            stdin,
            reader: BufReader::new(stdout),
        })
    }

    fn send_json(&mut self, payload: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.stdin, payload)
            .context("failed to serialize MCP payload")?;
        self.stdin
            .write_all(b"\n")
            .context("failed to terminate MCP payload line")?;
        self.stdin.flush().context("failed to flush MCP payload")?;
        Ok(())
    }

    fn read_response(&mut self, expected_id: i64) -> Result<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            let read = self
                .reader
                .read_line(&mut line)
                .context("failed to read MCP response")?;
            if read == 0 {
                bail!("webgpt-mcp closed stdout while waiting for id={expected_id}");
            }
            let trimmed = line.trim();
            let payload: Value = serde_json::from_str(trimmed).with_context(|| {
                format!("invalid MCP JSON response while waiting for id={expected_id}: {trimmed}")
            })?;
            if payload.get("id").and_then(Value::as_i64) != Some(expected_id) {
                continue;
            }
            if let Some(error) = payload.get("error") {
                bail!("MCP response id={expected_id} returned error: {error}");
            }
            return Ok(payload);
        }
    }
}

impl Drop for McpStdioClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn run_client_call_cli(args: Vec<String>) -> Result<()> {
    let options = parse_client_call_options(args)?;
    let result = call_mcp_tool_via_stdio(&options)?;
    match options.output {
        ClientCallOutput::SmokeJson => {
            let output = json!({
                "tool": options.tool_name,
                "content": result.text_blocks,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&output).context("failed to encode smoke output")?
            );
        }
        ClientCallOutput::TextBlocks => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result.text_blocks)
                    .context("failed to encode text blocks")?
            );
        }
        ClientCallOutput::FirstText => {
            let first = result
                .text_blocks
                .first()
                .context("webgpt-mcp tool call returned no text content")?;
            println!("{first}");
        }
        ClientCallOutput::RawCall => {
            println!(
                "{}",
                serde_json::to_string_pretty(&result.call_response)
                    .context("failed to encode raw call response")?
            );
        }
    }
    Ok(())
}

fn parse_client_call_options(args: Vec<String>) -> Result<ClientCallOptions> {
    let mut wrapper = None;
    let mut client_name = "webgpt-mcp-client".to_string();
    let mut tool_name = None;
    let mut arguments = json!({});
    let mut require_tool = false;
    let mut output = ClientCallOutput::TextBlocks;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--wrapper" => {
                let value = iter.next().context("--wrapper requires a path")?;
                wrapper = Some(PathBuf::from(value));
            }
            "--client-name" => {
                client_name = iter.next().context("--client-name requires a value")?;
            }
            "--tool" => {
                tool_name = Some(iter.next().context("--tool requires a name")?);
            }
            "--arguments" => {
                let raw = iter.next().context("--arguments requires JSON")?;
                arguments = serde_json::from_str(&raw)
                    .with_context(|| format!("invalid --arguments JSON: {raw}"))?;
            }
            "--require-tool" => {
                require_tool = true;
            }
            "--output" => {
                let raw = iter.next().context("--output requires a mode")?;
                output = ClientCallOutput::parse(&raw)?;
            }
            "-h" | "--help" => {
                bail!(
                    "usage: webgpt-mcp client-call --wrapper <path> --tool <name> [--arguments <json>] [--require-tool] [--client-name <name>] [--output smoke-json|text-blocks|first-text|raw-call]"
                );
            }
            other => bail!("unsupported client-call argument: {other}"),
        }
    }

    Ok(ClientCallOptions {
        wrapper: wrapper.context("--wrapper is required")?,
        client_name,
        tool_name: tool_name.context("--tool is required")?,
        arguments,
        require_tool,
        output,
    })
}

fn call_mcp_tool_via_stdio(options: &ClientCallOptions) -> Result<McpClientCallResult> {
    let mut client = McpStdioClient::spawn(&options.wrapper)?;
    client.send_json(&json!({
        "jsonrpc": "2.0",
        "id": MCP_CLIENT_INIT_ID,
        "method": "initialize",
        "params": {
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": options.client_name, "version": MCP_CLIENT_VERSION},
        },
    }))?;
    client
        .read_response(MCP_CLIENT_INIT_ID)
        .context("webgpt-mcp initialize failed")?;
    client.send_json(&json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    }))?;

    if options.require_tool {
        client.send_json(&json!({
            "jsonrpc": "2.0",
            "id": MCP_CLIENT_LIST_TOOLS_ID,
            "method": "tools/list",
            "params": {},
        }))?;
        let tools = client
            .read_response(MCP_CLIENT_LIST_TOOLS_ID)
            .context("webgpt-mcp tools/list failed")?;
        ensure_tool_advertised(&tools, &options.tool_name)?;
    }

    client.send_json(&json!({
        "jsonrpc": "2.0",
        "id": MCP_CLIENT_CALL_TOOL_ID,
        "method": "tools/call",
        "params": {"name": options.tool_name, "arguments": options.arguments},
    }))?;
    let call_response = client
        .read_response(MCP_CLIENT_CALL_TOOL_ID)
        .context("webgpt-mcp tools/call failed")?;
    let text_blocks = call_response
        .get("result")
        .and_then(|value| value.get("content"))
        .and_then(Value::as_array)
        .context("webgpt-mcp result missing content")?
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            } else {
                None
            }
        })
        .collect();
    Ok(McpClientCallResult {
        text_blocks,
        call_response,
    })
}

fn ensure_tool_advertised(tools_response: &Value, tool_name: &str) -> Result<()> {
    let names = tools_response
        .get("result")
        .and_then(|value| value.get("tools"))
        .and_then(Value::as_array)
        .context("webgpt-mcp tools/list response missing tools")?
        .iter()
        .filter_map(|item| item.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if names.contains(&tool_name) {
        return Ok(());
    }
    bail!("expected tool {tool_name:?} not found in advertised tools {names:?}");
}

#[derive(Clone)]
struct WebGptMcpServer {
    tools: Vec<Tool>,
}

impl WebGptMcpServer {
    fn new() -> Self {
        Self {
            tools: vec![
                Tool::new(
                    "webgpt_health",
                    "Report whether the separate WebGPT MCP surface is configured and fail-closed profile isolation is preserved.",
                    tool::schema_for_type::<WebGptHealthParams>(),
                ),
                Tool::new(
                    "webgpt_research",
                    "Run one WebGPT-backed research turn through the separate MCP browser worker and return normalized answer text plus health metadata.",
                    tool::schema_for_type::<WebGptResearchParams>(),
                ),
                Tool::new(
                    "webgpt_extract_images",
                    "Extract ChatGPT-generated images from a WebGPT conversation via the live browser page context and save them under the WebGPT MCP image cache.",
                    tool::schema_for_type::<WebGptImageExtractParams>(),
                ),
                Tool::new(
                    "webgpt_generate_image",
                    "Ask ChatGPT to generate one or more images, then extract the rendered PNGs through the live browser page context and save them under the WebGPT MCP image cache.",
                    tool::schema_for_type::<WebGptImageGenerateParams>(),
                ),
                Tool::new(
                    "webgpt_controls",
                    "Inspect the currently selected model / reasoning level and the visible control options on the separate MCP browser surface.",
                    tool::schema_for_type::<WebGptControlsParams>(),
                ),
            ],
        }
    }
}

impl ServerHandler for WebGptMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "WebGPT MCP server. Uses a separate stdio surface and a fail-closed profile root so browser automation stays outside the existing chrysomela-mcp main binary."
                    .to_string(),
            ),
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
            "webgpt_health" => {
                let params: WebGptHealthParams = tool::parse_json_object(arguments)?;
                let report = tokio::task::spawn_blocking(move || build_health_report(&params))
                    .await
                    .map_err(|error| {
                        McpError::internal_error(format!("health task join failed: {error}"), None)
                    })?
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&report)
                        .map_err(|error| McpError::internal_error(error.to_string(), None))?,
                )]))
            }
            "webgpt_research" => {
                let params: WebGptResearchParams = tool::parse_json_object(arguments)?;
                let answer = tokio::task::spawn_blocking(move || run_research(&params))
                    .await
                    .map_err(|error| {
                        McpError::internal_error(
                            format!("research task join failed: {error}"),
                            None,
                        )
                    })?
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&answer)
                        .map_err(|error| McpError::internal_error(error.to_string(), None))?,
                )]))
            }
            "webgpt_extract_images" => {
                let params: WebGptImageExtractParams = tool::parse_json_object(arguments)?;
                let extraction = tokio::task::spawn_blocking(move || run_image_extract(&params))
                    .await
                    .map_err(|error| {
                        McpError::internal_error(
                            format!("image extraction task join failed: {error}"),
                            None,
                        )
                    })?
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&extraction)
                        .map_err(|error| McpError::internal_error(error.to_string(), None))?,
                )]))
            }
            "webgpt_generate_image" => {
                let params: WebGptImageGenerateParams = tool::parse_json_object(arguments)?;
                let extraction = tokio::task::spawn_blocking(move || run_image_generate(&params))
                    .await
                    .map_err(|error| {
                        McpError::internal_error(
                            format!("image generation task join failed: {error}"),
                            None,
                        )
                    })?
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&extraction)
                        .map_err(|error| McpError::internal_error(error.to_string(), None))?,
                )]))
            }
            "webgpt_controls" => {
                let params: WebGptControlsParams = tool::parse_json_object(arguments)?;
                let controls = tokio::task::spawn_blocking(move || run_controls(&params))
                    .await
                    .map_err(|error| {
                        McpError::internal_error(
                            format!("controls task join failed: {error}"),
                            None,
                        )
                    })?
                    .map_err(|error| McpError::internal_error(error.to_string(), None))?;
                Ok(CallToolResult::success(vec![Content::text(
                    serde_json::to_string_pretty(&controls)
                        .map_err(|error| McpError::internal_error(error.to_string(), None))?,
                )]))
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

#[derive(Debug, Deserialize, JsonSchema)]
struct WebGptHealthParams {
    #[serde(default)]
    probe_live: Option<bool>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    auto_recover: Option<bool>,
    #[serde(default)]
    recovery_attempts: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebGptControlsParams {
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    auto_recover: Option<bool>,
    #[serde(default)]
    recovery_attempts: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebGptResearchParams {
    prompt: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    new_conversation: Option<bool>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    reasoning_level: Option<String>,
    #[serde(default)]
    auto_recover: Option<bool>,
    #[serde(default)]
    recovery_attempts: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebGptImageExtractParams {
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    max_images: Option<usize>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    auto_recover: Option<bool>,
    #[serde(default)]
    recovery_attempts: Option<u8>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct WebGptImageGenerateParams {
    prompt: String,
    #[serde(default)]
    conversation_id: Option<String>,
    #[serde(default)]
    reference_paths: Vec<String>,
    #[serde(default)]
    max_images: Option<usize>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    auto_recover: Option<bool>,
    #[serde(default)]
    recovery_attempts: Option<u8>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WebGptExtractedImage {
    index: usize,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    alt: String,
    width: u32,
    height: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    mime_type: String,
    byte_len: usize,
    sha256: String,
    path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    source_file_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WebGptImageExtraction {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    conversation_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    page_url: String,
    output_dir: String,
    images: Vec<WebGptExtractedImage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WorkerImageExtraction {
    #[serde(default)]
    conversation_id: String,
    #[serde(default)]
    page_url: String,
    #[serde(default)]
    images: Vec<WorkerGeneratedImage>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WorkerGeneratedImage {
    #[serde(default)]
    alt: String,
    #[serde(default)]
    src: String,
    #[serde(default)]
    natural_width: u32,
    #[serde(default)]
    natural_height: u32,
    #[serde(default)]
    mime_type: String,
    #[serde(default)]
    data_url: String,
    #[serde(default)]
    error: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WebGptControlOption {
    #[serde(default)]
    id: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    detail: String,
    #[serde(default)]
    selected: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct WebGptControls {
    #[serde(default)]
    current_model: String,
    #[serde(default)]
    current_reasoning_level: String,
    #[serde(default)]
    available_models: Vec<WebGptControlOption>,
    #[serde(default)]
    available_reasoning_levels: Vec<WebGptControlOption>,
}

#[derive(Debug, Deserialize)]
struct WebGptBootstrapMarker {
    #[serde(default)]
    source: String,
    #[serde(default)]
    source_profile_dir: String,
    #[serde(default)]
    recorded_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum WorkerEvent {
    Status {
        msg: String,
        #[serde(default)]
        conversation_id: Option<String>,
    },
    Partial {
        text: String,
        #[serde(default)]
        chunk_seq: u64,
        #[serde(default)]
        conversation_id: Option<String>,
        #[serde(default)]
        message_id: Option<String>,
    },
    Done {
        text: String,
        #[serde(default)]
        citations: Vec<WebGptCitation>,
        #[serde(default)]
        conversation_id: Option<String>,
        #[serde(default)]
        message_id: Option<String>,
    },
    HealthChange {
        state: String,
        #[serde(default)]
        detail: Option<String>,
        #[serde(default)]
        conversation_id: Option<String>,
    },
    Session {
        #[serde(default)]
        conversation_id: Option<String>,
        #[serde(default)]
        model: Option<String>,
        #[serde(default)]
        url: Option<String>,
    },
    ProfileSynced,
    ProfileRestored,
    ProfileSnapshot,
    Error {
        msg: String,
        #[serde(default)]
        conversation_id: Option<String>,
    },
}

#[derive(Debug, Deserialize)]
struct WorkerRpcError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct WorkerRpcResponse {
    id: Option<u64>,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<WorkerRpcError>,
}

#[derive(Debug)]
enum WorkerMessage {
    Event(WorkerEvent),
    Response(WorkerRpcResponse),
}

#[derive(Debug)]
struct WorkerSession {
    child: Child,
    stdin: ChildStdin,
    messages: Receiver<WorkerMessage>,
    config: WorkerConfig,
    last_health: WebGptHealthState,
    last_health_detail: String,
    last_conversation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct WorkerConfig {
    node_bin: String,
    worker_entry: PathBuf,
    selectors_path: PathBuf,
    profile_dir: PathBuf,
    chatgpt_url: String,
    chrome_bin: Option<String>,
    cdp_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    event_log_path: Option<PathBuf>,
}

impl WorkerConfig {
    fn from_env() -> Result<Self> {
        let worker_entry = resolve_worker_entry_path()?;
        let selectors_path = resolve_selectors_path();
        if !selectors_path.exists() {
            bail!("selectors file missing: {}", selectors_path.display());
        }

        let profile_dir = resolve_mcp_profile_dir();
        let interactive_profile_dir = resolve_interactive_profile_dir();
        if profile_dir == interactive_profile_dir {
            bail!(
                "fail-closed: MCP profile dir {} matches interactive profile dir {}; configure WEBGPT_MCP_PROFILE_DIR to a separate root",
                profile_dir.display(),
                interactive_profile_dir.display()
            );
        }

        Ok(Self {
            node_bin: resolve_node_bin(),
            worker_entry,
            selectors_path,
            profile_dir,
            chatgpt_url: resolve_chatgpt_url(),
            chrome_bin: resolve_optional_chrome_bin(),
            cdp_url: resolve_optional_cdp_url(),
            event_log_path: resolve_optional_env_path(ENV_WEBGPT_MCP_EVENT_LOG),
        })
    }

    fn worker_root(&self) -> Result<&Path> {
        self.worker_entry
            .parent()
            .and_then(Path::parent)
            .ok_or_else(|| {
                anyhow!(
                    "worker entry has no parent: {}",
                    self.worker_entry.display()
                )
            })
    }
}

impl WorkerSession {
    fn spawn() -> Result<Self> {
        let config = WorkerConfig::from_env()?;
        let mut command = Command::new(&config.node_bin);
        command
            .current_dir(config.worker_root()?)
            .arg(&config.worker_entry)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .env(
                ENV_WORKER_SELECTORS,
                config.selectors_path.display().to_string(),
            )
            .env(
                ENV_WORKER_PROFILE_DIR,
                config.profile_dir.display().to_string(),
            )
            .env(ENV_WORKER_CHATGPT_URL, config.chatgpt_url.clone());
        if let Some(chrome_bin) = &config.chrome_bin {
            command.env(ENV_WORKER_CHROME_BIN, chrome_bin);
        }
        if let Some(cdp_url) = &config.cdp_url {
            command.env(ENV_WORKER_CDP_URL, cdp_url);
        }

        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to spawn worker: {} {}",
                config.node_bin,
                config.worker_entry.display()
            )
        })?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow!("worker stdin pipe missing"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("worker stdout pipe missing"))?;

        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("webgpt-mcp-worker".to_string())
            .spawn(move || worker_reader_loop(stdout, &tx))
            .context("failed to spawn worker reader thread")?;

        Ok(Self {
            child,
            stdin,
            messages: rx,
            config,
            last_health: WebGptHealthState::Ready,
            last_health_detail: String::new(),
            last_conversation_id: None,
        })
    }

    fn write_request(&mut self, method: &str, params: &serde_json::Value) -> Result<()> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": DEFAULT_REQUEST_ID,
            "method": method,
            "params": params,
        });
        serde_json::to_writer(&mut self.stdin, &payload)
            .context("failed to encode worker request")?;
        self.stdin
            .write_all(b"\n")
            .context("failed to terminate worker request line")?;
        self.stdin
            .flush()
            .context("failed to flush worker request")?;
        Ok(())
    }

    fn send_text(&mut self, prompt: &str) -> Result<()> {
        self.write_request("send_text", &json!({ "text": prompt }))
    }

    fn open_new_conversation(&mut self, timeout: Duration) -> Result<()> {
        let _: Value = self.request_response("open_new_conversation", &json!({}), timeout)?;
        Ok(())
    }

    fn open_conversation(&mut self, conversation_id: &str, timeout: Duration) -> Result<()> {
        let _: Value = self.request_response(
            "open_session",
            &json!({ "conversation_id": conversation_id }),
            timeout,
        )?;
        Ok(())
    }

    fn request_response<T: DeserializeOwned>(
        &mut self,
        method: &str,
        params: &serde_json::Value,
        timeout: Duration,
    ) -> Result<T> {
        self.write_request(method, params)?;
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.kill_best_effort();
                bail!(
                    "worker request '{method}' timed out after {}s",
                    timeout.as_secs()
                );
            }

            let remaining = deadline.saturating_duration_since(now);
            match self.messages.recv_timeout(remaining) {
                Ok(WorkerMessage::Event(event)) => self.observe_event(event)?,
                Ok(WorkerMessage::Response(response)) => {
                    if response.id != Some(DEFAULT_REQUEST_ID) {
                        continue;
                    }
                    if let Some(error) = response.error {
                        bail!("worker request '{method}' failed: {}", error.message);
                    }
                    let result = response.result.unwrap_or(serde_json::Value::Null);
                    return serde_json::from_value(result).with_context(|| {
                        format!("failed to parse worker response for '{method}'")
                    });
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.kill_best_effort();
                    bail!(
                        "worker request '{method}' timed out after {}s",
                        timeout.as_secs()
                    );
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let exit_status = self
                        .child
                        .try_wait()
                        .context("failed to inspect worker status")?;
                    bail!("worker stdout closed unexpectedly: {exit_status:?}");
                }
            }
        }
    }

    fn controls(&mut self, timeout: Duration) -> Result<WebGptControls> {
        self.request_response("controls", &json!({}), timeout)
    }

    fn select_model(&mut self, model: &str, timeout: Duration) -> Result<WebGptControls> {
        self.request_response("select_model", &json!({ "model": model }), timeout)
    }

    fn select_reasoning_level(
        &mut self,
        reasoning_level: &str,
        timeout: Duration,
    ) -> Result<WebGptControls> {
        self.request_response(
            "select_reasoning",
            &json!({ "reasoning_level": reasoning_level }),
            timeout,
        )
    }

    fn extract_generated_images(
        &mut self,
        max_images: usize,
        timeout: Duration,
    ) -> Result<WorkerImageExtraction> {
        self.request_response(
            "extract_generated_images",
            &json!({ "max_images": max_images }),
            timeout,
        )
    }

    fn generate_image(
        &mut self,
        prompt: &str,
        reference_paths: &[String],
        max_images: usize,
        timeout_secs: u64,
        timeout: Duration,
    ) -> Result<WorkerImageExtraction> {
        self.request_response(
            "generate_image",
            &json!({
                "prompt": prompt,
                "reference_paths": reference_paths,
                "max_images": max_images,
                "timeout_secs": timeout_secs,
            }),
            timeout,
        )
    }

    fn observe_event(&mut self, event: WorkerEvent) -> Result<()> {
        self.append_event_log(&event)?;
        match event {
            WorkerEvent::Status {
                msg,
                conversation_id,
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                self.last_health_detail = msg;
            }
            WorkerEvent::Partial {
                conversation_id, ..
            }
            | WorkerEvent::Done {
                conversation_id, ..
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
            }
            WorkerEvent::HealthChange {
                state,
                detail,
                conversation_id,
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                self.last_health = WebGptHealthState::from_wire(&state);
                self.last_health_detail = detail.unwrap_or_default();
                if !self.last_health.is_sendable() {
                    bail!(
                        "worker health became {}{}",
                        self.last_health.as_wire(),
                        render_optional_detail(&self.last_health_detail)
                    );
                }
            }
            WorkerEvent::Session {
                conversation_id,
                model,
                url,
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                if let Some(model) = model {
                    self.last_health_detail = model;
                } else if let Some(url) = url {
                    self.last_health_detail = url;
                }
            }
            WorkerEvent::ProfileSynced
            | WorkerEvent::ProfileRestored
            | WorkerEvent::ProfileSnapshot => {}
            WorkerEvent::Error {
                msg,
                conversation_id,
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                bail!("worker error: {msg}");
            }
        }
        Ok(())
    }

    fn collect_answer(&mut self, timeout: Duration) -> Result<WebGptAnswer> {
        let deadline = Instant::now() + timeout;
        let mut partial_text = String::new();

        loop {
            let now = Instant::now();
            if now >= deadline {
                self.kill_best_effort();
                bail!("research timed out after {}s", timeout.as_secs());
            }
            let remaining = deadline.saturating_duration_since(now);
            match self.messages.recv_timeout(remaining) {
                Ok(WorkerMessage::Event(event)) => {
                    if let Some(answer) = self.handle_answer_event(event, &mut partial_text)? {
                        return Ok(answer);
                    }
                }
                Ok(WorkerMessage::Response(response)) => {
                    if let Some(error) = response.error {
                        bail!(
                            "worker request failed while collecting answer: {}",
                            error.message
                        );
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.kill_best_effort();
                    bail!("research timed out after {}s", timeout.as_secs());
                }
                Err(RecvTimeoutError::Disconnected) => {
                    let exit_status = self
                        .child
                        .try_wait()
                        .context("failed to inspect worker status")?;
                    bail!("worker stdout closed unexpectedly: {exit_status:?}");
                }
            }
        }
    }

    fn handle_answer_event(
        &mut self,
        event: WorkerEvent,
        partial_text: &mut String,
    ) -> Result<Option<WebGptAnswer>> {
        self.append_event_log(&event)?;
        match event {
            WorkerEvent::Status {
                msg,
                conversation_id,
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                self.last_health_detail = msg;
                Ok(None)
            }
            WorkerEvent::Partial {
                text,
                chunk_seq,
                conversation_id,
                ..
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                if chunk_seq == 0 {
                    partial_text.clear();
                }
                partial_text.push_str(&text);
                Ok(None)
            }
            WorkerEvent::Done {
                text,
                citations,
                conversation_id,
                ..
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                Ok(Some(build_research_answer(
                    self.config.profile_dir.display().to_string(),
                    text,
                    partial_text.clone(),
                    citations,
                    self.last_health,
                    self.last_conversation_id.clone(),
                )))
            }
            WorkerEvent::HealthChange {
                state,
                detail,
                conversation_id,
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                self.last_health = WebGptHealthState::from_wire(&state);
                self.last_health_detail = detail.unwrap_or_default();
                if !self.last_health.is_sendable() {
                    bail!(
                        "worker health became {}{}",
                        self.last_health.as_wire(),
                        render_optional_detail(&self.last_health_detail)
                    );
                }
                Ok(None)
            }
            WorkerEvent::Session {
                conversation_id, ..
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                Ok(None)
            }
            WorkerEvent::ProfileSynced
            | WorkerEvent::ProfileRestored
            | WorkerEvent::ProfileSnapshot => Ok(None),
            WorkerEvent::Error {
                msg,
                conversation_id,
            } => {
                self.last_conversation_id = conversation_id.or(self.last_conversation_id.clone());
                bail!("worker error: {msg}");
            }
        }
    }

    fn kill_best_effort(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn append_event_log(&self, event: &WorkerEvent) -> Result<()> {
        let Some(path) = self.config.event_log_path.as_deref() else {
            return Ok(());
        };
        append_worker_event_log(path, event)
    }
}

fn append_worker_event_log(path: &Path, event: &WorkerEvent) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let observed_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open {}", path.display()))?;
    let entry = json!({
        "observed_unix_ms": observed_unix_ms,
        "event": event,
    });
    serde_json::to_writer(&mut file, &entry)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(b"\n")
        .with_context(|| format!("failed to terminate {}", path.display()))?;
    Ok(())
}

fn build_research_answer(
    session_id: String,
    text: String,
    partial_text: String,
    citations: Vec<WebGptCitation>,
    health: WebGptHealthState,
    raw_conversation_id: Option<String>,
) -> WebGptAnswer {
    let answer_markdown = if text.is_empty() { partial_text } else { text };
    WebGptAnswer {
        session_id,
        answer_markdown,
        citations,
        search_used: true,
        health: Some(health),
        raw_conversation_id,
        current_model: None,
        current_reasoning_level: None,
    }
}

impl Drop for WorkerSession {
    fn drop(&mut self) {
        self.kill_best_effort();
    }
}

fn build_health_report(params: &WebGptHealthParams) -> Result<WebGptHealthReport> {
    let config = WorkerConfig::from_env()?;
    let auto_recover = params.auto_recover.unwrap_or(true);
    let recovery_attempts = normalize_recovery_attempts(params.recovery_attempts);
    let mut report = base_health_report(&config)?;
    if !params.probe_live.unwrap_or(false) {
        return Ok(report);
    }

    let timeout = Duration::from_secs(params.timeout_secs.unwrap_or(DEFAULT_CONTROL_TIMEOUT_SECS));
    let mut recovery_note: Option<String> = None;
    for attempt in 0..=usize::from(recovery_attempts) {
        let mut session = WorkerSession::spawn()?;
        match session.controls(timeout) {
            Ok(controls) => {
                report.live_probe = true;
                report.detail = format!(
                    "live probe succeeded via webgpt_controls; separate MCP browser session is usable{}{}{}",
                    if config.cdp_url.is_some() {
                        " via CDP attach"
                    } else {
                        ""
                    },
                    render_optional_detail(&session.last_health_detail),
                    render_recovery_suffix(recovery_note.as_deref())
                );
                if !controls.current_model.trim().is_empty() {
                    report.current_model = Some(controls.current_model.trim().to_string());
                }
                if !controls.current_reasoning_level.trim().is_empty() {
                    report.current_reasoning_level =
                        Some(controls.current_reasoning_level.trim().to_string());
                }
                report.bootstrap = if config.cdp_url.is_some() {
                    None
                } else {
                    load_bootstrap_marker(&config.profile_dir)?
                };
                return Ok(report);
            }
            Err(error) => {
                if let Some((state, detail)) = classify_session_failure(&session, &error) {
                    report.state = state;
                    report.live_probe = true;
                    if auto_recover
                        && attempt < usize::from(recovery_attempts)
                        && should_auto_rebootstrap(state)
                    {
                        session.kill_best_effort();
                        match force_bootstrap_mcp_profile() {
                            Ok(Some(note)) => {
                                report.bootstrap = load_bootstrap_marker(&config.profile_dir)?;
                                recovery_note = Some(note);
                                continue;
                            }
                            Ok(None) => {
                                report.detail = format!(
                                    "live probe reached worker health {}{}; no reusable bootstrap source was available{}",
                                    state.as_wire(),
                                    render_optional_detail(&detail),
                                    render_recovery_suffix(recovery_note.as_deref())
                                );
                                return Ok(report);
                            }
                            Err(recovery_error) => {
                                report.detail = format!(
                                    "live probe reached worker health {}{}; auto-rebootstrap failed: {}{}",
                                    state.as_wire(),
                                    render_optional_detail(&detail),
                                    recovery_error,
                                    render_recovery_suffix(recovery_note.as_deref())
                                );
                                return Ok(report);
                            }
                        }
                    }

                    report.detail = format!(
                        "live probe reached worker health {}{}{}",
                        state.as_wire(),
                        render_optional_detail(&detail),
                        render_recovery_suffix(recovery_note.as_deref())
                    );
                    return Ok(report);
                }

                return Err(error);
            }
        }
    }

    Ok(report)
}

fn base_health_report(config: &WorkerConfig) -> Result<WebGptHealthReport> {
    Ok(WebGptHealthReport {
        state: WebGptHealthState::Ready,
        session_kind: WebGptSessionKind::McpResearch,
        live_probe: false,
        detail: if config.cdp_url.is_some() {
            "worker assets present; CDP attach mode is configured. This is static readiness only: live browser/login challenge state is still deferred to webgpt_controls or webgpt_research.".to_string()
        } else {
            "worker assets present; using separate MCP profile root. This is static readiness only: live browser/login challenge state is still deferred to webgpt_controls or webgpt_research.".to_string()
        },
        worker_entry: config.worker_entry.display().to_string(),
        profile_dir: config.profile_dir.display().to_string(),
        bootstrap: if config.cdp_url.is_some() {
            None
        } else {
            load_bootstrap_marker(&config.profile_dir)?
        },
        current_model: None,
        current_reasoning_level: None,
    })
}

fn load_bootstrap_marker(profile_dir: &Path) -> Result<Option<WebGptProfileBootstrap>> {
    if is_protected_bootstrap_target(profile_dir) {
        return Ok(None);
    }

    let marker_path = profile_dir.join(BOOTSTRAP_MARKER_FILENAME);
    if !marker_path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(&marker_path).with_context(|| {
        format!(
            "failed to read bootstrap marker at {}",
            marker_path.display()
        )
    })?;
    let marker: WebGptBootstrapMarker = serde_json::from_str(&raw).with_context(|| {
        format!(
            "failed to parse bootstrap marker at {}",
            marker_path.display()
        )
    })?;

    if marker.source.trim().is_empty() {
        return Ok(None);
    }

    Ok(Some(WebGptProfileBootstrap {
        source: marker.source,
        source_profile_dir: marker.source_profile_dir,
        recorded_at: marker.recorded_at,
    }))
}

fn run_controls(params: &WebGptControlsParams) -> Result<WebGptControls> {
    let timeout = Duration::from_secs(params.timeout_secs.unwrap_or(DEFAULT_CONTROL_TIMEOUT_SECS));
    let auto_recover = params.auto_recover.unwrap_or(true);
    let recovery_attempts = normalize_recovery_attempts(params.recovery_attempts);
    let mut recovery_note: Option<String> = None;
    for attempt in 0..=usize::from(recovery_attempts) {
        let mut session = WorkerSession::spawn()?;
        match session.controls(timeout) {
            Ok(controls) => return Ok(controls),
            Err(error) => {
                if auto_recover
                    && attempt < usize::from(recovery_attempts)
                    && classify_session_failure(&session, &error)
                        .is_some_and(|(state, _)| should_auto_rebootstrap(state))
                {
                    session.kill_best_effort();
                    match force_bootstrap_mcp_profile() {
                        Ok(Some(note)) => {
                            recovery_note = Some(note);
                            continue;
                        }
                        Ok(None) => {
                            return Err(anyhow!(
                                "{error}; no reusable bootstrap source was available{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                        Err(recovery_error) => {
                            return Err(anyhow!(
                                "{error}; auto-rebootstrap failed: {recovery_error}{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                    }
                }
                return Err(anyhow!(
                    "{error}{}",
                    render_recovery_suffix(recovery_note.as_deref())
                ));
            }
        }
    }

    unreachable!("bounded controls recovery loop should always return")
}

fn run_research(params: &WebGptResearchParams) -> Result<WebGptAnswer> {
    if params.prompt.trim().is_empty() {
        bail!("prompt must not be empty");
    }

    let timeout = Duration::from_secs(params.timeout_secs.unwrap_or(DEFAULT_RESEARCH_TIMEOUT_SECS));
    let control_timeout = Duration::from_secs(DEFAULT_CONTROL_TIMEOUT_SECS);
    let auto_recover = params.auto_recover.unwrap_or(true);
    let recovery_attempts = normalize_recovery_attempts(params.recovery_attempts);
    let mut recovery_note: Option<String> = None;
    for attempt in 0..=usize::from(recovery_attempts) {
        let mut session = WorkerSession::spawn()?;
        let attempt_result = (|| -> Result<WebGptAnswer> {
            open_research_conversation(&mut session, params, control_timeout)?;
            let applied_controls = apply_research_controls(&mut session, params, control_timeout)?;
            session.send_text(params.prompt.trim())?;
            let mut answer = session.collect_answer(timeout)?;
            if let Some(controls) = applied_controls {
                apply_controls_to_answer(&mut answer, &controls);
            }
            Ok(answer)
        })();

        match attempt_result {
            Ok(answer) => return Ok(answer),
            Err(error) => {
                if auto_recover
                    && attempt < usize::from(recovery_attempts)
                    && classify_session_failure(&session, &error)
                        .is_some_and(|(state, _)| should_auto_rebootstrap(state))
                {
                    session.kill_best_effort();
                    match force_bootstrap_mcp_profile() {
                        Ok(Some(note)) => {
                            recovery_note = Some(note);
                            continue;
                        }
                        Ok(None) => {
                            return Err(anyhow!(
                                "{error}; no reusable bootstrap source was available{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                        Err(recovery_error) => {
                            return Err(anyhow!(
                                "{error}; auto-rebootstrap failed: {recovery_error}{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                    }
                }

                return Err(anyhow!(
                    "{error}{}",
                    render_recovery_suffix(recovery_note.as_deref())
                ));
            }
        }
    }

    unreachable!("bounded research recovery loop should always return")
}

fn run_image_extract(params: &WebGptImageExtractParams) -> Result<WebGptImageExtraction> {
    let timeout = Duration::from_secs(
        params
            .timeout_secs
            .unwrap_or(DEFAULT_IMAGE_EXTRACT_TIMEOUT_SECS),
    );
    let control_timeout = Duration::from_secs(DEFAULT_CONTROL_TIMEOUT_SECS);
    let auto_recover = params.auto_recover.unwrap_or(true);
    let recovery_attempts = normalize_recovery_attempts(params.recovery_attempts);
    let max_images = params.max_images.unwrap_or(4).clamp(1, 12);
    let conversation_id =
        normalize_optional_control_override("conversation_id", params.conversation_id.as_deref())?;
    let mut recovery_note: Option<String> = None;

    for attempt in 0..=usize::from(recovery_attempts) {
        let mut session = WorkerSession::spawn()?;
        let attempt_result = (|| -> Result<WebGptImageExtraction> {
            if let Some(conversation_id) = conversation_id.as_deref() {
                session.open_conversation(conversation_id, control_timeout)?;
            }
            let raw = session.extract_generated_images(max_images, timeout)?;
            persist_worker_images(raw)
        })();

        match attempt_result {
            Ok(extraction) => return Ok(extraction),
            Err(error) => {
                if auto_recover
                    && attempt < usize::from(recovery_attempts)
                    && classify_session_failure(&session, &error)
                        .is_some_and(|(state, _)| should_auto_rebootstrap(state))
                {
                    session.kill_best_effort();
                    match force_bootstrap_mcp_profile() {
                        Ok(Some(note)) => {
                            recovery_note = Some(note);
                            continue;
                        }
                        Ok(None) => {
                            return Err(anyhow!(
                                "{error}; no reusable bootstrap source was available{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                        Err(recovery_error) => {
                            return Err(anyhow!(
                                "{error}; auto-rebootstrap failed: {recovery_error}{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                    }
                }

                return Err(anyhow!(
                    "{error}{}",
                    render_recovery_suffix(recovery_note.as_deref())
                ));
            }
        }
    }

    unreachable!("bounded image extraction recovery loop should always return")
}

fn run_image_generate(params: &WebGptImageGenerateParams) -> Result<WebGptImageExtraction> {
    if params.prompt.trim().is_empty() {
        bail!("prompt must not be empty");
    }
    let timeout_secs = params
        .timeout_secs
        .unwrap_or(DEFAULT_IMAGE_EXTRACT_TIMEOUT_SECS);
    let timeout = Duration::from_secs(timeout_secs);
    let control_timeout = Duration::from_secs(DEFAULT_CONTROL_TIMEOUT_SECS);
    let auto_recover = params.auto_recover.unwrap_or(true);
    let recovery_attempts = normalize_recovery_attempts(params.recovery_attempts);
    let max_images = params.max_images.unwrap_or(1).clamp(1, 12);
    let reference_paths = normalize_image_reference_paths(&params.reference_paths)?;
    let conversation_id =
        normalize_optional_control_override("conversation_id", params.conversation_id.as_deref())?;
    let mut recovery_note: Option<String> = None;

    for attempt in 0..=usize::from(recovery_attempts) {
        let mut session = WorkerSession::spawn()?;
        let attempt_result = (|| -> Result<WebGptImageExtraction> {
            if let Some(conversation_id) = conversation_id.as_deref() {
                session.open_conversation(conversation_id, control_timeout)?;
            } else {
                session.open_new_conversation(control_timeout)?;
            }
            let raw = session.generate_image(
                params.prompt.trim(),
                reference_paths.as_slice(),
                max_images,
                timeout_secs,
                timeout,
            )?;
            persist_worker_images(raw)
        })();

        match attempt_result {
            Ok(extraction) => return Ok(extraction),
            Err(error) => {
                if auto_recover
                    && attempt < usize::from(recovery_attempts)
                    && classify_session_failure(&session, &error)
                        .is_some_and(|(state, _)| should_auto_rebootstrap(state))
                {
                    session.kill_best_effort();
                    match force_bootstrap_mcp_profile() {
                        Ok(Some(note)) => {
                            recovery_note = Some(note);
                            continue;
                        }
                        Ok(None) => {
                            return Err(anyhow!(
                                "{error}; no reusable bootstrap source was available{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                        Err(recovery_error) => {
                            return Err(anyhow!(
                                "{error}; auto-rebootstrap failed: {recovery_error}{}",
                                render_recovery_suffix(recovery_note.as_deref())
                            ));
                        }
                    }
                }

                return Err(anyhow!(
                    "{error}{}",
                    render_recovery_suffix(recovery_note.as_deref())
                ));
            }
        }
    }

    unreachable!("bounded image generation recovery loop should always return")
}

fn persist_worker_images(raw: WorkerImageExtraction) -> Result<WebGptImageExtraction> {
    let conversation_dir = sanitize_path_component(&raw.conversation_id);
    let output_dir = resolve_image_output_dir().join(conversation_dir);
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create image output dir {}", output_dir.display()))?;

    let mut images = Vec::new();
    for (index, image) in raw.images.into_iter().enumerate() {
        if image.error.is_empty() {
            let (mime_type, bytes) = decode_image_data_url(&image.data_url)
                .with_context(|| format!("failed to decode generated image #{index}"))?;
            let sha256 = hex::encode(Sha256::digest(&bytes));
            let extension = image_extension(&mime_type);
            let file_name = format!("{index:02}-{}.{}", &sha256[..16], extension);
            let path = output_dir.join(file_name);
            std::fs::write(&path, &bytes)
                .with_context(|| format!("failed to write generated image {}", path.display()))?;
            images.push(WebGptExtractedImage {
                index,
                alt: image.alt,
                width: image.natural_width,
                height: image.natural_height,
                mime_type,
                byte_len: bytes.len(),
                sha256,
                path: path.display().to_string(),
                source_file_id: extract_estuary_file_id(&image.src),
            });
        } else {
            bail!(
                "generated image #{index} was visible but not extractable: {}",
                image.error
            );
        }
    }

    Ok(WebGptImageExtraction {
        conversation_id: raw.conversation_id,
        page_url: raw.page_url,
        output_dir: output_dir.display().to_string(),
        images,
    })
}

fn decode_image_data_url(data_url: &str) -> Result<(String, Vec<u8>)> {
    let (header, encoded) = data_url
        .split_once(',')
        .context("image data_url missing comma separator")?;
    let mime_type = header
        .strip_prefix("data:")
        .and_then(|value| value.strip_suffix(";base64"))
        .context("image data_url is not base64 encoded")?
        .to_string();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .context("image data_url base64 decode failed")?;
    Ok((mime_type, bytes))
}

fn image_extension(mime_type: &str) -> &'static str {
    match mime_type {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        _ => "png",
    }
}

fn extract_estuary_file_id(src: &str) -> String {
    let Some((_, query)) = src.split_once('?') else {
        return String::new();
    };
    query
        .split('&')
        .filter_map(|part| part.split_once('='))
        .find_map(|(key, value)| (key == "id").then(|| value.to_string()))
        .unwrap_or_default()
}

fn sanitize_path_component(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "current".to_string()
    } else {
        sanitized
    }
}

fn resolve_image_output_dir() -> PathBuf {
    home_dir().join(DEFAULT_IMAGE_OUTPUT_SUBDIR)
}

fn open_research_conversation(
    session: &mut WorkerSession,
    params: &WebGptResearchParams,
    timeout: Duration,
) -> Result<()> {
    let conversation_id =
        normalize_optional_control_override("conversation_id", params.conversation_id.as_deref())?;
    if params.new_conversation.unwrap_or(false) && conversation_id.is_some() {
        bail!("new_conversation cannot be combined with conversation_id");
    }
    if let Some(conversation_id) = conversation_id.as_deref() {
        session.open_conversation(conversation_id, timeout)?;
    } else if params.new_conversation.unwrap_or(false) {
        session.open_new_conversation(timeout)?;
    }
    Ok(())
}

fn apply_research_controls(
    session: &mut WorkerSession,
    params: &WebGptResearchParams,
    timeout: Duration,
) -> Result<Option<WebGptControls>> {
    let model = normalize_optional_control_override("model", params.model.as_deref())?;
    let reasoning_level =
        normalize_optional_control_override("reasoning_level", params.reasoning_level.as_deref())?;
    if let Some(model) = model.as_deref() {
        let _ = session.select_model(model, timeout)?;
    }
    if let Some(reasoning_level) = reasoning_level.as_deref() {
        let _ = session.select_reasoning_level(reasoning_level, timeout)?;
    }
    if model.is_none() && reasoning_level.is_none() {
        Ok(None)
    } else {
        Ok(Some(session.controls(timeout)?))
    }
}

fn normalize_optional_control_override(label: &str, raw: Option<&str>) -> Result<Option<String>> {
    match raw {
        None => Ok(None),
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                bail!("{label} must not be empty when provided");
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

fn normalize_image_reference_paths(raw_paths: &[String]) -> Result<Vec<String>> {
    if raw_paths.len() > MAX_IMAGE_REFERENCE_ATTACHMENTS {
        bail!(
            "too many image reference attachments: max={}, actual={}",
            MAX_IMAGE_REFERENCE_ATTACHMENTS,
            raw_paths.len()
        );
    }

    let mut paths = Vec::with_capacity(raw_paths.len());
    for raw_path in raw_paths {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            bail!("image reference attachment path must not be empty");
        }
        let path = PathBuf::from(trimmed);
        if !path.is_file() {
            bail!(
                "image reference attachment is not a file: path={}",
                path.display()
            );
        }
        let extension = path
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp" | "gif") {
            bail!(
                "image reference attachment must be an image file: path={}, extension={}",
                path.display(),
                extension
            );
        }
        let canonical = path
            .canonicalize()
            .with_context(|| format!("failed to canonicalize {}", path.display()))?;
        paths.push(canonical.display().to_string());
    }
    Ok(paths)
}

fn apply_controls_to_answer(answer: &mut WebGptAnswer, controls: &WebGptControls) {
    let current_model = controls.current_model.trim();
    if !current_model.is_empty() {
        answer.current_model = Some(current_model.to_string());
    }
    let current_reasoning_level = controls.current_reasoning_level.trim();
    if !current_reasoning_level.is_empty() {
        answer.current_reasoning_level = Some(current_reasoning_level.to_string());
    }
}

fn worker_reader_loop(stdout: impl std::io::Read, tx: &mpsc::Sender<WorkerMessage>) {
    let reader = BufReader::new(stdout);
    for line_result in reader.lines() {
        let Ok(line) = line_result else {
            break;
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) else {
            eprintln!("[webgpt-mcp] worker emitted invalid JSON: {trimmed}");
            continue;
        };
        if value.get("event").is_none() {
            if let Ok(response) = serde_json::from_value::<WorkerRpcResponse>(value) {
                if tx.send(WorkerMessage::Response(response)).is_err() {
                    break;
                }
            }
            continue;
        }
        match serde_json::from_value::<WorkerEvent>(value) {
            Ok(event) => {
                if tx.send(WorkerMessage::Event(event)).is_err() {
                    break;
                }
            }
            Err(error) => eprintln!("[webgpt-mcp] worker event parse failed: {error}"),
        }
    }
}

fn resolve_worker_entry_path() -> Result<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_WORKER_ENTRY_RELATIVE);
    if path.exists() {
        Ok(path)
    } else {
        bail!(
            "chatgpt worker build missing: {} (run `npm install && npm run build` in crates/webgpt-mcp/chatgpt-worker/)",
            path.display()
        );
    }
}

fn resolve_selectors_path() -> PathBuf {
    if let Some(path) = std::env::var_os(ENV_WEBGPT_MCP_SELECTORS) {
        PathBuf::from(path)
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(DEFAULT_SELECTORS_RELATIVE)
    }
}

fn resolve_mcp_profile_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(ENV_WEBGPT_MCP_PROFILE_DIR) {
        PathBuf::from(path)
    } else {
        home_dir().join(DEFAULT_MCP_PROFILE_SUBDIR)
    }
}

fn resolve_interactive_profile_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(ENV_WEBGPT_INTERACTIVE_PROFILE_DIR) {
        PathBuf::from(path)
    } else {
        home_dir().join(DEFAULT_INTERACTIVE_PROFILE_SUBDIR)
    }
}

fn resolve_manual_profile_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(ENV_WEBGPT_MCP_MANUAL_PROFILE_DIR) {
        PathBuf::from(path)
    } else {
        home_dir().join(DEFAULT_MANUAL_PROFILE_SUBDIR)
    }
}

fn resolve_bootstrap_snapshot_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(ENV_WEBGPT_MCP_BOOTSTRAP_SNAPSHOT_DIR) {
        PathBuf::from(path)
    } else {
        PathBuf::from(format!(
            "{}-snapshot",
            resolve_interactive_profile_dir().display()
        ))
    }
}

fn resolve_node_bin() -> String {
    std::env::var(ENV_WEBGPT_MCP_NODE_BIN).unwrap_or_else(|_| DEFAULT_NODE_BIN.to_string())
}

fn resolve_chatgpt_url() -> String {
    std::env::var(ENV_WEBGPT_MCP_CHATGPT_URL).unwrap_or_else(|_| DEFAULT_CHATGPT_URL.to_string())
}

fn resolve_optional_chrome_bin() -> Option<String> {
    std::env::var(ENV_WEBGPT_MCP_CHROME_BIN).ok()
}

fn resolve_optional_cdp_url() -> Option<String> {
    std::env::var(ENV_WEBGPT_MCP_CDP_URL)
        .ok()
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
}

fn resolve_optional_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).and_then(|value| {
        let path = PathBuf::from(value);
        if path.as_os_str().is_empty() {
            None
        } else {
            Some(path)
        }
    })
}

fn home_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
}

fn render_optional_detail(detail: &str) -> String {
    if detail.is_empty() {
        String::new()
    } else {
        format!(" ({detail})")
    }
}

fn render_recovery_suffix(detail: Option<&str>) -> String {
    match detail {
        Some(detail) if !detail.is_empty() => format!("; {detail}"),
        _ => String::new(),
    }
}

fn normalize_recovery_attempts(value: Option<u8>) -> u8 {
    value
        .unwrap_or(DEFAULT_RECOVERY_ATTEMPTS)
        .min(MAX_RECOVERY_ATTEMPTS)
}

fn infer_health_state_from_error_message(message: &str) -> Option<WebGptHealthState> {
    let normalized = message.to_ascii_lowercase();
    if normalized.contains("challenge_page") {
        Some(WebGptHealthState::ChallengePage)
    } else if normalized.contains("expired_session") {
        Some(WebGptHealthState::ExpiredSession)
    } else if normalized.contains("selector_drift") {
        Some(WebGptHealthState::SelectorDrift)
    } else if normalized.contains("rate_limited") {
        Some(WebGptHealthState::RateLimited)
    } else if normalized.contains("processsingleton")
        || normalized.contains("profile in use")
        || normalized.contains("already in use")
    {
        Some(WebGptHealthState::Busy)
    } else if normalized.contains("timed out") {
        Some(WebGptHealthState::Degraded)
    } else if normalized.contains("stdout closed unexpectedly") {
        Some(WebGptHealthState::Unavailable)
    } else {
        None
    }
}

fn summarize_probe_failure(message: &str) -> String {
    message
        .strip_prefix("worker request 'controls' failed: ")
        .or_else(|| message.strip_prefix("worker error: "))
        .unwrap_or(message)
        .to_string()
}

fn classify_session_failure(
    session: &WorkerSession,
    error: &anyhow::Error,
) -> Option<(WebGptHealthState, String)> {
    if session.last_health != WebGptHealthState::Ready {
        return Some((session.last_health, session.last_health_detail.clone()));
    }

    let error_text = error.to_string();
    infer_health_state_from_error_message(&error_text)
        .map(|state| (state, summarize_probe_failure(&error_text)))
}

fn should_auto_rebootstrap(state: WebGptHealthState) -> bool {
    matches!(
        state,
        WebGptHealthState::ChallengePage | WebGptHealthState::ExpiredSession
    )
}

fn is_protected_bootstrap_target(target_dir: &Path) -> bool {
    target_dir == resolve_manual_profile_dir() || target_dir == resolve_bootstrap_snapshot_dir()
}

fn has_reusable_profile(dir: &Path) -> bool {
    if !dir.join("Local State").is_file() {
        return false;
    }
    if dir.join("Default").is_dir() {
        return true;
    }

    std::fs::read_dir(dir).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| name.starts_with("Profile "))
        })
    })
}

fn sync_profile_tree(source_dir: &Path, target_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(target_dir)
        .with_context(|| format!("failed to create MCP profile dir: {}", target_dir.display()))?;
    let status = Command::new("rsync")
        .arg("-a")
        .arg("--exclude=DevToolsActivePort")
        .arg("--exclude=SingletonCookie")
        .arg("--exclude=SingletonLock")
        .arg("--exclude=SingletonSocket")
        .arg("--exclude=BrowserMetrics*")
        .arg("--exclude=Crashpad*")
        .arg("--exclude=ShaderCache*")
        .arg("--exclude=GrShaderCache*")
        .arg(format!("{}/", source_dir.display()))
        .arg(format!("{}/", target_dir.display()))
        .status()
        .context("failed to spawn rsync for MCP profile bootstrap")?;
    if !status.success() {
        bail!(
            "rsync exited with status {} while bootstrapping {} -> {}",
            status,
            source_dir.display(),
            target_dir.display()
        );
    }
    Ok(())
}

fn bootstrap_recorded_at() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => format!("unix:{}", duration.as_secs()),
        Err(_) => "unix:0".to_string(),
    }
}

fn record_bootstrap_metadata(
    profile_dir: &Path,
    source_label: &str,
    source_dir: &Path,
) -> Result<()> {
    let marker_path = profile_dir.join(BOOTSTRAP_MARKER_FILENAME);
    let payload = json!({
        "source": source_label,
        "source_profile_dir": source_dir.display().to_string(),
        "recorded_at": bootstrap_recorded_at(),
    });
    let raw =
        serde_json::to_string_pretty(&payload).context("failed to encode bootstrap metadata")?;
    std::fs::write(&marker_path, format!("{raw}\n")).with_context(|| {
        format!(
            "failed to write bootstrap marker at {}",
            marker_path.display()
        )
    })?;
    Ok(())
}

fn force_bootstrap_mcp_profile() -> Result<Option<String>> {
    let target_dir = resolve_mcp_profile_dir();
    if is_protected_bootstrap_target(&target_dir) {
        return Ok(None);
    }
    for (source_dir, source_label) in [
        (resolve_manual_profile_dir(), "manual profile"),
        (resolve_bootstrap_snapshot_dir(), "interactive snapshot"),
    ] {
        if source_dir == target_dir || !has_reusable_profile(&source_dir) {
            continue;
        }
        sync_profile_tree(&source_dir, &target_dir)?;
        record_bootstrap_metadata(&target_dir, source_label, &source_dir)?;
        return Ok(Some(format!(
            "auto-rebootstrapped MCP profile from {source_label}"
        )));
    }
    Ok(None)
}

#[cfg(test)]
// Test fixtures use expect to keep setup failures explicit; production code remains expect-free.
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn render_optional_detail_omits_empty_text() {
        assert_eq!(render_optional_detail(""), "");
        assert_eq!(
            render_optional_detail("selector drift"),
            " (selector drift)"
        );
    }

    #[test]
    fn infer_health_state_maps_busy_profile_lock_failures() {
        assert_eq!(
            infer_health_state_from_error_message("worker error: ProcessSingleton profile in use"),
            Some(WebGptHealthState::Busy)
        );
    }

    #[test]
    fn infer_health_state_maps_terminal_worker_health_strings() {
        assert_eq!(
            infer_health_state_from_error_message(
                "worker health became challenge_page (잠시만 기다리십시오...)"
            ),
            Some(WebGptHealthState::ChallengePage)
        );
        assert_eq!(
            infer_health_state_from_error_message("worker health became expired_session"),
            Some(WebGptHealthState::ExpiredSession)
        );
    }

    #[test]
    fn summarize_probe_failure_trims_wrapper_prefix() {
        assert_eq!(
            summarize_probe_failure("worker request 'controls' failed: selector mismatch"),
            "selector mismatch"
        );
    }

    #[test]
    fn normalize_recovery_attempts_clamps_to_maximum() {
        assert_eq!(normalize_recovery_attempts(None), 1);
        assert_eq!(normalize_recovery_attempts(Some(9)), 3);
    }

    #[test]
    fn should_auto_rebootstrap_only_for_session_failures() {
        assert!(should_auto_rebootstrap(WebGptHealthState::ChallengePage));
        assert!(should_auto_rebootstrap(WebGptHealthState::ExpiredSession));
        assert!(!should_auto_rebootstrap(WebGptHealthState::Busy));
    }

    #[test]
    fn protected_bootstrap_targets_cover_manual_and_snapshot_profiles() {
        let manual = resolve_manual_profile_dir();
        let snapshot = resolve_bootstrap_snapshot_dir();
        assert!(is_protected_bootstrap_target(&manual));
        assert!(is_protected_bootstrap_target(&snapshot));
        assert!(!is_protected_bootstrap_target(&resolve_mcp_profile_dir()));
    }

    #[test]
    fn has_reusable_profile_requires_local_state_and_profile_dir() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("webgpt-mcp-profile-check-{unique}"));
        std::fs::create_dir_all(root.join("Default")).expect("default profile dir");
        assert!(!has_reusable_profile(&root));
        std::fs::write(root.join("Local State"), "{}").expect("local state");
        assert!(has_reusable_profile(&root));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn normalize_optional_control_override_rejects_blank_value() {
        let error = normalize_optional_control_override("model", Some("   "))
            .expect_err("blank override should fail");
        assert!(error.to_string().contains("model must not be empty"));
    }

    #[test]
    fn normalize_optional_control_override_trims_value() {
        let value = normalize_optional_control_override("reasoning_level", Some(" high "))
            .expect("trimmed override");
        assert_eq!(value.as_deref(), Some("high"));
    }

    #[test]
    fn image_reference_paths_canonicalize_existing_images() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("webgpt-mcp-ref-image-{unique}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let image = root.join("ref.png");
        std::fs::write(&image, b"not parsed by normalizer").expect("image fixture");

        let normalized = normalize_image_reference_paths(&[image.display().to_string()])
            .expect("existing image path should normalize");

        assert_eq!(
            normalized,
            vec![image.canonicalize().unwrap().display().to_string()]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn image_reference_paths_reject_non_image_files() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("webgpt-mcp-ref-text-{unique}"));
        std::fs::create_dir_all(&root).expect("temp dir");
        let text = root.join("ref.txt");
        std::fs::write(&text, b"not an image").expect("text fixture");

        let error = normalize_image_reference_paths(&[text.display().to_string()])
            .expect_err("non-image attachments must be rejected");

        assert!(error.to_string().contains("must be an image file"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn research_params_accept_conversation_routing_fields() {
        let params: WebGptResearchParams = serde_json::from_value(json!({
            "prompt": "review this",
            "conversation_id": "conv_123",
            "new_conversation": false
        }))
        .expect("research params");

        assert_eq!(params.conversation_id.as_deref(), Some("conv_123"));
        assert_eq!(params.new_conversation, Some(false));
    }

    #[test]
    fn apply_controls_to_answer_copies_model_and_reasoning_level() {
        let mut answer = WebGptAnswer::default();
        let controls = WebGptControls {
            current_model: "gpt-5".to_string(),
            current_reasoning_level: "high".to_string(),
            available_models: Vec::new(),
            available_reasoning_levels: Vec::new(),
        };

        apply_controls_to_answer(&mut answer, &controls);

        assert_eq!(answer.current_model.as_deref(), Some("gpt-5"));
        assert_eq!(answer.current_reasoning_level.as_deref(), Some("high"));
    }

    #[test]
    fn worker_done_event_deserializes_citations() {
        let event: WorkerEvent = serde_json::from_value(json!({
            "event": "done",
            "text": "answer",
            "conversation_id": "conv_123",
            "citations": [
                {
                    "title": "OpenAI release notes",
                    "url": "https://platform.openai.com/docs/changelog",
                    "snippet": "Newest model family."
                }
            ]
        }))
        .expect("done event should parse");

        match event {
            WorkerEvent::Done {
                text,
                citations,
                conversation_id,
                message_id,
            } => {
                assert_eq!(text, "answer");
                assert_eq!(conversation_id.as_deref(), Some("conv_123"));
                assert_eq!(message_id, None);
                assert_eq!(citations.len(), 1);
                assert_eq!(citations[0].title, "OpenAI release notes");
            }
            other => panic!("expected done event, got {other:?}"),
        }
    }

    #[test]
    fn worker_event_log_records_dom_completion_evidence() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        let path = std::env::temp_dir()
            .join(format!("webgpt-mcp-event-log-{unique}"))
            .join("events.jsonl");
        let event = WorkerEvent::Done {
            text: "answer".to_string(),
            citations: Vec::new(),
            conversation_id: Some("conv_123".to_string()),
            message_id: Some("msg_456".to_string()),
        };

        append_worker_event_log(&path, &event).expect("append event log");

        let line = std::fs::read_to_string(&path).expect("event log");
        let value: serde_json::Value = serde_json::from_str(line.trim()).expect("event log json");
        assert!(value["observed_unix_ms"].as_u64().is_some());
        assert_eq!(value["event"]["event"], "done");
        assert_eq!(value["event"]["conversation_id"], "conv_123");
        assert_eq!(value["event"]["message_id"], "msg_456");
        let _ = std::fs::remove_dir_all(path.parent().expect("parent"));
    }

    #[test]
    fn build_research_answer_preserves_done_citations() {
        let citations = vec![WebGptCitation {
            title: "OpenAI release notes".to_string(),
            url: "https://platform.openai.com/docs/changelog".to_string(),
            snippet: "Newest model family.".to_string(),
        }];

        let answer = build_research_answer(
            "profile".to_string(),
            String::new(),
            "fallback partial".to_string(),
            citations.clone(),
            WebGptHealthState::Ready,
            Some("conv_456".to_string()),
        );

        assert_eq!(answer.answer_markdown, "fallback partial");
        assert_eq!(answer.citations, citations);
        assert_eq!(answer.raw_conversation_id.as_deref(), Some("conv_456"));
        assert_eq!(answer.health, Some(WebGptHealthState::Ready));
    }

    #[test]
    fn load_bootstrap_marker_reads_source_metadata() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("unix epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("webgpt-mcp-bootstrap-{unique}"));
        let profile_dir = root.join("profile");
        std::fs::create_dir_all(&profile_dir).expect("profile dir");
        std::fs::write(
            profile_dir.join(BOOTSTRAP_MARKER_FILENAME),
            r#"{
  "source": "manual profile",
  "source_profile_dir": "/tmp/manual",
  "recorded_at": "2026-04-14T01:00:00Z"
}"#,
        )
        .expect("marker");

        let bootstrap = load_bootstrap_marker(&profile_dir)
            .expect("bootstrap marker")
            .expect("bootstrap metadata");

        assert_eq!(bootstrap.source, "manual profile");
        assert_eq!(bootstrap.source_profile_dir, "/tmp/manual");
        assert_eq!(bootstrap.recorded_at, "2026-04-14T01:00:00Z");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn load_bootstrap_marker_ignores_protected_source_profiles() {
        assert!(
            load_bootstrap_marker(&resolve_manual_profile_dir())
                .expect("manual marker load")
                .is_none()
        );
        assert!(
            load_bootstrap_marker(&resolve_bootstrap_snapshot_dir())
                .expect("snapshot marker load")
                .is_none()
        );
    }
}
