use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

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
    /// Analyze when needed, render, and verify the result.
    Render(Box<RenderArgs>),
    /// Verify an existing rendered video.
    Verify(VerifyArgs),
}

#[derive(Debug, Args)]
pub struct RefineArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Defaults to assets/refinements/<source>/refinement.toml.
    #[arg(long)]
    pub output: Option<PathBuf>,

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

    #[arg(long)]
    pub force: bool,

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

    #[arg(long)]
    pub force: bool,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Defaults to output/<source>.mkv.
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

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = FitMode::Maximize)]
    pub fit: FitMode,

    #[arg(long)]
    pub font_size: Option<f32>,

    #[arg(long, default_value_t = 4)]
    pub supersampling: u32,

    #[arg(long, default_value_t = 0.82)]
    pub target_fill: f32,

    #[arg(long, default_value_t = 3)]
    pub max_lines: usize,

    #[arg(long, default_value_t = 0.05)]
    pub padding: f32,

    #[arg(long, default_value_t = 1.16)]
    pub line_height: f32,

    #[arg(long, default_value_t = 0.0)]
    pub stroke_width: f32,

    #[arg(long, default_value = "#EBFFFFFF")]
    pub text_color: String,

    #[arg(long, default_value = "#03181ED2")]
    pub stroke_color: String,

    #[arg(long, default_value = "#69F2FA48")]
    pub glow_color: String,

    #[arg(long, default_value_t = 4)]
    pub glow_radius: u32,

    #[arg(long, value_enum, default_value_t = TextAlign::Center)]
    pub text_align: TextAlign,

    #[arg(long, value_enum, default_value_t = VerticalAlign::Center)]
    pub vertical_align: VerticalAlign,

    #[arg(long)]
    pub reanalyze: bool,

    #[arg(long)]
    pub skip_verify: bool,

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
    pub analysis: PathBuf,
    pub text: Option<String>,
    pub text_file: Option<PathBuf>,
    pub font: PathBuf,
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

impl RenderArgs {
    pub fn as_analyze_args(&self, output: PathBuf) -> AnalyzeArgs {
        AnalyzeArgs {
            input: self.input.clone(),
            output: Some(output),
            refinement: self.refinement.clone(),
            plaque: self.plaque.clone(),
            minimum_analysis_confidence: 0.70,
            allow_low_confidence: false,
            diagnostics: self.diagnostics.clone(),
            force: self.reanalyze,
            progress: self.progress,
            progress_interval_ms: self.progress_interval_ms,
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
            plaque_hint: None,
            plaque_frame: None,
            anchor_interval: 24,
            tracking_inertia: 0.35,
            candidate_samples: 24,
            extraction_samples: 72,
            local_refinement_radius: 12,
            occlusion_sensitivity: 1.0,
            disable_occlusion: false,
        }
    }

    pub fn as_compose_args(&self, analysis: PathBuf, output: PathBuf) -> ComposeArgs {
        ComposeArgs {
            analysis,
            text: self.text.clone(),
            text_file: self.text_file.clone(),
            font: self.font.clone(),
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
            text_align: self.text_align,
            vertical_align: self.vertical_align,
            encoder_args: self.encoder_args.clone(),
            progress: self.progress,
            progress_interval_ms: self.progress_interval_ms,
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
        }
    }

    pub fn as_verify_args(&self, analysis: PathBuf, output: PathBuf) -> VerifyArgs {
        VerifyArgs {
            analysis,
            rendered: output.clone(),
            original: Some(self.input.clone()),
            report: Some(output.with_extension("verification.json")),
            diagnostics: self.diagnostics.clone(),
            minimum_score: 0.95,
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
