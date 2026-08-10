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
    /// Detect plaque candidates and create an editable metadata sidecar.
    Init(InitArgs),
    /// Analyze a text-free plaque video and cache motion, masks, and confidence data.
    Analyze(AnalyzeArgs),
    /// Export a title-pack trajectory as an editable motion track.
    ExportTrack(ExportTrackArgs),
    /// Render new typography using an existing title-pack.
    Render(RenderArgs),
    /// Compare a rendered video with the source and issue sanity scores and remedies.
    Verify(VerifyArgs),
    /// Convenience command: analyze if needed, render, then verify.
    Replace(Box<ReplaceArgs>),
}

#[derive(Debug, Args)]
pub struct InitArgs {
    /// Source video that the sidecar describes.
    #[arg(long)]
    pub input: PathBuf,

    /// Destination. Defaults to <input-stem>.plaque.toml next to the video.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Replace an existing sidecar after explicit review.
    #[arg(long)]
    pub force: bool,

    /// Optional directory for candidate ranking and an annotated reference frame.
    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    /// Candidate detector used to create the initial proposal.
    #[arg(long, value_enum, default_value_t = CandidateDetector::Ensemble)]
    pub detector: CandidateDetector,

    /// Number of sampled frames used during candidate ranking.
    #[arg(long, default_value_t = 24)]
    pub candidate_samples: usize,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct ExportTrackArgs {
    /// Existing title-pack containing the trajectory to review.
    #[arg(long)]
    pub analysis: PathBuf,

    /// Commented TOML track to create.
    #[arg(long)]
    pub output: PathBuf,

    /// Plaque id written into the exported track. Defaults to the analyzed plaque.
    #[arg(long)]
    pub plaque: Option<String>,

    /// Replace an existing human track after explicit review.
    #[arg(long)]
    pub force: bool,

    /// Mark every exported frame as reviewed and authoritative.
    #[arg(long)]
    pub locked: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CandidateDetector {
    Ensemble,
    Geometry,
    Color,
    Text,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum MotionModel {
    Adaptive,
    Similarity,
    Affine,
    Projective,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum LoopClosure {
    Auto,
    On,
    Off,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FitMode {
    /// Select the largest font that fits every glyph and effect inside the content mask.
    Maximize,
    /// Prefer a target occupied-area ratio when several layouts fit.
    Balanced,
    /// Use --font-size exactly and fail if it does not fit.
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

#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Output directory. The `.titlepack` suffix is conventional, not mandatory.
    #[arg(long)]
    pub output: PathBuf,

    /// Human-owned TOML metadata. When omitted, <input-stem>.plaque.toml is loaded if present.
    #[arg(long)]
    pub metadata: Option<PathBuf>,

    /// Named plaque to analyze when metadata declares more than one.
    #[arg(long)]
    pub plaque: Option<String>,

    /// Optional x,y,width,height hint in source pixels. It identifies the plaque;
    /// tracking and structural refinement remain automatic.
    #[arg(long, value_parser = parse_rect)]
    pub plaque_hint: Option<[f64; 4]>,

    /// Frame on which --plaque-hint is defined. Defaults to frame 0.
    #[arg(long, requires = "plaque_hint")]
    pub plaque_frame: Option<usize>,

    /// Human-owned TOML quad track. Overrides the track referenced by metadata.
    #[arg(long, conflicts_with = "track_csv")]
    pub motion_track: Option<PathBuf>,

    /// Optional supervised plaque quad keyframes in frame,tl_x,tl_y,tr_x,tr_y,br_x,br_y,bl_x,bl_y CSV format.
    #[arg(long)]
    pub track_csv: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = CandidateDetector::Ensemble)]
    pub detector: CandidateDetector,

    #[arg(long, value_enum, default_value_t = MotionModel::Adaptive)]
    pub motion_model: MotionModel,

    #[arg(long, value_enum, default_value_t = LoopClosure::Auto)]
    pub loop_closure: LoopClosure,

    /// Refresh interval for the adaptive feature reference. Every frame is still measured.
    #[arg(long, default_value_t = 24)]
    pub anchor_interval: usize,

    /// Zero-phase temporal regularization. 0 disables it; values near 1 are stronger.
    #[arg(long, default_value_t = 0.35)]
    pub tracking_inertia: f64,

    /// Number of sampled frames used during candidate ranking.
    #[arg(long, default_value_t = 24)]
    pub candidate_samples: usize,

    /// Frames rectified into canonical plaque space for mask and structure analysis.
    #[arg(long, default_value_t = 72)]
    pub extraction_samples: usize,

    /// Maximum canonical-space local correction around the scene-motion prediction.
    #[arg(long, default_value_t = 12)]
    pub local_refinement_radius: i32,

    #[arg(long, default_value_t = 1.0)]
    pub occlusion_sensitivity: f64,

    /// Minimum aggregate confidence required to commit a reusable title-pack.
    #[arg(long, default_value_t = 0.70)]
    pub minimum_analysis_confidence: f64,

    /// Commit a low-confidence title-pack for diagnostics. Rendering still reports the warning.
    #[arg(long)]
    pub allow_low_confidence: bool,

    #[arg(long)]
    pub disable_occlusion: bool,

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    /// Replace an existing analysis directory rather than refusing to overwrite it.
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
}

#[derive(Debug, Args)]
pub struct RenderArgs {
    #[arg(long)]
    pub analysis: PathBuf,

    #[arg(long)]
    pub text: Option<String>,

    #[arg(long, conflicts_with = "text")]
    pub text_file: Option<PathBuf>,

    #[arg(long)]
    pub font: PathBuf,

    #[arg(long)]
    pub output: PathBuf,

    /// Optional directory for render previews and diagnostics.
    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = FitMode::Maximize)]
    pub fit: FitMode,

    /// Required only with --fit fixed. In other modes it acts as an upper bound.
    #[arg(long)]
    pub font_size: Option<f32>,

    #[arg(long, default_value_t = 4)]
    pub supersampling: u32,

    /// Soft preference used by --fit balanced; ignored by --fit maximize.
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

    /// Glow radius in canonical plaque pixels.
    #[arg(long, default_value_t = 4)]
    pub glow_radius: u32,

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

    /// Directory for exact worst-frame verification images.
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
pub struct ReplaceArgs {
    #[arg(long)]
    pub input: PathBuf,

    #[arg(long)]
    pub output: PathBuf,

    #[arg(long)]
    pub analysis: Option<PathBuf>,

    /// Human-owned TOML metadata. When omitted, <input-stem>.plaque.toml is loaded if present.
    #[arg(long)]
    pub metadata: Option<PathBuf>,

    /// Named plaque to analyze when metadata declares more than one.
    #[arg(long)]
    pub plaque: Option<String>,

    #[arg(long)]
    pub text: Option<String>,

    #[arg(long, conflicts_with = "text")]
    pub text_file: Option<PathBuf>,

    #[arg(long)]
    pub font: PathBuf,

    /// Optional x,y,width,height plaque bounds in source pixels. Tracking remains automatic.
    #[arg(long, value_parser = parse_rect)]
    pub plaque_hint: Option<[f64; 4]>,

    /// Frame on which --plaque-hint is defined. Defaults to frame 0.
    #[arg(long, requires = "plaque_hint")]
    pub plaque_frame: Option<usize>,

    /// Human-owned TOML quad track. Overrides the track referenced by metadata.
    #[arg(long, conflicts_with = "track_csv")]
    pub motion_track: Option<PathBuf>,

    #[arg(long)]
    pub track_csv: Option<PathBuf>,

    #[arg(long, value_enum, default_value_t = CandidateDetector::Ensemble)]
    pub detector: CandidateDetector,

    #[arg(long, value_enum, default_value_t = MotionModel::Adaptive)]
    pub motion_model: MotionModel,

    #[arg(long, value_enum, default_value_t = LoopClosure::Auto)]
    pub loop_closure: LoopClosure,

    /// Refresh interval for the adaptive feature reference. Every frame is still measured.
    #[arg(long, default_value_t = 24)]
    pub anchor_interval: usize,

    /// Zero-phase smoothing strength. Default 0.35; higher is smoother, lower follows faster motion.
    #[arg(long, default_value_t = 0.35)]
    pub tracking_inertia: f64,

    #[arg(long, default_value_t = 72)]
    pub extraction_samples: usize,

    #[arg(long, default_value_t = 12)]
    pub local_refinement_radius: i32,

    #[arg(long, default_value_t = 1.0)]
    pub occlusion_sensitivity: f64,

    #[arg(long, default_value_t = 0.70)]
    pub minimum_analysis_confidence: f64,

    #[arg(long)]
    pub allow_low_confidence: bool,

    #[arg(long)]
    pub disable_occlusion: bool,

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

    /// Discard a compatible cached title-pack and analyze the source again.
    #[arg(long)]
    pub reanalyze: bool,

    /// Render without running the verification gate.
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

impl ReplaceArgs {
    pub fn as_analyze_args(&self, output: PathBuf) -> AnalyzeArgs {
        AnalyzeArgs {
            input: self.input.clone(),
            output,
            metadata: self.metadata.clone(),
            plaque: self.plaque.clone(),
            plaque_hint: self.plaque_hint,
            plaque_frame: self.plaque_frame,
            motion_track: self.motion_track.clone(),
            track_csv: self.track_csv.clone(),
            detector: self.detector,
            motion_model: self.motion_model,
            loop_closure: self.loop_closure,
            anchor_interval: self.anchor_interval,
            tracking_inertia: self.tracking_inertia,
            candidate_samples: 24,
            extraction_samples: self.extraction_samples,
            local_refinement_radius: self.local_refinement_radius,
            occlusion_sensitivity: self.occlusion_sensitivity,
            minimum_analysis_confidence: self.minimum_analysis_confidence,
            allow_low_confidence: self.allow_low_confidence,
            disable_occlusion: self.disable_occlusion,
            diagnostics: self.diagnostics.clone(),
            force: self.reanalyze,
            progress: self.progress,
            progress_interval_ms: self.progress_interval_ms,
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
        }
    }

    pub fn as_render_args(&self, analysis: PathBuf) -> RenderArgs {
        RenderArgs {
            analysis,
            text: self.text.clone(),
            text_file: self.text_file.clone(),
            font: self.font.clone(),
            output: self.output.clone(),
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

    pub fn as_verify_args(&self, analysis: PathBuf) -> VerifyArgs {
        VerifyArgs {
            analysis,
            rendered: self.output.clone(),
            original: Some(self.input.clone()),
            report: Some(self.output.with_extension("verification.json")),
            diagnostics: self.diagnostics.clone(),
            minimum_score: 0.95,
            progress: self.progress,
            progress_interval_ms: self.progress_interval_ms,
            ffmpeg: self.ffmpeg.clone(),
            ffprobe: self.ffprobe.clone(),
        }
    }
}

fn parse_rect(value: &str) -> Result<[f64; 4], String> {
    let values = value
        .split(',')
        .map(str::trim)
        .map(str::parse::<f64>)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("invalid rectangle: {error}"))?;

    values
        .try_into()
        .map_err(|_| "rectangle must be x,y,width,height".to_string())
}
