use anyhow::{Context, Result};
use chrono::Utc;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use singulari_world::resolve_store_paths;
use singulari_world::{
    AdvanceTurnOptions, AgentCommitTurnOptions, AgentSubmitTurnOptions, AgentTurnResponse,
    ApplyCharacterAnchorOptions, InitWorldOptions, RenderPacketLoadOptions, ValidationStatus,
    advance_turn, apply_character_anchor, build_codex_view, build_resume_pack, commit_agent_turn,
    enqueue_agent_turn, force_chapter_summary, init_world, load_active_world, load_latest_snapshot,
    load_pending_agent_turn, load_render_packet, load_world_record, recent_entity_updates,
    recent_relationship_updates, refresh_world_docs, render_advanced_turn_report,
    render_chat_route, render_codex_view_section_markdown, render_packet_markdown,
    render_resume_pack_markdown, render_started_world_report, repair_world_db, resolve_world_id,
    route_chat_input, search_world_db, start_world, validate_world, world_db_stats,
};
use singulari_world::{
    BuildCodexViewOptions, BuildResumePackOptions, BuildVnPacketOptions,
    BuildWorldVisualAssetsOptions, ChatRouteOptions, ClaimVisualJobOptions, CodexViewSection,
    CompleteVisualJobOptions, ExportWorldOptions, ImportWorldOptions, ReleaseVisualJobClaimOptions,
    SaveCodexThreadBindingOptions, StartWorldOptions, VnServeOptions, build_vn_packet,
    build_world_visual_assets, claim_visual_job, clear_codex_thread_binding, complete_visual_job,
    export_world, import_world, load_codex_thread_binding, load_visual_job_claim,
    release_visual_job_claim, save_codex_thread_binding, serve_vn,
};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

    /// Atomically claim one pending Codex App image generation job.
    VisualJobClaim {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        slot: Option<String>,

        #[arg(long, default_value = "codex_app_image_worker")]
        claimed_by: String,

        #[arg(long)]
        force: bool,

        #[arg(long)]
        json: bool,
    },

    /// Mark a Codex App image generation job complete after the PNG is saved.
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

    /// Upsert a character speech/gesture/habit/drift anchor.
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

    /// Watch pending worldsim jobs for Codex App background execution.
    AgentWatch {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long, default_value_t = 1500)]
        interval_ms: u64,

        #[arg(long)]
        once: bool,

        #[arg(long)]
        no_visual_jobs: bool,

        /// Immediately dispatch pending narrative turns to this Codex thread.
        #[arg(long, env = "SINGULARI_WORLD_CODEX_THREAD_ID")]
        codex_thread_id: Option<String>,

        /// Codex CLI path used for realtime thread dispatch.
        #[arg(long, env = "SINGULARI_WORLD_CODEX_BIN")]
        codex_bin: Option<PathBuf>,
    },

    /// Run the embedding-host supervisor loop for text and visual jobs.
    HostWorker {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long, default_value_t = 750)]
        interval_ms: u64,

        #[arg(long)]
        once: bool,

        #[arg(long, value_enum, default_value = "codex-app-server")]
        text_backend: HostTextBackend,

        #[arg(long)]
        no_visual_jobs: bool,

        #[arg(long)]
        claim_visual_jobs: bool,

        /// Thread used by Codex text backends that resume a durable thread.
        #[arg(long, env = "SINGULARI_WORLD_CODEX_THREAD_ID")]
        codex_thread_id: Option<String>,

        /// Codex CLI path used by codex-exec-resume and managed app-server spawn.
        #[arg(long, env = "SINGULARI_WORLD_CODEX_BIN")]
        codex_bin: Option<PathBuf>,

        /// Official Codex app-server websocket URL used by the codex-app-server backend.
        #[arg(long, env = "SINGULARI_WORLD_CODEX_APP_SERVER_URL")]
        codex_app_server_url: Option<String>,

        /// Node.js binary used only by the codex-app-server backend helper.
        #[arg(long, env = "SINGULARI_WORLD_NODE_BIN")]
        node_bin: Option<PathBuf>,
    },

    /// Bind a world to the Codex thread that should receive realtime turns.
    CodexThreadBind {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long, env = "CODEX_THREAD_ID")]
        thread_id: Option<String>,

        #[arg(long, env = "SINGULARI_WORLD_CODEX_BIN")]
        codex_bin: Option<PathBuf>,

        #[arg(long)]
        json: bool,
    },

    /// Show the Codex realtime thread binding for a world.
    CodexThreadShow {
        #[arg(long)]
        world_id: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Clear the Codex realtime thread binding for a world.
    CodexThreadClear {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum HostTextBackend {
    /// Main Codex App path. The reference CLI emits a poller action event.
    #[value(name = "codex-app-poller", alias = "host-session-api")]
    CodexAppPoller,
    /// Official websocket app-server path. Idle costs zero model tokens.
    CodexAppServer,
    /// Reference on-demand path using `codex exec resume <thread-id> -`.
    CodexExecResume,
    /// Development path. Emit pending turn hints without dispatching.
    Manual,
}

impl HostTextBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::CodexAppPoller => "codex-app-poller",
            Self::CodexAppServer => "codex-app-server",
            Self::CodexExecResume => "codex-exec-resume",
            Self::Manual => "manual",
        }
    }
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
        Commands::AgentWatch {
            world_id,
            interval_ms,
            once,
            no_visual_jobs,
            codex_thread_id,
            codex_bin,
        } => handle_agent_watch(
            store_root.as_deref(),
            world_id.as_deref(),
            interval_ms,
            once,
            no_visual_jobs,
            codex_thread_id,
            codex_bin,
        )?,
        Commands::HostWorker {
            world_id,
            interval_ms,
            once,
            text_backend,
            no_visual_jobs,
            claim_visual_jobs,
            codex_thread_id,
            codex_bin,
            codex_app_server_url,
            node_bin,
        } => handle_host_worker(
            store_root.as_deref(),
            world_id.as_deref(),
            &HostWorkerOptions {
                interval_ms,
                once,
                text_backend,
                no_visual_jobs,
                claim_visual_jobs,
                codex_thread_id,
                codex_bin,
                codex_app_server_url,
                node_bin,
            },
        )?,
        Commands::CodexThreadBind {
            world_id,
            thread_id,
            codex_bin,
            json,
        } => handle_codex_thread_bind(
            store_root.as_deref(),
            world_id.as_deref(),
            thread_id,
            codex_bin,
            json,
        )?,
        Commands::CodexThreadShow { world_id, json } => {
            handle_codex_thread_show(store_root.as_deref(), world_id.as_deref(), json)?;
        }
        Commands::CodexThreadClear { world_id, json } => {
            handle_codex_thread_clear(store_root.as_deref(), world_id.as_deref(), json)?;
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
    let outcome = claim_visual_job(&ClaimVisualJobOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        slot,
        claimed_by,
        force,
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

fn handle_codex_thread_bind(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    thread_id: Option<String>,
    codex_bin: Option<PathBuf>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let thread_id = thread_id
        .filter(|value| !value.trim().is_empty())
        .with_context(|| {
            "missing Codex thread id: pass --thread-id or run from a Codex turn with CODEX_THREAD_ID"
        })?;
    let binding = save_codex_thread_binding(&SaveCodexThreadBindingOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id,
        thread_id,
        codex_bin: codex_bin.map(|path| path.display().to_string()),
        source: "codex_thread_bind_cli".to_owned(),
    })?;
    if json {
        println!("{}", serde_json::to_string_pretty(&binding)?);
    } else {
        println!("world: {}", binding.world_id);
        println!("thread: {}", binding.thread_id);
        if let Some(codex_bin) = &binding.codex_bin {
            println!("codex_bin: {codex_bin}");
        }
        println!("updated_at: {}", binding.updated_at);
    }
    Ok(())
}

fn handle_codex_thread_show(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let binding = load_codex_thread_binding(store_root, world_id.as_str())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": "singulari.codex_thread_binding_status.v1",
                "world_id": world_id,
                "binding": binding,
            }))?
        );
    } else if let Some(binding) = binding {
        println!("world: {}", binding.world_id);
        println!("thread: {}", binding.thread_id);
        if let Some(codex_bin) = &binding.codex_bin {
            println!("codex_bin: {codex_bin}");
        }
        println!("source: {}", binding.source);
        println!("updated_at: {}", binding.updated_at);
    } else {
        println!("world: {world_id}");
        println!("thread: <unbound>");
    }
    Ok(())
}

fn handle_codex_thread_clear(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    json: bool,
) -> Result<()> {
    let world_id = resolve_world_id(store_root, world_id)?;
    let cleared = clear_codex_thread_binding(store_root, world_id.as_str())?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": "singulari.codex_thread_binding_clear.v1",
                "world_id": world_id,
                "cleared": cleared,
            }))?
        );
    } else if let Some(binding) = cleared {
        println!("cleared: {}", binding.thread_id);
    } else {
        println!("thread: <already unbound>");
    }
    Ok(())
}

fn handle_agent_watch(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    interval_ms: u64,
    once: bool,
    no_visual_jobs: bool,
    codex_thread_id: Option<String>,
    codex_bin: Option<PathBuf>,
) -> Result<()> {
    let interval = Duration::from_millis(interval_ms.max(250));
    let mut emitted = HashSet::new();
    let mut dispatch_config = AgentWatchDispatchConfig::from_cli(codex_thread_id, codex_bin);
    loop {
        let world_id = resolve_world_id(store_root, world_id)?;
        let dispatch = dispatch_config.resolve(store_root, world_id.as_str())?;
        let mut emitted_this_tick = false;
        if emit_pending_agent_turn_event(
            store_root,
            world_id.as_str(),
            &mut emitted,
            dispatch.as_ref(),
        )? {
            emitted_this_tick = true;
        }
        if !no_visual_jobs
            && emit_pending_visual_job_events(store_root, world_id.as_str(), &mut emitted)?
        {
            emitted_this_tick = true;
        }
        if once {
            if !emitted_this_tick {
                emit_watch_event(&serde_json::json!({
                    "schema_version": "singulari.agent_watch_event.v1",
                    "event": "idle",
                    "world_id": world_id,
                    "consumer": "codex_app_background_job",
                }))?;
            }
            break;
        }
        thread::sleep(interval);
    }
    Ok(())
}

const HOST_WORKER_EVENT_SCHEMA_VERSION: &str = "singulari.host_worker_event.v1";
const HOST_WORKER_CONSUMER: &str = "codex_app_host_worker";
const CODEX_APP_SERVER_TURN_HELPER: &str = include_str!("codex_app_server_turn.mjs");

#[derive(Debug, Clone)]
struct HostWorkerOptions {
    interval_ms: u64,
    once: bool,
    text_backend: HostTextBackend,
    no_visual_jobs: bool,
    claim_visual_jobs: bool,
    codex_thread_id: Option<String>,
    codex_bin: Option<PathBuf>,
    codex_app_server_url: Option<String>,
    node_bin: Option<PathBuf>,
}

fn handle_host_worker(
    store_root: Option<&Path>,
    world_id: Option<&str>,
    options: &HostWorkerOptions,
) -> Result<()> {
    let interval = Duration::from_millis(options.interval_ms.max(250));
    let mut emitted = HashSet::new();
    let mut exec_dispatch_config = AgentWatchDispatchConfig::from_cli(
        options.codex_thread_id.clone(),
        options.codex_bin.clone(),
    );
    let mut app_server_dispatch_config = CodexAppServerDispatchConfig::from_cli(
        options.codex_app_server_url.clone(),
        options.codex_thread_id.clone(),
        options.node_bin.clone(),
        options.codex_bin.clone(),
    );
    let initial_world_id = resolve_world_id(store_root, world_id)?;
    emit_host_event(&serde_json::json!({
        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
        "event": "worker_started",
        "world_id": initial_world_id,
        "text_backend": options.text_backend.as_str(),
        "visual_jobs": if options.no_visual_jobs {
            "disabled"
        } else if options.claim_visual_jobs {
            "claim_and_emit"
        } else {
            "observe_only"
        },
        "consumer": HOST_WORKER_CONSUMER,
    }))?;

    loop {
        let world_id = resolve_world_id(store_root, world_id)?;
        let exec_dispatch = if options.text_backend == HostTextBackend::CodexExecResume {
            exec_dispatch_config.resolve(store_root, world_id.as_str())?
        } else {
            None
        };
        let app_server_dispatch = if options.text_backend == HostTextBackend::CodexAppServer {
            app_server_dispatch_config.resolve(store_root, world_id.as_str())?
        } else {
            None
        };
        let mut emitted_this_tick = false;
        if emit_host_pending_agent_turn_event(
            store_root,
            world_id.as_str(),
            options.text_backend,
            &mut emitted,
            exec_dispatch.as_ref(),
            app_server_dispatch.as_ref(),
        )? {
            emitted_this_tick = true;
        }
        if !options.no_visual_jobs
            && emit_host_visual_job_events(
                store_root,
                world_id.as_str(),
                options.claim_visual_jobs,
                &mut emitted,
            )?
        {
            emitted_this_tick = true;
        }
        if options.once {
            if !emitted_this_tick {
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "worker_idle",
                    "world_id": world_id,
                    "text_backend": options.text_backend.as_str(),
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
            }
            break;
        }
        thread::sleep(interval);
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "Host worker backend fan-out stays colocated so each JSONL event contract is visible at the CLI boundary"
)]
fn emit_host_pending_agent_turn_event(
    store_root: Option<&Path>,
    world_id: &str,
    text_backend: HostTextBackend,
    emitted: &mut HashSet<String>,
    exec_dispatch: Option<&AgentWatchDispatch>,
    app_server_dispatch: Option<&CodexAppServerDispatch>,
) -> Result<bool> {
    let Ok(pending) = load_pending_agent_turn(store_root, world_id) else {
        return Ok(false);
    };
    match text_backend {
        HostTextBackend::CodexAppPoller => {
            let event_key = format!("codex-app-poller:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&serde_json::json!({
                "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                "event": "codex_app_poller_turn_required",
                "world_id": pending.world_id,
                "turn_id": pending.turn_id,
                "player_input": pending.player_input,
                "pending_ref": pending.pending_ref,
                "consumer_contract": "codex_app_thread_poller",
                "official_host_api": false,
                "status": "poller_action_required",
                "action": "read pending_ref, author AgentTurnResponse in the active Codex App chat session, then commit the turn",
                "command_hint": format!("singulari-world agent-next --world-id {} --json", world_id),
                "consumer": HOST_WORKER_CONSUMER,
            }))?;
            Ok(true)
        }
        HostTextBackend::Manual => {
            let event_key = format!("manual:{}:{}", pending.world_id, pending.turn_id);
            if !emitted.insert(event_key) {
                return Ok(false);
            }
            emit_host_event(&serde_json::json!({
                "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                "event": "manual_agent_turn_required",
                "world_id": pending.world_id,
                "turn_id": pending.turn_id,
                "player_input": pending.player_input,
                "pending_ref": pending.pending_ref,
                "command_hint": format!("singulari-world agent-next --world-id {} --json", world_id),
                "consumer": HOST_WORKER_CONSUMER,
            }))?;
            Ok(true)
        }
        HostTextBackend::CodexExecResume => {
            let Some(dispatch) = exec_dispatch else {
                let event_key = format!(
                    "codex-exec-blocked:{}:{}",
                    pending.world_id, pending.turn_id
                );
                if !emitted.insert(event_key) {
                    return Ok(false);
                }
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "codex_exec_dispatch_blocked",
                    "reason": "missing_codex_thread_binding",
                    "world_id": pending.world_id,
                    "turn_id": pending.turn_id,
                    "pending_ref": pending.pending_ref,
                    "bind_command_hint": format!(
                        "singulari-world codex-thread-bind --world-id {} --thread-id <codex-thread-id> --codex-bin <codex>",
                        world_id
                    ),
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
                return Ok(true);
            };
            match dispatch_pending_agent_turn(store_root, &pending, dispatch)? {
                DispatchOutcome::Started(record) => {
                    let event_key = format!(
                        "codex-exec-started:{}:{}:{}",
                        pending.world_id, pending.turn_id, dispatch.thread_id
                    );
                    if !emitted.insert(event_key) {
                        return Ok(false);
                    }
                    emit_host_event(&serde_json::json!({
                        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                        "event": "codex_exec_dispatch_started",
                        "world_id": pending.world_id,
                        "turn_id": pending.turn_id,
                        "thread_id": dispatch.thread_id.as_str(),
                        "binding_source": dispatch.binding_source.as_str(),
                        "binding_updated_at": dispatch.binding_updated_at.as_str(),
                        "pid": record.pid,
                        "record_path": record.record_path,
                        "prompt_path": record.prompt_path,
                        "stdout_path": record.stdout_path,
                        "stderr_path": record.stderr_path,
                        "consumer": HOST_WORKER_CONSUMER,
                    }))?;
                    Ok(true)
                }
                DispatchOutcome::AlreadyDispatched(record_path) => {
                    let event_key = format!(
                        "codex-exec-skipped:{}:{}:{}",
                        pending.world_id, pending.turn_id, dispatch.thread_id
                    );
                    if !emitted.insert(event_key) {
                        return Ok(false);
                    }
                    emit_host_event(&serde_json::json!({
                        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                        "event": "codex_exec_dispatch_skipped",
                        "reason": "already_dispatched",
                        "world_id": pending.world_id,
                        "turn_id": pending.turn_id,
                        "thread_id": dispatch.thread_id.as_str(),
                        "record_path": record_path,
                        "consumer": HOST_WORKER_CONSUMER,
                    }))?;
                    Ok(true)
                }
            }
        }
        HostTextBackend::CodexAppServer => {
            let Some(dispatch) = app_server_dispatch else {
                let event_key = format!(
                    "codex-app-server-blocked:{}:{}",
                    pending.world_id, pending.turn_id
                );
                if !emitted.insert(event_key) {
                    return Ok(false);
                }
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "codex_app_server_dispatch_blocked",
                    "reason": "missing_codex_app_server_url",
                    "world_id": pending.world_id,
                    "turn_id": pending.turn_id,
                    "pending_ref": pending.pending_ref,
                    "start_command_hint": "codex app-server --listen ws://127.0.0.1:<port>",
                    "env_hint": "SINGULARI_WORLD_CODEX_APP_SERVER_URL=ws://127.0.0.1:<port>",
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
                return Ok(true);
            };
            match dispatch_pending_agent_turn_via_app_server(store_root, &pending, dispatch)? {
                AppServerDispatchOutcome::Started(record) => {
                    let event_key = format!(
                        "codex-app-server-started:{}:{}:{}",
                        pending.world_id,
                        pending.turn_id,
                        record.thread_id.as_deref().unwrap_or("new-thread")
                    );
                    if !emitted.insert(event_key) {
                        return Ok(false);
                    }
                    emit_host_event(&serde_json::json!({
                        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                        "event": "codex_app_server_dispatch_started",
                        "world_id": pending.world_id,
                        "turn_id": pending.turn_id,
                        "thread_id": record.thread_id,
                        "turn_status": record.status,
                        "app_server_url": dispatch.app_server_url.as_str(),
                        "app_server_managed": dispatch.app_server_managed,
                        "app_server_runtime_path": dispatch
                            .app_server_runtime_path
                            .as_ref()
                            .map(|path| path.display().to_string()),
                        "pid": record.pid,
                        "record_path": record.record_path,
                        "prompt_path": record.prompt_path,
                        "result_path": record.result_path,
                        "stdout_path": record.stdout_path,
                        "stderr_path": record.stderr_path,
                        "consumer": HOST_WORKER_CONSUMER,
                    }))?;
                    Ok(true)
                }
                AppServerDispatchOutcome::AlreadyDispatched(record_path) => {
                    let event_key = format!(
                        "codex-app-server-skipped:{}:{}:{}",
                        pending.world_id,
                        pending.turn_id,
                        dispatch.thread_id.as_deref().unwrap_or("new-thread")
                    );
                    if !emitted.insert(event_key) {
                        return Ok(false);
                    }
                    emit_host_event(&serde_json::json!({
                        "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                        "event": "codex_app_server_dispatch_skipped",
                        "reason": "already_dispatched",
                        "world_id": pending.world_id,
                        "turn_id": pending.turn_id,
                        "thread_id": dispatch.thread_id.as_deref(),
                        "app_server_managed": dispatch.app_server_managed,
                        "app_server_runtime_path": dispatch
                            .app_server_runtime_path
                            .as_ref()
                            .map(|path| path.display().to_string()),
                        "record_path": record_path,
                        "consumer": HOST_WORKER_CONSUMER,
                    }))?;
                    Ok(true)
                }
            }
        }
    }
}

fn emit_host_visual_job_events(
    store_root: Option<&Path>,
    world_id: &str,
    claim_visual_jobs: bool,
    emitted: &mut HashSet<String>,
) -> Result<bool> {
    let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id: world_id.to_owned(),
    })?;
    let mut any_emitted = false;
    for job in manifest.image_generation_jobs {
        if load_visual_job_claim(store_root, manifest.world_id.as_str(), job.slot.as_str())?
            .is_some()
        {
            continue;
        }
        if claim_visual_jobs {
            let outcome = claim_visual_job(&ClaimVisualJobOptions {
                store_root: store_root.map(Path::to_path_buf),
                world_id: manifest.world_id.clone(),
                slot: Some(job.slot.clone()),
                claimed_by: "singulari_host_worker".to_owned(),
                force: false,
            })?;
            if let singulari_world::VisualJobClaimOutcome::Claimed { claim } = outcome {
                let complete_command_hint = format!(
                    "singulari-world visual-job-complete --world-id {} --slot {} --claim-id {} --json",
                    claim.world_id, claim.slot, claim.claim_id
                );
                let release_command_hint = format!(
                    "singulari-world visual-job-release --world-id {} --slot {} --json",
                    claim.world_id, claim.slot
                );
                emit_host_event(&serde_json::json!({
                    "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
                    "event": "codex_app_image_generate_required",
                    "world_id": claim.world_id.as_str(),
                    "slot": claim.slot.as_str(),
                    "claim_id": claim.claim_id.as_str(),
                    "claimed_by": claim.claimed_by.as_str(),
                    "claimed_at": claim.claimed_at.as_str(),
                    "tool": claim.job.tool.as_str(),
                    "codex_app_call": &claim.job.codex_app_call,
                    "prompt": claim.job.prompt.as_str(),
                    "reference_paths": &claim.job.reference_paths,
                    "destination_path": claim.job.destination_path.as_str(),
                    "complete_command_hint": complete_command_hint,
                    "release_command_hint": release_command_hint,
                    "consumer": HOST_WORKER_CONSUMER,
                }))?;
                return Ok(true);
            }
            continue;
        }

        let event_key = format!("visual-available:{}:{}", manifest.world_id, job.slot);
        if !emitted.insert(event_key) {
            continue;
        }
        let claim_command_hint = format!(
            "singulari-world visual-job-claim --world-id {} --slot {} --json",
            manifest.world_id, job.slot
        );
        emit_host_event(&serde_json::json!({
            "schema_version": HOST_WORKER_EVENT_SCHEMA_VERSION,
            "event": "visual_job_available",
            "world_id": manifest.world_id.as_str(),
            "slot": job.slot.as_str(),
            "tool": job.tool.as_str(),
            "codex_app_call": &job.codex_app_call,
            "prompt": job.prompt.as_str(),
            "reference_paths": &job.reference_paths,
            "destination_path": job.destination_path.as_str(),
            "claim_command_hint": claim_command_hint,
            "consumer": HOST_WORKER_CONSUMER,
        }))?;
        any_emitted = true;
    }
    Ok(any_emitted)
}

fn emit_host_event(event: &serde_json::Value) -> Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string(event)?)?;
    stdout.flush()?;
    Ok(())
}

fn emit_pending_agent_turn_event(
    store_root: Option<&Path>,
    world_id: &str,
    emitted: &mut HashSet<String>,
    dispatch: Option<&AgentWatchDispatch>,
) -> Result<bool> {
    let Ok(pending) = load_pending_agent_turn(store_root, world_id) else {
        return Ok(false);
    };
    let event_key = format!("agent:{}:{}", pending.world_id, pending.turn_id);
    if !emitted.insert(event_key) {
        return Ok(false);
    }
    emit_watch_event(&serde_json::json!({
        "schema_version": "singulari.agent_watch_event.v1",
        "event": "agent_turn_pending",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "player_input": pending.player_input,
        "pending_ref": pending.pending_ref,
        "command_hint": format!("singulari-world agent-next --world-id {} --json", world_id),
        "consumer": "codex_app_background_job",
    }))?;
    if let Some(dispatch) = dispatch {
        match dispatch_pending_agent_turn(store_root, &pending, dispatch)? {
            DispatchOutcome::Started(record) => emit_watch_event(&serde_json::json!({
                "schema_version": "singulari.agent_watch_event.v1",
                "event": "agent_turn_dispatched",
                "world_id": pending.world_id,
                "turn_id": pending.turn_id,
                "thread_id": dispatch.thread_id.as_str(),
                "binding_source": dispatch.binding_source.as_str(),
                "binding_updated_at": dispatch.binding_updated_at.as_str(),
                "pid": record.pid,
                "record_path": record.record_path,
                "prompt_path": record.prompt_path,
                "stdout_path": record.stdout_path,
                "stderr_path": record.stderr_path,
                "consumer": "codex_app_background_job",
            }))?,
            DispatchOutcome::AlreadyDispatched(record_path) => {
                emit_watch_event(&serde_json::json!({
                    "schema_version": "singulari.agent_watch_event.v1",
                    "event": "agent_turn_dispatch_skipped",
                    "reason": "already_dispatched",
                    "world_id": pending.world_id,
                    "turn_id": pending.turn_id,
                    "thread_id": dispatch.thread_id.as_str(),
                    "binding_source": dispatch.binding_source.as_str(),
                    "binding_updated_at": dispatch.binding_updated_at.as_str(),
                    "record_path": record_path,
                    "consumer": "codex_app_background_job",
                }))?;
            }
        }
    }
    Ok(true)
}

fn emit_pending_visual_job_events(
    store_root: Option<&Path>,
    world_id: &str,
    emitted: &mut HashSet<String>,
) -> Result<bool> {
    let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: store_root.map(Path::to_path_buf),
        world_id: world_id.to_owned(),
    })?;
    let mut any_emitted = false;
    for job in manifest.image_generation_jobs {
        if load_visual_job_claim(store_root, manifest.world_id.as_str(), job.slot.as_str())?
            .is_some()
        {
            continue;
        }
        let event_key = format!("visual:{}:{}", manifest.world_id, job.slot);
        if !emitted.insert(event_key) {
            continue;
        }
        let claim_command_hint = format!(
            "singulari-world visual-job-claim --world-id {} --slot {} --json",
            manifest.world_id, job.slot
        );
        let complete_command_hint = format!(
            "singulari-world visual-job-complete --world-id {} --slot {} --json",
            manifest.world_id, job.slot
        );
        emit_watch_event(&serde_json::json!({
            "schema_version": "singulari.agent_watch_event.v1",
            "event": "visual_job_pending",
            "world_id": manifest.world_id,
            "slot": job.slot,
            "tool": job.tool,
            "codex_app_call": job.codex_app_call,
            "prompt": job.prompt,
            "destination_path": job.destination_path,
            "reference_paths": job.reference_paths,
            "overwrite": job.overwrite,
            "claim_command_hint": claim_command_hint,
            "complete_command_hint": complete_command_hint,
            "consumer": "codex_app_background_job",
        }))?;
        any_emitted = true;
    }
    Ok(any_emitted)
}

fn emit_watch_event(event: &serde_json::Value) -> Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{}", serde_json::to_string(event)?)?;
    stdout.flush()?;
    Ok(())
}

#[derive(Debug, Clone)]
struct AgentWatchDispatch {
    thread_id: String,
    codex_bin: PathBuf,
    binding_source: String,
    binding_updated_at: String,
}

#[derive(Debug, Clone)]
struct AgentWatchDispatchConfig {
    cli_thread_id: Option<String>,
    codex_bin: Option<PathBuf>,
    bound_worlds: HashSet<String>,
}

impl AgentWatchDispatchConfig {
    fn from_cli(thread_id: Option<String>, codex_bin: Option<PathBuf>) -> Self {
        Self {
            cli_thread_id: thread_id.filter(|value| !value.trim().is_empty()),
            codex_bin,
            bound_worlds: HashSet::new(),
        }
    }

    fn resolve(
        &mut self,
        store_root: Option<&Path>,
        world_id: &str,
    ) -> Result<Option<AgentWatchDispatch>> {
        if let Some(thread_id) = &self.cli_thread_id
            && self.bound_worlds.insert(world_id.to_owned())
        {
            save_codex_thread_binding(&SaveCodexThreadBindingOptions {
                store_root: store_root.map(Path::to_path_buf),
                world_id: world_id.to_owned(),
                thread_id: thread_id.clone(),
                codex_bin: self
                    .resolved_codex_bin()
                    .map(|path| path.display().to_string()),
                source: "agent_watch_cli".to_owned(),
            })?;
        }
        let Some(binding) = load_codex_thread_binding(store_root, world_id)? else {
            return Ok(None);
        };
        let codex_bin = binding
            .codex_bin
            .as_ref()
            .map(PathBuf::from)
            .or_else(|| self.resolved_codex_bin())
            .unwrap_or_else(|| PathBuf::from("codex"));
        Ok(Some(AgentWatchDispatch {
            thread_id: binding.thread_id,
            codex_bin,
            binding_source: binding.source,
            binding_updated_at: binding.updated_at,
        }))
    }

    fn resolved_codex_bin(&self) -> Option<PathBuf> {
        self.codex_bin
            .clone()
            .or_else(|| std::env::var_os("HESPERIDES_CODEX_BIN").map(PathBuf::from))
    }
}

#[derive(Debug, Clone)]
struct CodexAppServerDispatch {
    app_server_url: String,
    thread_id: Option<String>,
    node_bin: PathBuf,
    binding_source: Option<String>,
    binding_updated_at: Option<String>,
    app_server_managed: bool,
    app_server_runtime_path: Option<PathBuf>,
}

#[derive(Debug)]
struct CodexAppServerDispatchConfig {
    app_server_url: Option<String>,
    cli_thread_id: Option<String>,
    node_bin: Option<PathBuf>,
    codex_bin: Option<PathBuf>,
    bound_worlds: HashSet<String>,
    managed_app_server: Option<ManagedCodexAppServer>,
}

impl CodexAppServerDispatchConfig {
    fn from_cli(
        app_server_url: Option<String>,
        thread_id: Option<String>,
        node_bin: Option<PathBuf>,
        codex_bin: Option<PathBuf>,
    ) -> Self {
        Self {
            app_server_url: app_server_url
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty()),
            cli_thread_id: thread_id.filter(|value| !value.trim().is_empty()),
            node_bin,
            codex_bin,
            bound_worlds: HashSet::new(),
            managed_app_server: None,
        }
    }

    fn resolve(
        &mut self,
        store_root: Option<&Path>,
        world_id: &str,
    ) -> Result<Option<CodexAppServerDispatch>> {
        let Some(app_server_url) = self.resolved_app_server_url(store_root, world_id)? else {
            return Ok(None);
        };
        let app_server_runtime_path = self
            .managed_app_server
            .as_ref()
            .map(|runtime| runtime.record_path.clone());
        let app_server_managed = app_server_runtime_path.is_some();
        ensure_control_safe_runtime_value("codex_app_server_url", app_server_url.as_str())?;
        if let Some(thread_id) = &self.cli_thread_id
            && self.bound_worlds.insert(world_id.to_owned())
        {
            save_codex_thread_binding(&SaveCodexThreadBindingOptions {
                store_root: store_root.map(Path::to_path_buf),
                world_id: world_id.to_owned(),
                thread_id: thread_id.clone(),
                codex_bin: None,
                source: "codex_app_server_cli".to_owned(),
            })?;
        }
        let binding = load_codex_thread_binding(store_root, world_id)?;
        Ok(Some(CodexAppServerDispatch {
            app_server_url,
            thread_id: binding.as_ref().map(|value| value.thread_id.clone()),
            node_bin: self.resolved_node_bin(),
            binding_source: binding.as_ref().map(|value| value.source.clone()),
            binding_updated_at: binding.map(|value| value.updated_at),
            app_server_managed,
            app_server_runtime_path,
        }))
    }

    fn resolved_app_server_url(
        &mut self,
        store_root: Option<&Path>,
        world_id: &str,
    ) -> Result<Option<String>> {
        if let Some(url) = self.explicit_app_server_url() {
            return Ok(Some(url));
        }
        if let Some(runtime) = self.managed_app_server.as_mut()
            && runtime.is_running()?
        {
            return Ok(Some(runtime.url.clone()));
        }
        self.managed_app_server = None;
        let runtime = spawn_managed_codex_app_server(
            store_root,
            world_id,
            self.resolved_codex_bin().as_path(),
        )?;
        let url = runtime.url.clone();
        self.managed_app_server = Some(runtime);
        Ok(Some(url))
    }

    fn explicit_app_server_url(&self) -> Option<String> {
        self.app_server_url.clone().or_else(|| {
            std::env::var("SINGULARI_WORLD_CODEX_APP_SERVER_URL")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
    }

    fn resolved_node_bin(&self) -> PathBuf {
        self.node_bin
            .clone()
            .or_else(|| std::env::var_os("SINGULARI_WORLD_NODE_BIN").map(PathBuf::from))
            .or_else(|| std::env::var_os("HESPERIDES_NODE_BIN").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("node"))
    }

    fn resolved_codex_bin(&self) -> PathBuf {
        self.codex_bin
            .clone()
            .or_else(|| std::env::var_os("SINGULARI_WORLD_CODEX_BIN").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("codex"))
    }
}

#[derive(Debug)]
struct ManagedCodexAppServer {
    url: String,
    record_path: PathBuf,
    child: Child,
}

impl ManagedCodexAppServer {
    fn is_running(&mut self) -> Result<bool> {
        Ok(self
            .child
            .try_wait()
            .context("failed to inspect managed codex app-server process")?
            .is_none())
    }
}

impl Drop for ManagedCodexAppServer {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        mark_managed_codex_app_server_stopped(self.record_path.as_path());
    }
}

fn spawn_managed_codex_app_server(
    store_root: Option<&Path>,
    world_id: &str,
    codex_bin: &Path,
) -> Result<ManagedCodexAppServer> {
    let bridge_dir = codex_app_server_runtime_dir(store_root, world_id)?;
    fs::create_dir_all(&bridge_dir)
        .with_context(|| format!("failed to create {}", bridge_dir.display()))?;
    let port = reserve_loopback_port()?;
    let url = format!("ws://127.0.0.1:{port}");
    let record_path = bridge_dir.join("codex_app_server_runtime.json");
    let stdout_path = bridge_dir.join("codex_app_server_stdout.log");
    let stderr_path = bridge_dir.join("codex_app_server_stderr.log");
    let stdout = File::create(&stdout_path)
        .with_context(|| format!("failed to create {}", stdout_path.display()))?;
    let stderr = File::create(&stderr_path)
        .with_context(|| format!("failed to create {}", stderr_path.display()))?;
    let mut child = Command::new(codex_bin)
        .arg("app-server")
        .arg("--listen")
        .arg(url.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn managed codex app-server: codex_bin={}",
                codex_bin.display()
            )
        })?;
    wait_for_managed_codex_app_server(port, &mut child, stderr_path.as_path())?;
    let pid = child.id();
    fs::write(
        &record_path,
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "singulari.codex_app_server_runtime.v1",
            "status": "running",
            "world_id": world_id,
            "url": url.as_str(),
            "host": "127.0.0.1",
            "port": port,
            "pid": pid,
            "codex_bin": codex_bin.display().to_string(),
            "stdout_path": stdout_path.display().to_string(),
            "stderr_path": stderr_path.display().to_string(),
            "started_at": Utc::now().to_rfc3339(),
            "owner": HOST_WORKER_CONSUMER,
        }))?,
    )
    .with_context(|| format!("failed to write {}", record_path.display()))?;
    Ok(ManagedCodexAppServer {
        url,
        record_path,
        child,
    })
}

fn codex_app_server_runtime_dir(store_root: Option<&Path>, world_id: &str) -> Result<PathBuf> {
    let paths = resolve_store_paths(store_root)?;
    Ok(paths.worlds_dir.join(world_id).join("agent_bridge"))
}

fn reserve_loopback_port() -> Result<u16> {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).context("failed to reserve loopback port")?;
    Ok(listener
        .local_addr()
        .context("failed to read reserved loopback port")?
        .port())
}

fn wait_for_managed_codex_app_server(
    port: u16,
    child: &mut Child,
    stderr_path: &Path,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let addr = format!("127.0.0.1:{port}");
    while Instant::now() < deadline {
        if let Some(status) = child
            .try_wait()
            .context("failed to inspect managed codex app-server process")?
        {
            anyhow::bail!(
                "managed codex app-server exited before listening: status={}, stderr_path={}",
                status,
                stderr_path.display()
            );
        }
        if TcpStream::connect_timeout(
            &addr
                .parse()
                .context("failed to parse managed app-server socket address")?,
            Duration::from_millis(200),
        )
        .is_ok()
        {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!(
        "managed codex app-server did not start listening: port={}, stderr_path={}",
        port,
        stderr_path.display()
    );
}

fn mark_managed_codex_app_server_stopped(record_path: &Path) {
    let Ok(raw) = fs::read_to_string(record_path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    if let Some(object) = value.as_object_mut() {
        object.insert("status".to_owned(), serde_json::json!("stopped"));
        object.insert(
            "stopped_at".to_owned(),
            serde_json::json!(Utc::now().to_rfc3339()),
        );
        let _ = fs::write(
            record_path,
            serde_json::to_vec_pretty(&value).unwrap_or_else(|_| raw.into_bytes()),
        );
    }
}

fn ensure_control_safe_runtime_value(field: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        anyhow::bail!("{field} contains control characters");
    }
    Ok(())
}

enum DispatchOutcome {
    Started(Box<CodexDispatchRecord>),
    AlreadyDispatched(String),
}

enum AppServerDispatchOutcome {
    Started(Box<CodexAppServerDispatchRecord>),
    AlreadyDispatched(String),
}

#[derive(Debug, Serialize)]
struct CodexDispatchRecord {
    schema_version: &'static str,
    status: &'static str,
    world_id: String,
    turn_id: String,
    thread_id: String,
    binding_source: String,
    binding_updated_at: String,
    codex_bin: String,
    pid: u32,
    record_path: String,
    prompt_path: String,
    response_path: String,
    final_message_path: String,
    stdout_path: String,
    stderr_path: String,
    dispatched_at: String,
    exit_code: Option<i32>,
    completed_at: String,
}

#[derive(Debug, Serialize)]
struct CodexAppServerDispatchRecord {
    schema_version: &'static str,
    status: String,
    world_id: String,
    turn_id: String,
    thread_id: Option<String>,
    app_server_url: String,
    app_server_managed: bool,
    app_server_runtime_path: Option<String>,
    binding_source: Option<String>,
    binding_updated_at: Option<String>,
    node_bin: String,
    pid: u32,
    record_path: String,
    prompt_path: String,
    response_path: String,
    result_path: String,
    helper_path: String,
    stdout_path: String,
    stderr_path: String,
    dispatched_at: String,
    exit_code: Option<i32>,
    binding_clear_reason: Option<&'static str>,
    binding_cleared_at: Option<String>,
    completed_at: String,
}

fn dispatch_pending_agent_turn(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    dispatch: &AgentWatchDispatch,
) -> Result<DispatchOutcome> {
    let dispatch_dir = dispatch_dir_for_pending(pending)?;
    fs::create_dir_all(&dispatch_dir)
        .with_context(|| format!("failed to create {}", dispatch_dir.display()))?;
    let thread_component = safe_file_component(dispatch.thread_id.as_str());
    let record_path = dispatch_dir.join(format!("{}-{}.json", pending.turn_id, thread_component));
    if record_path.exists() {
        return Ok(DispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let prompt_path = dispatch_dir.join(format!("{}-prompt.md", pending.turn_id));
    let response_path = dispatch_dir.join(format!("{}-agent-response.json", pending.turn_id));
    let final_message_path = dispatch_dir.join(format!("{}-final-message.txt", pending.turn_id));
    let stdout_path = dispatch_dir.join(format!("{}-stdout.log", pending.turn_id));
    let stderr_path = dispatch_dir.join(format!("{}-stderr.log", pending.turn_id));
    let prompt = build_codex_realtime_prompt(store_root, pending, response_path.as_path())?;
    fs::write(&prompt_path, prompt.as_bytes())
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;

    let claim = serde_json::json!({
        "schema_version": "singulari.codex_dispatch_record.v1",
        "status": "dispatching",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "thread_id": dispatch.thread_id.as_str(),
        "binding_source": dispatch.binding_source.as_str(),
        "binding_updated_at": dispatch.binding_updated_at.as_str(),
        "codex_bin": dispatch.codex_bin.display().to_string(),
        "prompt_path": prompt_path.display().to_string(),
        "response_path": response_path.display().to_string(),
        "final_message_path": final_message_path.display().to_string(),
        "stdout_path": stdout_path.display().to_string(),
        "stderr_path": stderr_path.display().to_string(),
        "dispatched_at": Utc::now().to_rfc3339(),
    });
    if !write_dispatch_claim(record_path.as_path(), &claim)? {
        return Ok(DispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let stdout = File::create(&stdout_path)
        .with_context(|| format!("failed to create {}", stdout_path.display()))?;
    let stderr = File::create(&stderr_path)
        .with_context(|| format!("failed to create {}", stderr_path.display()))?;
    let mut child = Command::new(&dispatch.codex_bin)
        .arg("exec")
        .arg("resume")
        .arg("--output-last-message")
        .arg(&final_message_path)
        .arg(dispatch.thread_id.as_str())
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .with_context(|| {
            format!(
                "failed to spawn codex realtime dispatch: bin={}",
                dispatch.codex_bin.display()
            )
        })?;
    let pid = child.id();
    let Some(mut stdin) = child.stdin.take() else {
        anyhow::bail!("failed to open codex realtime dispatch stdin");
    };
    stdin
        .write_all(prompt.as_bytes())
        .context("failed to write codex realtime dispatch prompt")?;
    drop(stdin);
    let exit_status = child
        .wait()
        .context("failed to wait for codex realtime dispatch")?;
    let status = if exit_status.success() {
        "completed"
    } else {
        "failed"
    };

    let record = CodexDispatchRecord {
        schema_version: "singulari.codex_dispatch_record.v1",
        status,
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        thread_id: dispatch.thread_id.clone(),
        binding_source: dispatch.binding_source.clone(),
        binding_updated_at: dispatch.binding_updated_at.clone(),
        codex_bin: dispatch.codex_bin.display().to_string(),
        pid,
        record_path: record_path.display().to_string(),
        prompt_path: prompt_path.display().to_string(),
        response_path: response_path.display().to_string(),
        final_message_path: final_message_path.display().to_string(),
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        dispatched_at: Utc::now().to_rfc3339(),
        exit_code: exit_status.code(),
        completed_at: Utc::now().to_rfc3339(),
    };
    fs::write(&record_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", record_path.display()))?;
    Ok(DispatchOutcome::Started(Box::new(record)))
}

#[allow(
    clippy::too_many_lines,
    reason = "App-server dispatch keeps claim, helper execution, and durable record updates together at the process boundary"
)]
fn dispatch_pending_agent_turn_via_app_server(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    dispatch: &CodexAppServerDispatch,
) -> Result<AppServerDispatchOutcome> {
    let dispatch_dir = dispatch_dir_for_pending(pending)?;
    fs::create_dir_all(&dispatch_dir)
        .with_context(|| format!("failed to create {}", dispatch_dir.display()))?;
    let thread_component = dispatch
        .thread_id
        .as_deref()
        .map_or_else(|| "new-thread".to_owned(), safe_file_component);
    let record_path = dispatch_dir.join(format!(
        "{}-appserver-{}.json",
        pending.turn_id, thread_component
    ));
    if record_path.exists() {
        clear_stale_app_server_binding_from_existing_record(
            store_root,
            pending.world_id.as_str(),
            dispatch,
            record_path.as_path(),
        )?;
        return Ok(AppServerDispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let prompt_path = dispatch_dir.join(format!("{}-appserver-prompt.md", pending.turn_id));
    let response_path =
        dispatch_dir.join(format!("{}-appserver-agent-response.json", pending.turn_id));
    let result_path = dispatch_dir.join(format!("{}-appserver-result.json", pending.turn_id));
    let helper_path = dispatch_dir.join("codex_app_server_turn.mjs");
    let stdout_path = dispatch_dir.join(format!("{}-appserver-stdout.log", pending.turn_id));
    let stderr_path = dispatch_dir.join(format!("{}-appserver-stderr.log", pending.turn_id));
    let prompt = build_codex_realtime_prompt(store_root, pending, response_path.as_path())?;
    fs::write(&prompt_path, prompt.as_bytes())
        .with_context(|| format!("failed to write {}", prompt_path.display()))?;
    fs::write(&helper_path, CODEX_APP_SERVER_TURN_HELPER.as_bytes())
        .with_context(|| format!("failed to write {}", helper_path.display()))?;

    let claim = serde_json::json!({
        "schema_version": "singulari.codex_app_server_dispatch_record.v1",
        "status": "dispatching",
        "world_id": pending.world_id,
        "turn_id": pending.turn_id,
        "thread_id": dispatch.thread_id.as_deref(),
        "app_server_url": dispatch.app_server_url.as_str(),
        "app_server_managed": dispatch.app_server_managed,
        "app_server_runtime_path": dispatch
            .app_server_runtime_path
            .as_ref()
            .map(|path| path.display().to_string()),
        "binding_source": dispatch.binding_source.as_deref(),
        "binding_updated_at": dispatch.binding_updated_at.as_deref(),
        "node_bin": dispatch.node_bin.display().to_string(),
        "prompt_path": prompt_path.display().to_string(),
        "response_path": response_path.display().to_string(),
        "result_path": result_path.display().to_string(),
        "helper_path": helper_path.display().to_string(),
        "stdout_path": stdout_path.display().to_string(),
        "stderr_path": stderr_path.display().to_string(),
        "dispatched_at": Utc::now().to_rfc3339(),
    });
    if !write_dispatch_claim(record_path.as_path(), &claim)? {
        return Ok(AppServerDispatchOutcome::AlreadyDispatched(
            record_path.display().to_string(),
        ));
    }

    let stdout = File::create(&stdout_path)
        .with_context(|| format!("failed to create {}", stdout_path.display()))?;
    let stderr = File::create(&stderr_path)
        .with_context(|| format!("failed to create {}", stderr_path.display()))?;
    let cwd = std::env::current_dir().context("failed to resolve current working directory")?;
    let mut command = Command::new(&dispatch.node_bin);
    command
        .arg(&helper_path)
        .arg("--url")
        .arg(dispatch.app_server_url.as_str())
        .arg("--cwd")
        .arg(cwd)
        .arg("--prompt-path")
        .arg(&prompt_path)
        .arg("--result-path")
        .arg(&result_path)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    if let Some(thread_id) = &dispatch.thread_id {
        command.arg("--thread-id").arg(thread_id);
    }
    let mut child = command.spawn().with_context(|| {
        format!(
            "failed to spawn codex app-server dispatch helper: node={}",
            dispatch.node_bin.display()
        )
    })?;
    let pid = child.id();
    let exit_status = child
        .wait()
        .context("failed to wait for codex app-server dispatch helper")?;
    let result = read_json_value_if_present(result_path.as_path())?;
    let result_thread_id = result
        .as_ref()
        .and_then(|value| value.get("thread_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    let result_status = app_server_result_status(result.as_ref());
    let result_completed_without_turn_error =
        exit_status.success() && result_status.is_none_or(|status| status != "failed");
    if result_completed_without_turn_error
        && let Some(thread_id) = &result_thread_id
        && dispatch.thread_id.as_deref() != Some(thread_id.as_str())
    {
        save_codex_thread_binding(&SaveCodexThreadBindingOptions {
            store_root: store_root.map(Path::to_path_buf),
            world_id: pending.world_id.clone(),
            thread_id: thread_id.clone(),
            codex_bin: None,
            source: if dispatch.thread_id.is_some() {
                "codex_app_server_thread_resume".to_owned()
            } else {
                "codex_app_server_thread_start".to_owned()
            },
        })?;
    }
    let binding_clear_reason = app_server_stale_thread_binding_reason(result.as_ref(), dispatch);
    let binding_cleared_at = if binding_clear_reason.is_some() {
        clear_codex_thread_binding(store_root, pending.world_id.as_str())?;
        Some(Utc::now().to_rfc3339())
    } else {
        None
    };
    let pending_still_open = load_pending_agent_turn(store_root, pending.world_id.as_str())
        .is_ok_and(|open| open.turn_id == pending.turn_id);
    let status = if exit_status.success() && !pending_still_open {
        "completed"
    } else if exit_status.success() {
        "failed_uncommitted"
    } else {
        "failed"
    }
    .to_owned();

    let record = CodexAppServerDispatchRecord {
        schema_version: "singulari.codex_app_server_dispatch_record.v1",
        status,
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        thread_id: result_thread_id.or_else(|| dispatch.thread_id.clone()),
        app_server_url: dispatch.app_server_url.clone(),
        app_server_managed: dispatch.app_server_managed,
        app_server_runtime_path: dispatch
            .app_server_runtime_path
            .as_ref()
            .map(|path| path.display().to_string()),
        binding_source: dispatch.binding_source.clone(),
        binding_updated_at: dispatch.binding_updated_at.clone(),
        node_bin: dispatch.node_bin.display().to_string(),
        pid,
        record_path: record_path.display().to_string(),
        prompt_path: prompt_path.display().to_string(),
        response_path: response_path.display().to_string(),
        result_path: result_path.display().to_string(),
        helper_path: helper_path.display().to_string(),
        stdout_path: stdout_path.display().to_string(),
        stderr_path: stderr_path.display().to_string(),
        dispatched_at: Utc::now().to_rfc3339(),
        exit_code: exit_status.code(),
        binding_clear_reason,
        binding_cleared_at,
        completed_at: Utc::now().to_rfc3339(),
    };
    fs::write(&record_path, serde_json::to_vec_pretty(&record)?)
        .with_context(|| format!("failed to update {}", record_path.display()))?;
    Ok(AppServerDispatchOutcome::Started(Box::new(record)))
}

fn clear_stale_app_server_binding_from_existing_record(
    store_root: Option<&Path>,
    world_id: &str,
    dispatch: &CodexAppServerDispatch,
    record_path: &Path,
) -> Result<()> {
    let Some(thread_id) = &dispatch.thread_id else {
        return Ok(());
    };
    let Some(record) = read_json_value_if_present(record_path)? else {
        return Ok(());
    };
    let result_path = record
        .get("result_path")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from);
    let result = match result_path {
        Some(path) => read_json_value_if_present(path.as_path())?,
        None => None,
    };
    if app_server_stale_thread_binding_reason(result.as_ref(), dispatch).is_some() {
        let current = load_codex_thread_binding(store_root, world_id)?;
        if current.as_ref().map(|binding| binding.thread_id.as_str()) == Some(thread_id.as_str()) {
            clear_codex_thread_binding(store_root, world_id)?;
        }
    }
    Ok(())
}

fn app_server_stale_thread_binding_reason(
    result: Option<&serde_json::Value>,
    dispatch: &CodexAppServerDispatch,
) -> Option<&'static str> {
    dispatch.thread_id.as_ref()?;
    let result = result?;
    if app_server_result_status(Some(result)) != Some("failed") {
        return None;
    }
    let failure_stage = result
        .get("failure_stage")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let error_text = app_server_result_error_text(result);
    if app_server_error_is_auth_failure(error_text.as_str()) {
        return None;
    }
    if failure_stage == "thread_resume" {
        return Some("thread_resume_failed");
    }
    if app_server_error_mentions_missing_thread(error_text.as_str()) {
        return Some("thread_not_found");
    }
    None
}

fn app_server_result_status(result: Option<&serde_json::Value>) -> Option<&str> {
    result
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str)
}

fn app_server_result_error_text(result: &serde_json::Value) -> String {
    let mut fragments = Vec::new();
    if let Some(error) = result.get("error") {
        fragments.push(error.to_string());
    }
    if let Some(error) = result.get("turn_error") {
        fragments.push(error.to_string());
    }
    if let Some(errors) = result.get("server_errors") {
        fragments.push(errors.to_string());
    }
    fragments.join("\n").to_ascii_lowercase()
}

fn app_server_error_is_auth_failure(error_text: &str) -> bool {
    error_text.contains("unauthorized")
        || error_text.contains("access token")
        || error_text.contains("sign in")
        || error_text.contains("authentication")
}

fn app_server_error_mentions_missing_thread(error_text: &str) -> bool {
    error_text.contains("thread_not_found")
        || error_text.contains("thread not found")
        || error_text.contains("not found")
        || error_text.contains("no such thread")
        || error_text.contains("unknown thread")
        || error_text.contains("invalid thread")
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

fn build_codex_realtime_prompt(
    store_root: Option<&Path>,
    pending: &singulari_world::PendingAgentTurn,
    response_path: &Path,
) -> Result<String> {
    let binary = current_binary_for_prompt()?;
    let store_args = store_root
        .map(|path| {
            let normalized = normalize_prompt_path(path);
            format!(
                " --store-root {}",
                shell_quote(normalized.display().to_string().as_str())
            )
        })
        .unwrap_or_default();
    let world_arg = shell_quote(pending.world_id.as_str());
    let response_arg = shell_quote(response_path.display().to_string().as_str());
    let binary_arg = shell_quote(binary.display().to_string().as_str());
    let commit_command = format!(
        "{binary_arg}{store_args} agent-commit --world-id {world_arg} --response {response_arg} --json"
    );
    let pending_packet = serde_json::to_string_pretty(pending)
        .context("failed to serialize pending turn for realtime prompt")?;
    Ok(format!(
        r#"Singulari World realtime event가 들어왔어. 이 턴 하나만 처리하고 멈춰.

역할:
- 너는 Singulari World의 trusted narrative agent다.
- 플레이어에게 다시 묻지 말고, 아래 pending turn JSON만 보고 바로 서사 턴을 작성한다.
- hidden/private context는 판정에만 쓰고, visible_scene/canon_event/choice text에는 절대 누출하지 않는다.
- 출력 서사는 한국어 VN prose다. 대화, 제스처, 말버릇을 살리고, 게임식 수치 계산처럼 보이게 쓰지 않는다.
- player_input이 "세계 개막"이면 그것은 선택지가 아니라 시드에서 첫 서사를 여는 bootstrap turn이다. 첫 장소, 첫 감각, 첫 사건의 hook을 visible_scene에 바로 작성한다.
- 이 Codex thread의 이전 대화 맥락은 말맛과 리듬을 위한 working context다. 세계의 사실/상태/source of truth는 아래 pending turn JSON과 world store다.
- thread context가 compact 되었거나 pending packet과 충돌하면 pending packet을 우선한다.
- slot 4는 항상 안내자의 선택이고 preview는 숨긴다: "맡긴다. 세부 내용은 선택 후 드러난다."
- slot 7은 항상 자유서술이며 inline prose를 요구하는 선택지로 둔다.
- 소스 파일을 읽거나 repo를 탐색하지 마라. 필요한 스키마와 pending packet은 이 프롬프트 안에 있다.
- 허용된 외부 명령은 마지막 commit 명령뿐이다. commit이 스키마 에러를 내면 그 에러만 보고 JSON을 고쳐 한 번 더 commit한다.

AgentTurnResponse 스키마:
```json
{{
  "schema_version": "singulari.agent_turn_response.v1",
  "world_id": "{world_id}",
  "turn_id": "{turn_id}",
  "visible_scene": {{
    "schema_version": "singulari.narrative_scene.v1",
    "text_blocks": ["한국어 VN 본문 2-4문단"],
    "tone_notes": ["짧은 톤 메모"]
  }},
  "adjudication": {{
    "outcome": "accepted",
    "summary": "플레이어-visible 한 줄 요약",
    "gates": [
      {{"gate":"body","status":"pass","reason":"..."}},
      {{"gate":"resource","status":"pass","reason":"..."}},
      {{"gate":"time","status":"pass","reason":"..."}},
      {{"gate":"social_permission","status":"pass","reason":"..."}},
      {{"gate":"knowledge","status":"pass","reason":"..."}}
    ],
    "visible_constraints": ["아직 확인되지 않은 플레이어-visible 제약"],
    "consequences": ["이번 턴의 플레이어-visible 결과"]
  }},
  "canon_event": {{
    "visibility": "player_visible",
    "kind": "guided_choice",
    "summary": "플레이어-visible 사건 요약"
  }},
  "entity_updates": [],
  "relationship_updates": [],
  "hidden_state_delta": [],
  "next_choices": [
    {{"slot":1,"tag":"정로","intent":"..."}},
    {{"slot":2,"tag":"관찰","intent":"..."}},
    {{"slot":3,"tag":"관계","intent":"..."}},
    {{"slot":4,"tag":"안내자의 선택","intent":"맡긴다. 세부 내용은 선택 후 드러난다."}},
    {{"slot":5,"tag":"기록","intent":"현재 알려진 세계 기록을 연다"}},
    {{"slot":6,"tag":"흐름","intent":"..."}}
  ]
}}
```

pending turn JSON:
```json
{pending_packet}
```

절차:
1. AgentTurnResponse JSON을 작성해서 아래 경로에 저장해라.
   {response_path}
2. 저장한 JSON을 commit해라.
   {commit_command}
3. 최종 답변은 짧은 한국어 상태 한 줄만 남겨라. AgentTurnResponse JSON 본문을 채팅에 붙이지 마라.

고정값:
- world_id: {world_id}
- turn_id: {turn_id}
- player_input: {player_input}
"#,
        pending_packet = pending_packet,
        response_path = response_path.display(),
        commit_command = commit_command,
        world_id = pending.world_id,
        turn_id = pending.turn_id,
        player_input = pending.player_input,
    ))
}

fn current_binary_for_prompt() -> Result<PathBuf> {
    std::env::current_exe().context("failed to resolve current singulari-world binary")
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

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '-' | '_' | ':'))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', r"'\''"))
    }
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

    fn app_server_dispatch(thread_id: Option<&str>) -> CodexAppServerDispatch {
        CodexAppServerDispatch {
            app_server_url: "ws://127.0.0.1:48713".to_owned(),
            thread_id: thread_id.map(str::to_owned),
            node_bin: PathBuf::from("node"),
            binding_source: Some("test".to_owned()),
            binding_updated_at: Some("2026-04-28T00:00:00Z".to_owned()),
            app_server_managed: false,
            app_server_runtime_path: None,
        }
    }

    #[test]
    fn app_server_resume_failure_marks_bound_thread_stale() {
        let result = serde_json::json!({
            "status": "failed",
            "failure_stage": "thread_resume",
            "error": "thread not found"
        });
        assert_eq!(
            app_server_stale_thread_binding_reason(
                Some(&result),
                &app_server_dispatch(Some("thread-old"))
            ),
            Some("thread_resume_failed")
        );
    }

    #[test]
    fn app_server_auth_failure_keeps_thread_binding() {
        let result = serde_json::json!({
            "status": "failed",
            "failure_stage": "thread_resume",
            "error": "unauthorized: access token could not be refreshed"
        });
        assert_eq!(
            app_server_stale_thread_binding_reason(
                Some(&result),
                &app_server_dispatch(Some("thread-old"))
            ),
            None
        );
    }

    #[test]
    fn app_server_new_thread_failure_has_no_binding_to_clear() {
        let result = serde_json::json!({
            "status": "failed",
            "failure_stage": "thread_start",
            "error": "thread not found"
        });
        assert_eq!(
            app_server_stale_thread_binding_reason(Some(&result), &app_server_dispatch(None)),
            None
        );
    }
}
