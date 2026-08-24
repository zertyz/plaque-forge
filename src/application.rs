//! Programmatic application API.
//!
//! The CLI is an adapter over these request types. Core workflows depend on
//! application concepts rather than `clap` argument structures, so GUIs, tests,
//! and other Rust programs can invoke the same operations without shelling out.

use std::path::PathBuf;

use anyhow::{Result, ensure};

use crate::infrastructure::CommandExecutor;
use crate::writable_region::ResolvedWritableRegion;

/// Replaceable external services used by application workflows.
///
/// Keep this set intentionally narrow. Streaming video decode/encode has a stateful
/// lifecycle and remains behind `video` rather than being forced into this collected
/// command contract.
pub struct ApplicationServices<'a> {
    commands: &'a dyn CommandExecutor,
}

impl<'a> ApplicationServices<'a> {
    /// Build services around a caller-supplied external command executor.
    pub fn new(commands: &'a dyn CommandExecutor) -> Self {
        Self { commands }
    }

    fn production() -> Self {
        Self::new(&crate::infrastructure::OS_COMMAND_EXECUTOR)
    }

    pub(crate) fn commands(&self) -> &'a dyn CommandExecutor {
        self.commands
    }
}

/// Typography fitting policy used by rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitMode {
    Maximize,
    Balanced,
    /// Search word-boundary line breaks and score visual balance before fitting.
    Artistic,
    Fixed,
}

/// Horizontal title alignment inside the writable region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Vertical title alignment inside the writable region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerticalAlign {
    Top,
    Center,
    Bottom,
}

/// Progress reporting policy for long-running workflows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}

/// Analyze a source video and materialize its reusable scene cache.
#[derive(Debug, Clone)]
pub struct AnalyzeRequest {
    pub input: PathBuf,
    pub source_is_text_free: bool,
    pub output: Option<PathBuf>,
    pub scene: Option<PathBuf>,
    pub surface: Option<String>,
    pub minimum_analysis_confidence: f64,
    pub diagnostics: Option<PathBuf>,
    pub force: bool,
    pub if_needed: bool,
    pub segmentation_worker: Option<PathBuf>,
    /// `auto` delegates semantic model/refiner selection to Rust.
    pub segmentation_backend: String,
    /// `auto` uses the model selected by the Rust strategy planner.
    pub segmentation_model: String,
    /// Execution backend only; does not imply numeric precision.
    pub segmentation_device: String,
    /// preview | balanced | canonical
    pub segmentation_profile: String,
    /// auto | fp32 | bf16. Resolved before device selection.
    pub segmentation_precision: String,
    pub force_ml: bool,
    pub progress: ProgressMode,
    pub progress_interval_ms: u64,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    // Expert tuning. Kept explicit so tests and advanced callers can reproduce
    // exactly the same analysis path as the CLI.
    pub surface_hint: Option<[f64; 4]>,
    pub surface_frame: Option<usize>,
    pub writable_region_hint: Option<ResolvedWritableRegion>,
    pub anchor_interval: usize,
    pub tracking_inertia: f64,
    pub candidate_samples: usize,
    pub extraction_samples: usize,
    pub local_scene_radius: i32,
    pub occlusion_sensitivity: f64,
    pub disable_occlusion: bool,
}

/// Source of the title text. Exactly one source is represented by construction.
#[derive(Debug, Clone)]
pub enum TitleSource {
    Text(String),
    File(PathBuf),
}

/// Render from a previously materialized analysis cache.
#[derive(Debug, Clone)]
pub struct RenderRequest {
    pub input: PathBuf,
    pub analysis: PathBuf,
    pub scene: Option<PathBuf>,
    pub surface: Option<String>,
    pub title: TitleSource,
    pub font: PathBuf,
    pub style_file: Option<PathBuf>,
    pub output: PathBuf,
    pub diagnostics: Option<PathBuf>,
    pub fit: FitMode,
    pub font_size: Option<f32>,
    /// OpenType/CSS font weight used when no style file overrides direct style options.
    pub font_weight: u16,
    pub supersampling: u32,
    pub target_fill: f32,
    pub max_lines: usize,
    pub padding: f32,
    pub line_height: f32,
    pub stroke_width: f32,
    pub text_color: String,
    pub stroke_color: String,
    pub glow_color: String,
    pub glow_radius: u32,
    pub shadow_offset_x: f32,
    pub shadow_offset_y: f32,
    pub shadow_blur_radius: u32,
    pub shadow_color: String,
    pub text_align: TextAlign,
    pub vertical_align: VerticalAlign,
    pub encoder_args: Vec<String>,
    pub progress: ProgressMode,
    pub progress_interval_ms: u64,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// Verify broad measurable correctness of an existing render.
#[derive(Debug, Clone)]
pub struct VerifyRequest {
    pub analysis: PathBuf,
    pub rendered: PathBuf,
    pub original: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub diagnostics: Option<PathBuf>,
    pub minimum_score: f64,
    pub progress: ProgressMode,
    pub progress_interval_ms: u64,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

/// Audit the capability-oriented homologation matrix.
#[derive(Debug, Clone)]
pub struct HomologationCoverageRequest {
    pub matrix: PathBuf,
    pub report: Option<PathBuf>,
    pub require_complete: bool,
}

/// Enforce a human-reviewed homologation contract.
#[derive(Debug, Clone)]
pub struct HomologateRequest {
    pub contract: PathBuf,
    pub rendered: PathBuf,
    pub report: Option<PathBuf>,
    /// Root directory for rich failure diagnostics. Files are emitted only for
    /// failed semantic witnesses.
    pub diagnostics: Option<PathBuf>,
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

impl AnalyzeRequest {
    /// Start an analysis request while explicitly asserting that the source surface
    /// is title-free.
    pub fn text_free(input: impl Into<PathBuf>) -> Self {
        Self {
            input: input.into(),
            source_is_text_free: true,
            output: None,
            scene: None,
            surface: None,
            minimum_analysis_confidence: 0.70,
            diagnostics: None,
            force: false,
            if_needed: false,
            segmentation_worker: None,
            segmentation_backend: "auto".into(),
            segmentation_model: "auto".into(),
            segmentation_device: "auto".into(),
            segmentation_profile: "canonical".into(),
            segmentation_precision: "auto".into(),
            force_ml: false,
            progress: ProgressMode::Auto,
            progress_interval_ms: 500,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            surface_hint: None,
            surface_frame: None,
            writable_region_hint: None,
            anchor_interval: 24,
            tracking_inertia: 0.35,
            candidate_samples: 24,
            extraction_samples: 72,
            local_scene_radius: 12,
            occlusion_sensitivity: 1.0,
            disable_occlusion: false,
        }
    }
}

impl RenderRequest {
    /// Create a render request with the same stable defaults as the CLI.
    pub fn new(
        input: impl Into<PathBuf>,
        analysis: impl Into<PathBuf>,
        output: impl Into<PathBuf>,
        title: TitleSource,
        font: impl Into<PathBuf>,
    ) -> Self {
        Self {
            input: input.into(),
            analysis: analysis.into(),
            scene: None,
            surface: None,
            title,
            font: font.into(),
            style_file: None,
            output: output.into(),
            diagnostics: None,
            fit: FitMode::Artistic,
            font_size: None,
            font_weight: 600,
            supersampling: 4,
            target_fill: 0.94,
            max_lines: 5,
            padding: 0.03,
            line_height: 1.08,
            stroke_width: 0.0,
            text_color: "#EBFFFFFF".into(),
            stroke_color: "#03181ED2".into(),
            glow_color: "#69F2FA90".into(),
            glow_radius: 10,
            shadow_offset_x: 0.0,
            shadow_offset_y: 0.0,
            shadow_blur_radius: 0,
            shadow_color: "#00000000".into(),
            text_align: TextAlign::Center,
            vertical_align: VerticalAlign::Center,
            encoder_args: Vec::new(),
            progress: ProgressMode::Auto,
            progress_interval_ms: 500,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}

impl VerifyRequest {
    /// Create a verification request with the same stable defaults as the CLI.
    pub fn new(analysis: impl Into<PathBuf>, rendered: impl Into<PathBuf>) -> Self {
        Self {
            analysis: analysis.into(),
            rendered: rendered.into(),
            original: None,
            report: None,
            diagnostics: None,
            minimum_score: 0.95,
            progress: ProgressMode::Auto,
            progress_interval_ms: 500,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}

impl HomologateRequest {
    /// Create a homologation request with production defaults and no diagnostic output.
    pub fn new(contract: impl Into<PathBuf>, rendered: impl Into<PathBuf>) -> Self {
        Self {
            contract: contract.into(),
            rendered: rendered.into(),
            report: None,
            diagnostics: None,
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
        }
    }
}

impl HomologationCoverageRequest {
    /// Create a non-strict capability audit request.
    pub fn new(matrix: impl Into<PathBuf>) -> Self {
        Self {
            matrix: matrix.into(),
            report: None,
            require_complete: false,
        }
    }
}

/// Analyze through the same production workflow used by the CLI.
pub fn analyze(request: AnalyzeRequest) -> Result<()> {
    analyze_with(request, &ApplicationServices::production())
}

/// Analyze with explicitly supplied external services.
pub fn analyze_with(request: AnalyzeRequest, services: &ApplicationServices<'_>) -> Result<()> {
    crate::analyze::run(request, services.commands())
}

/// Render through the same production workflow used by the CLI.
pub fn render(request: RenderRequest) -> Result<()> {
    render_with(request, &ApplicationServices::production())
}

/// Render with explicitly supplied external services.
pub fn render_with(request: RenderRequest, services: &ApplicationServices<'_>) -> Result<()> {
    crate::render::run(request, services.commands())
}

/// Verify through the same production workflow used by the CLI.
pub fn verify(request: VerifyRequest) -> Result<()> {
    verify_with(request, &ApplicationServices::production())
}

/// Verify with explicitly supplied external services.
pub fn verify_with(request: VerifyRequest, services: &ApplicationServices<'_>) -> Result<()> {
    crate::verify::run(request, services.commands())
}

/// Homologate through the same production workflow used by the CLI.
pub fn homologate(request: HomologateRequest) -> Result<()> {
    homologate_with(request, &ApplicationServices::production())
}

/// Homologate with explicitly supplied external services.
pub fn homologate_with(
    request: HomologateRequest,
    services: &ApplicationServices<'_>,
) -> Result<()> {
    crate::homologation::run(request, services.commands())
}

/// Audit which behavioral capabilities have human-reviewed regression sentinels.
pub fn homologation_coverage(
    request: HomologationCoverageRequest,
) -> Result<crate::homologation::matrix::CapabilityCoverageReport> {
    let report = crate::homologation::audit_capabilities(&request.matrix)?;
    if let Some(path) = request.report {
        let json = serde_json::to_vec_pretty(&report)?;
        crate::staged_output::write_file(&path, &json, true)?;
    }
    if request.require_complete {
        ensure!(
            report.complete,
            "homologation capability coverage is incomplete: {}/{} capabilities have contracts",
            report.homologated,
            report.capabilities
        );
    }
    Ok(report)
}

/// Media kind selector for the inventory workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Videos,
    Styles,
    Plaques,
    Textures,
    Fonts,
    All,
}

/// Request the media inventory from a catalog backend.
#[derive(Debug, Clone, Copy)]
pub struct ListRequest {
    pub kind: MediaKind,
}

impl ListRequest {
    /// List every media kind the build can name.
    pub fn all() -> Self {
        Self {
            kind: MediaKind::All,
        }
    }
}

/// Serializable media inventory; sections stay empty when not requested.
#[derive(Debug, Default, serde::Serialize)]
pub struct MediaInventory {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub videos: Vec<crate::media::VideoListing>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub styles: Vec<crate::media::StyleListing>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plaques: Vec<crate::media::PlaqueListing>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<crate::media::TextureListing>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fonts: Vec<crate::media::FontListing>,
}

impl MediaInventory {
    fn with(kind: MediaKind, catalog: &dyn crate::media::MediaCatalog) -> Result<Self> {
        let mut inventory = Self::default();
        let wanted = |selected: MediaKind| matches!(kind, MediaKind::All) || kind == selected;
        if wanted(MediaKind::Videos) {
            inventory.videos = catalog.videos()?;
        }
        if wanted(MediaKind::Styles) {
            inventory.styles = catalog.styles()?;
        }
        if wanted(MediaKind::Plaques) {
            inventory.plaques = catalog.plaques()?;
        }
        if wanted(MediaKind::Textures) {
            inventory.textures = catalog.textures()?;
        }
        if wanted(MediaKind::Fonts) {
            inventory.fonts = catalog.fonts()?;
        }
        Ok(inventory)
    }
}

/// List the media available to this build through the given catalog backend.
pub fn list(
    request: ListRequest,
    catalog: &dyn crate::media::MediaCatalog,
) -> Result<MediaInventory> {
    MediaInventory::with(request.kind, catalog)
}
