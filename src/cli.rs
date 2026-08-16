use std::path::PathBuf;

use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum, builder::PossibleValue};

use crate::{
    application::{
        AnalyzeRequest, FitMode, HomologateRequest, HomologationCoverageRequest, ProgressMode,
        RenderRequest, TextAlign, TitleSource, VerifyRequest, VerticalAlign,
    },
    writable_region::ResolvedWritableRegion,
};

#[derive(Debug, Parser)]
#[command(
    author,
    version,
    long_version = crate::build_info::LONG_VERSION,
    about,
    propagate_version = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Detect a title-bearing surface and create an editable scene manifest.
    CreateScene(CreateSceneArgs),
    /// Place an external plaque image and create a scene-canvas surface.
    PlaceSurface(PlaceSurfaceArgs),
    /// Analyze the source and cache reusable motion, masks, and confidence data.
    Analyze(AnalyzeArgs),
    /// Export analyzed surface motion as a reviewed trajectory.
    ExportTrajectory(ExportTrajectoryArgs),
    /// Generate a declared scene layer with an external segmentation worker.
    Segment(SegmentArgs),
    /// Render from an existing analysis cache.
    Render(Box<RenderArgs>),
    /// Verify an existing rendered video.
    Verify(VerifyArgs),
    /// Enforce a human-homologated visual acceptance contract against a render.
    Homologate(HomologateArgs),
    /// Audit which behavioral capability classes have homologated sentinels.
    HomologationCoverage(HomologationCoverageArgs),
    /// Build a human-oriented HTML report from analysis and verification diagnostics.
    Review(ReviewArgs),
}

#[derive(Debug, Args)]
pub struct CreateSceneArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Defaults to assets/scenes/<source>/scene.toml.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Replace the scene file if it already exists.
    #[arg(long)]
    pub force: bool,

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct PlaceSurfaceArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Transparent PNG to composite as the virtual plaque.
    /// It is normalized/copied into the scene directory.
    #[arg(long)]
    pub image: PathBuf,

    /// Defaults to assets/scenes/<source>/scene.toml.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Explicit [x,y,width,height] source-pixel placement. Accepts comma-separated values.
    /// Omit this to let Plaque Forge propose a quiet region automatically.
    #[arg(long, value_delimiter = ',', num_args = 4)]
    pub bounds: Vec<f64>,

    /// Coordinate space for the placed image.
    #[arg(long, value_enum, default_value_t = PlacementSpace::ScreenCanvas)]
    pub space: PlacementSpace,

    /// Fractional [left,top,right,bottom] writable inset inside the plaque PNG.
    #[arg(long, value_delimiter = ',', num_args = 4)]
    pub inset: Vec<f64>,

    /// Replace an existing injected-plaque scene.
    #[arg(long)]
    pub force: bool,

    /// Skip writing the placement preview PNG.
    #[arg(long)]
    pub no_preview: bool,

    #[arg(long, default_value = "ffmpeg")]
    pub ffmpeg: PathBuf,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct AnalyzeArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Assert that the selected writing surface contains no title/text to remove.
    /// Plaque Forge composites new typography; it does not perform inpainting.
    #[arg(long, required = true)]
    pub source_is_text_free: bool,

    /// Defaults to assets/analysis/<source>/.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Editable scene manifest. Defaults from the source path.
    #[arg(long)]
    pub scene: Option<PathBuf>,

    #[arg(long)]
    pub surface: Option<String>,

    #[arg(long, default_value_t = 0.70)]
    pub minimum_analysis_confidence: f64,

    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    /// Delete and replace the existing analysis output after a successful rebuild.
    #[arg(long)]
    pub force: bool,

    /// Return successfully without recomputing when the existing cache is current for
    /// the source, analyzer version, and scenes. Intended for high-level scripts.
    #[arg(long)]
    pub if_needed: bool,

    /// Optional ML segmentation worker used to materialize missing prompted scene layers.
    #[arg(long)]
    pub segmentation_worker: Option<PathBuf>,

    /// `auto` lets Rust choose the semantic tracker/refiner from scene intent.
    #[arg(long, default_value = "auto")]
    pub segmentation_backend: String,

    /// `auto` uses the model selected by the Rust strategy planner.
    #[arg(long, default_value = "auto")]
    pub segmentation_model: String,

    /// Execution device only. Numeric precision is controlled independently.
    #[arg(long, default_value = "auto")]
    pub segmentation_device: String,

    /// ML quality/performance policy: preview, balanced, or canonical.
    #[arg(long, default_value = "balanced")]
    pub segmentation_profile: String,

    /// Numeric policy: auto, fp32, or bf16. `auto` is resolved from the profile.
    #[arg(long, default_value = "auto")]
    pub segmentation_precision: String,

    /// Regenerate all ML layer artifacts even when their cache files already exist.
    #[arg(long)]
    pub force_ml: bool,

    #[arg(long, value_enum, default_value_t = ProgressMode::Auto)]
    pub progress: ProgressMode,

    #[arg(long, default_value_t = 500)]
    pub progress_interval_ms: u64,

    #[arg(long, default_value = "ffmpeg")]
    pub ffmpeg: PathBuf,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,

    #[arg(skip)]
    pub surface_hint: Option<[f64; 4]>,
    #[arg(skip)]
    pub surface_frame: Option<usize>,
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
    pub local_scene_radius: i32,
    #[arg(skip = 1.0)]
    pub occlusion_sensitivity: f64,
    #[arg(skip)]
    pub disable_occlusion: bool,
}

#[derive(Debug, Args)]
pub struct ExportTrajectoryArgs {
    #[arg(long)]
    pub analysis: PathBuf,

    /// Defaults beside the source scene manifest.
    #[arg(long)]
    pub output: Option<PathBuf>,

    #[arg(long)]
    pub surface: Option<String>,

    /// Replace the trajectory file if it already exists.
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

    /// Editable scene manifest. Defaults from the source path.
    #[arg(long)]
    pub scene: Option<PathBuf>,

    #[arg(long)]
    pub surface: Option<String>,

    #[arg(long)]
    pub layer: String,

    #[arg(long)]
    pub worker: PathBuf,

    #[arg(long, default_value = "auto")]
    pub backend: String,

    #[arg(long, default_value = "auto")]
    pub model: String,

    #[arg(long, default_value = "auto")]
    pub device: String,

    #[arg(long, default_value = "balanced")]
    pub profile: String,

    #[arg(long, default_value = "auto")]
    pub precision: String,

    /// Defaults to the layer directory declared by the scene.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Delete and replace the existing segmentation output after a successful run.
    #[arg(long)]
    pub force: bool,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
#[command(group(
    ArgGroup::new("title_source")
        .required(true)
        .multiple(false)
        .args(["text", "text_file"])
))]
pub struct RenderArgs {
    #[arg(long)]
    pub input: PathBuf,

    /// Rendered video. Replaces an existing file. Defaults to output/<source>.mkv.
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Generated cache. Defaults to assets/analysis/<source>/.
    #[arg(long)]
    pub analysis: Option<PathBuf>,

    /// Editable scene manifest. Defaults from the source path.
    #[arg(long)]
    pub scene: Option<PathBuf>,

    #[arg(long)]
    pub surface: Option<String>,

    /// Title text. Exactly one of --text or --text-file is required.
    #[arg(long)]
    pub text: Option<String>,

    /// UTF-8 title text file. Exactly one of --text or --text-file is required.
    #[arg(long)]
    pub text_file: Option<PathBuf>,

    /// Font file. The manifest stores its basename and SHA-256, never this workstation path.
    #[arg(long)]
    pub font: PathBuf,

    /// TOML text style. When set, it replaces the direct fill/stroke/glow/shadow paint flags.
    #[arg(long)]
    pub style_file: Option<PathBuf>,

    /// Directory for an optional, hash-verified render contact sheet.
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

    /// Self-contained FFmpeg output argument. Absolute/file-backed paths are rejected.
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
pub struct HomologateArgs {
    /// Human-reviewed acceptance contract.
    #[arg(long)]
    pub contract: PathBuf,

    /// Rendered video whose adjacent render manifest will also be checked.
    #[arg(long)]
    pub rendered: PathBuf,

    /// Optional JSON report. Replaces an existing file.
    #[arg(long)]
    pub report: Option<PathBuf>,

    /// Root directory for rich diagnostics emitted for failed semantic witnesses.
    #[arg(long)]
    pub diagnostics: Option<PathBuf>,

    #[arg(long, default_value = "ffmpeg")]
    pub ffmpeg: PathBuf,

    #[arg(long, default_value = "ffprobe")]
    pub ffprobe: PathBuf,
}

#[derive(Debug, Args)]
pub struct HomologationCoverageArgs {
    /// Capability matrix describing representative behavioral sentinels.
    #[arg(long, default_value = "assets/homologation/capabilities.toml")]
    pub matrix: PathBuf,

    /// Optional JSON coverage report.
    #[arg(long)]
    pub report: Option<PathBuf>,

    /// Fail unless every capability has a human-reviewed contract.
    #[arg(long)]
    pub require_complete: bool,
}

#[derive(Debug, Args)]
pub struct ReviewArgs {
    #[arg(long)]
    pub analysis: PathBuf,

    /// Optional human scene manifest. Used only to explain current intent in the report.
    #[arg(long)]
    pub scene: Option<PathBuf>,

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

impl From<AnalyzeArgs> for AnalyzeRequest {
    fn from(args: AnalyzeArgs) -> Self {
        Self {
            input: args.input,
            source_is_text_free: args.source_is_text_free,
            output: args.output,
            scene: args.scene,
            surface: args.surface,
            minimum_analysis_confidence: args.minimum_analysis_confidence,
            diagnostics: args.diagnostics,
            force: args.force,
            if_needed: args.if_needed,
            segmentation_worker: args.segmentation_worker,
            segmentation_backend: args.segmentation_backend,
            segmentation_model: args.segmentation_model,
            segmentation_device: args.segmentation_device,
            segmentation_profile: args.segmentation_profile,
            segmentation_precision: args.segmentation_precision,
            force_ml: args.force_ml,
            progress: args.progress,
            progress_interval_ms: args.progress_interval_ms,
            ffmpeg: args.ffmpeg,
            ffprobe: args.ffprobe,
            surface_hint: args.surface_hint,
            surface_frame: args.surface_frame,
            writable_region_hint: args.writable_region_hint,
            anchor_interval: args.anchor_interval,
            tracking_inertia: args.tracking_inertia,
            candidate_samples: args.candidate_samples,
            extraction_samples: args.extraction_samples,
            local_scene_radius: args.local_scene_radius,
            occlusion_sensitivity: args.occlusion_sensitivity,
            disable_occlusion: args.disable_occlusion,
        }
    }
}

impl RenderArgs {
    pub fn into_request(self, analysis: PathBuf, output: PathBuf) -> RenderRequest {
        RenderRequest {
            input: self.input,
            analysis,
            scene: self.scene,
            surface: self.surface,
            title: match (self.text, self.text_file) {
                (Some(text), None) => TitleSource::Text(text),
                (None, Some(path)) => TitleSource::File(path),
                _ => unreachable!("clap title_source group guarantees exactly one title source"),
            },
            font: self.font,
            style_file: self.style_file,
            output,
            diagnostics: self.diagnostics,
            fit: self.fit,
            font_size: self.font_size,
            supersampling: self.supersampling,
            target_fill: self.target_fill,
            max_lines: self.max_lines,
            padding: self.padding,
            line_height: self.line_height,
            stroke_width: self.stroke_width,
            text_color: self.text_color,
            stroke_color: self.stroke_color,
            glow_color: self.glow_color,
            glow_radius: self.glow_radius,
            shadow_offset_x: self.shadow_offset_x,
            shadow_offset_y: self.shadow_offset_y,
            shadow_blur_radius: self.shadow_blur_radius,
            shadow_color: self.shadow_color,
            text_align: self.text_align,
            vertical_align: self.vertical_align,
            encoder_args: self.encoder_args,
            progress: self.progress,
            progress_interval_ms: self.progress_interval_ms,
            ffmpeg: self.ffmpeg,
            ffprobe: self.ffprobe,
        }
    }
}

impl From<VerifyArgs> for VerifyRequest {
    fn from(args: VerifyArgs) -> Self {
        Self {
            analysis: args.analysis,
            rendered: args.rendered,
            original: args.original,
            report: args.report,
            diagnostics: args.diagnostics,
            minimum_score: args.minimum_score,
            progress: args.progress,
            progress_interval_ms: args.progress_interval_ms,
            ffmpeg: args.ffmpeg,
            ffprobe: args.ffprobe,
        }
    }
}

impl From<HomologateArgs> for HomologateRequest {
    fn from(args: HomologateArgs) -> Self {
        Self {
            contract: args.contract,
            rendered: args.rendered,
            report: args.report,
            diagnostics: args.diagnostics,
            ffmpeg: args.ffmpeg,
            ffprobe: args.ffprobe,
        }
    }
}

impl From<HomologationCoverageArgs> for HomologationCoverageRequest {
    fn from(args: HomologationCoverageArgs) -> Self {
        Self {
            matrix: args.matrix,
            report: args.report,
            require_complete: args.require_complete,
        }
    }
}

macro_rules! impl_value_enum {
    ($type:ty, [$($variant:path => $name:literal),+ $(,)?]) => {
        impl ValueEnum for $type {
            fn value_variants<'a>() -> &'a [Self] {
                const VARIANTS: &[$type] = &[$($variant),+];
                VARIANTS
            }

            fn to_possible_value(&self) -> Option<PossibleValue> {
                Some(match *self {
                    $($variant => PossibleValue::new($name)),+
                })
            }
        }
    };
}

impl_value_enum!(
    FitMode,
    [
        FitMode::Maximize => "maximize",
        FitMode::Balanced => "balanced",
        FitMode::Artistic => "artistic",
        FitMode::Fixed => "fixed",
    ]
);
impl_value_enum!(
    TextAlign,
    [
        TextAlign::Left => "left",
        TextAlign::Center => "center",
        TextAlign::Right => "right",
    ]
);
impl_value_enum!(
    VerticalAlign,
    [
        VerticalAlign::Top => "top",
        VerticalAlign::Center => "center",
        VerticalAlign::Bottom => "bottom",
    ]
);
impl_value_enum!(
    ProgressMode,
    [
        ProgressMode::Auto => "auto",
        ProgressMode::Always => "always",
        ProgressMode::Never => "never",
    ]
);

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PlacementSpace {
    /// Attach the image to a physical planar surface in the scene.
    ScenePlane,
    /// Place the image intentionally in screen coordinates.
    ScreenCanvas,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_arguments(title: &[&str]) -> Vec<String> {
        [
            "plaque-forge",
            "render",
            "--input",
            "source.mp4",
            "--font",
            "font.ttf",
        ]
        .into_iter()
        .chain(title.iter().copied())
        .map(str::to_string)
        .collect()
    }

    #[test]
    fn render_requires_exactly_one_title_source() {
        assert!(Cli::try_parse_from(render_arguments(&[])).is_err());
        assert!(Cli::try_parse_from(render_arguments(&["--text", "Title"])).is_ok());
        assert!(Cli::try_parse_from(render_arguments(&["--text-file", "title.txt"])).is_ok());
        assert!(
            Cli::try_parse_from(render_arguments(&[
                "--text",
                "Title",
                "--text-file",
                "title.txt",
            ]))
            .is_err()
        );
    }

    #[test]
    fn cli_non_render_defaults_match_programmatic_request_defaults() {
        let analyze_cli = Cli::try_parse_from([
            "plaque-forge",
            "analyze",
            "--input",
            "source.mp4",
            "--source-is-text-free",
        ])
        .unwrap();
        let Command::Analyze(analyze_args) = analyze_cli.command else {
            panic!("analyze arguments produced a different command")
        };
        let actual: AnalyzeRequest = analyze_args.into();
        let expected = AnalyzeRequest::text_free("source.mp4");
        assert_eq!(
            actual.minimum_analysis_confidence,
            expected.minimum_analysis_confidence
        );
        assert_eq!(actual.segmentation_backend, expected.segmentation_backend);
        assert_eq!(actual.segmentation_model, expected.segmentation_model);
        assert_eq!(actual.segmentation_device, expected.segmentation_device);
        assert_eq!(actual.segmentation_profile, expected.segmentation_profile);
        assert_eq!(
            actual.segmentation_precision,
            expected.segmentation_precision
        );
        assert_eq!(actual.progress, expected.progress);
        assert_eq!(actual.progress_interval_ms, expected.progress_interval_ms);
        assert_eq!(actual.ffmpeg, expected.ffmpeg);
        assert_eq!(actual.ffprobe, expected.ffprobe);
        assert_eq!(actual.anchor_interval, expected.anchor_interval);
        assert_eq!(actual.tracking_inertia, expected.tracking_inertia);
        assert_eq!(actual.candidate_samples, expected.candidate_samples);
        assert_eq!(actual.extraction_samples, expected.extraction_samples);
        assert_eq!(actual.local_scene_radius, expected.local_scene_radius);
        assert_eq!(actual.occlusion_sensitivity, expected.occlusion_sensitivity);

        let verify_cli = Cli::try_parse_from([
            "plaque-forge",
            "verify",
            "--analysis",
            "analysis",
            "--rendered",
            "output.mkv",
        ])
        .unwrap();
        let Command::Verify(verify_args) = verify_cli.command else {
            panic!("verify arguments produced a different command")
        };
        let actual: VerifyRequest = verify_args.into();
        let expected = VerifyRequest::new("analysis", "output.mkv");
        assert_eq!(actual.minimum_score, expected.minimum_score);
        assert_eq!(actual.progress, expected.progress);
        assert_eq!(actual.progress_interval_ms, expected.progress_interval_ms);
        assert_eq!(actual.ffmpeg, expected.ffmpeg);
        assert_eq!(actual.ffprobe, expected.ffprobe);

        let homologate_cli = Cli::try_parse_from([
            "plaque-forge",
            "homologate",
            "--contract",
            "contract.toml",
            "--rendered",
            "output.mkv",
        ])
        .unwrap();
        let Command::Homologate(homologate_args) = homologate_cli.command else {
            panic!("homologate arguments produced a different command")
        };
        let actual: HomologateRequest = homologate_args.into();
        let expected = HomologateRequest::new("contract.toml", "output.mkv");
        assert_eq!(actual.ffmpeg, expected.ffmpeg);
        assert_eq!(actual.ffprobe, expected.ffprobe);
        assert!(actual.diagnostics.is_none());

        let coverage_cli = Cli::try_parse_from(["plaque-forge", "homologation-coverage"]).unwrap();
        let Command::HomologationCoverage(coverage_args) = coverage_cli.command else {
            panic!("coverage arguments produced a different command")
        };
        let actual: HomologationCoverageRequest = coverage_args.into();
        let expected = HomologationCoverageRequest::new("assets/homologation/capabilities.toml");
        assert_eq!(actual.matrix, expected.matrix);
        assert_eq!(actual.require_complete, expected.require_complete);
    }

    #[test]
    fn cli_render_defaults_match_programmatic_request_defaults() {
        let cli = Cli::try_parse_from(render_arguments(&["--text", "Title"])).unwrap();
        let Command::Render(args) = cli.command else {
            panic!("render arguments produced a different command")
        };
        let actual = (*args).into_request("analysis".into(), "output.mkv".into());
        let expected = RenderRequest::new(
            "source.mp4",
            "analysis",
            "output.mkv",
            TitleSource::Text("Title".to_string()),
            "font.ttf",
        );

        assert_eq!(actual.fit, expected.fit);
        assert_eq!(actual.supersampling, expected.supersampling);
        assert_eq!(actual.target_fill, expected.target_fill);
        assert_eq!(actual.max_lines, expected.max_lines);
        assert_eq!(actual.padding, expected.padding);
        assert_eq!(actual.line_height, expected.line_height);
        assert_eq!(actual.stroke_width, expected.stroke_width);
        assert_eq!(actual.text_color, expected.text_color);
        assert_eq!(actual.stroke_color, expected.stroke_color);
        assert_eq!(actual.glow_color, expected.glow_color);
        assert_eq!(actual.glow_radius, expected.glow_radius);
        assert_eq!(actual.shadow_offset_x, expected.shadow_offset_x);
        assert_eq!(actual.shadow_offset_y, expected.shadow_offset_y);
        assert_eq!(actual.shadow_blur_radius, expected.shadow_blur_radius);
        assert_eq!(actual.shadow_color, expected.shadow_color);
        assert_eq!(actual.text_align, expected.text_align);
        assert_eq!(actual.vertical_align, expected.vertical_align);
        assert_eq!(actual.progress, expected.progress);
        assert_eq!(actual.progress_interval_ms, expected.progress_interval_ms);
        assert_eq!(actual.ffmpeg, expected.ffmpeg);
        assert_eq!(actual.ffprobe, expected.ffprobe);
    }
}
