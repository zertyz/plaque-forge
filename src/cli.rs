use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::writable_region::ResolvedWritableRegion;

#[derive(Debug, Parser)]
#[command(author, version, long_version = crate::build_info::LONG_VERSION, about, propagate_version = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Detect a plaque and create an editable refinement manifest.
    Refine(RefineArgs),
    /// Analyze the source and cache reusable motion, masks, and confidence data.
    Analyze(AnalyzeArgs),
    /// Export analyzed plaque motion as an editable refinement track.
    ExportMotion(ExportMotionArgs),
    /// Generate a declared refinement layer with an external segmentation worker.
    Segment(SegmentArgs),
    /// Render from an existing analysis cache.
    Render(Box<RenderArgs>),
    /// Verify an existing rendered video.
    Verify(VerifyArgs),
    /// Build a human-oriented HTML report from analysis and verification diagnostics.
    Review(ReviewArgs),
}

#[derive(Debug, Args)]
pub struct RefineArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Defaults to assets/refinements/<source>/refinement.toml.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Replace the refinement file if it already exists.
    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Defaults to assets/analysis/<source>/.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Editable refinement manifest. Defaults from the source path.
    #[arg(long)]
    pub refinement: Option<PathBuf>,

    #[arg(long)]
    pub plaque: Option<String>,

    #[arg(long, default_value_t = 0.70)]
    pub minimum_analysis_confidence: f64,

    #[arg(long)]
    pub allow_low_confidence: bool,

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    /// Delete and replace the existing analysis output after a successful rebuild.
    #[arg(long)]
    pub force: bool,

    /// Return successfully without recomputing when the existing cache is current for
    /// the source, analyzer version, and refinements. Intended for high-level scripts.
    #[arg(long)]
    pub if_needed: bool,

    /// Optional ML segmentation worker used to materialize missing prompted refinement layers.
    #[arg(long)]
    pub segmentation_worker: Option<PathBuf>,

    #[arg(long, default_value = "sam2-cutie-vitmatte")]
    pub segmentation_backend: String,

    #[arg(long, default_value = "facebook/sam2.1-hiera-large")]
    pub segmentation_model: String,

    #[arg(long, default_value = "auto")]
    pub segmentation_device: String,

    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    pub progress: ProgressMode,

    #[arg(long, default_value_t = 500)]
    pub progress_interval_ms: u64,

    #[arg(long, default_value = "ffmpeg")]
    pub ffmpeg: PathBuf,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,

    #[arg(skip)]
    pub plaque_hint: Option<[f64; 4]>,
    #[arg(skip)]
    pub plaque_frame: Option<usize>,
    #[arg(skip)]
    pub writable_region_hint: Option<ResolvedWritableRegion>,
    #[arg(skip = 24usize)]
    pub anchor_interval: usize,
    #[arg(skip = 0.35)]
    pub tracking_inertia: f64,
    #[arg(skip = 24usize)]
    pub candidate_samples: usize,
    #[arg(skip = 72usize)]
    pub extraction_samples: usize,
    #[arg(skip = 12)]
    pub local_refinement_radius: i32,
    #[arg(skip = 1.0)]
    pub occlusion_sensitivity: f64,
    #[arg(skip)]
    pub disable_occlusion: bool,
}

#[derive(Debug, Args)]
pub struct ExportMotionArgs {
    #[arg(long)]
    pub analysis: PathBuf,

    /// Defaults beside the source refinement manifest.
    #[arg(long)]
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub plaque: Option<String>,

    /// Replace the motion refinement file if it already exists.
    #[arg(long)]
    pub force: bool,

    /// Make every exported frame authoritative.
    #[arg(long)]
    pub locked: bool,
}

#[derive(Debug, Args)]
pub struct SegmentArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Editable refinement manifest. Defaults from the source path.
    #[arg(long)]
    pub refinement: Option<PathBuf>,

    #[arg(long)]
    pub plaque: Option<String>,

    #[arg(long)]
    pub layer: String,

    #[arg(long)]
    pub worker: PathBuf,

    #[arg(long)]
    pub backend: String,

    #[arg(long)]
    pub model: String,

    #[arg(long, default_value = "auto")]
    pub device: String,

    /// Defaults to the layer directory declared by the refinement.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Delete and replace the existing segmentation output after a successful run.
    #[arg(long)]
    pub force: bool,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Rendered video. Replaces an existing file. Defaults to output/<source>.mkv.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Generated cache. Defaults to assets/analysis/<source>/.
    #[arg(long)]
    pub analysis: Option<PathBuf>,

    /// Editable refinement manifest. Defaults from the source path.
    #[arg(long)]
    pub refinement: Option<PathBuf>,

    #[arg(long)]
    pub plaque: Option<String>,

    #[arg(long)]
    pub text: Option<String>,

    #[arg(long, conflicts_with = "text")]
    pub text_file: Option<PathBuf>,

    #[arg(long)]
    pub font: PathBuf,

    /// TOML text style. When set, it replaces the direct fill/stroke/glow/shadow paint flags.
    #[arg(long)]
    pub style_file: Option<PathBuf>,

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = FitMode::Artistic)]
    pub fit: FitMode,

    #[arg(long)]
    pub font_size: Option<f32>,

    #[arg(long, default_value_t = 4)]
    pub supersampling: u32,

    #[arg(long, default_value_t = 0.94)]
    pub target_fill: f32,

    #[arg(long, default_value_t = 5)]
    pub max_lines: usize,

    #[arg(long, default_value_t = 0.03)]
    pub padding: f32,

    #[arg(long, default_value_t = 1.08)]
    pub line_height: f32,

    #[arg(long, default_value_t = 0.0)]
    pub stroke_width: f32,

    #[arg(long, default_value = "#EBFFFFFF")]
    pub text_color: String,

    #[arg(long, default_value = "#03181ED2")]
    pub stroke_color: String,

    #[arg(long, default_value = "#69F2FA90")]
    pub glow_color: String,

    #[arg(long, default_value_t = 10)]
    pub glow_radius: u32,

    /// Horizontal shadow offset as a fraction of the fitted font size.
    #[arg(long, default_value_t = 0.0)]
    pub shadow_offset_x: f32,

    /// Vertical shadow offset as a fraction of the fitted font size.
    #[arg(long, default_value_t = 0.0)]
    pub shadow_offset_y: f32,

    #[arg(long, default_value_t = 0)]
    pub shadow_blur_radius: u32,

    #[arg(long, default_value = "#00000000")]
    pub shadow_color: String,

    #[arg(long, value_enum, default_value_t = TextAlign::Center)]
    pub text_align: TextAlign,

    #[arg(long, value_enum, default_value_t = VerticalAlign::Center)]
    pub vertical_align: VerticalAlign,

    #[arg(long = "encoder-arg", allow_hyphen_values = true)]
    pub encoder_args: Vec<String>,

    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    pub progress: ProgressMode,

    #[arg(long, default_value_t = 500)]
    pub progress_interval_ms: u64,

    #[arg(long, default_value = "ffmpeg")]
    pub ffmpeg: PathBuf,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug)]
pub struct ComposeArgs {
    pub input: PathBuf,
    pub analysis: PathBuf,
    pub refinement: Option<PathBuf>,
    pub plaque: Option<String>,
    pub text: Option<String>,
    pub text_file: Option<PathBuf>,
    pub font: PathBuf,
    pub style_file: Option<PathBuf>,
    pub output: PathBuf,
    pub diagnostics: Option<PathBuf>,
    pub fit: FitMode,
    pub font_size: Option<f32>,
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

#[derive(Debug, Args)]
pub struct VerifyArgs {
    #[arg(long)]
    pub analysis: PathBuf,

    #[arg(long)]
    pub rendered: PathBuf,

    #[arg(long)]
    pub original: Option<PathBuf>,

    /// Verification report. Replaces an existing file.
    #[arg(long)]
    pub report: Option<PathBuf>,

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    #[arg(long, default_value_t = 0.95)]
    pub minimum_score: f64,

    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    pub progress: ProgressMode,

    #[arg(long, default_value_t = 500)]
    pub progress_interval_ms: u64,

    #[arg(long, default_value = "ffmpeg")]
    pub ffmpeg: PathBuf,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct ReviewArgs {
    #[arg(long)]
    pub analysis: PathBuf,

    /// Optional verification JSON produced by `plaque-forge verify`.
    #[arg(long)]
    pub verification: Option<PathBuf>,

    /// Optional render manifest produced beside a rendered video.
    #[arg(long)]
    pub render_manifest: Option<PathBuf>,

    /// Defaults to <analysis>/diagnostics/review.html.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

impl RenderArgs {
    pub fn as_compose_args(&self, analysis: PathBuf, output: PathBuf) -> ComposeArgs {
        ComposeArgs {
            input: self.input.clone(),
            analysis,
            refinement: self.refinement.clone(),
            plaque: self.plaque.clone(),
            text: self.text.clone(),
            text_file: self.text_file.clone(),
            font: self.font.clone(),
            style_file: self.style_file.clone(),
            output,
            diagnostics: self.diagnostics.clone(),
            fit: self.fit,
            font_size: self.font_size,
            supersampling: self.supersampling,
            target_fill: self.target_fill,
            max_lines: self.max_lines,
            padding: self.padding,
            line_height: self.line_height,
            stroke_width: self.stroke_width,
            text_color: self.text_color.clone(),
            stroke_color: self.stroke_color.clone(),
            glow_color: self.glow_color.clone(),
            glow_radius: self.glow_radius,
            shadow_offset_x: self.shadow_offset_x,
            shadow_offset_y: self.shadow_offset_y,
            shadow_blur_radius: self.shadow_blur_radius,
            shadow_color: self.shadow_color.clone(),
            text_align: self.text_align,
            vertical_align: self.vertical_align,
            encoder_args: self.encoder_args.clone(),
            progress: self.progress,
            progress_interval_ms: self.progress_interval_ms,
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FitMode {
    Maximize,
    Balanced,
    /// Search explicit word-boundary line breaks and score visual balance before fitting.
    Artistic,
    Fixed,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum VerticalAlign {
    Top,
    Center,
    Bottom,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum ProgressMode {
    Auto,
    Always,
    Never,
}
