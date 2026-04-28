use crate::models::{
    ANCHOR_CHARACTER_ID, CharacterRecord, EntityRecords, OPENING_LOCATION_ID,
    PROTAGONIST_CHARACTER_ID, PlaceRecord, RenderPacket, WorldRecord,
};
use crate::store::{read_json, resolve_store_paths, world_file_paths, write_json};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::path::{Path, PathBuf};

pub const WORLD_VISUAL_ASSETS_SCHEMA_VERSION: &str = "singulari.world_visual_assets.v1";
pub const VISUAL_JOB_CLAIM_SCHEMA_VERSION: &str = "singulari.visual_job_claim.v1";
pub const VISUAL_JOB_CLAIM_RELEASE_SCHEMA_VERSION: &str = "singulari.visual_job_claim_release.v1";
pub const VISUAL_JOB_COMPLETION_SCHEMA_VERSION: &str = "singulari.visual_job_completion.v1";
pub const VISUAL_ASSETS_FILENAME: &str = "visual_assets.json";
pub const VN_ASSETS_DIR: &str = "assets/vn";
pub const MENU_BACKGROUND_FILENAME: &str = "menu_background.png";
pub const STAGE_BACKGROUND_FILENAME: &str = "stage_background.png";
pub const CHARACTER_SHEETS_DIR: &str = "character_sheets";
pub const LOCATION_SHEETS_DIR: &str = "location_sheets";
pub const CODEX_APP_IMAGE_GENERATION_TOOL: &str = "codex_app.image.generate";
const DEFAULT_TURN_CG_CADENCE_MIN: u32 = 5;
const DEFAULT_AUTO_RETRY_LIMIT: u32 = 1;
const DEFAULT_MAX_REFERENCE_IMAGES: u8 = 3;
const TURN_CG_SCENE_HINT_BLOCKS: usize = 2;
const TURN_CG_SCENE_HINT_CHARS: usize = 1_800;
const VISUAL_JOBS_DIR: &str = "visual_jobs";
const VISUAL_JOB_CLAIMS_DIR: &str = "claims";
const VISUAL_JOB_COMPLETIONS_DIR: &str = "completed";
const TURN_CG_SLOT_PREFIX: &str = "turn_cg:";
const TURN_CG_JOBS_DIR: &str = "cg_jobs";
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

#[derive(Debug, Clone)]
pub struct BuildWorldVisualAssetsOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
}

#[derive(Debug, Clone)]
pub struct ClaimVisualJobOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub slot: Option<String>,
    pub claimed_by: String,
    pub force: bool,
    pub extra_jobs: Vec<ImageGenerationJob>,
}

#[derive(Debug, Clone)]
pub struct CompleteVisualJobOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub slot: String,
    pub claim_id: Option<String>,
    pub generated_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct ReleaseVisualJobClaimOptions {
    pub store_root: Option<PathBuf>,
    pub world_id: String,
    pub slot: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldVisualAssets {
    pub schema_version: String,
    pub world_id: String,
    pub style_profile: WorldVisualStyleProfile,
    pub budget_policy: VisualBudgetPolicy,
    pub menu_background: WorldVisualAsset,
    pub stage_background: WorldVisualAsset,
    pub visual_entities: Vec<VisualEntityAsset>,
    pub image_generation_jobs: Vec<ImageGenerationJob>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldVisualStyleProfile {
    pub style_prompt: String,
    pub palette_prompt: String,
    pub camera_language: String,
    pub negative_prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualBudgetPolicy {
    pub mode: String,
    pub turn_cg_cadence_min: u32,
    pub auto_retry_limit: u32,
    pub max_reference_images: u8,
    pub character_sheet_threshold: String,
    pub location_sheet_threshold: String,
    pub force_generate_on: Vec<String>,
}

impl Default for VisualBudgetPolicy {
    fn default() -> Self {
        Self {
            mode: "balanced".to_owned(),
            turn_cg_cadence_min: DEFAULT_TURN_CG_CADENCE_MIN,
            auto_retry_limit: DEFAULT_AUTO_RETRY_LIMIT,
            max_reference_images: DEFAULT_MAX_REFERENCE_IMAGES,
            character_sheet_threshold: "major_or_repeated".to_owned(),
            location_sheet_threshold: "repeated_location".to_owned(),
            force_generate_on: vec![
                "first_major_character_appearance".to_owned(),
                "new_major_location".to_owned(),
                "climax".to_owned(),
                "combat_start".to_owned(),
                "revelation".to_owned(),
                "user_request".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldVisualAsset {
    pub slot: String,
    pub prompt: String,
    pub recommended_path: String,
    pub asset_url: String,
    pub exists: bool,
    pub prompt_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualEntityAsset {
    pub entity_id: String,
    pub entity_type: String,
    pub display_name: String,
    pub slot: String,
    pub prompt: String,
    pub recommended_path: String,
    pub asset_url: String,
    pub exists: bool,
    pub generation_policy: String,
    pub prompt_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageGenerationJob {
    pub tool: String,
    pub codex_app_call: CodexAppImageGenerationCall,
    pub slot: String,
    pub prompt: String,
    pub destination_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_asset_urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_paths: Vec<String>,
    pub overwrite: bool,
    pub register_policy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodexAppImageGenerationCall {
    pub capability: String,
    pub slot: String,
    pub prompt: String,
    pub destination_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_paths: Vec<String>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualJobClaim {
    pub schema_version: String,
    pub world_id: String,
    pub slot: String,
    pub claim_id: String,
    pub claimed_by: String,
    pub claimed_at: String,
    pub job: ImageGenerationJob,
    pub claim_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum VisualJobClaimOutcome {
    Claimed { claim: Box<VisualJobClaim> },
    NoPending { world_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualJobCompletion {
    pub schema_version: String,
    pub world_id: String,
    pub slot: String,
    pub claim_id: Option<String>,
    pub completed_at: String,
    pub destination_path: String,
    pub source_path: Option<String>,
    pub bytes: u64,
    pub completion_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisualJobClaimRelease {
    pub schema_version: String,
    pub world_id: String,
    pub slot: String,
    pub released_at: String,
    pub claim: Option<VisualJobClaim>,
}

#[derive(Debug, Clone)]
pub struct CompiledVisualPrompt {
    pub prompt: String,
    pub reference_asset_urls: Vec<String>,
    pub reference_paths: Vec<String>,
    pub prompt_policy: String,
}

/// Build or refresh the player-visible visual asset manifest for a world.
///
/// # Errors
///
/// Returns an error when the world record or store paths cannot be read, or
/// when the manifest cannot be written.
pub fn build_world_visual_assets(
    options: &BuildWorldVisualAssetsOptions,
) -> Result<WorldVisualAssets> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let world: WorldRecord = read_json(&files.world)?;
    let entities: EntityRecords = read_json(&files.entities)?;
    let assets_dir = files.dir.join(VN_ASSETS_DIR);
    let menu_path = assets_dir.join(MENU_BACKGROUND_FILENAME);
    let stage_path = assets_dir.join(STAGE_BACKGROUND_FILENAME);
    let style_profile = world_style_profile(&world);
    let menu = WorldVisualAsset {
        slot: "menu_background".to_owned(),
        prompt: menu_background_prompt(&world, &style_profile),
        recommended_path: menu_path.display().to_string(),
        asset_url: world_asset_url(world.world_id.as_str(), MENU_BACKGROUND_FILENAME),
        exists: menu_path.is_file(),
        prompt_policy: prompt_policy(),
    };
    let stage = WorldVisualAsset {
        slot: "stage_background".to_owned(),
        prompt: stage_background_prompt(&world, &style_profile),
        recommended_path: stage_path.display().to_string(),
        asset_url: world_asset_url(world.world_id.as_str(), STAGE_BACKGROUND_FILENAME),
        exists: stage_path.is_file(),
        prompt_policy: prompt_policy(),
    };
    let visual_entities = visual_entity_assets(&world, &entities, &assets_dir);
    let mut jobs = [&menu, &stage]
        .into_iter()
        .filter(|asset| !asset.exists)
        .map(image_generation_job)
        .collect::<Vec<_>>();
    jobs.extend(
        visual_entities
            .iter()
            .filter(|asset| !asset.exists && should_queue_visual_entity_asset(asset))
            .map(visual_entity_generation_job),
    );
    let manifest = WorldVisualAssets {
        schema_version: WORLD_VISUAL_ASSETS_SCHEMA_VERSION.to_owned(),
        world_id: world.world_id,
        style_profile,
        budget_policy: VisualBudgetPolicy::default(),
        menu_background: menu,
        stage_background: stage,
        visual_entities,
        image_generation_jobs: jobs,
        updated_at: chrono::Utc::now().to_rfc3339(),
    };
    write_json(&files.dir.join(VISUAL_ASSETS_FILENAME), &manifest)?;
    Ok(manifest)
}

/// Atomically claim one pending visual generation job for a Codex App host worker.
///
/// # Errors
///
/// Returns an error when the world cannot be read, the selected slot is absent,
/// or the claim file cannot be created.
pub fn claim_visual_job(options: &ClaimVisualJobOptions) -> Result<VisualJobClaimOutcome> {
    ensure_nonempty_human_field("claimed_by", options.claimed_by.as_str())?;
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
    })?;
    let jobs = selectable_visual_jobs(
        &manifest,
        options
            .extra_jobs
            .iter()
            .filter(|job| !job.destination_path.is_empty())
            .cloned()
            .collect(),
        options.slot.as_deref(),
    )?;
    for job in jobs {
        let claim_path = visual_job_claim_path(&files.dir, job.slot.as_str());
        if claim_path.exists() && !options.force {
            continue;
        }
        if options.force && claim_path.exists() {
            fs::remove_file(&claim_path)
                .with_context(|| format!("failed to remove {}", claim_path.display()))?;
        }
        let claim = VisualJobClaim {
            schema_version: VISUAL_JOB_CLAIM_SCHEMA_VERSION.to_owned(),
            world_id: options.world_id.clone(),
            slot: job.slot.clone(),
            claim_id: visual_job_claim_id(job.slot.as_str()),
            claimed_by: options.claimed_by.trim().to_owned(),
            claimed_at: chrono::Utc::now().to_rfc3339(),
            job,
            claim_path: claim_path.display().to_string(),
        };
        if write_visual_job_claim_atomically(&claim_path, &claim)? {
            return Ok(VisualJobClaimOutcome::Claimed {
                claim: Box::new(claim),
            });
        }
    }
    Ok(VisualJobClaimOutcome::NoPending {
        world_id: options.world_id.clone(),
    })
}

/// Complete a visual generation job after the Codex App host saves the asset.
///
/// # Errors
///
/// Returns an error when the slot is unknown, the generated file is missing or
/// not a PNG, the claim id does not match, or completion metadata cannot be
/// written.
pub fn complete_visual_job(options: &CompleteVisualJobOptions) -> Result<VisualJobCompletion> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let claim_path = visual_job_claim_path(&files.dir, options.slot.as_str());
    let claim = load_visual_job_claim_at(&claim_path)?;
    if let Some(expected) = options.claim_id.as_deref() {
        let Some(claim) = claim.as_ref() else {
            bail!(
                "visual job claim missing for completion: slot={}, expected_claim_id={expected}",
                options.slot
            );
        };
        if claim.claim_id != expected {
            bail!(
                "visual job claim mismatch: slot={}, expected_claim_id={}, actual_claim_id={}",
                options.slot,
                expected,
                claim.claim_id
            );
        }
    }

    let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
    })?;
    let job = claim
        .as_ref()
        .map(|claim| claim.job.clone())
        .or_else(|| visual_job_for_slot(&manifest, options.slot.as_str()))
        .with_context(|| format!("unknown visual job slot: {}", options.slot))?;
    let destination = PathBuf::from(job.destination_path.as_str());
    if let Some(source) = options.generated_path.as_ref() {
        copy_generated_asset(source, &destination)?;
    }
    validate_generated_png(destination.as_path())?;
    let bytes = fs::metadata(&destination)
        .with_context(|| format!("failed to stat {}", destination.display()))?
        .len();
    let completion_path = visual_job_completion_path(&files.dir, options.slot.as_str());
    let completion = VisualJobCompletion {
        schema_version: VISUAL_JOB_COMPLETION_SCHEMA_VERSION.to_owned(),
        world_id: options.world_id.clone(),
        slot: options.slot.clone(),
        claim_id: claim.as_ref().map(|claim| claim.claim_id.clone()),
        completed_at: chrono::Utc::now().to_rfc3339(),
        destination_path: destination.display().to_string(),
        source_path: options
            .generated_path
            .as_ref()
            .map(|path| path.display().to_string()),
        bytes,
        completion_path: completion_path.display().to_string(),
    };
    ensure_parent_dir(&completion_path)?;
    write_json(&completion_path, &completion)?;
    if claim_path.exists() {
        fs::remove_file(&claim_path)
            .with_context(|| format!("failed to remove {}", claim_path.display()))?;
    }
    clear_turn_cg_retry_marker(&files.dir, options.slot.as_str())?;
    let _refreshed_manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
        store_root: options.store_root.clone(),
        world_id: options.world_id.clone(),
    })?;
    Ok(completion)
}

fn clear_turn_cg_retry_marker(world_dir: &Path, slot: &str) -> Result<()> {
    let Some(turn_id) = slot.strip_prefix(TURN_CG_SLOT_PREFIX) else {
        return Ok(());
    };
    let retry_path = world_dir
        .join(VN_ASSETS_DIR)
        .join(TURN_CG_JOBS_DIR)
        .join(format!("{turn_id}_retry.json"));
    if retry_path.exists() {
        fs::remove_file(&retry_path)
            .with_context(|| format!("failed to remove {}", retry_path.display()))?;
    }
    Ok(())
}

/// Release an active visual job claim without accepting an asset.
///
/// # Errors
///
/// Returns an error when the claim exists but cannot be read or removed.
pub fn release_visual_job_claim(
    options: &ReleaseVisualJobClaimOptions,
) -> Result<VisualJobClaimRelease> {
    let store_paths = resolve_store_paths(options.store_root.as_deref())?;
    let files = world_file_paths(&store_paths, options.world_id.as_str());
    let claim_path = visual_job_claim_path(&files.dir, options.slot.as_str());
    let claim = load_visual_job_claim_at(&claim_path)?;
    if claim.is_some() {
        fs::remove_file(&claim_path)
            .with_context(|| format!("failed to remove {}", claim_path.display()))?;
    }
    Ok(VisualJobClaimRelease {
        schema_version: VISUAL_JOB_CLAIM_RELEASE_SCHEMA_VERSION.to_owned(),
        world_id: options.world_id.clone(),
        slot: options.slot.clone(),
        released_at: chrono::Utc::now().to_rfc3339(),
        claim,
    })
}

/// Return the active claim for a visual slot, if one exists.
///
/// # Errors
///
/// Returns an error when the claim exists but is malformed.
pub fn load_visual_job_claim(
    store_root: Option<&Path>,
    world_id: &str,
    slot: &str,
) -> Result<Option<VisualJobClaim>> {
    let store_paths = resolve_store_paths(store_root)?;
    let files = world_file_paths(&store_paths, world_id);
    load_visual_job_claim_at(&visual_job_claim_path(&files.dir, slot))
}

fn image_generation_job(asset: &WorldVisualAsset) -> ImageGenerationJob {
    visual_generation_job(
        asset.slot.clone(),
        asset.prompt.clone(),
        asset.recommended_path.clone(),
        Vec::new(),
        Vec::new(),
        "save exactly to destination_path; the VN server auto-detects the file on the next packet refresh",
    )
}

fn visual_entity_generation_job(asset: &VisualEntityAsset) -> ImageGenerationJob {
    visual_generation_job(
        asset.slot.clone(),
        asset.prompt.clone(),
        asset.recommended_path.clone(),
        Vec::new(),
        Vec::new(),
        "save exactly to destination_path; future turn CG jobs may use this sheet as a reference",
    )
}

#[must_use]
pub fn visual_generation_job(
    slot: String,
    prompt: String,
    destination_path: String,
    reference_asset_urls: Vec<String>,
    reference_paths: Vec<String>,
    register_policy: &str,
) -> ImageGenerationJob {
    ImageGenerationJob {
        tool: CODEX_APP_IMAGE_GENERATION_TOOL.to_owned(),
        codex_app_call: CodexAppImageGenerationCall {
            capability: "image_generation".to_owned(),
            slot: slot.clone(),
            prompt: prompt.clone(),
            destination_path: destination_path.clone(),
            reference_paths: reference_paths.clone(),
            overwrite: false,
        },
        slot,
        prompt,
        destination_path,
        reference_asset_urls,
        reference_paths,
        overwrite: false,
        register_policy: register_policy.to_owned(),
    }
}

fn selectable_visual_jobs(
    manifest: &WorldVisualAssets,
    extra_jobs: Vec<ImageGenerationJob>,
    slot: Option<&str>,
) -> Result<Vec<ImageGenerationJob>> {
    let mut jobs = manifest.image_generation_jobs.clone();
    for job in extra_jobs {
        if jobs.iter().any(|candidate| candidate.slot == job.slot) {
            continue;
        }
        jobs.push(job);
    }
    if let Some(slot) = slot {
        return jobs
            .iter()
            .find(|job| job.slot == slot)
            .cloned()
            .map(|job| vec![job])
            .with_context(|| format!("no pending visual job for slot: {slot}"));
    }
    Ok(jobs)
}

fn visual_job_for_slot(manifest: &WorldVisualAssets, slot: &str) -> Option<ImageGenerationJob> {
    if manifest.menu_background.slot == slot {
        return Some(image_generation_job(&manifest.menu_background));
    }
    if manifest.stage_background.slot == slot {
        return Some(image_generation_job(&manifest.stage_background));
    }
    manifest
        .visual_entities
        .iter()
        .find(|asset| asset.slot == slot)
        .map(visual_entity_generation_job)
}

fn write_visual_job_claim_atomically(path: &Path, claim: &VisualJobClaim) -> Result<bool> {
    ensure_parent_dir(path)?;
    let body = serde_json::to_string_pretty(claim)
        .with_context(|| format!("failed to serialize {}", path.display()))?;
    let file = OpenOptions::new().write(true).create_new(true).open(path);
    let mut file = match file {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("failed to create visual job claim {}", path.display()));
        }
    };
    writeln!(file, "{body}").with_context(|| format!("failed to write {}", path.display()))?;
    Ok(true)
}

fn load_visual_job_claim_at(path: &Path) -> Result<Option<VisualJobClaim>> {
    if !path.exists() {
        return Ok(None);
    }
    read_json(path).map(Some)
}

fn copy_generated_asset(source: &Path, destination: &Path) -> Result<()> {
    validate_generated_png(source)?;
    ensure_parent_dir(destination)?;
    if destination.exists() {
        let destination_metadata = fs::symlink_metadata(destination)
            .with_context(|| format!("failed to inspect {}", destination.display()))?;
        if destination_metadata.file_type().is_symlink() {
            bail!(
                "visual asset destination rejected symlink: {}",
                destination.display()
            );
        }
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "failed to copy generated visual asset: source={}, destination={}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn validate_generated_png(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("generated visual asset not found: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "generated visual asset rejected symlink: {}",
            path.display()
        );
    }
    if !metadata.is_file() {
        bail!("generated visual asset is not a file: {}", path.display());
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    if !bytes.starts_with(PNG_SIGNATURE) {
        bail!("generated visual asset is not a PNG: {}", path.display());
    }
    Ok(())
}

fn visual_job_claim_path(world_dir: &Path, slot: &str) -> PathBuf {
    world_dir
        .join(VISUAL_JOBS_DIR)
        .join(VISUAL_JOB_CLAIMS_DIR)
        .join(format!("{}.json", visual_job_slot_file_stem(slot)))
}

fn visual_job_completion_path(world_dir: &Path, slot: &str) -> PathBuf {
    world_dir
        .join(VISUAL_JOBS_DIR)
        .join(VISUAL_JOB_COMPLETIONS_DIR)
        .join(format!("{}.json", visual_job_slot_file_stem(slot)))
}

fn visual_job_slot_file_stem(slot: &str) -> String {
    let stem = slot
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if stem.is_empty() {
        "unknown_slot".to_owned()
    } else {
        stem
    }
}

fn visual_job_claim_id(slot: &str) -> String {
    format!(
        "vjc_{}_{}",
        chrono::Utc::now().timestamp_micros(),
        visual_job_slot_file_stem(slot)
    )
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn ensure_nonempty_human_field(field: &str, value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() {
        bail!("visual job {field} must not be empty");
    }
    if value.chars().any(char::is_control) {
        bail!("visual job {field} must not contain control characters");
    }
    Ok(())
}

fn world_asset_url(world_id: &str, filename: &str) -> String {
    format!("/world-assets/{world_id}/{VN_ASSETS_DIR}/{filename}")
}

fn visual_entity_asset_url(world_id: &str, dir: &str, entity_id: &str) -> String {
    format!(
        "/world-assets/{world_id}/{VN_ASSETS_DIR}/{dir}/{}.png",
        asset_file_stem(entity_id)
    )
}

fn menu_background_prompt(world: &WorldRecord, style: &WorldVisualStyleProfile) -> String {
    format!(
        "Create a raster background image for the main menu of the visual novel world \"{}\". Use only player-visible premise details: genre \"{}\", protagonist premise \"{}\", opening state \"{}\". Style: {} Palette: {} Camera/material language: {} Negative prompt: {}. The image should establish this world's mood for a menu screen, with enough calm negative space for overlaid HTML buttons and Korean UI text. Do not render any text. Do not include spoilers or unrevealed symbols.",
        world.title,
        world.premise.genre,
        world.premise.protagonist,
        world.premise.opening_state,
        style.style_prompt,
        style.palette_prompt,
        style.camera_language,
        style.negative_prompt
    )
}

fn stage_background_prompt(world: &WorldRecord, style: &WorldVisualStyleProfile) -> String {
    format!(
        "Create a raster base background layer for the visual novel stage of the world \"{}\". Use only player-visible premise details: genre \"{}\", protagonist premise \"{}\", opening state \"{}\". Style: {} Palette: {} Camera/material language: {} Negative prompt: {}. This is the deepest background layer behind turn CG and text UI, so it should be atmospheric and readable under translucent overlays. Do not render any text. Do not include spoilers or unrevealed symbols.",
        world.title,
        world.premise.genre,
        world.premise.protagonist,
        world.premise.opening_state,
        style.style_prompt,
        style.palette_prompt,
        style.camera_language,
        style.negative_prompt
    )
}

fn prompt_policy() -> String {
    "visual prompts use world record player-visible premise only; hidden_state is never read"
        .to_owned()
}

fn world_style_profile(world: &WorldRecord) -> WorldVisualStyleProfile {
    WorldVisualStyleProfile {
        style_prompt: format!(
            "consistent visual novel key art for {}, grounded in {}",
            world.title, world.premise.genre
        ),
        palette_prompt:
            "world-specific palette extracted from accepted backgrounds when available; restrained neutral palette before assets exist"
                .to_owned(),
        camera_language:
            "cinematic 3/4 compositions, readable silhouettes, stable character proportions"
                .to_owned(),
        negative_prompt: "no rendered UI text, no spoilers, no unrevealed symbols".to_owned(),
    }
}

fn visual_entity_assets(
    world: &WorldRecord,
    entities: &EntityRecords,
    assets_dir: &std::path::Path,
) -> Vec<VisualEntityAsset> {
    let mut assets = Vec::new();
    assets.extend(
        entities
            .characters
            .iter()
            .filter(|character| character_is_visual_candidate(character))
            .map(|character| character_sheet_asset(world, character, assets_dir)),
    );
    assets.extend(
        entities
            .places
            .iter()
            .filter(|place| place_is_visual_candidate(place))
            .map(|place| location_sheet_asset(world, place, assets_dir)),
    );
    assets
}

fn character_is_visual_candidate(character: &CharacterRecord) -> bool {
    if character.id == PROTAGONIST_CHARACTER_ID {
        return true;
    }
    if character.id == ANCHOR_CHARACTER_ID {
        return character.knowledge_state == "known" || character.knowledge_state == "visible";
    }
    character.knowledge_state != "hidden" && character.knowledge_state != "veiled"
}

fn place_is_visual_candidate(place: &PlaceRecord) -> bool {
    place.known_to_protagonist && (place.id == OPENING_LOCATION_ID || place.name != "미정")
}

fn character_sheet_asset(
    world: &WorldRecord,
    character: &CharacterRecord,
    assets_dir: &std::path::Path,
) -> VisualEntityAsset {
    let file_stem = asset_file_stem(character.id.as_str());
    let path = assets_dir
        .join(CHARACTER_SHEETS_DIR)
        .join(format!("{file_stem}.png"));
    let display_name = character_display_name(character);
    VisualEntityAsset {
        entity_id: character.id.clone(),
        entity_type: "character".to_owned(),
        display_name: display_name.clone(),
        slot: format!("character_sheet:{}", character.id),
        prompt: character_sheet_prompt(world, character, display_name.as_str()),
        recommended_path: path.display().to_string(),
        asset_url: visual_entity_asset_url(
            world.world_id.as_str(),
            CHARACTER_SHEETS_DIR,
            character.id.as_str(),
        ),
        exists: path.is_file(),
        generation_policy:
            "queue when major_or_repeated; protagonist and anchor are major by default".to_owned(),
        prompt_policy: prompt_policy(),
    }
}

fn location_sheet_asset(
    world: &WorldRecord,
    place: &PlaceRecord,
    assets_dir: &std::path::Path,
) -> VisualEntityAsset {
    let file_stem = asset_file_stem(place.id.as_str());
    let path = assets_dir
        .join(LOCATION_SHEETS_DIR)
        .join(format!("{file_stem}.png"));
    let display_name = if place.name == "미정" {
        "opening location".to_owned()
    } else {
        place.name.clone()
    };
    VisualEntityAsset {
        entity_id: place.id.clone(),
        entity_type: "location".to_owned(),
        display_name: display_name.clone(),
        slot: format!("location_sheet:{}", place.id),
        prompt: location_sheet_prompt(world, place, display_name.as_str()),
        recommended_path: path.display().to_string(),
        asset_url: visual_entity_asset_url(
            world.world_id.as_str(),
            LOCATION_SHEETS_DIR,
            place.id.as_str(),
        ),
        exists: path.is_file(),
        generation_policy: "queue when repeated_location or first major location reveal".to_owned(),
        prompt_policy: prompt_policy(),
    }
}

fn should_queue_visual_entity_asset(asset: &VisualEntityAsset) -> bool {
    if asset.entity_type == "character" {
        return asset.entity_id == PROTAGONIST_CHARACTER_ID
            || asset.entity_id == ANCHOR_CHARACTER_ID;
    }
    asset.entity_type == "location" && asset.display_name != "opening location"
}

fn character_display_name(character: &CharacterRecord) -> String {
    if character.name.visible != "미정" {
        return character.name.visible.clone();
    }
    if character.id == PROTAGONIST_CHARACTER_ID {
        return "주인공".to_owned();
    }
    character.role.clone()
}

fn character_sheet_prompt(
    world: &WorldRecord,
    character: &CharacterRecord,
    display_name: &str,
) -> String {
    format!(
        "Create a character sheet reference image for \"{}\" in the visual novel world \"{}\". Role: {}. Player-visible confirmed traits: {}. Voice/gesture anchors: {}. Show front view, 3/4 view, neutral expression, two emotional expression busts, and core outfit silhouette on a clean neutral background. Keep proportions and color anchors stable for future scene CG. Do not render text. Do not include spoilers or unrevealed symbols.",
        display_name,
        world.title,
        player_visible_character_role(character),
        list_or(&player_visible_character_traits(character), "none"),
        list_or(&player_visible_gestures(character), "none")
    )
}

fn location_sheet_prompt(world: &WorldRecord, place: &PlaceRecord, display_name: &str) -> String {
    format!(
        "Create a location sheet reference image for \"{}\" in the visual novel world \"{}\". Genre: {}. Player-visible notes: {}. Show stable architecture, key landmarks, color palette, daylight lighting, and a wide establishing view. Do not render text. Do not reveal hidden routes, secret symbols, future damage, or unrevealed faction marks.",
        display_name,
        world.title,
        world.premise.genre,
        list_or(&place.notes, "none")
    )
}

#[must_use]
pub fn compile_turn_visual_prompt(
    world: &WorldRecord,
    packet: &RenderPacket,
    manifest: &WorldVisualAssets,
) -> CompiledVisualPrompt {
    let references = select_reference_assets(packet, manifest);
    let unresolved_major_character_directive =
        unresolved_major_character_design_directive(manifest);
    let reference_hint = if references.is_empty() {
        "No accepted reference sheets are available yet.".to_owned()
    } else {
        references
            .iter()
            .map(|asset| {
                format!(
                    "{} {} -> {}",
                    asset.entity_type, asset.display_name, asset.recommended_path
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    CompiledVisualPrompt {
        prompt: turn_scene_prompt(
            world,
            packet,
            manifest,
            reference_hint.as_str(),
            unresolved_major_character_directive.as_deref(),
        ),
        reference_asset_urls: references
            .iter()
            .map(|asset| asset.asset_url.clone())
            .collect(),
        reference_paths: references
            .iter()
            .map(|asset| asset.recommended_path.clone())
            .collect(),
        prompt_policy:
            "compiled from player-visible render packet, world style profile, and accepted visual entity sheets only; unresolved major character designs cannot be directly depicted in turn CG"
                .to_owned(),
    }
}

fn select_reference_assets<'a>(
    packet: &RenderPacket,
    manifest: &'a WorldVisualAssets,
) -> Vec<&'a VisualEntityAsset> {
    let max_refs = usize::from(manifest.budget_policy.max_reference_images.max(1));
    let mut refs = Vec::new();
    refs.extend(
        manifest
            .visual_entities
            .iter()
            .filter(|asset| asset.exists && asset.entity_type == "character")
            .take(max_refs),
    );
    if refs.len() < max_refs {
        refs.extend(
            manifest
                .visual_entities
                .iter()
                .filter(|asset| {
                    asset.exists
                        && asset.entity_type == "location"
                        && packet
                            .visible_state
                            .dashboard
                            .location
                            .contains(asset.entity_id.as_str())
                })
                .take(max_refs - refs.len()),
        );
    }
    refs
}

fn turn_scene_prompt(
    world: &WorldRecord,
    packet: &RenderPacket,
    manifest: &WorldVisualAssets,
    reference_hint: &str,
    unresolved_major_character_directive: Option<&str>,
) -> String {
    let dashboard = &packet.visible_state.dashboard;
    let scene_text = turn_cg_scene_hint(packet).unwrap_or_else(|| dashboard.status.clone());
    let scan_targets = packet
        .visible_state
        .scan_targets
        .iter()
        .take(4)
        .map(player_visible_focus_label)
        .collect::<Vec<_>>()
        .join(", ");
    let major_character_policy = unresolved_major_character_directive.unwrap_or(
        "All major character designs needed for this scene have accepted reference sheets; keep direct character depictions faithful to those references.",
    );
    format!(
        "Visualize one full-screen visual novel scene CG for the current turn. Use only player-visible information. World: {}. Turn: {}. Location: {}. Current event: {}. Scene narrative: {}. Visible focus: {}. World style: {} Palette: {} Camera/material language: {} Major character design gate: {} Reference assets, if attached by the runtime, have priority over prose: {}. No rendered text. Do not include spoilers or unrevealed symbols.",
        world.title,
        packet.turn_id,
        dashboard.location,
        dashboard.current_event,
        scene_text,
        scan_targets,
        manifest.style_profile.style_prompt,
        manifest.style_profile.palette_prompt,
        manifest.style_profile.camera_language,
        major_character_policy,
        reference_hint
    )
}

fn unresolved_major_character_design_directive(manifest: &WorldVisualAssets) -> Option<String> {
    let unresolved = manifest
        .visual_entities
        .iter()
        .filter(|asset| unresolved_major_character_design(asset))
        .map(|asset| asset.display_name.clone())
        .collect::<Vec<_>>();
    if unresolved.is_empty() {
        return None;
    }
    Some(format!(
        "Do not directly depict unresolved major characters before their character-sheet design is accepted: {}. Avoid faces, full-body front views, distinctive outfits, clear silhouettes, and identifiable close-ups for those characters. Use POV framing, environment-only composition, off-screen presence, shadows cast on scenery, cropped non-identifying hands/feet, or over-the-shoulder ambiguity instead.",
        unresolved.join(", ")
    ))
}

fn unresolved_major_character_design(asset: &VisualEntityAsset) -> bool {
    asset.entity_type == "character" && !asset.exists && major_character_asset(asset)
}

fn major_character_asset(asset: &VisualEntityAsset) -> bool {
    asset.entity_id == PROTAGONIST_CHARACTER_ID || asset.entity_id == ANCHOR_CHARACTER_ID
}

fn turn_cg_scene_hint(packet: &RenderPacket) -> Option<String> {
    let scene = packet.narrative_scene.as_ref()?;
    let text = scene
        .text_blocks
        .iter()
        .rev()
        .take(TURN_CG_SCENE_HINT_BLOCKS)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(truncate_prompt_chars(trimmed, TURN_CG_SCENE_HINT_CHARS))
    }
}

fn truncate_prompt_chars(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated} ...")
    } else {
        truncated
    }
}

fn player_visible_focus_label(target: &crate::models::ScanTarget) -> String {
    if leaks_internal_anchor_or_hidden_text([target.target.as_str(), target.class.as_str()]) {
        return "미확정 단서 (unknown)".to_owned();
    }
    format!("{} ({})", target.target, target.class)
}

fn player_visible_character_role(character: &CharacterRecord) -> String {
    if character.id == PROTAGONIST_CHARACTER_ID {
        return "주인공".to_owned();
    }
    if leaks_internal_anchor_or_hidden_text([character.role.as_str()]) {
        return "핵심 인물".to_owned();
    }
    character.role.clone()
}

fn player_visible_character_traits(character: &CharacterRecord) -> Vec<String> {
    character
        .traits
        .confirmed
        .iter()
        .filter(|trait_text| !leaks_internal_anchor_or_hidden_text([trait_text.as_str()]))
        .cloned()
        .collect()
}

fn player_visible_gestures(character: &CharacterRecord) -> Vec<String> {
    character
        .voice_anchor
        .gestures
        .iter()
        .filter(|gesture| !leaks_internal_anchor_or_hidden_text([gesture.as_str()]))
        .cloned()
        .collect()
}

fn leaks_internal_anchor_or_hidden_text<'a>(parts: impl IntoIterator<Item = &'a str>) -> bool {
    parts.into_iter().any(|part| {
        [
            "숨겨진 진실",
            "숨겨져",
            "hidden",
            "secret",
            "anchor_character",
            "앵커 인물",
            "시드가 정한",
            "정체와 역할",
            "seed-defined",
        ]
        .iter()
        .any(|needle| part.contains(needle))
    })
}

fn list_or(values: &[String], empty_text: &str) -> String {
    if values.is_empty() {
        empty_text.to_owned()
    } else {
        values.join(" / ")
    }
}

fn asset_file_stem(entity_id: &str) -> String {
    entity_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        BuildWorldVisualAssetsOptions, CHARACTER_SHEETS_DIR, CODEX_APP_IMAGE_GENERATION_TOOL,
        ClaimVisualJobOptions, CompleteVisualJobOptions, ReleaseVisualJobClaimOptions,
        VN_ASSETS_DIR, VisualJobClaimOutcome, build_world_visual_assets, claim_visual_job,
        compile_turn_visual_prompt, complete_visual_job, release_visual_job_claim,
        turn_cg_scene_hint, visual_generation_job,
    };
    use crate::models::{
        DashboardSummary, NarrativeScene, RenderPacket, ScanTarget, TurnChoice, VisibleState,
    };
    use crate::store::{InitWorldOptions, init_world};
    use tempfile::tempdir;

    const MINIMAL_PNG: &[u8] = b"\x89PNG\r\n\x1a\nminimal-test-png";

    #[test]
    fn visual_assets_manifest_creates_pending_image_generation_jobs() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_visual_assets
title: "빛 없는 첫 메뉴"
premise:
  genre: "서정 판타지"
  protagonist: "현대인의 전생, 남자 주인공"
  opening_state: "아직 아무 장면도 정해지지 않은 문턱"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
            store_root: Some(store),
            world_id: "stw_visual_assets".to_owned(),
        })?;
        assert_eq!(manifest.budget_policy.mode, "balanced");
        assert_eq!(manifest.budget_policy.turn_cg_cadence_min, 5);
        assert_eq!(manifest.image_generation_jobs.len(), 3);
        assert!(
            manifest
                .image_generation_jobs
                .iter()
                .any(|job| job.slot == "character_sheet:char:protagonist")
        );
        assert_eq!(manifest.visual_entities.len(), 2);
        for job in &manifest.image_generation_jobs {
            assert_eq!(job.tool, CODEX_APP_IMAGE_GENERATION_TOOL);
            assert_eq!(job.codex_app_call.capability, "image_generation");
            assert_eq!(job.codex_app_call.slot, job.slot);
            assert_eq!(job.codex_app_call.prompt, job.prompt);
            assert_eq!(job.codex_app_call.destination_path, job.destination_path);
            for hidden_marker in [
                "앵커 인물",
                "anchor_character",
                "시드가 정한",
                "seed-defined",
            ] {
                assert!(
                    !job.prompt.contains(hidden_marker),
                    "visual job leaked marker: {hidden_marker}"
                );
            }
        }
        assert!(manifest.menu_background.prompt.contains("빛 없는 첫 메뉴"));
        assert!(
            manifest
                .menu_background
                .asset_url
                .ends_with("menu_background.png")
        );
        assert!(!manifest.menu_background.exists);
        Ok(())
    }

    #[test]
    fn visual_job_claim_and_complete_updates_manifest() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_visual_claim
title: "이미지 클레임"
premise:
  genre: "해무 낀 항구 판타지"
  protagonist: "현대인의 전생, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let claimed = claim_visual_job(&ClaimVisualJobOptions {
            store_root: Some(store.clone()),
            world_id: "stw_visual_claim".to_owned(),
            slot: Some("menu_background".to_owned()),
            claimed_by: "test-worker".to_owned(),
            force: false,
            extra_jobs: Vec::new(),
        })?;
        let VisualJobClaimOutcome::Claimed { claim } = claimed else {
            anyhow::bail!("menu background should be claimable");
        };
        assert_eq!(claim.slot, "menu_background");
        assert_eq!(claim.claimed_by, "test-worker");

        let source = temp.path().join("generated.png");
        std::fs::write(&source, MINIMAL_PNG)?;
        let completion = complete_visual_job(&CompleteVisualJobOptions {
            store_root: Some(store.clone()),
            world_id: "stw_visual_claim".to_owned(),
            slot: "menu_background".to_owned(),
            claim_id: Some(claim.claim_id.clone()),
            generated_path: Some(source),
        })?;
        assert_eq!(completion.slot, "menu_background");
        assert!(completion.destination_path.ends_with("menu_background.png"));

        let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
            store_root: Some(store.clone()),
            world_id: "stw_visual_claim".to_owned(),
        })?;
        assert!(manifest.menu_background.exists);
        assert!(
            !manifest
                .image_generation_jobs
                .iter()
                .any(|job| job.slot == "menu_background")
        );

        let stage_claim = claim_visual_job(&ClaimVisualJobOptions {
            store_root: Some(store.clone()),
            world_id: "stw_visual_claim".to_owned(),
            slot: Some("stage_background".to_owned()),
            claimed_by: "test-worker".to_owned(),
            force: false,
            extra_jobs: Vec::new(),
        })?;
        let VisualJobClaimOutcome::Claimed { claim: stage_claim } = stage_claim else {
            anyhow::bail!("stage background should be claimable");
        };
        let release = release_visual_job_claim(&ReleaseVisualJobClaimOptions {
            store_root: Some(store),
            world_id: "stw_visual_claim".to_owned(),
            slot: "stage_background".to_owned(),
        })?;
        assert_eq!(
            release.claim.map(|claim| claim.claim_id),
            Some(stage_claim.claim_id)
        );
        Ok(())
    }

    #[test]
    fn visual_job_claim_accepts_extra_turn_cg_jobs() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_visual_turn_cg
title: "턴 CG 클레임"
premise:
  genre: "해무 낀 항구 판타지"
  protagonist: "현대인의 전생, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let turn_cg_path = initialized
            .world_dir
            .join(VN_ASSETS_DIR)
            .join("turn_cg")
            .join("turn_0001.png");
        let extra_job = visual_generation_job(
            "turn_cg:turn_0001".to_owned(),
            "player-visible turn CG prompt".to_owned(),
            turn_cg_path.display().to_string(),
            Vec::new(),
            Vec::new(),
            "save exactly to destination_path without blocking VN flow",
        );

        let claimed = claim_visual_job(&ClaimVisualJobOptions {
            store_root: Some(store.clone()),
            world_id: "stw_visual_turn_cg".to_owned(),
            slot: Some("turn_cg:turn_0001".to_owned()),
            claimed_by: "test-worker".to_owned(),
            force: false,
            extra_jobs: vec![extra_job],
        })?;
        let VisualJobClaimOutcome::Claimed { claim } = claimed else {
            anyhow::bail!("turn CG should be claimable through extra jobs");
        };
        assert_eq!(claim.slot, "turn_cg:turn_0001");

        let source = temp.path().join("turn-generated.png");
        std::fs::write(&source, MINIMAL_PNG)?;
        let completion = complete_visual_job(&CompleteVisualJobOptions {
            store_root: Some(store),
            world_id: "stw_visual_turn_cg".to_owned(),
            slot: "turn_cg:turn_0001".to_owned(),
            claim_id: Some(claim.claim_id),
            generated_path: Some(source),
        })?;

        assert_eq!(completion.slot, "turn_cg:turn_0001");
        assert!(turn_cg_path.is_file());
        Ok(())
    }

    #[test]
    fn turn_cg_scene_hint_uses_recent_bounded_visible_blocks() {
        let packet = RenderPacket {
            schema_version: "singulari.render_packet.v1".to_owned(),
            world_id: "stw_visual_hint".to_owned(),
            turn_id: "turn_0007".to_owned(),
            mode: "normal".to_owned(),
            narrative_contract: "agent_authored".to_owned(),
            narrative_scene: Some(NarrativeScene {
                schema_version: "singulari.narrative_scene.v1".to_owned(),
                speaker: None,
                text_blocks: vec![
                    "old-block".to_owned(),
                    "middle-block".to_owned(),
                    "가".repeat(2_400),
                ],
                tone_notes: Vec::new(),
            }),
            visible_state: VisibleState {
                dashboard: DashboardSummary {
                    phase: "opening".to_owned(),
                    location: "place:test".to_owned(),
                    anchor_invariant: "player-visible".to_owned(),
                    current_event: "event:test".to_owned(),
                    status: "fallback".to_owned(),
                },
                scan_targets: Vec::<ScanTarget>::new(),
                choices: Vec::<TurnChoice>::new(),
            },
            adjudication: None,
            codex_view: None,
            canon_delta_refs: Vec::new(),
            forbidden_reveals: Vec::new(),
            style_notes: Vec::new(),
        };

        let Some(hint) = turn_cg_scene_hint(&packet) else {
            panic!("scene hint should be present");
        };
        assert!(!hint.contains("old-block"));
        assert!(hint.contains("middle-block"));
        assert!(hint.ends_with(" ..."));
        assert!(hint.chars().count() < 1_900);
    }

    #[test]
    fn turn_cg_hides_major_character_until_design_sheet_exists() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_major_design_gate
title: "디자인 게이트"
premise:
  genre: "중세 판타지"
  protagonist: "현대인의 전생, 남자 주인공"
"#,
        )?;
        let initialized = init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;
        let packet = RenderPacket {
            schema_version: "singulari.render_packet.v1".to_owned(),
            world_id: "stw_major_design_gate".to_owned(),
            turn_id: "turn_0001".to_owned(),
            mode: "normal".to_owned(),
            narrative_contract: "agent_authored".to_owned(),
            narrative_scene: Some(NarrativeScene {
                schema_version: "singulari.narrative_scene.v1".to_owned(),
                speaker: None,
                text_blocks: vec!["주인공은 무너진 문 앞에서 손을 뻗었다.".to_owned()],
                tone_notes: Vec::new(),
            }),
            visible_state: VisibleState {
                dashboard: DashboardSummary {
                    phase: "opening".to_owned(),
                    location: "place:opening".to_owned(),
                    anchor_invariant: "player-visible".to_owned(),
                    current_event: "첫 문".to_owned(),
                    status: "문 앞에 섰다".to_owned(),
                },
                scan_targets: Vec::<ScanTarget>::new(),
                choices: Vec::<TurnChoice>::new(),
            },
            adjudication: None,
            codex_view: None,
            canon_delta_refs: Vec::new(),
            forbidden_reveals: Vec::new(),
            style_notes: Vec::new(),
        };

        let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
            store_root: Some(store.clone()),
            world_id: "stw_major_design_gate".to_owned(),
        })?;
        let gated_prompt = compile_turn_visual_prompt(&initialized.world, &packet, &manifest);

        assert!(gated_prompt.reference_paths.is_empty());
        assert!(
            gated_prompt
                .prompt
                .contains("Do not directly depict unresolved major characters")
        );
        assert!(gated_prompt.prompt.contains("주인공"));
        assert!(gated_prompt.prompt.contains("POV framing"));

        let character_sheet_dir = initialized
            .world_dir
            .join(VN_ASSETS_DIR)
            .join(CHARACTER_SHEETS_DIR);
        std::fs::create_dir_all(&character_sheet_dir)?;
        std::fs::write(
            character_sheet_dir.join("char_protagonist.png"),
            MINIMAL_PNG,
        )?;
        let manifest = build_world_visual_assets(&BuildWorldVisualAssetsOptions {
            store_root: Some(store),
            world_id: "stw_major_design_gate".to_owned(),
        })?;
        let ungated_prompt = compile_turn_visual_prompt(&initialized.world, &packet, &manifest);

        assert_eq!(ungated_prompt.reference_paths.len(), 1);
        assert!(
            !ungated_prompt
                .prompt
                .contains("Do not directly depict unresolved major characters")
        );
        assert!(ungated_prompt.prompt.contains("accepted reference sheets"));
        Ok(())
    }
}
