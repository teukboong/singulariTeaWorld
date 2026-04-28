use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use singulari_world::{
    ACTIVE_WORLD_FILENAME, AdvanceTurnOptions, AgentCommitTurnOptions, AgentRevivalCompileOptions,
    AgentSubmitTurnOptions, AgentTurnResponse, ApplyCharacterAnchorOptions, InitWorldOptions,
    RenderPacketLoadOptions, ValidationStatus, WorldTextBackend, WorldVisualBackend, advance_turn,
    apply_character_anchor, build_agent_revival_packet, build_codex_view, build_resume_pack,
    commit_agent_turn, enqueue_agent_turn, force_chapter_summary, init_world, load_active_world,
    load_latest_snapshot, load_pending_agent_turn, load_render_packet,
    load_world_backend_selection, load_world_record, recent_entity_updates,
    recent_relationship_updates, refresh_world_docs, render_advanced_turn_report,
    render_chat_route, render_codex_view_section_markdown, render_host_supervisor_plan,
    render_packet_markdown, render_projection_health_report, render_resume_pack_markdown,
    render_started_world_report, repair_world_db, resolve_store_paths, resolve_world_id,
    route_chat_input, search_world_db, start_world, validate_world, world_db_stats,
};
use singulari_world::{
    BuildCodexViewOptions, BuildResumePackOptions, BuildVnPacketOptions,
    BuildWorldVisualAssetsOptions, ChatRouteOptions, ClaimVisualJobOptions, CodexViewSection,
    CompleteVisualJobOptions, ExportWorldOptions, ImageGenerationJob, ImportWorldOptions,
    ReleaseVisualJobClaimOptions, StartWorldOptions, VisualArtifactKind, VnServeOptions,
    build_host_supervisor_plan, build_projection_health_report, build_vn_packet,
    build_world_visual_assets, claim_visual_job, complete_visual_job, export_world, import_world,
    load_visual_job_claim, release_visual_job_claim, serve_vn,
};
use std::collections::HashSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "singulari-world",
    about = "File-backed persistent world simulator kernel"
)]
struct Cli {
    /// Override the world store root. Also supported through `SINGULARI_WORLD_HOME`.
    #[arg(long)]
    store_root: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HostWorkerTextBackend {
    /// Use `WebGPT` as the narrative engine.
    Webgpt,
}

impl HostWorkerTextBackend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Webgpt => "webgpt",
        }
    }
}

impl fmt::Display for HostWorkerTextBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<WorldTextBackend> for HostWorkerTextBackend {
    fn from(value: WorldTextBackend) -> Self {
        match value {
            WorldTextBackend::Webgpt => Self::Webgpt,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HostWorkerVisualBackend {
    /// Use `ChatGPT` Web image generation through `WebGPT` MCP.
    Webgpt,
    /// Do not claim or generate visual jobs from this worker.
    None,
}

impl HostWorkerVisualBackend {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Webgpt => "webgpt",
            Self::None => "none",
        }
    }
}

impl fmt::Display for HostWorkerVisualBackend {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<WorldVisualBackend> for HostWorkerVisualBackend {
    fn from(value: WorldVisualBackend) -> Self {
        match value {
            WorldVisualBackend::Webgpt => Self::Webgpt,
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new world from a JSON/YAML seed.
    Init {
        #[arg(long)]
        seed: PathBuf,

        #[arg(long)]
        session_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Initialize a new world from compact worldsim chat seed text.
    Start {
        #[arg(long)]
        seed_text: String,

        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        title: Option<String>,

        #[arg(long)]
        session_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Print the world record and latest snapshot.
    Snapshot {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Advance one world turn from user input.
    Turn {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        input: String,

        #[arg(long)]
        json: bool,

        #[arg(long)]
        render: bool,
    },

    /// Render a stored render packet as Singulari World Markdown.
    Render {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        turn_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Print or write a visual-novel projection packet for the latest turn.
    VnPacket {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        turn_id: Option<String>,

        #[arg(long)]
        scene_image_url: Option<String>,

        #[arg(long)]
        output: Option<PathBuf>,

        #[arg(long)]
        json: bool,
    },

    /// Serve the local VN web UI and turn API.
    VnServe {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long, default_value = "127.0.0.1")]
        host: String,

        #[arg(long, default_value_t = 4177)]
        port: u16,
    },

    /// Print world visual asset status and `image_provider` MCP jobs.
    VisualAssets {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Atomically claim one pending host image generation job.
    VisualJobClaim {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        slot: Option<String>,

        #[arg(long, default_value = "webgpt_image_worker")]
        claimed_by: String,

        #[arg(long)]
        force: bool,

        #[arg(long)]
        json: bool,
    },

    /// Mark a host image generation job complete after the PNG is saved.
    VisualJobComplete {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        slot: String,

        #[arg(long)]
        claim_id: Option<String>,

        #[arg(long)]
        generated_path: Option<PathBuf>,

        #[arg(long)]
        json: bool,
    },

    /// Release a visual generation claim without accepting an asset.
    VisualJobRelease {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        slot: String,

        #[arg(long)]
        json: bool,
    },

    /// Validate a persisted world.
    Validate {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Print cross-projection health for one world.
    ProjectionHealth {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Print the current worldsim chat active world binding.
    Active {
        #[arg(long)]
        json: bool,
    },

    /// Print long-term world.db projection counts.
    DbStats {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Refresh human-readable world documents for the active or explicit world.
    Docs {
        #[arg(long)]
        world_id: Option<String>,
    },

    /// Force a deterministic summary for unsummarized canon events.
    Summarize {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Print a compact continuation packet for worldsim chat resume.
    ResumePack {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long, default_value_t = 8)]
        recent_events: usize,

        #[arg(long, default_value_t = 8)]
        recent_memories: usize,

        #[arg(long, default_value_t = 3)]
        chapters: usize,

        #[arg(long)]
        json: bool,
    },

    /// Print the real player-visible DB-backed Archive View.
    CodexView {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        query: Option<String>,

        #[arg(long, default_value = "all")]
        section: String,

        #[arg(long, default_value_t = 12)]
        limit: usize,

        #[arg(long)]
        json: bool,
    },

    /// Search player-visible world DB projections with FTS.
    Search {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        query: String,

        #[arg(long, default_value_t = 10)]
        limit: usize,

        #[arg(long)]
        json: bool,
    },

    /// Show recent structured entity and relationship updates.
    EntityUpdates {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long, default_value_t = 12)]
        limit: usize,

        #[arg(long)]
        json: bool,
    },

    /// Upsert a character speech/ending/tone/gesture/habit/drift anchor.
    CharacterAnchor {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        character_id: String,

        #[arg(long)]
        name: Option<String>,

        #[arg(long)]
        role: Option<String>,

        #[arg(long)]
        knowledge_state: Option<String>,

        #[arg(long = "speech")]
        speech: Vec<String>,

        #[arg(long = "ending")]
        endings: Vec<String>,

        #[arg(long = "tone")]
        tone: Vec<String>,

        #[arg(long = "gesture")]
        gestures: Vec<String>,

        #[arg(long = "habit")]
        habits: Vec<String>,

        #[arg(long = "drift")]
        drift: Vec<String>,

        #[arg(long)]
        replace: bool,

        #[arg(long)]
        json: bool,
    },

    /// Rebuild world.db from JSON/JSONL evidence files.
    #[command(alias = "backfill-db")]
    RepairDb {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Classify a worldsim chat message into the next simulator command.
    ChatRoute {
        #[arg(long)]
        message: String,

        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Queue player input for worldsim-agent narrative authorship.
    AgentSubmit {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        input: String,

        #[arg(long)]
        json: bool,
    },

    /// Print the pending worldsim-agent turn packet.
    AgentNext {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Commit an agent-authored narrative response JSON.
    AgentCommit {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        response: PathBuf,

        #[arg(long)]
        json: bool,
    },

    /// Run the embedding-host supervisor loop for text and visual jobs.
    HostWorker {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long, default_value_t = 750)]
        interval_ms: u64,

        #[arg(long)]
        once: bool,

        /// Narrative engine used by the common VN web frontend.
        #[arg(
            long,
            env = "SINGULARI_WORLD_TEXT_BACKEND",
            value_enum,
            default_value_t = HostWorkerTextBackend::Webgpt
        )]
        text_backend: HostWorkerTextBackend,

        /// Visual job backend used by the common VN web frontend.
        #[arg(
            long,
            env = "SINGULARI_WORLD_VISUAL_BACKEND",
            value_enum,
            default_value_t = HostWorkerVisualBackend::Webgpt
        )]
        visual_backend: HostWorkerVisualBackend,

        /// Executable adapter used only by the webgpt text backend.
        #[arg(long, env = "SINGULARI_WORLD_WEBGPT_TURN_COMMAND")]
        webgpt_turn_command: Option<PathBuf>,

        /// `WebGPT` MCP wrapper used by the built-in webgpt text backend.
        #[arg(long, env = "SINGULARI_WORLD_WEBGPT_MCP_WRAPPER")]
        webgpt_mcp_wrapper: Option<PathBuf>,

        /// Optional `WebGPT` model override for narrative turns.
        #[arg(long, env = "SINGULARI_WORLD_WEBGPT_MODEL")]
        webgpt_model: Option<String>,

        /// Optional `WebGPT` reasoning-level override for narrative turns.
        #[arg(long, env = "SINGULARI_WORLD_WEBGPT_REASONING_LEVEL")]
        webgpt_reasoning_level: Option<String>,

        /// Dedicated `WebGPT` text-lane browser profile.
        #[arg(long, env = "SINGULARI_WORLD_WEBGPT_TEXT_PROFILE_DIR")]
        webgpt_text_profile_dir: Option<PathBuf>,

        /// Dedicated `WebGPT` image-lane browser profile.
        #[arg(long, env = "SINGULARI_WORLD_WEBGPT_IMAGE_PROFILE_DIR")]
        webgpt_image_profile_dir: Option<PathBuf>,

        /// Dedicated `WebGPT` text-lane CDP port.
        #[arg(
            long,
            env = "SINGULARI_WORLD_WEBGPT_TEXT_CDP_PORT",
            default_value_t = DEFAULT_WEBGPT_TEXT_CDP_PORT
        )]
        webgpt_text_cdp_port: u16,

        /// Dedicated `WebGPT` image-lane CDP port.
        #[arg(
            long,
            env = "SINGULARI_WORLD_WEBGPT_IMAGE_CDP_PORT",
            default_value_t = DEFAULT_WEBGPT_IMAGE_CDP_PORT
        )]
        webgpt_image_cdp_port: u16,

        /// Timeout in seconds for one `WebGPT` narrative turn.
        #[arg(
            long,
            env = "SINGULARI_WORLD_WEBGPT_TIMEOUT_SECS",
            default_value_t = 900
        )]
        webgpt_timeout_secs: u64,
    },

    /// Print deterministic host supervisor lane plan without dispatching jobs.
    HostSupervisor {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Export a world as a filesystem bundle directory.
    ExportWorld {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        output: PathBuf,

        #[arg(long)]
        json: bool,
    },

    /// Import a filesystem world bundle.
    ImportWorld {
        #[arg(long)]
        bundle: PathBuf,

        #[arg(long)]
        activate: bool,

        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    dispatch(cli)
}

#[allow(
    clippy::too_many_lines,
    reason = "CLI dispatch stays flat at the clap boundary; command behavior lives in handlers"
)]
fn dispatch(cli: Cli) -> Result<()> {
    let store_root = cli.store_root;
    match cli.command {
        Commands::Init {
            seed,
            session_id,
            json,
        } => handle_init(store_root.as_deref(), seed, session_id, json)?,
        Commands::Start {
            seed_text,
            world_id,
            title,
            session_id,
            json,
        } => handle_start(
            store_root.as_deref(),
            seed_text,
            world_id,
            title,
            session_id,
            json,
        )?,
        Commands::Snapshot { world_id, json } => {
            handle_snapshot(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::Turn {
            world_id,
            input,
            json,
            render,
        } => handle_turn(
            store_root.as_deref(),
            world_id.as_deref(),
            input,
            json,
            render,
        )?,
        Commands::Render {
            world_id,
            turn_id,
            json,
        } => handle_render(store_root.as_deref(), world_id.as_deref(), turn_id, json)?,
        Commands::VnPacket {
            world_id,
            turn_id,
            scene_image_url,
            output,
            json,
        } => handle_vn_packet(
            store_root.as_deref(),
            world_id.as_deref(),
            turn_id,
            scene_image_url,
            output,
            json,
        )?,
        Commands::VnServe {
            world_id,
            host,
            port,
        } => handle_vn_serve(store_root.as_deref(), world_id, host, port)?,
        Commands::VisualAssets { world_id, json } => {
            handle_visual_assets(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::VisualJobClaim {
            world_id,
            slot,
            claimed_by,
            force,
            json,
        } => handle_visual_job_claim(
            store_root.as_deref(),
            world_id.as_deref(),
            slot,
            claimed_by,
            force,
            json,
        )?,
        Commands::VisualJobComplete {
            world_id,
            slot,
            claim_id,
            generated_path,
            json,
        } => handle_visual_job_complete(
            store_root.as_deref(),
            world_id.as_deref(),
            slot,
            claim_id,
            generated_path,
            json,
        )?,
        Commands::VisualJobRelease {
            world_id,
            slot,
            json,
        } => handle_visual_job_release(store_root.as_deref(), world_id.as_deref(), slot, json)?,
        Commands::Validate { world_id, json } => {
            handle_validate(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::ProjectionHealth { world_id, json } => {
            handle_projection_health(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::Active { json } => handle_active(store_root.as_deref(), json)?,
        Commands::DbStats { world_id, json } => {
            handle_db_stats(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::Docs { world_id } => handle_docs(store_root.as_deref(), world_id.as_deref())?,
        Commands::Summarize { world_id, json } => {
            handle_summarize(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::ResumePack {
            world_id,
            recent_events,
            recent_memories,
            chapters,
            json,
        } => handle_resume_pack(
            store_root.as_deref(),
            world_id.as_deref(),
            recent_events,
            recent_memories,
            chapters,
            json,
        )?,
        Commands::CodexView {
            world_id,
            query,
            section,
            limit,
            json,
        } => handle_codex_view(
            store_root.as_deref(),
            world_id.as_deref(),
            query,
            section.as_str(),
            limit,
            json,
        )?,
        Commands::Search {
            world_id,
            query,
            limit,
            json,
        } => handle_search(
            store_root.as_deref(),
            world_id.as_deref(),
            query.as_str(),
            limit,
            json,
        )?,
        Commands::EntityUpdates {
            world_id,
            limit,
            json,
        } => handle_entity_updates(store_root.as_deref(), world_id.as_deref(), limit, json)?,
        Commands::CharacterAnchor {
            world_id,
            character_id,
            name,
            role,
            knowledge_state,
            speech,
            endings,
            tone,
            gestures,
            habits,
            drift,
            replace,
            json,
        } => handle_character_anchor(
            store_root.as_deref(),
            world_id.as_deref(),
            CharacterAnchorInput {
                character_id,
                name,
                role,
                knowledge_state,
                speech,
                endings,
                tone,
                gestures,
                habits,
                drift,
                replace,
            },
            json,
        )?,
        Commands::RepairDb { world_id, json } => {
            handle_repair_db(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::ChatRoute {
            message,
            world_id,
            json,
        } => handle_chat_route(message, world_id, json)?,
        Commands::AgentSubmit {
            world_id,
            input,
            json,
        } => handle_agent_submit(store_root.as_deref(), world_id.as_deref(), input, json)?,
        Commands::AgentNext { world_id, json } => {
            handle_agent_next(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::AgentCommit {
            world_id,
            response,
            json,
        } => handle_agent_commit(
            store_root.as_deref(),
            world_id.as_deref(),
            response.as_path(),
            json,
        )?,
        Commands::HostWorker {
            world_id,
            interval_ms,
            once,
            text_backend,
            visual_backend,
            webgpt_turn_command,
            webgpt_mcp_wrapper,
            webgpt_model,
            webgpt_reasoning_level,
            webgpt_text_profile_dir,
            webgpt_image_profile_dir,
            webgpt_text_cdp_port,
            webgpt_image_cdp_port,
            webgpt_timeout_secs,
        } => handle_host_worker(
            store_root.as_deref(),
            world_id.as_deref(),
            &HostWorkerOptions {
                interval_ms,
                once,
                text_backend,
                visual_backend,
                webgpt_turn_command,
                webgpt_mcp_wrapper,
                webgpt_model,
                webgpt_reasoning_level,
                webgpt_text_profile_dir,
                webgpt_image_profile_dir,
                webgpt_text_cdp_port,
                webgpt_image_cdp_port,
                webgpt_timeout_secs,
            },
        )?,
        Commands::HostSupervisor { world_id, json } => {
            handle_host_supervisor(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::ExportWorld {
            world_id,
            output,
            json,
        } => handle_export_world(store_root.as_deref(), world_id.as_deref(), output, json)?,
        Commands::ImportWorld {
            bundle,
            activate,
            json,
        } => handle_import_world(store_root.as_deref(), bundle, activate, json)?,
    }
    Ok(())
}

fn handle_init(
    store_root: Option<&Path>,
    seed: PathBuf,
    session_id: Option<String>,
    json: bool,
) -> Result<()> {
    let initialized = init_world(&InitWorldOptions {
        seed_path: seed,
        store_root: store_root.map(Path::to_path_buf),
        session_id,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&initialized.world)?);
    } else {
        println!("world: {}", initialized.world.world_id);
        println!("title: {}", initialized.world.title);
        println!("session: {}", initialized.session_id);
        println!("world_dir: {}", initialized.world_dir.display());
        println!("snapshot: {}", initialized.snapshot_path.display());
        println!(
            "anchor_invariant: {}",
            initialized.world.anchor_character.invariant
        );
    }
    Ok(())
}

fn handle_start(
    store_root: Option<&Path>,
    seed_text: String,
    world_id: Option<String>,
    title: Option<String>,
    session_id: Option<String>,
    json: bool,
) -> Result<()> {
    let started = start_world(&StartWorldOptions {
        seed_text,
        world_id,
        title,
        store_root: store_root.map(Path::to_path_buf),
        session_id,
    })?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "seed": started.seed,
                "world": started.initialized.world,
                "active": started.active_binding,
                "session_id": started.initialized.session_id,
                "world_dir": started.initialized.world_dir,
                "snapshot": started.initialized.snapshot_path,
            }))?
        );
    } else {
        println!("{}", render_started_world_report(&started));
    }
    Ok(())
}

fn handle_snapshot(store_root: Option<&Path>, world_id: Option<&str>, json: bool) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let world = load_world_record(store_root, world_id.as_str())?;
    let snapshot = load_latest_snapshot(store_root, world_id.as_str())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "world": world,
                "latest_snapshot": snapshot,
            }))?
        );
    } else {
        println!("world: {}", world.world_id);
        println!("title: {}", world.title);
        println!("anchor_invariant: {}", world.anchor_character.invariant);
        println!("turn: {}", snapshot.turn_id);
        println!("phase: {}", snapshot.phase);
        println!("session: {}", snapshot.session_id);
        println!("location: {}", snapshot.protagonist_state.location);
    }
    Ok(())
}

fn handle_turn(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    input: String,
    json: bool,
    render: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let turn = advance_turn(&AdvanceTurnOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        input,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&turn.render_packet)?);
    } else if render {
        println!("{}", render_packet_markdown(&turn.render_packet));
    } else {
        println!("{}", render_advanced_turn_report(&turn));
    }
    Ok(())
}

fn handle_render(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    turn_id: Option<String>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let packet = load_render_packet(&RenderPacketLoadOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        turn_id,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&packet)?);
    } else {
        println!("{}", render_packet_markdown(&packet));
    }
    Ok(())
}

fn handle_vn_packet(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    turn_id: Option<String>,
    scene_image_url: Option<String>,
    output: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let mut options = BuildVnPacketOptions::new(world_id);
    options.store_root = store_root.map(Path::to_path_buf);
    options.turn_id = turn_id;
    options.scene_image_url = scene_image_url;
    let packet = build_vn_packet(&options)?;
    let serialized = serde_json::to_string_pretty(&packet)?;
    if let Some(output) = output {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&output, serialized)?;
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "output": output,
                    "world_id": packet.world_id,
                    "turn_id": packet.turn_id,
                }))?
            );
        } else {
            println!("vn_packet: {}", output.display());
            println!("world: {}", packet.world_id);
            println!("turn: {}", packet.turn_id);
        }
    } else {
        println!("{serialized}");
    }
    Ok(())
}

fn handle_vn_serve(
    store_root: Option<&Path>,
    world_id: Option<String>,
    host: String,
    port: u16,
) -> Result<()> {
    let mut options = VnServeOptions::new(world_id, port);
    options.store_root = store_root.map(Path::to_path_buf);
    options.host = host;
    serve_vn(&options)
}

fn handle_visual_assets(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&manifest)?);
    } else {
        println!("world: {}", manifest.world_id);
        println!(
            "menu_background: {} exists={}",
            manifest.menu_background.recommended_path, manifest.menu_background.exists
        );
        println!(
            "stage_background: {} exists={}",
            manifest.stage_background.recommended_path, manifest.stage_background.exists
        );
        println!(
            "image_generation_jobs: {}",
            manifest.image_generation_jobs.len()
        );
        for job in manifest.image_generation_jobs {
            println!("- [{}] {}", job.slot, job.destination_path);
            println!("  tool: {}", job.tool);
            println!("  prompt: {}", job.prompt);
        }
    }
    Ok(())
}

fn handle_visual_job_claim(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    slot: Option<String>,
    claimed_by: String,
    force: bool,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let extra_jobs = current_turn_visual_jobs(store_root, world_id.as_str())?;
    let outcome = claim_visual_job(&ClaimVisualJobOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        slot,
        claimed_by,
        force,
        extra_jobs,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&outcome)?);
    } else {
        match outcome {
            singulari_world::VisualJobClaimOutcome::Claimed { claim } => {
                println!("world: {}", claim.world_id);
                println!("slot: {}", claim.slot);
                println!("claim_id: {}", claim.claim_id);
                println!("destination_path: {}", claim.job.destination_path);
                println!("prompt: {}", claim.job.prompt);
            }
            singulari_world::VisualJobClaimOutcome::NoPending { world_id } => {
                println!("world: {world_id}");
                println!("visual_job: <none>");
            }
        }
    }
    Ok(())
}

fn handle_visual_job_complete(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    slot: String,
    claim_id: Option<String>,
    generated_path: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let completion = complete_visual_job(&CompleteVisualJobOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        slot,
        claim_id,
        generated_path,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&completion)?);
    } else {
        println!("world: {}", completion.world_id);
        println!("slot: {}", completion.slot);
        println!("destination_path: {}", completion.destination_path);
        println!("bytes: {}", completion.bytes);
    }
    Ok(())
}

fn handle_visual_job_release(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    slot: String,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let release = release_visual_job_claim(&ReleaseVisualJobClaimOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        slot,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&release)?);
    } else {
        println!("world: {}", release.world_id);
        println!("slot: {}", release.slot);
        if let Some(claim) = &release.claim {
            println!("released_claim: {}", claim.claim_id);
        } else {
            println!("released_claim: <none>");
        }
    }
    Ok(())
}

fn handle_validate(store_root: Option<&Path>, world_id: Option<&str>, json: bool) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let report = validate_world(store_root, world_id.as_str())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!(
            "{}",
            singulari_world::validate::render_validation_report(&report)
        );
    }
    if report.status == ValidationStatus::Failed {
        std::process::exit(1);
    }
    Ok(())
}

fn handle_projection_health(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let report = build_projection_health_report(store_root, world_id.as_str())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("{}", render_projection_health_report(&report));
    }
    Ok(())
}

fn handle_active(store_root: Option<&Path>, json: bool) -> Result<()> {
    let active = load_active_world(store_root)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&active)?);
    } else {
        println!("world: {}", active.world_id);
        println!("session: {}", active.session_id);
        println!("updated_at: {}", active.updated_at);
    }
    Ok(())
}

fn handle_db_stats(store_root: Option<&Path>, world_id: Option<&str>, json: bool) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let world = load_world_record(store_root, world_id.as_str())?;
    let paths = singulari_world::resolve_store_paths(store_root)?;
    let world_dir = paths.worlds_dir.join(world.world_id.as_str());
    let stats = world_db_stats(&world_dir, world.world_id.as_str())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("world: {}", stats.world_id);
        println!("db: {}", stats.db_path.display());
        println!("schema: {}", stats.schema_version);
        println!("world_facts: {}", stats.world_facts);
        println!("canon_events: {}", stats.canon_events);
        println!("character_memories: {}", stats.character_memories);
        println!("state_changes: {}", stats.state_changes);
        println!("entity_records: {}", stats.entity_records);
        println!("entity_updates: {}", stats.entity_updates);
        println!("relationship_updates: {}", stats.relationship_updates);
        println!("snapshots: {}", stats.snapshots);
        println!("chapter_summaries: {}", stats.chapter_summaries);
        println!("search_documents: {}", stats.search_documents);
    }
    Ok(())
}

fn handle_docs(store_root: Option<&Path>, world_id: Option<&str>) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let paths = singulari_world::resolve_store_paths(store_root)?;
    let world_dir = paths.worlds_dir.join(world_id.as_str());
    refresh_world_docs(&world_dir)?;
    println!(
        "docs: {}",
        world_dir.join(singulari_world::WORLD_DOCS_DIR).display()
    );
    Ok(())
}

fn handle_summarize(store_root: Option<&Path>, world_id: Option<&str>, json: bool) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let paths = singulari_world::resolve_store_paths(store_root)?;
    let world_dir = paths.worlds_dir.join(world_id.as_str());
    let summary = force_chapter_summary(&world_dir, world_id.as_str())?;
    refresh_world_docs(&world_dir)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else if let Some(summary) = summary {
        println!("summary: {}", summary.summary_id);
        println!("title: {}", summary.title);
        println!(
            "range: {} -> {}",
            summary.source_turn_start, summary.source_turn_end
        );
        println!("{}", summary.summary);
    } else {
        println!("summary: none");
    }
    Ok(())
}

fn handle_codex_view(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    query: Option<String>,
    section: &str,
    limit: usize,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let section = section.parse::<CodexViewSection>()?;
    let mut options = BuildCodexViewOptions::new(world_id);
    options.store_root = store_root.map(Path::to_path_buf);
    options.query = query;
    options.limit = limit;
    let view = build_codex_view(&options)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&view)?);
    } else {
        println!("{}", render_codex_view_section_markdown(&view, section));
    }
    Ok(())
}

fn handle_search(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    query: &str,
    limit: usize,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let paths = singulari_world::resolve_store_paths(store_root)?;
    let world_dir = paths.worlds_dir.join(world_id.as_str());
    let hits = search_world_db(&world_dir, world_id.as_str(), query, limit)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&hits)?);
    } else if hits.is_empty() {
        println!("search: no visible hits");
    } else {
        for hit in hits {
            println!(
                "{}:{} [{}] {}",
                hit.source_table, hit.source_id, hit.title, hit.snippet
            );
        }
    }
    Ok(())
}

fn handle_entity_updates(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let paths = singulari_world::resolve_store_paths(store_root)?;
    let world_dir = paths.worlds_dir.join(world_id.as_str());
    let entity_updates = recent_entity_updates(&world_dir, world_id.as_str(), limit)?;
    let relationship_updates = recent_relationship_updates(&world_dir, world_id.as_str(), limit)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "entity_updates": entity_updates,
                "relationship_updates": relationship_updates,
            }))?
        );
    } else {
        println!("entity_updates:");
        for update in &entity_updates {
            println!(
                "- `{}` `{}` {}: {}",
                update.turn_id, update.entity_id, update.update_kind, update.summary
            );
        }
        println!("relationship_updates:");
        for update in &relationship_updates {
            println!(
                "- `{}` `{}` -> `{}` {}: {}",
                update.turn_id,
                update.source_entity_id,
                update.target_entity_id,
                update.relation_kind,
                update.summary
            );
        }
    }
    Ok(())
}

struct CharacterAnchorInput {
    character_id: String,
    name: Option<String>,
    role: Option<String>,
    knowledge_state: Option<String>,
    speech: Vec<String>,
    endings: Vec<String>,
    tone: Vec<String>,
    gestures: Vec<String>,
    habits: Vec<String>,
    drift: Vec<String>,
    replace: bool,
}

fn handle_character_anchor(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    input: CharacterAnchorInput,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let report = apply_character_anchor(&ApplyCharacterAnchorOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        character_id: input.character_id,
        name: input.name,
        role: input.role,
        knowledge_state: input.knowledge_state,
        speech: input.speech,
        endings: input.endings,
        tone: input.tone,
        gestures: input.gestures,
        habits: input.habits,
        drift: input.drift,
        replace: input.replace,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("world: {}", report.world_id);
        println!("character: {} {}", report.character_id, report.name);
        println!("role: {}", report.role);
        println!("knowledge_state: {}", report.knowledge_state);
        println!("created_character: {}", report.created_character);
        println!("changed_fields: {}", report.changed_fields.join(", "));
        println!("update: {}", report.update_id);
        println!("db_rebuilt: {}", report.db_repair.rebuilt);
        println!("docs: {}", report.docs_dir.display());
    }
    Ok(())
}

fn handle_repair_db(store_root: Option<&Path>, world_id: Option<&str>, json: bool) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let paths = singulari_world::resolve_store_paths(store_root)?;
    let world_dir = paths.worlds_dir.join(world_id.as_str());
    let report = repair_world_db(&world_dir, world_id.as_str())?;
    refresh_world_docs(&world_dir)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("world: {}", report.world_id);
        println!("db: {}", report.db_path.display());
        println!("rebuilt: {}", report.rebuilt);
        println!("canon_events: {}", report.canon_events);
        println!("snapshots: {}", report.snapshots);
        println!("render_packets: {}", report.render_packets);
        println!("search_documents: {}", report.search_documents);
    }
    Ok(())
}

fn handle_host_supervisor(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let plan = build_host_supervisor_plan(store_root, world_id.as_str())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
    } else {
        println!("{}", render_host_supervisor_plan(&plan));
    }
    Ok(())
}

fn handle_chat_route(message: String, world_id: Option<String>, json: bool) -> Result<()> {
    let route = route_chat_input(&ChatRouteOptions { message, world_id });
    if json {
        println!("{}", serde_json::to_string_pretty(&route)?);
    } else {
        println!("{}", render_chat_route(&route));
    }
    Ok(())
}

fn handle_agent_submit(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    input: String,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        input,
        narrative_level: None,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pending)?);
    } else {
        println!("status: {}", pending.status);
        println!("world: {}", pending.world_id);
        println!("turn: {}", pending.turn_id);
        println!("pending: {}", pending.pending_ref);
        println!(
            "next: singulari-world agent-next --world-id {} --json",
            pending.world_id
        );
    }
    Ok(())
}

fn handle_agent_next(store_root: Option<&Path>, world_id: Option<&str>, json: bool) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let pending = load_pending_agent_turn(store_root, world_id.as_str())?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pending)?);
    } else {
        println!("status: {}", pending.status);
        println!("world: {}", pending.world_id);
        println!("turn: {}", pending.turn_id);
        println!("input: {}", pending.player_input);
        if let Some(choice) = &pending.selected_choice {
            println!(
                "choice: {}. {} — {}",
                choice.slot, choice.tag, choice.visible_intent
            );
        }
        println!("pending: {}", pending.pending_ref);
    }
    Ok(())
}

fn handle_agent_commit(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    response: &Path,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let raw_body = std::fs::read_to_string(response)
        .with_context(|| format!("failed to read {}", response.display()))?;
    let raw = serde_json::from_str::<AgentTurnResponse>(&raw_body)
        .with_context(|| format!("failed to parse {}", response.display()))?;
    let committed = commit_agent_turn(&AgentCommitTurnOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        response: raw,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&committed)?);
    } else {
        println!("world: {}", committed.world_id);
        println!("turn: {}", committed.turn_id);
        println!("render_packet: {}", committed.render_packet_path);
        println!("response: {}", committed.response_path);
        println!("commit_record: {}", committed.commit_record_path);
    }
    Ok(())
}

const HOST_WORKER_EVENT_SCHEMA_VERSION: &str = "singulari.host_worker_event.v1";
const HOST_WORKER_CONSUMER: &str = "webgpt_host_worker";
const DEFAULT_WEBGPT_TEXT_CDP_PORT: u16 = 9238;
const DEFAULT_WEBGPT_IMAGE_CDP_PORT: u16 = 9239;

#[derive(Debug, Clone)]
struct HostWorkerOptions {
    interval_ms: u64,
    once: bool,
    text_backend: HostWorkerTextBackend,
    visual_backend: HostWorkerVisualBackend,
    webgpt_turn_command: Option<PathBuf>,
    webgpt_mcp_wrapper: Option<PathBuf>,
    webgpt_model: Option<String>,
    webgpt_reasoning_level: Option<String>,
    webgpt_text_profile_dir: Option<PathBuf>,
    webgpt_image_profile_dir: Option<PathBuf>,
    webgpt_text_cdp_port: u16,
    webgpt_image_cdp_port: u16,
    webgpt_timeout_secs: u64,
}

#[allow(
    clippy::too_many_lines,
    reason = "Host worker loop keeps backend resolution, startup events, and idle behavior visible at the CLI boundary"
)]
fn handle_host_worker(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    options: &HostWorkerOptions,
) -> Result<()> {
    let interval = Duration::from_millis(options.interval_ms.max(250));
    ensure_webgpt_lane_runtime_isolated(options)?;
    let mut emitted = HashSet::new();
    let initial_world_id = resolve_host_worker_world_id(store_root, world_id)?;
    emit_host_event(&serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "worker_started",
        "world_id": initial_world_id.as_deref(),
        "text_backend": options.text_backend.as_str(),
        "visual_backend": options.visual_backend.as_str(),
        "visual_jobs": host_worker_visual_jobs_label(options.visual_backend),
        "consumer": HOST_WORKER_CONSUMER,
    }))?;

    loop {
        let Some(world_id) = resolve_host_worker_world_id(store_root, world_id)? else {
            if emitted.insert("worker-waiting-for-active-world".to_owned()) {
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "worker_waiting_for_active_world",
                    "world_id": null,
                    "text_backend": options.text_backend.as_str(),
                    "visual_backend": options.visual_backend.as_str(),
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
            }
            if options.once {
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "worker_idle",
                    "world_id": null,
                    "text_backend": options.text_backend.as_str(),
                    "visual_backend": options.visual_backend.as_str(),
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
                break;
            }
            thread::sleep(interval);
            continue;
        };
        let (text_backend, visual_backend) =
            effective_host_worker_backends(store_root, world_id.as_str(), options)?;
        let mut emitted_this_tick = false;
        if visual_backend == HostWorkerVisualBackend::None {
            if emit_host_pending_agent_turn_event(
                store_root,
                world_id.as_str(),
                &mut emitted,
                text_backend,
                options,
            )? {
                emitted_this_tick = true;
            }
        } else if emit_host_text_and_visual_events_parallel(
            store_root,
            world_id.as_str(),
            &mut emitted,
            text_backend,
            visual_backend,
            options,
        )? {
            emitted_this_tick = true;
        }
        if options.once {
            if !emitted_this_tick {
                emit_host_event(&serde_json::json!({
                        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "worker_idle",
                    "world_id": world_id,
                        "text_backend": text_backend.as_str(),
                        "visual_backend": visual_backend.as_str(),
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
            }
            break;
        }
        thread::sleep(interval);
    }
    Ok(())
}

fn effective_host_worker_backends(
    store_root: Option<&Path>,
    world_id: &str,
    options: &HostWorkerOptions,
) -> Result<(HostWorkerTextBackend, HostWorkerVisualBackend)> {
    let Some(selection) = load_world_backend_selection(store_root, world_id)? else {
        return Ok((options.text_backend, options.visual_backend));
    };
    if !selection.locked {
        anyhow::bail!("backend selection is not locked for world_id={world_id}");
    }
    Ok((
        HostWorkerTextBackend::from(selection.text_backend),
        HostWorkerVisualBackend::from(selection.visual_backend),
    ))
}

const fn host_worker_visual_jobs_label(backend: HostWorkerVisualBackend) -> &'static str {
    match backend {
        HostWorkerVisualBackend::Webgpt => "claim_and_generate_webgpt",
        HostWorkerVisualBackend::None => "disabled",
    }
}

fn resolve_host_worker_world_id(
    store_root: Option<&Path>,
    world_id: Option<&str>,
) -> Result<Option<String>> {
    if world_id.is_some() {
        return resolve_world_id(store_root, world_id).map(Some);
    }
    let store_paths = resolve_store_paths(store_root)?;
    let active_path = store_paths.root.join(ACTIVE_WORLD_FILENAME);
    if !active_path.exists() {
        return Ok(None);
    }
    Ok(Some(load_active_world(store_root)?.world_id))
}

fn emit_host_pending_agent_turn_event(
    store_root: Option<&Path>,
    world_id: &str,
    emitted: &mut HashSet<String>,
    text_backend: HostWorkerTextBackend,
    options: &HostWorkerOptions,
) -> Result<bool> {
    let Ok(pending) = load_pending_agent_turn(store_root, world_id) else {
        return Ok(false);
    };
    match text_backend {
        HostWorkerTextBackend::Webgpt => {
            emit_webgpt_pending_agent_turn_event(store_root, emitted, &pending, options)
        }
    }
}

fn emit_webgpt_pending_agent_turn_event(
    store_root: Option<&Path>,
    emitted: &mut HashSet<String>,
    pending: &singulari_world::PendingAgentTurn,
    options: &HostWorkerOptions,
) -> Result<bool> {
    match dispatch_pending_agent_turn_via_webgpt(store_root, pending, options)? {
        WebGptDispatchOutcome::Started(record) => {
            let event_key = format!("webgpt-started:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&webgpt_dispatch_started_event(pending, &record))?;
            Ok(true)
        }
        WebGptDispatchOutcome::AlreadyDispatched(record_path) => {
            let event_key = format!("webgpt-skipped:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&serde_json::json!({
                "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                "event": "webgpt_dispatch_skipped",
                "reason": "already_dispatched",
                "world_id": pending.world_id,
                "turn_id": pending.turn_id,
                "record_path": record_path,
                "consumer": HOST_WORKER_CONSUMER,
            }))?;
            Ok(true)
        }
    }
}

enum HostTextDispatchResult {
    Webgpt { outcome: WebGptDispatchOutcome },
}

enum HostVisualDispatchResult {
    Webgpt(WebGptImageDispatchRecord),
}

fn emit_host_text_and_visual_events_parallel(
    store_root: Option<&Path>,
    world_id: &str,
    emitted: &mut HashSet<String>,
    text_backend: HostWorkerTextBackend,
    visual_backend: HostWorkerVisualBackend,
    options: &HostWorkerOptions,
) -> Result<bool> {
    let pending = load_pending_agent_turn(store_root, world_id).ok();
    let visual_claim = match visual_backend {
        HostWorkerVisualBackend::Webgpt => {
            claim_next_host_visual_job(store_root, world_id, "singulari_webgpt_image_worker")?
        }
        HostWorkerVisualBackend::None => None,
    };
    if pending.is_none() && visual_claim.is_none() {
        return Ok(false);
    }

    let (text_result, image_result) = thread::scope(|scope| {
        let text_handle = pending.as_ref().map(|pending| {
            scope.spawn(move || match text_backend {
                HostWorkerTextBackend::Webgpt => {
                    let outcome =
                        dispatch_pending_agent_turn_via_webgpt(store_root, pending, options)?;
                    Ok(HostTextDispatchResult::Webgpt { outcome })
                }
            })
        });
        let image_handle = visual_claim.as_ref().map(|claim| {
            scope.spawn(move || match visual_backend {
                HostWorkerVisualBackend::Webgpt => {
                    dispatch_visual_job_via_webgpt(store_root, claim, options)
                        .map(HostVisualDispatchResult::Webgpt)
                }
                HostWorkerVisualBackend::None => {
                    anyhow::bail!("visual backend none cannot dispatch visual claim")
                }
            })
        });

        let text_result = text_handle.map(|handle| {
            handle
                .join()
                .unwrap_or_else(|panic| Err(thread_panic_error("text dispatch", panic.as_ref())))
        });
        let image_result = image_handle.map(|handle| {
            handle
                .join()
                .unwrap_or_else(|panic| Err(thread_panic_error("image dispatch", panic.as_ref())))
        });
        (text_result, image_result)
    });

    let mut emitted_any = false;
    if let Some((pending, result)) = pending.as_ref().zip(text_result)
        && emit_text_dispatch_result(pending, result?, emitted)?
    {
        emitted_any = true;
    }

    let initial_image_dispatched = image_result.is_some();
    if let Some(result) = image_result
        && emit_visual_dispatch_result(result?)?
    {
        emitted_any = true;
    }
    if !initial_image_dispatched
        && let Some(result) =
            dispatch_host_visual_job_once(store_root, world_id, visual_backend, options)?
        && emit_visual_dispatch_result(result)?
    {
        emitted_any = true;
    }
    Ok(emitted_any)
}

fn dispatch_host_visual_job_once(
    store_root: Option<&Path>,
    world_id: &str,
    visual_backend: HostWorkerVisualBackend,
    options: &HostWorkerOptions,
) -> Result<Option<HostVisualDispatchResult>> {
    let visual_claim = match visual_backend {
        HostWorkerVisualBackend::Webgpt => {
            claim_next_host_visual_job(store_root, world_id, "singulari_webgpt_image_worker")?
        }
        HostWorkerVisualBackend::None => None,
    };
    let Some(claim) = visual_claim else {
        return Ok(None);
    };
    let result = match visual_backend {
        HostWorkerVisualBackend::Webgpt => {
            dispatch_visual_job_via_webgpt(store_root, &claim, options)
                .map(HostVisualDispatchResult::Webgpt)?
        }
        HostWorkerVisualBackend::None => {
            anyhow::bail!("visual backend none cannot dispatch visual claim")
        }
    };
    Ok(Some(result))
}

fn emit_text_dispatch_result(
    pending: &singulari_world::PendingAgentTurn,
    result: HostTextDispatchResult,
    emitted: &mut HashSet<String>,
) -> Result<bool> {
    match result {
        HostTextDispatchResult::Webgpt { outcome } => {
            emit_webgpt_text_dispatch_result(pending, outcome, emitted)
        }
    }
}

fn emit_webgpt_text_dispatch_result(
    pending: &singulari_world::PendingAgentTurn,
    outcome: WebGptDispatchOutcome,
    emitted: &mut HashSet<String>,
) -> Result<bool> {
    match outcome {
        WebGptDispatchOutcome::Started(record) => {
            let event_key = format!("webgpt-started:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&webgpt_dispatch_started_event(pending, &record))?;
        }
        WebGptDispatchOutcome::AlreadyDispatched(record_path) => {
            let event_key = format!("webgpt-skipped:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&webgpt_dispatch_skipped_event(
                pending,
                record_path.as_str(),
            ))?;
        }
    }
    Ok(true)
}

fn emit_visual_dispatch_result(result: HostVisualDispatchResult) -> Result<bool> {
    match result {
        HostVisualDispatchResult::Webgpt(record) => {
            emit_host_event(&webgpt_image_completed_event(&record))?;
        }
    }
    Ok(true)
}

fn thread_panic_error(context: &str, panic: &(dyn std::any::Any + Send)) -> anyhow::Error {
    let reason = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown panic payload");
    anyhow::anyhow!("{context} panicked: {reason}")
}

fn webgpt_dispatch_started_event(
    pending: &singulari_world::PendingAgentTurn,
    record: &WebGptDispatchRecord,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "webgpt_dispatch_started",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "turn_status": record.status,
        "adapter_command": record.adapter_command,
        "mcp_wrapper": record.mcp_wrapper,
        "conversation_id": record.raw_conversation_id,
        "pid": record.pid,
        "record_path": record.record_path,
        "prompt_path": record.prompt_path,
        "response_path": record.response_path,
        "result_path": record.result_path,
        "stdout_path": record.stdout_path,
        "stderr_path": record.stderr_path,
        "committed_turn_id": record.committed_turn_id,
        "consumer": HOST_WORKER_CONSUMER,
    })
}

fn webgpt_dispatch_skipped_event(
    pending: &singulari_world::PendingAgentTurn,
    record_path: &str,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "webgpt_dispatch_skipped",
        "reason": "already_dispatched",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "record_path": record_path,
        "consumer": HOST_WORKER_CONSUMER,
    })
}

fn claim_next_host_visual_job(
    store_root: Option<&Path>,
    world_id: &str,
    claimed_by: &str,
) -> Result<Option<singulari_world::VisualJobClaim>> {
    let jobs = current_host_visual_jobs(store_root, world_id)?;
    for job in jobs {
        if load_visual_job_claim(store_root, world_id, job.slot.as_str())?.is_some() {
            continue;
        }
        let outcome = claim_visual_job(&ClaimVisualJobOptions {
            store_root: store_root.map(Path::to_path_buf),
            world_id: world_id.to_owned(),
            slot: Some(job.slot.clone()),
            claimed_by: claimed_by.to_owned(),
            force: false,
            extra_jobs: current_turn_visual_jobs(store_root, world_id)?,
        })?;
        let singulari_world::VisualJobClaimOutcome::Claimed { claim } = outcome else {
            anyhow::bail!(
                "visual job vanished before claim: world_id={world_id}, slot={}",
                job.slot
            );
        };
        return Ok(Some(*claim));
    }
    Ok(None)
}

fn webgpt_image_completed_event(record: &WebGptImageDispatchRecord) -> serde_json::Value {
    serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "webgpt_image_generate_completed",
        "world_id": record.world_id.as_str(),
        "slot": record.slot.as_str(),
        "claim_id": record.claim_id.as_deref(),
        "generated_path": record.generated_path.as_deref(),
        "destination_path": record.destination_path.as_str(),
        "record_path": record.record_path.as_str(),
        "status": record.status.as_str(),
        "consumer": HOST_WORKER_CONSUMER,
    })
}

fn current_host_visual_jobs(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<Vec<ImageGenerationJob>> {
    let mut packet_options = BuildVnPacketOptions::new(world_id.to_owned());
    packet_options.store_root = store_root.map(Path::to_path_buf);
    let packet = build_vn_packet(&packet_options)?;
    if let Some(job) = packet.image.image_generation_job {
        let mut jobs = vec![job];
        jobs.extend(packet.visual_assets.image_generation_jobs);
        return Ok(dedupe_visual_jobs_by_slot(jobs));
    }
    let jobs = packet.visual_assets.image_generation_jobs;
    Ok(dedupe_visual_jobs_by_slot(jobs))
}

fn current_turn_visual_jobs(
    store_root: Option<&Path>,
    world_id: &str,
) -> Result<Vec<ImageGenerationJob>> {
    Ok(current_host_visual_jobs(store_root, world_id)?
        .into_iter()
        .filter(|job| job.slot.starts_with("turn_cg:"))
        .collect())
}

fn dedupe_visual_jobs_by_slot(jobs: Vec<ImageGenerationJob>) -> Vec<ImageGenerationJob> {
    let mut seen = HashSet::new();
    let mut deduped = Vec::new();
    for job in jobs {
        if seen.insert(job.slot.clone()) {
            deduped.push(job);
        }
    }
    deduped
}

fn emit_host_event(event: &serde_json::Value) -> Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string(event)?)?;
    stdout.flush()?;
    Ok(())
}

fn ensure_control_safe_runtime_value(field: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        anyhow::bail!("{field} contains control characters");
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct WebGptLaneRuntime {
    lane: WebGptConversationLane,
    profile_dir: PathBuf,
    cdp_port: u16,
}

impl WebGptLaneRuntime {
    fn new(lane: WebGptConversationLane, options: &HostWorkerOptions) -> Result<Self> {
        Ok(Self {
            lane,
            profile_dir: webgpt_lane_profile_dir(lane, options)?,
            cdp_port: match lane {
                WebGptConversationLane::Text => options.webgpt_text_cdp_port,
                WebGptConversationLane::Image => options.webgpt_image_cdp_port,
            },
        })
    }

    fn cdp_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.cdp_port)
    }

    fn apply_to_command(&self, command: &mut Command) {
        command
            .env("WEBGPT_MCP_CDP_PORT", self.cdp_port.to_string())
            .env("WEBGPT_MCP_PROFILE_DIR", self.profile_dir.as_os_str())
            .env(
                "WEBGPT_MCP_MANUAL_PROFILE_DIR",
                self.profile_dir.as_os_str(),
            )
            .env(
                "WEBGPT_MCP_BOOTSTRAP_SNAPSHOT_DIR",
                self.profile_dir.with_extension("snapshot").as_os_str(),
            );
    }
}

fn ensure_webgpt_lane_runtime_isolated(options: &HostWorkerOptions) -> Result<()> {
    let text = WebGptLaneRuntime::new(WebGptConversationLane::Text, options)?;
    let image = WebGptLaneRuntime::new(WebGptConversationLane::Image, options)?;
    if text.cdp_port == image.cdp_port {
        anyhow::bail!(
            "webgpt text/image lanes must use distinct CDP ports: port={}",
            text.cdp_port
        );
    }
    if text.profile_dir == image.profile_dir {
        anyhow::bail!(
            "webgpt text/image lanes must use distinct profile dirs: profile_dir={}",
            text.profile_dir.display()
        );
    }
    Ok(())
}

fn webgpt_lane_profile_dir(
    lane: WebGptConversationLane,
    options: &HostWorkerOptions,
) -> Result<PathBuf> {
    let configured = match lane {
        WebGptConversationLane::Text => options.webgpt_text_profile_dir.clone(),
        WebGptConversationLane::Image => options.webgpt_image_profile_dir.clone(),
    };
    if let Some(path) = configured {
        return Ok(path);
    }
    let root = if let Some(path) = std::env::var_os("SINGULARI_WORLD_WEBGPT_PROFILE_ROOT") {
        PathBuf::from(path)
    } else {
        webgpt_default_profile_root()?
    };
    Ok(root.join(lane.profile_dir_name()))
}

fn webgpt_default_profile_root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is required for WebGPT profile defaults")?;
    Ok(PathBuf::from(home)
        .join(".hesperides")
        .join("singulari-world")
        .join("webgpt"))
}

enum WebGptDispatchOutcome {
    Started(Box<WebGptDispatchRecord>),
    AlreadyDispatched(String),
}

#[derive(Debug, Serialize)]
struct WebGptDispatchRecord {
    schema_version: &'static str,
    status: String,
    world_id: String,
    turn_id: String,
    adapter_command: Option<String>,
    mcp_wrapper: Option<String>,
    mcp_profile_dir: Option<String>,
    mcp_cdp_port: Option<u16>,
    mcp_cdp_url: Option<String>,
    conversation_id: Option<String>,
    raw_conversation_id: Option<String>,
    current_model: Option<String>,
    current_reasoning_level: Option<String>,
    pid: u32,
    record_path: String,
    prompt_path: String,
    response_path: String,
    result_path: Option<String>,
    stdout_path: String,
    stderr_path: String,
    dispatched_at: String,
    exit_code: Option<i32>,
    committed_turn_id: Option<String>,
    render_packet_path: Option<String>,
    commit_record_path: Option<String>,
    error: Option<String>,
    completed_at: String,
}

#[derive(Debug, Serialize)]
struct WebGptImageDispatchRecord {
    schema_version: &'static str,
    status: String,
    world_id: String,
    slot: String,
    claim_id: Option<String>,
    mcp_wrapper: String,
    mcp_profile_dir: String,
    mcp_cdp_port: u16,
    mcp_cdp_url: String,
    image_session_kind: String,
    reference_paths: Vec<String>,
    conversation_id: Option<String>,
    raw_conversation_id: Option<String>,
    pid: u32,
    record_path: String,
    prompt_path: String,
    result_path: String,
    stdout_path: String,
    stderr_path: String,
    generated_path: Option<String>,
    generated_sha256: Option<String>,
    generated_bytes: Option<usize>,
    destination_path: String,
    completion_path: Option<String>,
    dispatched_at: String,
    exit_code: Option<i32>,
    error: Option<String>,
    completed_at: String,
}

fn dispatch_pending_agent_turn_via_webgpt(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    options: &HostWorkerOptions,
) -> Result<WebGptDispatchOutcome> {
    let dispatch_dir = dispatch_dir_for_pending(pending)?;
    fs::create_dir_all(&dispatch_dir)
        .with_context(|| format!("failed to create {}", dispatch_dir.display()))?;
    let record_path = dispatch_dir.join(format!("{}-webgpt.json", pending.turn_id));
    if record_path.exists() {
        return Ok(WebGptDispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let prompt_path = dispatch_dir.join(format!("{}-webgpt-prompt.md", pending.turn_id));
    let response_path =
        dispatch_dir.join(format!("{}-webgpt-agent-response.json", pending.turn_id));
    let result_path = dispatch_dir.join(format!("{}-webgpt-result.json", pending.turn_id));
    let stdout_path = dispatch_dir.join(format!("{}-webgpt-stdout.log", pending.turn_id));
    let stderr_path = dispatch_dir.join(format!("{}-webgpt-stderr.log", pending.turn_id));
    let prompt = build_webgpt_turn_prompt(store_root, pending)?;
    fs::write(&prompt_path, prompt.as_bytes())
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;
    let dispatcher = resolve_webgpt_dispatcher(store_root, pending, options)?;

    let paths = WebGptTurnPaths {
        world_id: pending.world_id.as_str(),
        turn_id: pending.turn_id.as_str(),
        prompt_path: prompt_path.as_path(),
        response_path: response_path.as_path(),
        result_path: result_path.as_path(),
        stdout_path: stdout_path.as_path(),
        stderr_path: stderr_path.as_path(),
    };
    let claim = webgpt_dispatch_claim(pending, &dispatcher, paths);
    if !write_dispatch_claim(record_path.as_path(), &claim)? {
        return Ok(WebGptDispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let dispatch_result = dispatcher.run(paths)?;
    if let Some(raw_conversation_id) = dispatch_result.raw_conversation_id.as_deref() {
        save_webgpt_conversation_binding(
            store_root,
            pending.world_id.as_str(),
            WebGptConversationLane::Text,
            raw_conversation_id,
        )?;
    }

    let commit_result = commit_webgpt_dispatch_if_success(
        store_root,
        pending,
        response_path.as_path(),
        &dispatch_result,
    );

    let record = WebGptDispatchRecord {
        schema_version: "singulari.webgpt_dispatch_record.v1",
        status: commit_result.status,
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        adapter_command: dispatcher.adapter_command_display(),
        mcp_wrapper: dispatcher.mcp_wrapper_display(),
        mcp_profile_dir: dispatcher.mcp_profile_dir_display(),
        mcp_cdp_port: dispatcher.mcp_cdp_port(),
        mcp_cdp_url: dispatcher.mcp_cdp_url(),
        conversation_id: dispatcher.conversation_id().map(str::to_owned),
        raw_conversation_id: dispatch_result.raw_conversation_id,
        current_model: dispatch_result.current_model,
        current_reasoning_level: dispatch_result.current_reasoning_level,
        pid: dispatch_result.pid,
        record_path: record_path.display().to_string(),
        prompt_path: prompt_path.display().to_string(),
        response_path: response_path.display().to_string(),
        result_path: Some(result_path.display().to_string()),
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        dispatched_at: Utc::now().to_rfc3339(),
        exit_code: dispatch_result.exit_code,
        committed_turn_id: commit_result
            .committed
            .as_ref()
            .map(|value| value.turn_id.clone()),
        render_packet_path: commit_result
            .committed
            .as_ref()
            .map(|value| value.render_packet_path.clone()),
        commit_record_path: commit_result
            .committed
            .as_ref()
            .map(|value| value.commit_record_path.clone()),
        error: dispatch_result.error.or(commit_result.error),
        completed_at: Utc::now().to_rfc3339(),
    };
    fs::write(&record_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", record_path.display()))?;
    Ok(WebGptDispatchOutcome::Started(Box::new(record)))
}

fn webgpt_dispatch_claim(
    pending: &singulari_world::PendingAgentTurn,
    dispatcher: &WebGptDispatcher,
    paths: WebGptTurnPaths<'_>,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": "singulari.webgpt_dispatch_record.v1",
        "status": "dispatching",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "adapter_command": dispatcher.adapter_command_display(),
        "mcp_wrapper": dispatcher.mcp_wrapper_display(),
        "mcp_profile_dir": dispatcher.mcp_profile_dir_display(),
        "mcp_cdp_port": dispatcher.mcp_cdp_port(),
        "mcp_cdp_url": dispatcher.mcp_cdp_url(),
        "conversation_id": dispatcher.conversation_id(),
        "prompt_path": paths.prompt_path.display().to_string(),
        "response_path": paths.response_path.display().to_string(),
        "result_path": paths.result_path.display().to_string(),
        "stdout_path": paths.stdout_path.display().to_string(),
        "stderr_path": paths.stderr_path.display().to_string(),
        "dispatched_at": Utc::now().to_rfc3339(),
    })
}

struct WebGptCommitResult {
    status: String,
    committed: Option<singulari_world::CommittedAgentTurn>,
    error: Option<String>,
}

fn commit_webgpt_dispatch_if_success(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    response_path: &Path,
    dispatch_result: &WebGptDispatchResult,
) -> WebGptCommitResult {
    if !dispatch_result.success {
        return WebGptCommitResult {
            status: "failed".to_owned(),
            committed: None,
            error: None,
        };
    }
    match commit_webgpt_agent_response(store_root, pending, response_path) {
        Ok(committed) => WebGptCommitResult {
            status: "completed".to_owned(),
            committed: Some(committed),
            error: None,
        },
        Err(error) => WebGptCommitResult {
            status: "failed_uncommitted".to_owned(),
            committed: None,
            error: Some(error.to_string()),
        },
    }
}

#[derive(Clone, Copy)]
struct WebGptTurnPaths<'a> {
    world_id: &'a str,
    turn_id: &'a str,
    prompt_path: &'a Path,
    response_path: &'a Path,
    result_path: &'a Path,
    stdout_path: &'a Path,
    stderr_path: &'a Path,
}

struct WebGptDispatchResult {
    success: bool,
    pid: u32,
    exit_code: Option<i32>,
    raw_conversation_id: Option<String>,
    current_model: Option<String>,
    current_reasoning_level: Option<String>,
    error: Option<String>,
}

enum WebGptDispatcher {
    ExternalCommand {
        command: PathBuf,
    },
    McpResearch {
        wrapper: PathBuf,
        runtime: WebGptLaneRuntime,
        conversation_id: Option<String>,
        model: Option<String>,
        reasoning_level: Option<String>,
        timeout_secs: u64,
    },
}

impl WebGptDispatcher {
    fn adapter_command_display(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { command } => Some(command.display().to_string()),
            Self::McpResearch { .. } => None,
        }
    }

    fn mcp_wrapper_display(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { wrapper, .. } => Some(wrapper.display().to_string()),
        }
    }

    fn mcp_profile_dir_display(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { runtime, .. } => Some(runtime.profile_dir.display().to_string()),
        }
    }

    fn mcp_cdp_port(&self) -> Option<u16> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { runtime, .. } => Some(runtime.cdp_port),
        }
    }

    fn mcp_cdp_url(&self) -> Option<String> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch { runtime, .. } => Some(runtime.cdp_url()),
        }
    }

    fn conversation_id(&self) -> Option<&str> {
        match self {
            Self::ExternalCommand { .. } => None,
            Self::McpResearch {
                conversation_id, ..
            } => conversation_id.as_deref(),
        }
    }

    fn run(&self, paths: WebGptTurnPaths<'_>) -> Result<WebGptDispatchResult> {
        match self {
            Self::ExternalCommand { command } => run_external_webgpt_turn_command(command, paths),
            Self::McpResearch {
                wrapper,
                runtime,
                conversation_id,
                model,
                reasoning_level,
                timeout_secs,
            } => run_webgpt_mcp_research_turn(
                wrapper,
                conversation_id.as_deref(),
                model.as_deref(),
                reasoning_level.as_deref(),
                *timeout_secs,
                runtime,
                paths,
            ),
        }
    }
}

fn resolve_webgpt_dispatcher(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    options: &HostWorkerOptions,
) -> Result<WebGptDispatcher> {
    if let Some(command) = &options.webgpt_turn_command {
        return Ok(WebGptDispatcher::ExternalCommand {
            command: command.clone(),
        });
    }
    Ok(WebGptDispatcher::McpResearch {
        wrapper: resolve_webgpt_mcp_wrapper(options)?,
        runtime: WebGptLaneRuntime::new(WebGptConversationLane::Text, options)?,
        conversation_id: load_webgpt_conversation_binding(
            store_root,
            pending.world_id.as_str(),
            WebGptConversationLane::Text,
        )?,
        model: options.webgpt_model.clone(),
        reasoning_level: options.webgpt_reasoning_level.clone(),
        timeout_secs: options.webgpt_timeout_secs,
    })
}

fn run_external_webgpt_turn_command(
    command: &Path,
    paths: WebGptTurnPaths<'_>,
) -> Result<WebGptDispatchResult> {
    let child = Command::new(command)
        .arg("--prompt-path")
        .arg(paths.prompt_path)
        .arg("--response-path")
        .arg(paths.response_path)
        .arg("--world-id")
        .arg(paths.world_id)
        .arg("--turn-id")
        .arg(paths.turn_id)
        .env("SINGULARI_WORLD_ENGINE", "webgpt")
        .env("SINGULARI_WORLD_PROMPT_PATH", paths.prompt_path)
        .env("SINGULARI_WORLD_RESPONSE_PATH", paths.response_path)
        .env("SINGULARI_WORLD_WORLD_ID", paths.world_id)
        .env("SINGULARI_WORLD_TURN_ID", paths.turn_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn webgpt turn adapter: command={}",
                command.display()
            )
        })?;
    let pid = child.id();
    let output = child
        .wait_with_output()
        .context("failed to wait for webgpt turn adapter")?;
    fs::write(paths.stdout_path, &output.stdout)
        .with_context(|| format!("failed to write {}", paths.stdout_path.display()))?;
    fs::write(paths.stderr_path, &output.stderr)
        .with_context(|| format!("failed to write {}", paths.stderr_path.display()))?;
    Ok(WebGptDispatchResult {
        success: output.status.success(),
        pid,
        exit_code: output.status.code(),
        raw_conversation_id: None,
        current_model: None,
        current_reasoning_level: None,
        error: if output.status.success() {
            None
        } else {
            Some(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                .filter(|value| !value.is_empty())
        },
    })
}

fn run_webgpt_mcp_research_turn(
    wrapper: &Path,
    conversation_id: Option<&str>,
    model: Option<&str>,
    reasoning_level: Option<&str>,
    timeout_secs: u64,
    runtime: &WebGptLaneRuntime,
    paths: WebGptTurnPaths<'_>,
) -> Result<WebGptDispatchResult> {
    let prompt = fs::read_to_string(paths.prompt_path)
        .with_context(|| format!("failed to read {}", paths.prompt_path.display()))?;
    let arguments = build_webgpt_research_arguments(
        prompt.as_str(),
        conversation_id,
        model,
        reasoning_level,
        timeout_secs,
    );
    let arguments_raw = serde_json::to_string(&arguments)?;
    let mut command = Command::new(wrapper);
    runtime.apply_to_command(&mut command);
    let child = command
            .arg("client-call")
            .arg("--wrapper")
            .arg(wrapper)
            .arg("--client-name")
            .arg("singulari-world-webgpt-turn")
            .arg("--require-tool")
            .arg("--tool")
            .arg("webgpt_research")
            .arg("--arguments")
            .arg(arguments_raw)
            .arg("--output")
            .arg("first-text")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| {
                format!(
                    "failed to spawn webgpt-mcp client-call: wrapper={}, lane={}, cdp_url={}, profile_dir={}",
                    wrapper.display(),
                    runtime.lane.as_str(),
                    runtime.cdp_url(),
                    runtime.profile_dir.display()
                )
            })?;
    let pid = child.id();
    let output = child
        .wait_with_output()
        .context("failed to wait for webgpt-mcp client-call")?;
    fs::write(paths.stdout_path, &output.stdout)
        .with_context(|| format!("failed to write {}", paths.stdout_path.display()))?;
    fs::write(paths.stderr_path, &output.stderr)
        .with_context(|| format!("failed to write {}", paths.stderr_path.display()))?;
    if !output.status.success() {
        return Ok(WebGptDispatchResult {
            success: false,
            pid,
            exit_code: output.status.code(),
            raw_conversation_id: None,
            current_model: None,
            current_reasoning_level: None,
            error: Some(String::from_utf8_lossy(&output.stderr).trim().to_owned())
                .filter(|value| !value.is_empty()),
        });
    }
    let raw_result = String::from_utf8(output.stdout)
        .context("webgpt-mcp client-call stdout was not valid UTF-8")?;
    fs::write(paths.result_path, raw_result.as_bytes())
        .with_context(|| format!("failed to write {}", paths.result_path.display()))?;
    let result = serde_json::from_str::<serde_json::Value>(&raw_result)
        .context("failed to parse webgpt_research result JSON")?;
    let answer_markdown = result
        .get("answer_markdown")
        .and_then(serde_json::Value::as_str)
        .context("webgpt_research result missing answer_markdown")?;
    let agent_response_json = extract_json_object_text(answer_markdown)
        .context("webgpt answer did not contain an AgentTurnResponse JSON object")?;
    fs::write(paths.response_path, agent_response_json.as_bytes())
        .with_context(|| format!("failed to write {}", paths.response_path.display()))?;
    Ok(WebGptDispatchResult {
        success: true,
        pid,
        exit_code: output.status.code(),
        raw_conversation_id: result
            .get("raw_conversation_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        current_model: result
            .get("current_model")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        current_reasoning_level: result
            .get("current_reasoning_level")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        error: None,
    })
}

fn build_webgpt_research_arguments(
    prompt: &str,
    conversation_id: Option<&str>,
    model: Option<&str>,
    reasoning_level: Option<&str>,
    timeout_secs: u64,
) -> serde_json::Value {
    let mut arguments = serde_json::json!({
        "prompt": prompt,
        "timeout_secs": timeout_secs.max(60),
        "auto_recover": true,
        "recovery_attempts": 1,
    });
    if let Some(object) = arguments.as_object_mut() {
        if let Some(conversation_id) = conversation_id.filter(|value| !value.trim().is_empty()) {
            object.insert(
                "conversation_id".to_owned(),
                serde_json::json!(conversation_id),
            );
        } else {
            object.insert("new_conversation".to_owned(), serde_json::json!(true));
        }
        if let Some(model) = model.filter(|value| !value.trim().is_empty()) {
            object.insert("model".to_owned(), serde_json::json!(model));
        }
        if let Some(reasoning_level) = reasoning_level.filter(|value| !value.trim().is_empty()) {
            object.insert(
                "reasoning_level".to_owned(),
                serde_json::json!(reasoning_level),
            );
        }
    }
    arguments
}

fn resolve_webgpt_mcp_wrapper(options: &HostWorkerOptions) -> Result<PathBuf> {
    if let Some(wrapper) = &options.webgpt_mcp_wrapper {
        return Ok(wrapper.clone());
    }
    if let Some(wrapper) = std::env::var_os("SINGULARI_WORLD_WEBGPT_MCP_WRAPPER").map(PathBuf::from)
    {
        return Ok(wrapper);
    }
    if let Some(wrapper) = local_env_value("SINGULARI_WORLD_WEBGPT_MCP_WRAPPER")?.map(PathBuf::from)
    {
        return Ok(wrapper);
    }
    if let Some(wrapper) = std::env::var_os("WEBGPT_MCP_WRAPPER").map(PathBuf::from) {
        return Ok(wrapper);
    }
    if let Some(wrapper) = find_webgpt_mcp_wrapper_from_current_dir()? {
        return Ok(wrapper);
    }
    anyhow::bail!(
        "webgpt text backend requires --webgpt-mcp-wrapper, SINGULARI_WORLD_WEBGPT_MCP_WRAPPER in env/.env, or a sibling webgpt-mcp-checkout/scripts/webgpt-mcp.sh"
    );
}

fn local_env_value(name: &str) -> Result<Option<String>> {
    let path = std::env::current_dir()
        .context("failed to resolve current working directory")?
        .join(".env");
    if !path.is_file() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path.as_path())
        .with_context(|| format!("failed to read {}", path.display()))?;
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != name {
            continue;
        }
        let value = unquote_local_env_value(value.trim());
        ensure_control_safe_runtime_value(name, value.as_str())?;
        if !value.is_empty() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

fn unquote_local_env_value(value: &str) -> String {
    if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        value[1..value.len() - 1].to_owned()
    } else {
        value.to_owned()
    }
}

fn find_webgpt_mcp_wrapper_from_current_dir() -> Result<Option<PathBuf>> {
    let mut dir = std::env::current_dir().context("failed to resolve current working directory")?;
    loop {
        let direct = dir.join("webgpt-mcp-checkout/scripts/webgpt-mcp.sh");
        if direct.is_file() {
            return Ok(Some(direct));
        }
        let sibling = dir.join("../webgpt-mcp-checkout/scripts/webgpt-mcp.sh");
        if sibling.is_file() {
            return Ok(Some(normalize_prompt_path(sibling.as_path())));
        }
        if !dir.pop() {
            return Ok(None);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum WebGptConversationLane {
    Text,
    Image,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum WebGptImageSessionKind {
    TurnCg,
    ReferenceAsset,
}

impl WebGptImageSessionKind {
    fn from_slot(slot: &str) -> Self {
        if slot.starts_with("turn_cg:") {
            Self::TurnCg
        } else {
            Self::ReferenceAsset
        }
    }

    const fn as_str(self) -> &'static str {
        match self {
            Self::TurnCg => "turn_cg",
            Self::ReferenceAsset => "reference_asset",
        }
    }

    const fn binding_filename(self) -> &'static str {
        match self {
            Self::TurnCg => "webgpt_image_conversation_binding.json",
            Self::ReferenceAsset => "webgpt_reference_asset_conversation_binding.json",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::TurnCg => "webgpt_mcp_image_generation_turn_cg",
            Self::ReferenceAsset => "webgpt_mcp_image_generation_reference_asset",
        }
    }
}

impl WebGptConversationLane {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }

    const fn binding_filename(self) -> &'static str {
        match self {
            Self::Text => "webgpt_conversation_binding.json",
            Self::Image => "webgpt_image_conversation_binding.json",
        }
    }

    const fn source(self) -> &'static str {
        match self {
            Self::Text => "webgpt_mcp_research",
            Self::Image => "webgpt_mcp_image_generation",
        }
    }

    const fn profile_dir_name(self) -> &'static str {
        match self {
            Self::Text => "text-profile",
            Self::Image => "image-profile",
        }
    }
}

fn webgpt_conversation_url(conversation_id: &str) -> String {
    format!("https://chatgpt.com/c/{conversation_id}")
}

fn webgpt_conversation_binding_path(
    store_root: Option<&Path>,
    world_id: &str,
    lane: WebGptConversationLane,
) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths
        .root
        .join("worlds")
        .join(world_id)
        .join("agent_bridge")
        .join(lane.binding_filename()))
}

fn load_webgpt_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    lane: WebGptConversationLane,
) -> Result<Option<String>> {
    let path = webgpt_conversation_binding_path(store_root, world_id, lane)?;
    if !path.exists() {
        return Ok(None);
    }
    let value = read_json_value_if_present(path.as_path())?
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(value
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty()))
}

fn webgpt_image_conversation_binding_path(
    store_root: Option<&Path>,
    world_id: &str,
    session_kind: WebGptImageSessionKind,
) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths
        .root
        .join("worlds")
        .join(world_id)
        .join("agent_bridge")
        .join(session_kind.binding_filename()))
}

fn load_webgpt_image_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    session_kind: WebGptImageSessionKind,
) -> Result<Option<String>> {
    let path = webgpt_image_conversation_binding_path(store_root, world_id, session_kind)?;
    if !path.exists() {
        return Ok(None);
    }
    let value = read_json_value_if_present(path.as_path())?
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(value
        .get("conversation_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .filter(|value| !value.trim().is_empty()))
}

fn save_webgpt_image_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    session_kind: WebGptImageSessionKind,
    conversation_id: &str,
) -> Result<()> {
    let path = webgpt_image_conversation_binding_path(store_root, world_id, session_kind)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "singulari.webgpt_image_conversation_binding.v1",
            "world_id": world_id,
            "lane": WebGptConversationLane::Image.as_str(),
            "image_session_kind": session_kind.as_str(),
            "conversation_id": conversation_id,
            "conversation_url": webgpt_conversation_url(conversation_id),
            "source": session_kind.source(),
            "updated_at": Utc::now().to_rfc3339(),
        }))?,
    )
    .with_context(|| format!("failed to write {}", path.display()))
}

fn save_webgpt_conversation_binding(
    store_root: Option<&Path>,
    world_id: &str,
    lane: WebGptConversationLane,
    conversation_id: &str,
) -> Result<()> {
    let path = webgpt_conversation_binding_path(store_root, world_id, lane)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(
        &path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "singulari.webgpt_conversation_binding.v1",
            "world_id": world_id,
            "lane": lane.as_str(),
            "conversation_id": conversation_id,
            "conversation_url": webgpt_conversation_url(conversation_id),
            "source": lane.source(),
            "updated_at": Utc::now().to_rfc3339(),
        }))?,
    )
    .with_context(|| format!("failed to write {}", path.display()))
}

fn extract_json_object_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok_and(|value| value.is_object()) {
        return Some(trimmed.to_owned());
    }
    if let Some(fenced) = extract_fenced_json_text(trimmed) {
        return Some(fenced);
    }
    extract_first_balanced_json_object(trimmed)
}

fn extract_fenced_json_text(raw: &str) -> Option<String> {
    let fence_start = raw.find("```")?;
    let after_start = &raw[fence_start + 3..];
    let after_header = after_start
        .find('\n')
        .map_or(after_start, |index| &after_start[index + 1..]);
    let fence_end = after_header.find("```")?;
    let candidate = after_header[..fence_end].trim();
    if serde_json::from_str::<serde_json::Value>(candidate).is_ok_and(|value| value.is_object()) {
        Some(candidate.to_owned())
    } else {
        None
    }
}

fn extract_first_balanced_json_object(raw: &str) -> Option<String> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let candidate = raw[start?..=index].trim();
                    if serde_json::from_str::<serde_json::Value>(candidate)
                        .is_ok_and(|value| value.is_object())
                    {
                        return Some(candidate.to_owned());
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}

fn commit_webgpt_agent_response(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    response_path: &Path,
) -> Result<singulari_world::CommittedAgentTurn> {
    let raw_body = fs::read_to_string(response_path)
        .with_context(|| format!("failed to read {}", response_path.display()))?;
    let response = serde_json::from_str::<AgentTurnResponse>(&raw_body)
        .with_context(|| format!("failed to parse {}", response_path.display()))?;
    commit_agent_turn(&AgentCommitTurnOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id: pending.world_id.clone(),
        response,
    })
}

#[allow(
    clippy::too_many_lines,
    reason = "WebGPT image dispatch keeps MCP image generation, extraction, and visual-job completion in one durable record"
)]
fn dispatch_visual_job_via_webgpt(
    store_root: Option<&Path>,
    claim: &singulari_world::VisualJobClaim,
    options: &HostWorkerOptions,
) -> Result<WebGptImageDispatchRecord> {
    let dispatch_dir = visual_dispatch_dir_for_world(store_root, claim.world_id.as_str())?;
    fs::create_dir_all(&dispatch_dir)
        .with_context(|| format!("failed to create {}", dispatch_dir.display()))?;
    let slot_component = safe_file_component(claim.slot.as_str());
    let claim_component = safe_file_component(claim.claim_id.as_str());
    let record_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image.json"
    ));
    let prompt_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-prompt.md"
    ));
    let result_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-result.json"
    ));
    let stdout_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-stdout.log"
    ));
    let stderr_path = dispatch_dir.join(format!(
        "{slot_component}-{claim_component}-webgpt-image-stderr.log"
    ));
    let image_session_kind = WebGptImageSessionKind::from_slot(claim.slot.as_str());
    ensure_image_job_matches_session_kind(&claim.job, image_session_kind)?;
    let conversation_id = load_webgpt_image_conversation_binding(
        store_root,
        claim.world_id.as_str(),
        image_session_kind,
    )?;
    let prompt = build_webgpt_image_generation_prompt(
        claim.world_id.as_str(),
        &claim.job,
        conversation_id.as_deref(),
        image_session_kind,
    );
    let reference_paths = webgpt_image_reference_paths(&claim.job)?;
    fs::write(&prompt_path, prompt.as_bytes())
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;
    let wrapper = resolve_webgpt_mcp_wrapper(options)?;
    let runtime = WebGptLaneRuntime::new(WebGptConversationLane::Image, options)?;

    let dispatched_at = Utc::now().to_rfc3339();
    let claim_record = serde_json::json!({
        "schema_version": "singulari.webgpt_image_dispatch_record.v1",
        "status": "dispatching",
        "world_id": claim.world_id.as_str(),
        "slot": claim.slot.as_str(),
        "claim_id": claim.claim_id.as_str(),
        "mcp_wrapper": wrapper.display().to_string(),
        "mcp_profile_dir": runtime.profile_dir.display().to_string(),
        "mcp_cdp_port": runtime.cdp_port,
        "mcp_cdp_url": runtime.cdp_url(),
        "image_session_kind": image_session_kind.as_str(),
        "reference_paths": reference_paths.as_slice(),
        "conversation_id": conversation_id.as_deref(),
        "prompt_path": prompt_path.display().to_string(),
        "result_path": result_path.display().to_string(),
        "stdout_path": stdout_path.display().to_string(),
        "stderr_path": stderr_path.display().to_string(),
        "destination_path": claim.job.destination_path.as_str(),
        "dispatched_at": dispatched_at.as_str(),
    });
    if !write_dispatch_claim(record_path.as_path(), &claim_record)? {
        anyhow::bail!(
            "webgpt image dispatch already exists: record_path={}",
            record_path.display()
        );
    }

    let child = spawn_webgpt_image_generation(
        wrapper.as_path(),
        &runtime,
        conversation_id.as_deref(),
        reference_paths.as_slice(),
        prompt.as_str(),
        options.webgpt_timeout_secs,
    )?;
    let pid = child.id();
    let output = child
        .wait_with_output()
        .context("failed to wait for webgpt image generation")?;
    fs::write(stdout_path.as_path(), &output.stdout)
        .with_context(|| format!("failed to write {}", stdout_path.display()))?;
    fs::write(stderr_path.as_path(), &output.stderr)
        .with_context(|| format!("failed to write {}", stderr_path.display()))?;
    let mut destination_path = claim.job.destination_path.clone();
    let mut completion_path = None;
    let mut generated_path = None;
    let mut generated_sha256 = None;
    let mut generated_bytes = None;
    let mut raw_conversation_id = None;
    let mut error = if output.status.success() {
        None
    } else {
        Some(String::from_utf8_lossy(&output.stderr).trim().to_owned())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                Some(format!(
                    "webgpt image generation exited with {}",
                    output.status
                ))
            })
    };

    if error.is_none() {
        let raw_result = String::from_utf8(output.stdout.clone())
            .context("webgpt image generation stdout was not valid UTF-8")?;
        fs::write(result_path.as_path(), raw_result.as_bytes())
            .with_context(|| format!("failed to write {}", result_path.display()))?;
        let result = serde_json::from_str::<serde_json::Value>(&raw_result)
            .context("failed to parse webgpt_generate_image result JSON")?;
        raw_conversation_id = result
            .get("conversation_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        if let Some(raw_conversation_id) = raw_conversation_id.as_deref() {
            save_webgpt_image_conversation_binding(
                store_root,
                claim.world_id.as_str(),
                image_session_kind,
                raw_conversation_id,
            )?;
        }
        match first_webgpt_generated_image(&result) {
            Some(image) => {
                generated_path = image
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                generated_sha256 = image
                    .get("sha256")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                generated_bytes = image
                    .get("byte_len")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok());
            }
            None => {
                error = Some("webgpt_generate_image returned no images".to_owned());
            }
        }
    }

    if error.is_none() {
        match generated_path.as_deref().map(PathBuf::from) {
            Some(path) => match complete_visual_job(&CompleteVisualJobOptions {
                store_root: store_root.map(Path::to_path_buf),
                world_id: claim.world_id.clone(),
                slot: claim.slot.clone(),
                claim_id: Some(claim.claim_id.clone()),
                generated_path: Some(path),
            }) {
                Ok(completion) => {
                    destination_path = completion.destination_path;
                    completion_path = Some(completion.completion_path);
                }
                Err(complete_error) => {
                    error = Some(complete_error.to_string());
                }
            },
            None => {
                error = Some("webgpt_generate_image returned image without path".to_owned());
            }
        }
    }

    let status = if error.is_none() {
        "completed"
    } else {
        "failed"
    }
    .to_owned();
    let record = WebGptImageDispatchRecord {
        schema_version: "singulari.webgpt_image_dispatch_record.v1",
        status,
        world_id: claim.world_id.clone(),
        slot: claim.slot.clone(),
        claim_id: Some(claim.claim_id.clone()),
        mcp_wrapper: wrapper.display().to_string(),
        mcp_profile_dir: runtime.profile_dir.display().to_string(),
        mcp_cdp_port: runtime.cdp_port,
        mcp_cdp_url: runtime.cdp_url(),
        image_session_kind: image_session_kind.as_str().to_owned(),
        reference_paths,
        conversation_id,
        raw_conversation_id,
        pid,
        record_path: record_path.display().to_string(),
        prompt_path: prompt_path.display().to_string(),
        result_path: result_path.display().to_string(),
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        generated_path,
        generated_sha256,
        generated_bytes,
        destination_path,
        completion_path,
        dispatched_at,
        exit_code: output.status.code(),
        error,
        completed_at: Utc::now().to_rfc3339(),
    };
    fs::write(record_path.as_path(), serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", record_path.display()))?;
    if let Some(error) = &record.error {
        anyhow::bail!("{error}");
    }
    Ok(record)
}

fn ensure_image_job_matches_session_kind(
    job: &ImageGenerationJob,
    image_session_kind: WebGptImageSessionKind,
) -> Result<()> {
    let valid = match image_session_kind {
        WebGptImageSessionKind::TurnCg => {
            job.artifact_kind == VisualArtifactKind::SceneCg
                && job.display_allowed
                && !job.reference_allowed
                && job.canonical_use == job.artifact_kind.canonical_use()
        }
        WebGptImageSessionKind::ReferenceAsset => {
            matches!(
                job.artifact_kind,
                VisualArtifactKind::CharacterDesignSheet | VisualArtifactKind::LocationDesignSheet
            ) && !job.display_allowed
                && job.reference_allowed
                && job.canonical_use == job.artifact_kind.canonical_use()
        }
    };
    if valid {
        return Ok(());
    }
    anyhow::bail!(
        "webgpt image job kind/session mismatch: slot={}, artifact_kind={:?}, canonical_use={}, display_allowed={}, reference_allowed={}, image_session_kind={}",
        job.slot,
        job.artifact_kind,
        job.canonical_use,
        job.display_allowed,
        job.reference_allowed,
        image_session_kind.as_str()
    )
}

fn webgpt_image_reference_paths(job: &ImageGenerationJob) -> Result<Vec<String>> {
    job.reference_paths
        .iter()
        .map(|raw_path| {
            let path = PathBuf::from(raw_path);
            if !path.is_file() {
                anyhow::bail!(
                    "webgpt image reference asset missing: slot={}, path={}",
                    job.slot,
                    path.display()
                );
            }
            path.canonicalize()
                .with_context(|| {
                    format!(
                        "failed to canonicalize webgpt image reference asset: slot={}, path={}",
                        job.slot,
                        path.display()
                    )
                })
                .map(|path| path.display().to_string())
        })
        .collect()
}

fn spawn_webgpt_image_generation(
    wrapper: &Path,
    runtime: &WebGptLaneRuntime,
    conversation_id: Option<&str>,
    reference_paths: &[String],
    prompt: &str,
    timeout_secs: u64,
) -> Result<Child> {
    let mut arguments = serde_json::json!({
        "prompt": prompt,
        "max_images": 1,
        "timeout_secs": timeout_secs.max(60),
        "auto_recover": true,
        "recovery_attempts": 1,
    });
    if let Some(object) = arguments.as_object_mut()
        && let Some(conversation_id) = conversation_id.filter(|value| !value.trim().is_empty())
    {
        object.insert(
            "conversation_id".to_owned(),
            serde_json::json!(conversation_id),
        );
    }
    if let Some(object) = arguments.as_object_mut()
        && !reference_paths.is_empty()
    {
        object.insert(
            "reference_paths".to_owned(),
            serde_json::json!(reference_paths),
        );
    }
    let mut command = Command::new(wrapper);
    runtime.apply_to_command(&mut command);
    command
        .arg("client-call")
        .arg("--wrapper")
        .arg(wrapper)
        .arg("--client-name")
        .arg("singulari-world-webgpt-image")
        .arg("--require-tool")
        .arg("--tool")
        .arg("webgpt_generate_image")
        .arg("--arguments")
        .arg(serde_json::to_string(&arguments)?)
        .arg("--output")
        .arg("first-text")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn webgpt image generation: wrapper={}, lane={}, cdp_url={}, profile_dir={}",
                wrapper.display(),
                runtime.lane.as_str(),
                runtime.cdp_url(),
                runtime.profile_dir.display()
            )
        })
}

fn first_webgpt_generated_image(result: &serde_json::Value) -> Option<&serde_json::Value> {
    result
        .get("images")
        .and_then(serde_json::Value::as_array)
        .and_then(|images| images.first())
}

fn build_webgpt_image_generation_prompt(
    world_id: &str,
    job: &ImageGenerationJob,
    conversation_id: Option<&str>,
    session_kind: WebGptImageSessionKind,
) -> String {
    let mut prompt = String::new();
    match session_kind {
        WebGptImageSessionKind::TurnCg => {
            prompt.push_str(
                "Generate exactly one full-screen visual novel scene CG for Singulari World.\n",
            );
            prompt.push_str(
                "This ChatGPT conversation is the dedicated world-scoped turn-CG session for ",
            );
            prompt.push_str(world_id);
            prompt.push_str(
                ". Reuse this same session URL only for scene-CG continuity across this world.\n",
            );
        }
        WebGptImageSessionKind::ReferenceAsset => {
            prompt.push_str("Generate exactly one reference asset image for Singulari World.\n");
            prompt.push_str("This ChatGPT conversation is the dedicated world-scoped reference-asset session for ");
            prompt.push_str(world_id);
            prompt.push_str(". Reuse this same session URL only for source-material continuity, not scene-CG continuity.\n");
        }
    }
    if let Some(conversation_id) = conversation_id {
        prompt.push_str("Current image session URL: ");
        prompt.push_str(webgpt_conversation_url(conversation_id).as_str());
        prompt.push('\n');
    }
    match session_kind {
        WebGptImageSessionKind::TurnCg => {
            prompt.push_str("Treat previous turn-CG images in this same conversation as continuity references for palette, line quality, camera language, and recurring setting motifs.\n");
            prompt.push_str("Reference assets named below are source material only; use them as continuity references, but never render a character design sheet, contact sheet, asset board, or UI resource as the scene itself.\n");
        }
        WebGptImageSessionKind::ReferenceAsset => {
            prompt.push_str("Reference assets are source material only. The resulting image must be saved to its requested asset path and must never be treated as or displayed as a turn scene CG.\n");
            prompt.push_str("Do not use turn-CG conversation history or previous scene CGs as source instructions unless they are explicitly listed below.\n");
        }
    }
    prompt.push_str("Return no prose unless ChatGPT requires a short title. Do not make a collage, grid, contact sheet, or variants.\n");
    prompt.push_str("Image job slot: ");
    prompt.push_str(job.slot.as_str());
    prompt.push_str("\nArtifact kind: ");
    prompt.push_str(job.artifact_kind.as_str());
    prompt.push_str("\nCanonical use: ");
    prompt.push_str(job.canonical_use.as_str());
    prompt.push_str("\nDestination path: ");
    prompt.push_str(job.destination_path.as_str());
    prompt.push('\n');
    prompt.push_str("Use the image prompt below as the sole visual brief.\n\n");
    prompt.push_str(job.prompt.as_str());
    if !job.reference_paths.is_empty() {
        prompt.push_str("\n\nReference continuity notes: ");
        prompt.push_str(job.reference_paths.join(", ").as_str());
    }
    prompt
}

fn visual_dispatch_dir_for_world(store_root: Option<&Path>, world_id: &str) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths
        .worlds_dir
        .join(world_id)
        .join("visual_jobs")
        .join("dispatches"))
}

fn read_json_value_if_present(path: &Path) -> Result<Option<serde_json::Value>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    let value = serde_json::from_str(raw.as_str())
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(value))
}

fn dispatch_dir_for_pending(pending: &singulari_world::PendingAgentTurn) -> Result<PathBuf> {
    let pending_ref = Path::new(pending.pending_ref.as_str());
    let Some(agent_bridge_dir) = pending_ref.parent() else {
        anyhow::bail!(
            "pending turn has invalid pending_ref: world_id={}, turn_id={}, pending_ref={}",
            pending.world_id,
            pending.turn_id,
            pending.pending_ref
        );
    };
    Ok(agent_bridge_dir.join("dispatches"))
}

fn write_dispatch_claim(path: &Path, claim: &serde_json::Value) -> Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(serde_json::to_vec_pretty(claim)?.as_slice())?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(error) => Err(error).with_context(|| format!("failed to create {}", path.display())),
    }
}

const AGENT_TURN_RESPONSE_SCHEMA_GUIDE: &str = r#"AgentTurnResponse 스키마:
```json
{
  "schema_version": "singulari.agent_turn_response.v1",
  "world_id": "<world_id>",
  "turn_id": "<turn_id>",
  "visible_scene": {
    "schema_version": "singulari.narrative_scene.v1",
    "text_blocks": ["위 서사 출력 지시와 pending.output_contract.narrative_budget에 맞춘 한국어 VN 본문"],
    "tone_notes": ["짧은 톤 메모"]
  },
  "adjudication": {
    "outcome": "accepted",
    "summary": "플레이어-visible 한 줄 요약",
    "gates": [
      {"gate":"body","status":"pass","reason":"..."},
      {"gate":"resource","status":"pass","reason":"..."},
      {"gate":"time","status":"pass","reason":"..."},
      {"gate":"social_permission","status":"pass","reason":"..."},
      {"gate":"knowledge","status":"pass","reason":"..."}
    ],
    "visible_constraints": ["아직 확인되지 않은 플레이어-visible 제약"],
    "consequences": ["이번 턴의 플레이어-visible 결과"]
  },
  "canon_event": {
    "visibility": "player_visible",
    "kind": "guided_choice",
    "summary": "플레이어-visible 사건 요약"
  },
  "entity_updates": [],
  "relationship_updates": [],
  "extra_contacts": [],
  "hidden_state_delta": [],
  "next_choices": [
    {"slot":1,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"현재 장면 단서와 player_input에서 이어지는 구체 행동"},
    {"slot":2,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"몸, 장소, 물건, 흔적 중 이번 장면에 실제로 나온 단서를 살핀다"},
    {"slot":3,"tag":"현재 장면에 맞춘 짧은 선택명","intent":"이번 장면에 실제로 있는 인물, 기척, 관계 신호에 반응한다"},
    {"slot":4,"tag":"현재 장면에 맞춘 기록 선택명","intent":"이번 장면에서 드러난 기록/단서/세계 지식을 확인한다"},
    {"slot":5,"tag":"현재 장면에 맞춘 흐름 선택명","intent":"이번 사건의 시간 흐름이나 주변 움직임을 한 박자 멀리서 본다"},
    {"slot":6,"tag":"자유서술","intent":"플레이어가 원하는 행동과 말, 내면 독백을 직접 서술한다"},
    {"slot":7,"tag":"판단 위임","intent":"맡긴다. 세부 내용은 선택 후 드러난다."}
  ]
}
```
- next_choices는 서사 생성과 같은 응답에서 반드시 함께 작성한다. 별도 선택지 재생성 턴을 만들지 않는다.
- slot 1,2,3,4,5의 tag/intent는 템플릿 문구가 아니라 이번 visible_scene에서 바로 이어지는 구체 선택지여야 한다.
- next_choices 안에는 label/preview/choices 필드를 쓰지 않는다. 오직 slot/tag/intent만 쓴다.
- slot 번호가 기능 계약이다. tag는 UI 문구이므로 장면에 맞게 짧게 바꿔도 된다. 단 slot 7 tag는 "판단 위임"으로 유지한다.
- extra_contacts는 주변 인물이 플레이어와 직접 상호작용했거나, 의미 있는 목격/거래/도움/위협/감정 흔적을 남겼을 때만 쓴다.
- extra_contacts 항목을 쓸 때는 surface_label, contact_summary를 반드시 실제 장면 내용으로 채운다. 스키마 설명 문구나 예시 문구를 값으로 복사하지 않는다.
- 단순 배경 군중은 extra_contacts에 넣지 않는다. 한 번 스쳐간 인물은 memory_action "trace", 다시 떠올릴 이유가 분명하면 "remember"를 쓴다."#;

const NARRATIVE_TEXT_DESIGN_DIRECTIVE: &str = r"- 캐릭터 voice_anchors는 캐릭터 텍스트 디자인이다. speech는 화법, endings는 어미/말끝, tone은 어투/거리감/어휘, gestures는 반복 제스처, habits는 행동 습관, drift는 변화 방향으로 적용한다.
- 문체와 서사 작법은 캐릭터에 귀속하지 말고 visible_scene의 전역 서사에만 적용한다. 기본 문체는 감각 단서 -> 행동/반응 -> 유보된 추론 -> 선택 압박 -> 여운 순서로 문단을 쌓는 VN prose다.
- 문체를 설명문으로 노출하지 마라. 대사 말끝, 행동 습관, 문단 박자, 장면 압력으로만 체감되게 써라.
- tone_notes에는 이번 턴에서 실제로 반영한 캐릭터 화법/어미/어투와 전역 서사 문체를 짧게 기록한다.";

fn build_webgpt_turn_prompt(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
) -> Result<String> {
    let revival_packet = build_agent_revival_packet(&AgentRevivalCompileOptions {
        store_root,
        pending,
        engine_session_kind: "webgpt_project_session",
    })?;
    let revival_packet = serde_json::to_string_pretty(&revival_packet)
        .context("failed to serialize webgpt revival packet")?;
    let narrative_budget = &pending.output_contract.narrative_budget;
    Ok(format!(
        r#"Singulari World web frontend에서 pending turn 하나가 들어왔어. 너는 WebGPT narrative engine adapter다.

서사 출력 지시:
- 이번 턴 서사 목표: {level_label}. 기본 선택 턴이면 {standard_blocks}문단 / 약 {target_chars}자까지 충분히 써라. 큰 사건이면 {major_blocks}문단 / 약 {major_target_chars}자까지 확장해라.
- text_blocks는 한 항목을 너무 길게 뭉치지 말고, 장면 박자마다 별도 문단으로 나눠라.
- 짧은 로그나 요약이 아니라 한국어 VN prose로 써라. 장면, 감각, 행동, 반응, 여운을 각각 분리해서 쌓아라.

역할:
- 너는 Singulari World의 trusted narrative agent다.
- 플레이어에게 다시 묻지 말고, 아래 revival packet만 보고 바로 서사 턴을 작성한다.
- hidden/private context는 판정에만 쓰고, visible_scene/canon_event/choice text에는 절대 누출하지 않는다.
- 출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고, 게임식 수치 계산처럼 보이게 쓰지 않는다.
{text_design_directive}
- 출력량은 revival packet의 output_contract.narrative_level과 narrative_budget을 따른다. 레벨 간 차이는 확연해야 한다.
- 레벨 1은 표준 VN 밀도, 레벨 2는 장면 확장 밀도, 레벨 3은 장편 연재 밀도다. 레벨 2/3에서는 같은 사건도 감각, 행동, 반응, 여운, 압박을 더 길게 쌓는다.
- player_input이 "세계 개막"이면 그것은 선택지가 아니라 시드에서 첫 서사를 여는 bootstrap turn이다.
- 시드나 visible facts에 명시되지 않은 장르 문법을 추론해서 주입하지 마라. 특히 현대인 전생, 환생, 빙의, 회귀, 이세계 전이, 시스템/치트/상태창은 seed premise나 player-visible canon에 명시된 경우에만 쓴다.
- protagonist가 낯선 환경을 모른다는 사실만으로 현대 기억, 병원/전기/주소 같은 현대 대비, 전생물 독백, 이름 상실 클리셰를 만들지 마라.
- 매 턴 survival/social/material/threat/mystery/desire/moral_cost/time_pressure 중 최소 하나의 장면 압력을 visible_scene과 next_choices에 반영한다. 편향을 지우더라도 무미건조한 로그로 쓰지 마라.
- `anchor_character` 저장 필드는 호환용이다. 시드나 visible canon이 명시하지 않으면 숨은 인물, 운명적 안내자, 히로인, 흑막으로 해석하지 마라. 극점은 인물/장소/물건/세력/맹세/위협/질문 중 visible evidence가 만든다.
- slot 7은 항상 판단 위임이고 preview는 숨긴다: "맡긴다. 세부 내용은 선택 후 드러난다."
- slot 6은 항상 자유서술이며 inline prose를 요구하는 선택지로 둔다.
- 이 WebGPT conversation의 이전 turn들은 말맛, 직전 감정선, 장면 리듬을 잇는 working context다.
- ChatGPT Project의 새 세션이나 기존 conversation history는 기억 저장소가 아니다. 세계 연속성은 revival packet으로만 회생한다.
- WebGPT는 기억 회생을 더 적극적으로 한다. memory_revival.active_memory_revival의 Archive View, query recall, recent entity/relationship updates를 먼저 훑고 이번 player_input과 이어지는 장면 압력을 반영한다.
- 세계의 사실/상태/source of truth는 아래 revival packet과 world store다. 웹 채팅 UI나 이전 MCP tool 결과를 source of truth로 쓰지 마라.
- conversation/project context가 compact 되었거나 revival packet과 충돌하면 revival packet을 우선한다.
- 웹 검색, 외부 사이트 탐색, repo 탐색, 소스 파일 읽기를 하지 마라. 필요한 스키마와 revival packet은 이 프롬프트 안에 있다.

{agent_schema}

revival packet JSON:
```json
{revival_packet}
```

출력:
- AgentTurnResponse JSON 하나만 반환한다.
- Markdown fence, 설명문, 도입문 없이 JSON 본문만 반환한다.
- world_id는 "{world_id}", turn_id는 "{turn_id}"와 정확히 같아야 한다.
"#,
        level_label = narrative_budget.level_label,
        standard_blocks = narrative_budget.standard_choice_turn_blocks,
        target_chars = narrative_budget.target_chars,
        major_blocks = narrative_budget.major_turn_blocks,
        major_target_chars = narrative_budget.major_target_chars,
        text_design_directive = NARRATIVE_TEXT_DESIGN_DIRECTIVE,
        agent_schema = AGENT_TURN_RESPONSE_SCHEMA_GUIDE,
        revival_packet = revival_packet,
        world_id = pending.world_id,
        turn_id = pending.turn_id,
    ))
}

fn normalize_prompt_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn safe_file_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn handle_export_world(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    output: PathBuf,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let report = export_world(&ExportWorldOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        output,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("world: {}", report.world_id);
        println!("bundle: {}", report.bundle_dir.display());
        println!("manifest: {}", report.manifest_path.display());
        println!("files_copied: {}", report.files_copied);
    }
    Ok(())
}

fn handle_import_world(
    store_root: Option<&Path>,
    bundle: PathBuf,
    activate: bool,
    json: bool,
) -> Result<()> {
    let report = import_world(&ImportWorldOptions {
        store_root: store_root.map(Path::to_path_buf),
        bundle,
        activate,
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("world: {}", report.world_id);
        println!("world_dir: {}", report.world_dir.display());
        println!("rebuilt: {}", report.repair_report.rebuilt);
        println!(
            "search_documents: {}",
            report.repair_report.search_documents
        );
        if let Some(active) = report.active_binding {
            println!("active_session: {}", active.session_id);
        }
    }
    Ok(())
}

fn handle_resume_pack(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    recent_events: usize,
    recent_memories: usize,
    chapters: usize,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let mut options = BuildResumePackOptions::new(world_id);
    options.store_root = store_root.map(Path::to_path_buf);
    options.recent_events = recent_events;
    options.recent_memories = recent_memories;
    options.chapter_limit = chapters;
    let pack = build_resume_pack(&options)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&pack)?);
    } else {
        println!("{}", render_resume_pack_markdown(&pack));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webgpt_answer_json_extractor_accepts_fenced_response() -> anyhow::Result<()> {
        let raw = r#"좋아.

```json
{"schema_version":"singulari.agent_turn_response.v1","world_id":"stw","turn_id":"turn_0001"}
```
"#;
        let Some(extracted) = extract_json_object_text(raw) else {
            anyhow::bail!("json should be extracted");
        };
        let value: serde_json::Value = serde_json::from_str(extracted.as_str())?;
        assert_eq!(value["world_id"], serde_json::json!("stw"));
        Ok(())
    }

    #[test]
    fn webgpt_research_arguments_reuse_bound_conversation() {
        let arguments = build_webgpt_research_arguments(
            "prompt",
            Some("conv-123"),
            Some("gpt-5.5"),
            Some("high"),
            12,
        );
        assert_eq!(arguments["conversation_id"], serde_json::json!("conv-123"));
        assert!(arguments.get("new_conversation").is_none());
        assert_eq!(arguments["model"], serde_json::json!("gpt-5.5"));
        assert_eq!(arguments["reasoning_level"], serde_json::json!("high"));
        assert_eq!(arguments["timeout_secs"], serde_json::json!(60));
    }

    #[test]
    fn webgpt_conversation_bindings_are_world_scoped_per_lane() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        save_webgpt_conversation_binding(
            Some(store.as_path()),
            "stw_lane",
            WebGptConversationLane::Text,
            "text-conv",
        )?;
        save_webgpt_conversation_binding(
            Some(store.as_path()),
            "stw_lane",
            WebGptConversationLane::Image,
            "image-conv",
        )?;

        assert_eq!(
            load_webgpt_conversation_binding(
                Some(store.as_path()),
                "stw_lane",
                WebGptConversationLane::Text
            )?,
            Some("text-conv".to_owned())
        );
        assert_eq!(
            load_webgpt_conversation_binding(
                Some(store.as_path()),
                "stw_lane",
                WebGptConversationLane::Image
            )?,
            Some("image-conv".to_owned())
        );

        let text_binding = std::fs::read_to_string(
            store
                .join("worlds/stw_lane/agent_bridge")
                .join(WebGptConversationLane::Text.binding_filename()),
        )?;
        let image_binding = std::fs::read_to_string(
            store
                .join("worlds/stw_lane/agent_bridge")
                .join(WebGptConversationLane::Image.binding_filename()),
        )?;
        assert!(text_binding.contains("\"lane\": \"text\""));
        assert!(text_binding.contains("https://chatgpt.com/c/text-conv"));
        assert!(image_binding.contains("\"lane\": \"image\""));
        assert!(image_binding.contains("https://chatgpt.com/c/image-conv"));
        Ok(())
    }

    #[test]
    fn webgpt_image_conversation_bindings_separate_turn_cg_from_reference_assets()
    -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        save_webgpt_image_conversation_binding(
            Some(store.as_path()),
            "stw_visual_sessions",
            WebGptImageSessionKind::TurnCg,
            "turn-cg-conv",
        )?;
        save_webgpt_image_conversation_binding(
            Some(store.as_path()),
            "stw_visual_sessions",
            WebGptImageSessionKind::ReferenceAsset,
            "asset-conv",
        )?;

        assert_eq!(
            load_webgpt_image_conversation_binding(
                Some(store.as_path()),
                "stw_visual_sessions",
                WebGptImageSessionKind::TurnCg
            )?,
            Some("turn-cg-conv".to_owned())
        );
        assert_eq!(
            load_webgpt_image_conversation_binding(
                Some(store.as_path()),
                "stw_visual_sessions",
                WebGptImageSessionKind::ReferenceAsset
            )?,
            Some("asset-conv".to_owned())
        );

        let bridge_dir = store.join("worlds/stw_visual_sessions/agent_bridge");
        let turn_cg_binding =
            std::fs::read_to_string(bridge_dir.join("webgpt_image_conversation_binding.json"))?;
        let asset_binding = std::fs::read_to_string(
            bridge_dir.join("webgpt_reference_asset_conversation_binding.json"),
        )?;
        assert!(turn_cg_binding.contains("\"image_session_kind\": \"turn_cg\""));
        assert!(turn_cg_binding.contains("https://chatgpt.com/c/turn-cg-conv"));
        assert!(asset_binding.contains("\"image_session_kind\": \"reference_asset\""));
        assert!(asset_binding.contains("https://chatgpt.com/c/asset-conv"));
        Ok(())
    }

    #[test]
    fn webgpt_lane_runtime_defaults_are_isolated() -> anyhow::Result<()> {
        let options = HostWorkerOptions {
            interval_ms: 750,
            once: true,
            text_backend: HostWorkerTextBackend::Webgpt,
            visual_backend: HostWorkerVisualBackend::Webgpt,
            webgpt_turn_command: None,
            webgpt_mcp_wrapper: None,
            webgpt_model: None,
            webgpt_reasoning_level: None,
            webgpt_text_profile_dir: Some("/tmp/singulari-webgpt-text".into()),
            webgpt_image_profile_dir: Some("/tmp/singulari-webgpt-image".into()),
            webgpt_text_cdp_port: DEFAULT_WEBGPT_TEXT_CDP_PORT,
            webgpt_image_cdp_port: DEFAULT_WEBGPT_IMAGE_CDP_PORT,
            webgpt_timeout_secs: 900,
        };

        let text = WebGptLaneRuntime::new(WebGptConversationLane::Text, &options)?;
        let image = WebGptLaneRuntime::new(WebGptConversationLane::Image, &options)?;

        assert_eq!(text.cdp_port, 9238);
        assert_eq!(image.cdp_port, 9239);
        assert_ne!(text.cdp_url(), image.cdp_url());
        assert_ne!(text.profile_dir, image.profile_dir);
        ensure_webgpt_lane_runtime_isolated(&options)?;
        Ok(())
    }

    #[test]
    fn webgpt_lane_runtime_rejects_shared_port() -> anyhow::Result<()> {
        let options = HostWorkerOptions {
            interval_ms: 750,
            once: true,
            text_backend: HostWorkerTextBackend::Webgpt,
            visual_backend: HostWorkerVisualBackend::Webgpt,
            webgpt_turn_command: None,
            webgpt_mcp_wrapper: None,
            webgpt_model: None,
            webgpt_reasoning_level: None,
            webgpt_text_profile_dir: Some("/tmp/singulari-webgpt-text".into()),
            webgpt_image_profile_dir: Some("/tmp/singulari-webgpt-image".into()),
            webgpt_text_cdp_port: 9238,
            webgpt_image_cdp_port: 9238,
            webgpt_timeout_secs: 900,
        };

        let Err(error) = ensure_webgpt_lane_runtime_isolated(&options) else {
            anyhow::bail!("shared WebGPT CDP ports reached dispatch");
        };
        assert!(error.to_string().contains("distinct CDP ports"));
        Ok(())
    }

    #[test]
    fn webgpt_turn_cg_prompt_reuses_turn_cg_session_url() {
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "turn_cg:turn_0002".to_owned(),
                prompt: "draw the scene".to_owned(),
                destination_path: "/tmp/turn_0002.png".to_owned(),
                reference_paths: vec!["/tmp/char.png".to_owned()],
                overwrite: false,
            },
            slot: "turn_cg:turn_0002".to_owned(),
            artifact_kind: VisualArtifactKind::SceneCg,
            canonical_use: "display_scene".to_owned(),
            display_allowed: true,
            reference_allowed: false,
            prompt: "draw the scene".to_owned(),
            destination_path: "/tmp/turn_0002.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: vec!["/tmp/char.png".to_owned()],
            overwrite: false,
            register_policy: "test".to_owned(),
        };
        let prompt = build_webgpt_image_generation_prompt(
            "stw_visual",
            &job,
            Some("image-conv"),
            WebGptImageSessionKind::TurnCg,
        );

        assert!(prompt.contains("dedicated world-scoped turn-CG session for stw_visual"));
        assert!(prompt.contains("https://chatgpt.com/c/image-conv"));
        assert!(prompt.contains("previous turn-CG images in this same conversation"));
        assert!(prompt.contains("never render a character design sheet"));
        assert!(prompt.contains("Image job slot: turn_cg:turn_0002"));
        assert!(prompt.contains("Reference continuity notes: /tmp/char.png"));
    }

    #[test]
    fn webgpt_reference_asset_prompt_is_not_a_scene_cg_prompt() {
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "character_sheet:char:protagonist".to_owned(),
                prompt: "draw the character sheet".to_owned(),
                destination_path: "/tmp/char_protagonist.png".to_owned(),
                reference_paths: Vec::new(),
                overwrite: false,
            },
            slot: "character_sheet:char:protagonist".to_owned(),
            artifact_kind: VisualArtifactKind::CharacterDesignSheet,
            canonical_use: "reference_generation".to_owned(),
            display_allowed: false,
            reference_allowed: true,
            prompt: "draw the character sheet".to_owned(),
            destination_path: "/tmp/char_protagonist.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: Vec::new(),
            overwrite: false,
            register_policy: "test".to_owned(),
        };
        let prompt = build_webgpt_image_generation_prompt(
            "stw_visual",
            &job,
            Some("asset-conv"),
            WebGptImageSessionKind::ReferenceAsset,
        );

        assert!(prompt.contains("reference asset image"));
        assert!(prompt.contains("dedicated world-scoped reference-asset session"));
        assert!(prompt.contains("must never be treated as or displayed as a turn scene CG"));
        assert!(prompt.contains("Do not use turn-CG conversation history"));
        assert!(prompt.contains("Image job slot: character_sheet:char:protagonist"));
        assert!(!prompt.contains("full-screen visual novel scene CG"));
    }

    #[test]
    fn webgpt_image_reference_paths_canonicalize_assets() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let reference = temp.path().join("char.png");
        std::fs::write(&reference, b"png fixture")?;
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "turn_cg:turn_0002".to_owned(),
                prompt: "draw the scene".to_owned(),
                destination_path: "/tmp/turn_0002.png".to_owned(),
                reference_paths: vec![reference.display().to_string()],
                overwrite: false,
            },
            slot: "turn_cg:turn_0002".to_owned(),
            artifact_kind: VisualArtifactKind::SceneCg,
            canonical_use: "display_scene".to_owned(),
            display_allowed: true,
            reference_allowed: false,
            prompt: "draw the scene".to_owned(),
            destination_path: "/tmp/turn_0002.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: vec![reference.display().to_string()],
            overwrite: false,
            register_policy: "test".to_owned(),
        };

        assert_eq!(
            webgpt_image_reference_paths(&job)?,
            vec![reference.canonicalize()?.display().to_string()]
        );
        Ok(())
    }

    #[test]
    fn webgpt_image_reference_paths_fail_loud_when_asset_is_missing() -> anyhow::Result<()> {
        let job = ImageGenerationJob {
            tool: singulari_world::IMAGE_GENERATION_TOOL.to_owned(),
            image_generation_call: singulari_world::HostImageGenerationCall {
                capability: "image_generation".to_owned(),
                slot: "turn_cg:turn_0002".to_owned(),
                prompt: "draw the scene".to_owned(),
                destination_path: "/tmp/turn_0002.png".to_owned(),
                reference_paths: vec!["/tmp/singulari-missing-reference.png".to_owned()],
                overwrite: false,
            },
            slot: "turn_cg:turn_0002".to_owned(),
            artifact_kind: VisualArtifactKind::SceneCg,
            canonical_use: "display_scene".to_owned(),
            display_allowed: true,
            reference_allowed: false,
            prompt: "draw the scene".to_owned(),
            destination_path: "/tmp/turn_0002.png".to_owned(),
            reference_asset_urls: Vec::new(),
            reference_paths: vec!["/tmp/singulari-missing-reference.png".to_owned()],
            overwrite: false,
            register_policy: "test".to_owned(),
        };

        let Err(error) = webgpt_image_reference_paths(&job) else {
            anyhow::bail!("missing reference asset reached image dispatch");
        };
        assert!(
            error
                .to_string()
                .contains("webgpt image reference asset missing")
        );
        Ok(())
    }

    #[test]
    fn webgpt_prompt_carries_realtime_agent_contract() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_contract
title: "webgpt contract test"
premise:
  genre: "fantasy"
  protagonist: "modern reincarnated protagonist"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
            store_root: Some(store.clone()),
            world_id: "stw_contract".to_owned(),
            input: "2".to_owned(),
            narrative_level: None,
        })?;
        let prompt = build_webgpt_turn_prompt(Some(store.as_path()), &pending)?;

        for required in [
            "너는 Singulari World의 trusted narrative agent다.",
            "출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고",
            "speech는 화법, endings는 어미/말끝, tone은 어투/거리감/어휘",
            "문체와 서사 작법은 캐릭터에 귀속하지 말고 visible_scene의 전역 서사에만 적용한다.",
            "레벨 1은 표준 VN 밀도, 레벨 2는 장면 확장 밀도, 레벨 3은 장편 연재 밀도다.",
            "시드나 visible facts에 명시되지 않은 장르 문법을 추론해서 주입하지 마라.",
            "현대인 전생, 환생, 빙의, 회귀, 이세계 전이, 시스템/치트/상태창은 seed premise나 player-visible canon에 명시된 경우에만 쓴다.",
            "병원/전기/주소 같은 현대 대비",
            "이 WebGPT conversation의 이전 turn들은 말맛, 직전 감정선, 장면 리듬을 잇는 working context다.",
            "conversation/project context가 compact 되었거나 revival packet과 충돌하면 revival packet을 우선한다.",
            "WebGPT는 기억 회생을 더 적극적으로 한다.",
            "웹 검색, 외부 사이트 탐색, repo 탐색, 소스 파일 읽기를 하지 마라.",
            "\"schema_version\": \"singulari.agent_revival_packet.v1\"",
            "\"retrieval_profile\"",
            "\"name\": \"webgpt_active_memory\"",
            "\"memory_revival\"",
            "\"resume_pack\"",
            "\"active_memory_revival\"",
            "\"player_visible_archive_view\"",
            "\"query_recall\"",
            "\"recent_entity_updates\"",
            "\"recent_relationship_updates\"",
            "\"source_of_truth_policy\"",
            "\"continuity_source\": \"memory_revival.resume_pack + memory_revival.active_memory_revival\"",
            "\"conflict_rule\": \"revival_packet_wins\"",
            "AgentTurnResponse 스키마:",
            "\"schema_version\": \"singulari.agent_turn_response.v1\"",
            "{\"slot\":6,\"tag\":\"자유서술\"",
            "{\"slot\":7,\"tag\":\"판단 위임\"",
            "world_id는 \"stw_contract\", turn_id는 \"turn_0001\"와 정확히 같아야 한다.",
        ] {
            assert!(
                prompt.contains(required),
                "webgpt prompt missing realtime contract: {required}"
            );
        }
        Ok(())
    }
}
