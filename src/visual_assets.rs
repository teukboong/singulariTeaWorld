use crate::job_ledger::{WorldJobStatus, WriteVisualJobOptions, write_visual_job};
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
pub const VISUAL_CANON_POLICY_SCHEMA_VERSION: &str = "singulari.visual_canon_policy.v1";
pub const VISUAL_CANON_AUDIT_SCHEMA_VERSION: &str = "singulari.visual_canon_audit.v1";
pub const VISUAL_ASSETS_FILENAME: &str = "visual_assets.json";
pub const VN_ASSETS_DIR: &str = "assets/vn";
pub const MENU_BACKGROUND_FILENAME: &str = "menu_background.png";
pub const STAGE_BACKGROUND_FILENAME: &str = "stage_background.png";
pub const CHARACTER_SHEETS_DIR: &str = "character_sheets";
pub const LOCATION_SHEETS_DIR: &str = "location_sheets";
pub const IMAGE_GENERATION_TOOL: &str = "worldsim.image.generate";
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualArtifactKind {
    SceneCg,
    CharacterDesignSheet,
    LocationDesignSheet,
    UiBackground,
}

impl VisualArtifactKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SceneCg => "scene_cg",
            Self::CharacterDesignSheet => "character_design_sheet",
            Self::LocationDesignSheet => "location_design_sheet",
            Self::UiBackground => "ui_background",
        }
    }

    #[must_use]
    pub const fn canonical_use(self) -> &'static str {
        match self {
            Self::SceneCg => "display_scene",
            Self::CharacterDesignSheet | Self::LocationDesignSheet => "reference_generation",
            Self::UiBackground => "display_ui_background",
        }
    }

    #[must_use]
    pub const fn display_allowed(self) -> bool {
        matches!(self, Self::SceneCg | Self::UiBackground)
    }

    #[must_use]
    pub const fn reference_allowed(self) -> bool {
        matches!(self, Self::CharacterDesignSheet | Self::LocationDesignSheet)
    }
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
    pub artifact_kind: VisualArtifactKind,
    pub canonical_use: String,
    pub display_allowed: bool,
    pub reference_allowed: bool,
    pub visual_canon_policy: VisualCanonPolicy,
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
    pub artifact_kind: VisualArtifactKind,
    pub canonical_use: String,
    pub display_allowed: bool,
    pub reference_allowed: bool,
    pub visual_canon_policy: VisualCanonPolicy,
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
    pub image_generation_call: HostImageGenerationCall,
    pub slot: String,
    pub artifact_kind: VisualArtifactKind,
    pub canonical_use: String,
    pub display_allowed: bool,
    pub reference_allowed: bool,
    #[serde(default)]
    pub visual_canon_policy: VisualCanonPolicy,
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
pub struct HostImageGenerationCall {
    pub capability: String,
    pub slot: String,
    pub prompt: String,
    pub destination_path: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_paths: Vec<String>,
    #[serde(default)]
    pub visual_canon_policy: VisualCanonPolicy,
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
    pub canon_audit: VisualCanonAudit,
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
    pub visual_canon_policy: VisualCanonPolicy,
    pub prompt_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualCanonPolicy {
    pub schema_version: String,
    pub authority: String,
    pub canon_locked_traits: Vec<String>,
    pub style_free_traits: Vec<String>,
    pub forbidden_inventions: Vec<String>,
    pub audit_policy: String,
    pub source_refs: Vec<String>,
}

impl Default for VisualCanonPolicy {
    fn default() -> Self {
        Self {
            schema_version: VISUAL_CANON_POLICY_SCHEMA_VERSION.to_owned(),
            authority: "player_visible_visual_policy".to_owned(),
            canon_locked_traits: Vec::new(),
            style_free_traits: default_style_free_traits(),
            forbidden_inventions: default_forbidden_inventions(),
            audit_policy: "generated images are visual artifacts only; never promote newly invented details to world canon without explicit accepted event evidence".to_owned(),
            source_refs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisualCanonAudit {
    pub schema_version: String,
    pub status: VisualCanonAuditStatus,
    pub review_required: bool,
    pub policy_snapshot: VisualCanonPolicy,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VisualCanonAuditStatus {
    PendingReview,
    /// File, destination, and display policy passed; generated pixels are not canon-approved facts.
    AcceptedDisplayOnly,
    /// File, destination, and reference policy passed; generated pixels are not canon-approved facts.
    AcceptedReferenceOnly,
    /// Reserved for a future explicit reviewer workflow; current code does not auto-transition here.
    ManualReviewRequired,
    RejectedPolicyViolation,
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
        artifact_kind: VisualArtifactKind::UiBackground,
        canonical_use: VisualArtifactKind::UiBackground.canonical_use().to_owned(),
        display_allowed: VisualArtifactKind::UiBackground.display_allowed(),
        reference_allowed: VisualArtifactKind::UiBackground.reference_allowed(),
        visual_canon_policy: menu_visual_canon_policy(&world),
        prompt: menu_background_prompt(&world, &style_profile),
        recommended_path: menu_path.display().to_string(),
        asset_url: world_asset_url(world.world_id.as_str(), MENU_BACKGROUND_FILENAME),
        exists: menu_path.is_file(),
        prompt_policy: prompt_policy(),
    };
    let stage = WorldVisualAsset {
        slot: "stage_background".to_owned(),
        artifact_kind: VisualArtifactKind::UiBackground,
        canonical_use: VisualArtifactKind::UiBackground.canonical_use().to_owned(),
        display_allowed: VisualArtifactKind::UiBackground.display_allowed(),
        reference_allowed: VisualArtifactKind::UiBackground.reference_allowed(),
        visual_canon_policy: stage_visual_canon_policy(&world),
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

/// Atomically claim one pending visual generation job for an image host worker.
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
        validate_visual_canon_policy_for_job(&job)?;
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
            write_visual_job(&WriteVisualJobOptions {
                store_root: options.store_root.as_deref(),
                world_id: options.world_id.as_str(),
                job: &claim.job,
                status: WorldJobStatus::Claimed,
                claim_id: Some(claim.claim_id.clone()),
                claim_owner: Some(claim.claimed_by.clone()),
                claimed_at: Some(claim.claimed_at.clone()),
                claim_path: Some(claim.claim_path.clone()),
                attempt_id: Some(claim.claim_id.clone()),
                output_ref: Some(claim.job.destination_path.clone()),
                last_error: None,
            })?;
            return Ok(VisualJobClaimOutcome::Claimed {
                claim: Box::new(claim),
            });
        }
    }
    Ok(VisualJobClaimOutcome::NoPending {
        world_id: options.world_id.clone(),
    })
}

/// Complete a visual generation job after the image host saves the asset.
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
    let canon_audit = completed_visual_canon_audit(&job, &destination, bytes)?;
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
        canon_audit,
    };
    ensure_parent_dir(&completion_path)?;
    write_json(&completion_path, &completion)?;
    write_visual_job(&WriteVisualJobOptions {
        store_root: options.store_root.as_deref(),
        world_id: options.world_id.as_str(),
        job: &job,
        status: WorldJobStatus::Completed,
        claim_id: completion.claim_id.clone(),
        claim_owner: claim.as_ref().map(|claim| claim.claimed_by.clone()),
        claimed_at: claim.as_ref().map(|claim| claim.claimed_at.clone()),
        claim_path: claim.as_ref().map(|claim| claim.claim_path.clone()),
        attempt_id: completion.claim_id.clone(),
        output_ref: Some(completion.destination_path.clone()),
        last_error: None,
    })?;
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
    if let Some(claim) = claim.as_ref() {
        write_visual_job(&WriteVisualJobOptions {
            store_root: options.store_root.as_deref(),
            world_id: options.world_id.as_str(),
            job: &claim.job,
            status: WorldJobStatus::Pending,
            claim_id: None,
            claim_owner: None,
            claimed_at: None,
            claim_path: None,
            attempt_id: Some(claim.claim_id.clone()),
            output_ref: Some(claim.job.destination_path.clone()),
            last_error: None,
        })?;
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
    apply_visual_canon_policy(
        visual_generation_job(
            asset.slot.clone(),
            asset.artifact_kind,
            asset.prompt.clone(),
            asset.recommended_path.clone(),
            Vec::new(),
            Vec::new(),
            "save exactly to destination_path; the VN server auto-detects the file on the next packet refresh",
        ),
        asset.visual_canon_policy.clone(),
    )
}

fn visual_entity_generation_job(asset: &VisualEntityAsset) -> ImageGenerationJob {
    apply_visual_canon_policy(
        visual_generation_job(
            asset.slot.clone(),
            asset.artifact_kind,
            asset.prompt.clone(),
            asset.recommended_path.clone(),
            Vec::new(),
            Vec::new(),
            "save exactly to destination_path; future turn CG jobs may use this sheet as a reference",
        ),
        asset.visual_canon_policy.clone(),
    )
}

#[must_use]
pub fn visual_generation_job(
    slot: String,
    artifact_kind: VisualArtifactKind,
    prompt: String,
    destination_path: String,
    reference_asset_urls: Vec<String>,
    reference_paths: Vec<String>,
    register_policy: &str,
) -> ImageGenerationJob {
    let visual_canon_policy = default_visual_canon_policy_for_kind(artifact_kind);
    ImageGenerationJob {
        tool: IMAGE_GENERATION_TOOL.to_owned(),
        image_generation_call: HostImageGenerationCall {
            capability: "image_generation".to_owned(),
            slot: slot.clone(),
            prompt: prompt.clone(),
            destination_path: destination_path.clone(),
            reference_paths: reference_paths.clone(),
            visual_canon_policy: visual_canon_policy.clone(),
            overwrite: false,
        },
        slot,
        artifact_kind,
        canonical_use: artifact_kind.canonical_use().to_owned(),
        display_allowed: artifact_kind.display_allowed(),
        reference_allowed: artifact_kind.reference_allowed(),
        visual_canon_policy,
        prompt,
        destination_path,
        reference_asset_urls,
        reference_paths,
        overwrite: false,
        register_policy: register_policy.to_owned(),
    }
}

#[must_use]
pub fn apply_visual_canon_policy(
    mut job: ImageGenerationJob,
    visual_canon_policy: VisualCanonPolicy,
) -> ImageGenerationJob {
    job.image_generation_call.visual_canon_policy = visual_canon_policy.clone();
    job.visual_canon_policy = visual_canon_policy;
    job
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

/// Render the canon policy into the host-facing image prompt.
#[must_use]
pub fn visual_canon_policy_prompt(policy: &VisualCanonPolicy) -> String {
    format!(
        "Visual canon policy JSON:\n{}\nGenerated pixels are not world truth. If the image seems to add a new fact, treat it as an invalid visual artifact until a later accepted world event explicitly promotes that fact.",
        serde_json::to_string_pretty(policy).unwrap_or_else(|_| {
            "{\"error\":\"visual canon policy serialization failed\"}".to_owned()
        })
    )
}

/// Ensure an image job carries a self-consistent canon policy before a host can claim it.
///
/// # Errors
///
/// Returns an error when the policy is malformed, missing required invention
/// guards, or diverges between the public job and host call payload.
pub fn validate_visual_canon_policy_for_job(job: &ImageGenerationJob) -> Result<()> {
    let policy = &job.visual_canon_policy;
    if policy.schema_version != VISUAL_CANON_POLICY_SCHEMA_VERSION {
        bail!(
            "visual canon policy schema mismatch: slot={}, expected={}, actual={}",
            job.slot,
            VISUAL_CANON_POLICY_SCHEMA_VERSION,
            policy.schema_version
        );
    }
    if job.image_generation_call.visual_canon_policy != *policy {
        bail!(
            "visual canon policy diverged between job and host call: slot={}",
            job.slot
        );
    }
    if policy.canon_locked_traits.is_empty() {
        bail!(
            "visual canon policy missing locked traits: slot={}",
            job.slot
        );
    }
    if policy.style_free_traits.is_empty() {
        bail!(
            "visual canon policy missing style-free traits: slot={}",
            job.slot
        );
    }
    if policy.forbidden_inventions.is_empty() {
        bail!(
            "visual canon policy missing forbidden inventions: slot={}",
            job.slot
        );
    }
    if job.artifact_kind == VisualArtifactKind::SceneCg
        && !policy_forbids(policy, "new character")
        && !policy_forbids(policy, "new visible character")
    {
        bail!(
            "scene CG visual canon policy must forbid new character invention: slot={}",
            job.slot
        );
    }
    if job.artifact_kind == VisualArtifactKind::CharacterDesignSheet
        && !policy_forbids(policy, "hidden identity")
    {
        bail!(
            "character design visual canon policy must forbid hidden identity invention: slot={}",
            job.slot
        );
    }
    Ok(())
}

fn policy_forbids(policy: &VisualCanonPolicy, needle: &str) -> bool {
    policy
        .forbidden_inventions
        .iter()
        .any(|item| item.to_ascii_lowercase().contains(needle))
}

fn completed_visual_canon_audit(
    job: &ImageGenerationJob,
    destination: &Path,
    bytes: u64,
) -> Result<VisualCanonAudit> {
    validate_visual_canon_policy_for_job(job)?;
    if bytes == 0 {
        bail!(
            "visual canon completion audit rejected empty artifact: slot={}",
            job.slot
        );
    }
    if destination != Path::new(job.destination_path.as_str()) {
        bail!(
            "visual canon completion audit destination mismatch: slot={}, expected={}, actual={}",
            job.slot,
            job.destination_path,
            destination.display()
        );
    }

    let (status, use_label) = match job.artifact_kind {
        VisualArtifactKind::SceneCg | VisualArtifactKind::UiBackground => {
            if !job.display_allowed {
                return Ok(completed_visual_policy_violation(
                    job,
                    "display artifact is not display_allowed",
                ));
            }
            (VisualCanonAuditStatus::AcceptedDisplayOnly, "display")
        }
        VisualArtifactKind::CharacterDesignSheet | VisualArtifactKind::LocationDesignSheet => {
            if !job.reference_allowed {
                return Ok(completed_visual_policy_violation(
                    job,
                    "reference artifact is not reference_allowed",
                ));
            }
            (VisualCanonAuditStatus::AcceptedReferenceOnly, "reference")
        }
    };

    Ok(VisualCanonAudit {
        schema_version: VISUAL_CANON_AUDIT_SCHEMA_VERSION.to_owned(),
        status,
        review_required: false,
        policy_snapshot: job.visual_canon_policy.clone(),
        message: format!(
            "completed visual artifact for slot={} is accepted for {use_label} use only; generated pixels are not world truth and cannot promote new canon without accepted event evidence",
            job.slot
        ),
    })
}

fn completed_visual_policy_violation(job: &ImageGenerationJob, reason: &str) -> VisualCanonAudit {
    VisualCanonAudit {
        schema_version: VISUAL_CANON_AUDIT_SCHEMA_VERSION.to_owned(),
        status: VisualCanonAuditStatus::RejectedPolicyViolation,
        review_required: true,
        policy_snapshot: job.visual_canon_policy.clone(),
        message: format!(
            "completed visual artifact for slot={} is rejected by visual canon policy: {reason}",
            job.slot
        ),
    }
}

fn default_style_free_traits() -> Vec<String> {
    [
        "lighting",
        "composition",
        "camera distance",
        "weather texture",
        "brushwork and rendering style",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_forbidden_inventions() -> Vec<String> {
    [
        "new character not named in player-visible state",
        "new weapon, item, symbol, insignia, scar, age, or faction mark not locked by policy",
        "hidden route, hidden identity, secret relationship, or future event",
        "rendered text or UI words",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn default_visual_canon_policy_for_kind(artifact_kind: VisualArtifactKind) -> VisualCanonPolicy {
    VisualCanonPolicy {
        authority: "default_artifact_kind_policy".to_owned(),
        canon_locked_traits: vec![
            format!("artifact_kind: {}", artifact_kind.as_str()),
            format!("canonical_use: {}", artifact_kind.canonical_use()),
        ],
        source_refs: vec![format!("artifact_kind:{}", artifact_kind.as_str())],
        ..VisualCanonPolicy::default()
    }
}

fn menu_visual_canon_policy(world: &WorldRecord) -> VisualCanonPolicy {
    premise_visual_canon_policy(
        "menu_background",
        world,
        "ui background for main menu; negative space for VN controls is allowed",
    )
}

fn stage_visual_canon_policy(world: &WorldRecord) -> VisualCanonPolicy {
    premise_visual_canon_policy(
        "stage_background",
        world,
        "ui background for VN stage layer; atmospheric readability is allowed",
    )
}

fn premise_visual_canon_policy(
    slot: &str,
    world: &WorldRecord,
    extra_locked: &str,
) -> VisualCanonPolicy {
    VisualCanonPolicy {
        authority: "player_visible_world_premise".to_owned(),
        canon_locked_traits: vec![
            format!("world title: {}", world.title),
            format!("genre: {}", world.premise.genre),
            format!("protagonist premise: {}", world.premise.protagonist),
            format!("opening state: {}", world.premise.opening_state),
            extra_locked.to_owned(),
        ],
        source_refs: vec![
            format!("world:{}", world.world_id),
            format!("visual_slot:{slot}"),
        ],
        ..VisualCanonPolicy::default()
    }
}

fn character_visual_canon_policy(
    world: &WorldRecord,
    character: &CharacterRecord,
    display_name: &str,
) -> VisualCanonPolicy {
    let mut locked = vec![
        format!("character display name: {display_name}"),
        format!("role: {}", player_visible_character_role(character)),
    ];
    locked.extend(
        player_visible_character_traits(character)
            .into_iter()
            .map(|trait_text| format!("confirmed trait: {trait_text}")),
    );
    locked.extend(
        player_visible_gestures(character)
            .into_iter()
            .map(|gesture| format!("voice/gesture anchor: {gesture}")),
    );
    VisualCanonPolicy {
        authority: "player_visible_character_record".to_owned(),
        canon_locked_traits: locked,
        source_refs: vec![
            format!("world:{}", world.world_id),
            format!("entity:{}", character.id),
        ],
        forbidden_inventions: [
            "hidden identity, hidden relationship, or undisclosed motive",
            "new scar, weapon, insignia, age, injury, body mark, or faction mark not listed in locked traits",
            "new costume detail that implies social rank or allegiance not listed in locked traits",
            "rendered text or UI words",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        ..VisualCanonPolicy::default()
    }
}

fn location_visual_canon_policy(
    world: &WorldRecord,
    place: &PlaceRecord,
    display_name: &str,
) -> VisualCanonPolicy {
    let mut locked = vec![
        format!("location display name: {display_name}"),
        format!("genre: {}", world.premise.genre),
    ];
    locked.extend(
        place
            .notes
            .iter()
            .filter(|note| !leaks_internal_anchor_or_hidden_text([note.as_str()]))
            .map(|note| format!("player-visible location note: {note}")),
    );
    VisualCanonPolicy {
        authority: "player_visible_location_record".to_owned(),
        canon_locked_traits: locked,
        source_refs: vec![
            format!("world:{}", world.world_id),
            format!("place:{}", place.id),
        ],
        forbidden_inventions: [
            "hidden route, secret room, unrevealed symbol, future damage, or faction mark",
            "new character, creature, weapon, ritual object, or signage not listed in locked traits",
            "new geography that changes reachable paths",
            "rendered text or UI words",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        ..VisualCanonPolicy::default()
    }
}

fn turn_visual_canon_policy(
    packet: &RenderPacket,
    references: &[&VisualEntityAsset],
) -> VisualCanonPolicy {
    let dashboard = &packet.visible_state.dashboard;
    let mut locked = vec![
        format!("turn: {}", packet.turn_id),
        format!("location: {}", dashboard.location),
        format!("current event: {}", dashboard.current_event),
        format!("visible status: {}", dashboard.status),
    ];
    locked.extend(
        packet
            .visible_state
            .scan_targets
            .iter()
            .take(4)
            .map(|target| format!("visible focus: {}", player_visible_focus_label(target))),
    );
    locked.extend(references.iter().map(|asset| {
        format!(
            "accepted reference sheet: {} {} -> {}",
            asset.entity_type, asset.display_name, asset.recommended_path
        )
    }));
    let mut source_refs = vec![format!("render_packet:{}", packet.turn_id)];
    source_refs.extend(packet.canon_delta_refs.iter().cloned());
    VisualCanonPolicy {
        authority: "player_visible_render_packet".to_owned(),
        canon_locked_traits: locked,
        source_refs,
        forbidden_inventions: [
            "new character not named in player-visible state or accepted reference sheets",
            "new weapon, item, symbol, scar, age, injury, faction mark, route, or location not named in visible state",
            "hidden truth, secret identity, future consequence, or offscreen event not visible this turn",
            "rendered text or UI words",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        ..VisualCanonPolicy::default()
    }
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
        artifact_kind: VisualArtifactKind::CharacterDesignSheet,
        canonical_use: VisualArtifactKind::CharacterDesignSheet
            .canonical_use()
            .to_owned(),
        display_allowed: VisualArtifactKind::CharacterDesignSheet.display_allowed(),
        reference_allowed: VisualArtifactKind::CharacterDesignSheet.reference_allowed(),
        visual_canon_policy: character_visual_canon_policy(world, character, display_name.as_str()),
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
        artifact_kind: VisualArtifactKind::LocationDesignSheet,
        canonical_use: VisualArtifactKind::LocationDesignSheet
            .canonical_use()
            .to_owned(),
        display_allowed: VisualArtifactKind::LocationDesignSheet.display_allowed(),
        reference_allowed: VisualArtifactKind::LocationDesignSheet.reference_allowed(),
        visual_canon_policy: location_visual_canon_policy(world, place, display_name.as_str()),
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
        visual_canon_policy: turn_visual_canon_policy(packet, &references),
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
            .filter(|asset| {
                asset.exists
                    && asset.reference_allowed
                    && asset.artifact_kind == VisualArtifactKind::CharacterDesignSheet
            })
            .take(max_refs),
    );
    if refs.len() < max_refs {
        refs.extend(
            manifest
                .visual_entities
                .iter()
                .filter(|asset| {
                    asset.exists
                        && asset.reference_allowed
                        && asset.artifact_kind == VisualArtifactKind::LocationDesignSheet
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
        BuildWorldVisualAssetsOptions, CHARACTER_SHEETS_DIR, ClaimVisualJobOptions,
        CompleteVisualJobOptions, IMAGE_GENERATION_TOOL, ReleaseVisualJobClaimOptions,
        VN_ASSETS_DIR, VisualArtifactKind, VisualCanonAuditStatus, VisualJobClaimOutcome,
        build_world_visual_assets, claim_visual_job, compile_turn_visual_prompt,
        complete_visual_job, release_visual_job_claim, turn_cg_scene_hint,
        validate_visual_canon_policy_for_job, visual_generation_job,
    };
    use crate::models::{
        DashboardSummary, NarrativeScene, RenderPacket, ScanTarget, TurnChoice, VisibleState,
    };
    use crate::store::{InitWorldOptions, init_world};
    use anyhow::Context;
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
  protagonist: "변경 순찰자, 남자 주인공"
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
        assert_eq!(
            manifest.menu_background.artifact_kind,
            VisualArtifactKind::UiBackground
        );
        assert!(manifest.menu_background.display_allowed);
        assert!(!manifest.menu_background.reference_allowed);
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
        let protagonist_asset = manifest
            .visual_entities
            .iter()
            .find(|asset| asset.slot == "character_sheet:char:protagonist")
            .context("protagonist character sheet asset missing")?;
        assert_eq!(
            protagonist_asset.artifact_kind,
            VisualArtifactKind::CharacterDesignSheet
        );
        assert!(!protagonist_asset.display_allowed);
        assert!(protagonist_asset.reference_allowed);
        for job in &manifest.image_generation_jobs {
            assert_eq!(job.tool, IMAGE_GENERATION_TOOL);
            assert_eq!(job.image_generation_call.capability, "image_generation");
            assert_eq!(job.image_generation_call.slot, job.slot);
            assert_eq!(job.image_generation_call.prompt, job.prompt);
            assert_eq!(job.canonical_use, job.artifact_kind.canonical_use());
            assert_eq!(job.display_allowed, job.artifact_kind.display_allowed());
            assert_eq!(job.reference_allowed, job.artifact_kind.reference_allowed());
            assert_eq!(
                job.image_generation_call.destination_path,
                job.destination_path
            );
            assert_eq!(
                job.image_generation_call.visual_canon_policy,
                job.visual_canon_policy
            );
            assert!(!job.visual_canon_policy.canon_locked_traits.is_empty());
            assert!(!job.visual_canon_policy.style_free_traits.is_empty());
            assert!(
                job.visual_canon_policy
                    .forbidden_inventions
                    .iter()
                    .any(|item| item.contains("rendered text"))
            );
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
  protagonist: "변경 순찰자, 남자 주인공"
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
        assert_eq!(
            completion.canon_audit.status,
            VisualCanonAuditStatus::AcceptedDisplayOnly
        );
        assert!(!completion.canon_audit.review_required);
        assert!(
            completion
                .canon_audit
                .message
                .contains("accepted for display use only")
        );
        assert!(
            completion
                .canon_audit
                .message
                .contains("generated pixels are not world truth")
        );

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
    fn visual_job_completion_accepts_reference_only_assets() -> anyhow::Result<()> {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(
            &seed_path,
            r#"
schema_version: singulari.world_seed.v1
world_id: stw_visual_reference_audit
title: "레퍼런스 감사"
premise:
  genre: "해무 낀 항구 판타지"
  protagonist: "변경 순찰자, 남자 주인공"
"#,
        )?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        let claimed = claim_visual_job(&ClaimVisualJobOptions {
            store_root: Some(store.clone()),
            world_id: "stw_visual_reference_audit".to_owned(),
            slot: Some("character_sheet:char:protagonist".to_owned()),
            claimed_by: "test-worker".to_owned(),
            force: false,
            extra_jobs: Vec::new(),
        })?;
        let VisualJobClaimOutcome::Claimed { claim } = claimed else {
            anyhow::bail!("protagonist character sheet should be claimable");
        };
        assert_eq!(
            claim.job.artifact_kind,
            VisualArtifactKind::CharacterDesignSheet
        );

        let source = temp.path().join("character-generated.png");
        std::fs::write(&source, MINIMAL_PNG)?;
        let completion = complete_visual_job(&CompleteVisualJobOptions {
            store_root: Some(store),
            world_id: "stw_visual_reference_audit".to_owned(),
            slot: "character_sheet:char:protagonist".to_owned(),
            claim_id: Some(claim.claim_id),
            generated_path: Some(source),
        })?;

        assert_eq!(
            completion.canon_audit.status,
            VisualCanonAuditStatus::AcceptedReferenceOnly
        );
        assert!(!completion.canon_audit.review_required);
        assert!(
            completion
                .canon_audit
                .message
                .contains("accepted for reference use only")
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
  protagonist: "변경 순찰자, 남자 주인공"
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
            VisualArtifactKind::SceneCg,
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
        assert_eq!(
            completion.canon_audit.status,
            VisualCanonAuditStatus::AcceptedDisplayOnly
        );
        assert!(!completion.canon_audit.review_required);
        assert!(
            completion
                .canon_audit
                .policy_snapshot
                .forbidden_inventions
                .iter()
                .any(|item| item.contains("new character"))
        );
        Ok(())
    }

    #[test]
    fn visual_canon_policy_rejects_divergent_host_payload() {
        let mut job = visual_generation_job(
            "turn_cg:turn_0001".to_owned(),
            VisualArtifactKind::SceneCg,
            "player-visible turn CG prompt".to_owned(),
            "/tmp/turn_0001.png".to_owned(),
            Vec::new(),
            Vec::new(),
            "test",
        );
        job.image_generation_call
            .visual_canon_policy
            .canon_locked_traits
            .clear();

        let Err(error) = validate_visual_canon_policy_for_job(&job) else {
            panic!("divergent host payload should fail visual canon policy validation");
        };
        assert!(
            error
                .to_string()
                .contains("diverged between job and host call")
        );
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
  protagonist: "변경 순찰자, 남자 주인공"
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
                .visual_canon_policy
                .source_refs
                .contains(&"render_packet:turn_0001".to_owned())
        );
        assert!(
            gated_prompt
                .visual_canon_policy
                .forbidden_inventions
                .iter()
                .any(|item| item.contains("new character"))
        );
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
