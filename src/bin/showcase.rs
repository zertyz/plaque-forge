//! plaque-forge-showcase — interactive stage for text styles over live video.
//!
//! Plays every analyzed asset in a loop and composites a live-editable title
//! through the same pipeline the CLI renders with. Interaction logic lives in
//! `plaque_forge::showcase` modules; this binary owns decoding, scaling,
//! OpenCV highgui plumbing, and key dispatch.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result};
use clap::Parser;
use opencv::core::Point as CvPoint;
use opencv::core::{CV_8UC4, Mat, Scalar, Vector};
use opencv::imgcodecs::imencode;
use opencv::imgproc::{FONT_HERSHEY_SIMPLEX, LINE_8, put_text, rectangle_points};
use opencv::prelude::*;

use plaque_forge::analysis::{
    Analysis, CONTENT_MASK_FILE, LayerAsset, resolve_asset, sequence_path,
};
use plaque_forge::media::{
    FilesystemCatalog, FontListing, MediaCatalog, curated,
    fonts::{FamilyIndex, SystemFonts},
};
use plaque_forge::render::compositor::{FrameCompositor, load_injected_surface};
use plaque_forge::render::{effects, load_full_luma};
use plaque_forge::scene::{LayerArtifactKind, LayerCoordinates};
use plaque_forge::showcase::composer::{Direction as EditDirection, EditModel};
use plaque_forge::showcase::driver::{Driver, Script};
use plaque_forge::showcase::fonts::FontPicker;
use plaque_forge::showcase::keys::{Key, normalize};
use plaque_forge::showcase::quality::Tier;
use plaque_forge::showcase::state::{DemoState, Mode};
use plaque_forge::surface::Surface;
use plaque_forge::video::{self, DecodeAhead};
use plaque_forge::workspace;

const WINDOW: &str = "plaque-forge-showcase";
/// Set once from --headless so every drawer skips real display.
static HEADLESS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn headless() -> bool {
    HEADLESS.load(std::sync::atomic::Ordering::Relaxed)
}
const DEFAULT_TEXT: &str = "Press ENTER to change this text";
const YELLOW: Scalar = Scalar::new(0.0, 255.0, 255.0, 255.0);
const WHITE: Scalar = Scalar::new(255.0, 255.0, 255.0, 255.0);
const BLACK: Scalar = Scalar::new(0.0, 0.0, 0.0, 255.0);
const GREEN_FILL: [u8; 3] = [8, 220, 8];

/// Command-line options.
#[derive(Debug, Parser)]
#[command(name = "plaque-forge-showcase", about, version)]
struct Args {
    /// Repository root containing assets/, styles/, fonts/.
    #[arg(long, default_value = ".")]
    root: PathBuf,

    /// Initial font file (defaults to the pinned reference font).
    #[arg(long)]
    font: Option<PathBuf>,

    /// Initial style preset name from styles/.
    #[arg(long)]
    style: Option<String>,

    /// Display width in pixels; height follows the video aspect.
    #[arg(long, default_value_t = 960)]
    width: u32,

    /// Headless self-check: composite N frames of one asset and exit.
    #[arg(long)]
    smoke: Option<usize>,

    /// Scripted UI driver (wait/press/text/shot/quit), for automated testing.
    #[arg(long)]
    driver: Option<PathBuf>,

    /// Run without any window; pairs with --driver for headless UI tests.
    #[arg(long)]
    headless: bool,

    /// Base-frame cache budget in MiB (0 disables caching).
    #[arg(long, default_value_t = 800)]
    cache_mib: usize,

    /// Typography fit policy: maximize | balanced | artistic | fixed.
    #[arg(long, default_value = "artistic")]
    fit: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            for cause in error.chain().skip(1) {
                eprintln!("  caused by: {cause}");
            }
            ExitCode::FAILURE
        }
    }
}

struct Asset {
    stem: String,
    input: PathBuf,
}

struct Player {
    ffmpeg: PathBuf,
    input: PathBuf,
    info: video::VideoInfo,
    decoder: Option<DecodeAhead>,
    index: usize,
    paused: bool,
    previous: Option<Surface>,
}

impl Player {
    fn spawn(ffmpeg: &Path, input: &Path) -> Result<Self> {
        let info = video::probe(Path::new("ffprobe"), input)?;
        info.ensure_supported_compositing_color()?;
        let decoder = Some(DecodeAhead::spawn(ffmpeg, input, &info, 0)?);
        Ok(Self {
            ffmpeg: ffmpeg.to_path_buf(),
            input: input.to_path_buf(),
            info,
            decoder,
            index: 0,
            paused: false,
            previous: None,
        })
    }

    fn respawn_at(&mut self, frame: usize) -> Result<()> {
        self.decoder = Some(DecodeAhead::spawn(
            &self.ffmpeg,
            &self.input,
            &self.info,
            frame,
        )?);
        self.index = frame;
        Ok(())
    }

    fn advance(&mut self) -> Result<Option<Surface>> {
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(None);
        };
        let Some(frame) = decoder.next_frame()? else {
            return Ok(None);
        };
        self.previous = Some(frame.clone());
        self.index += 1;
        Ok(Some(frame))
    }

    fn step_back(&mut self) -> Option<Surface> {
        if self.index == 0 {
            return None;
        }
        self.paused = true;
        self.index -= 1;
        self.decoder = None; // resume respawns exactly here
        self.previous.take()
    }

    fn step_forward(&mut self) -> Result<Option<Surface>> {
        self.paused = true;
        if self.decoder.is_none() {
            self.respawn_at(self.index)?;
        }
        self.advance()
    }

    fn seek_by(&mut self, delta_frames: i64) -> Result<Option<Surface>> {
        let target = (self.index as i64 + delta_frames).clamp(0, self.info.frames.max(1) as i64 - 1)
            as usize;
        if target == self.index {
            return Ok(None);
        }
        self.paused = true;
        self.respawn_at(target)?;
        self.advance()
    }

    fn restart(&mut self) -> Result<Option<Surface>> {
        self.paused = false;
        self.respawn_at(0)?;
        self.advance()
    }
}

struct Bake {
    compositor: FrameCompositor,
    stale: bool,
}

/// Content-addressed-by-generation cache of scaled BGRA frames.
#[derive(Debug)]
struct BaseCache {
    generation: u64,
    video_index: usize,
    frames: Vec<Option<Vec<u8>>>,
    width: u32,
    height: u32,
}

impl BaseCache {
    fn frame(&self, index: usize) -> Option<&Vec<u8>> {
        self.frames.get(index).and_then(|slot| slot.as_ref())
    }

    fn fits(budget_bytes: usize, frames: usize, stride: usize) -> bool {
        frames != 0 && frames.saturating_mul(stride) <= budget_bytes
    }
}

/// Rolling per-stage nanos, printed at exit under PLAQUE_PROFILE=1.
#[derive(Default)]
struct StageTimes {
    decode: u128,
    composite: u128,
    scale: u128,
    present: u128,
}

impl StageTimes {
    fn report(&self, frames: u64) {
        if std::env::var("PLAQUE_PROFILE").is_err() || frames == 0 {
            return;
        }
        let ms = |ns: u128| ns as f64 / 1e6 / frames as f64;
        println!(
            "profile: decode {:.1}ms | composite {:.1}ms | scale {:.1}ms | present {:.1}ms (avg/frame)",
            ms(self.decode),
            ms(self.composite),
            ms(self.scale),
            ms(self.present)
        );
    }
}

struct Session {
    stages: StageTimes,
    fit: plaque_forge::application::FitMode,
    display_width: u32,
    headless: bool,
    driver: Option<Driver>,
    cache_mib: usize,
    cache: Option<BaseCache>,
    cache_generation: u64,
    fps_ema: f64,
    frames_shown: u64,
    playback_started: Option<Instant>,
    last_present: Option<Instant>,
    welcome_until: Instant,
    help_visible: bool,
    videos: Vec<Asset>,
    style_names: Vec<String>,
    font_choices: Vec<(String, bool)>,
    font_files: Vec<Option<PathBuf>>,
    font_index: usize,
    style_index: usize,
    text: String,
    tier: Tier,
    inspect: bool,
    layer_toggles: Vec<bool>,
    video_index: usize,
    player: Player,
    bake: Option<Bake>,
    bake_note: String,
    mode: Mode,
    saved_picks: (usize, usize),
    pending_save: Option<String>,
    pending_screenshot: Option<String>,
    toast: String,
}

impl Session {
    fn cache_active(&self) -> bool {
        self.cache
            .as_ref()
            .is_some_and(|cache| cache.video_index == self.video_index && !cache.frames.is_empty())
    }

    /// Pure-index move while the cache serves frames (decoder stays parked).
    fn move_index(&mut self, delta: i64) {
        let last = self
            .cache
            .as_ref()
            .map_or(0, |cache| cache.frames.len().saturating_sub(1));
        self.player.paused = true;
        self.player.index = ((self.player.index as i64 + delta).clamp(0, last as i64)) as usize;
    }

    fn move_to(&mut self, index: usize) {
        self.player.paused = true;
        self.player.decoder = None;
        self.player.index = index.min(
            self.cache
                .as_ref()
                .map_or(self.player.info.frames, |cache| cache.frames.len())
                .saturating_sub(1),
        );
    }

    fn asset(&self) -> &Asset {
        &self.videos[self.video_index]
    }

    fn font_path(&self) -> PathBuf {
        self.font_files
            .get(self.font_index)
            .cloned()
            .flatten()
            .unwrap_or_else(|| PathBuf::from("fonts/NotoSerif-Regular.ttf"))
    }

    fn style_path(&self) -> PathBuf {
        let name = self
            .style_names
            .get(self.style_index)
            .cloned()
            .unwrap_or_default();
        PathBuf::from("styles").join(format!("{name}.toml"))
    }

    fn active_style(&mut self) -> Result<effects::Style> {
        if let Mode::Composing(model) = &self.mode {
            return model.preview_style();
        }
        effects::Style::from_file(&self.style_path())
    }

    fn rebake(&mut self) -> Result<()> {
        self.bake = None;
        self.bake_note.clear();
        let analysis_dir = workspace::analysis_path(&self.asset().input)?;
        let pack = match Analysis::open(&analysis_dir) {
            Ok(pack) => pack,
            Err(_) => {
                self.bake_note = "no analysis cache — run ./scripts/analyze_assets.sh".into();
                return Ok(());
            }
        };
        let stale = pack.require_current_analyzer().is_err();
        let mask = plaque_forge::image_io::load_luma(
            &pack.require_asset(CONTENT_MASK_FILE)?,
            pack.manifest.canonical_width,
            pack.manifest.canonical_height,
        )?;
        let injected_surface = load_injected_surface(&pack)?;
        let style = self.active_style()?;
        let compositor =
            FrameCompositor::open(plaque_forge::render::compositor::CompositorSetup {
                pack,
                preview_warp: self.tier == Tier::Fast,
                mask,
                injected_surface,
                style,
                font_path: self.font_path(),
                text: self.text.clone(),
                fit: self.fit,
                requested_font_size: None,
                supersampling: self.tier.supersampling(),
                target_fill: 0.94,
                max_lines: 5,
                padding_ratio: 0.03,
                line_height_ratio: 1.08,
                text_align: plaque_forge::application::TextAlign::Center,
                vertical_align: plaque_forge::application::VerticalAlign::Center,
            })?;
        self.layer_toggles = vec![true; compositor.pack().manifest.layers.len()];
        self.bake = Some(Bake { compositor, stale });
        self.rebuild_cache();
        Ok(())
    }

    /// (Re)build the base-frame cache sized against the current asset.
    fn rebuild_cache(&mut self) {
        self.cache_generation = self.cache_generation.wrapping_add(1);
        let generation = self.cache_generation;
        let video_index = self.video_index;
        let (width, height) = scaled_dims(
            self.player.info.width,
            self.player.info.height,
            self.display_width,
        );
        let stride = width as usize * height as usize * 4;
        let budget_mib = std::env::var("PLAQUE_SHOWCASE_CACHE_MB")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(self.cache_mib);
        let budget = budget_mib.saturating_mul(1024 * 1024);
        self.cache = BaseCache::fits(budget, self.player.info.frames, stride).then(|| BaseCache {
            generation,
            video_index,
            frames: vec![None; self.player.info.frames],
            width,
            height,
        });
    }

    fn cycle_video(&mut self, delta: i32) -> Result<()> {
        let count = self.videos.len() as i64;
        self.video_index = wrap_index(self.video_index as i64 + delta as i64, count as usize);
        let asset_input = PathBuf::from(format!("assets/{}.mp4", self.asset().stem));
        self.player = Player::spawn(Path::new("ffmpeg"), &asset_input)?;
        self.rebake()
    }

    fn cycle_style(&mut self, delta: i32) -> Result<()> {
        let count = self.style_names.len() as i64;
        self.style_index = wrap_index(self.style_index as i64 + delta as i64, count as usize);
        self.rebake()
    }

    fn apply_font_choice(&mut self, choice_index: usize) -> Result<()> {
        self.font_index = choice_index.min(self.font_choices.len().saturating_sub(1));
        self.rebake()
    }
}

fn wrap_index(index: i64, len: usize) -> usize {
    (((index % len.max(1) as i64) + len.max(1) as i64) % len.max(1) as i64) as usize
}

fn run(args: Args) -> Result<()> {
    let root = args
        .root
        .canonicalize()
        .unwrap_or_else(|_| args.root.clone());
    std::env::set_current_dir(&root).ok();
    let driver = match &args.driver {
        Some(path) => Some(Driver::new(Script::load(path)?)),
        None => None,
    };
    let catalog = FilesystemCatalog::production()?;
    let videos: Vec<Asset> = catalog
        .videos()?
        .into_iter()
        .map(|video| {
            let stem = video.stem.clone();
            Asset {
                stem: stem.clone(),
                input: PathBuf::from(format!("assets/{stem}.mp4")),
            }
        })
        .collect();
    anyhow::ensure!(!videos.is_empty(), "no input videos found under assets/");
    let style_names: Vec<String> = catalog.styles()?.into_iter().map(|s| s.name).collect();

    // Curated entries first (resolved to concrete files where possible), then
    // every other installed family.
    let system = SystemFonts::load();
    let curated_source =
        std::fs::read_to_string(plaque_forge::media::CURATED_FONTS_FILE).unwrap_or_default();
    let mut font_choices: Vec<(String, bool)> = Vec::new();
    let mut font_files: Vec<Option<PathBuf>> = Vec::new();
    for entry in curated::parse_curated_fonts(&curated_source)? {
        match entry {
            curated::CuratedFont::Repository { path } => {
                let absolute = root.join(&path);
                let label = Path::new(&path)
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or(path.as_str())
                    .to_string();
                let present = absolute.is_file();
                font_choices.push((label, true));
                font_files.push(present.then_some(absolute));
            }
            curated::CuratedFont::Family { pattern } => match system.face_file_and_family(&pattern)
            {
                Some((path, family)) => {
                    font_choices.push((family, true));
                    font_files.push(Some(path));
                }
                None => {
                    font_choices.push((pattern, true));
                    font_files.push(None);
                }
            },
        }
    }
    for listing in catalog.fonts()? {
        if font_choices
            .iter()
            .any(|(label, _)| label.eq_ignore_ascii_case(&listing.label))
        {
            continue;
        }
        font_choices.push((listing.label.clone(), false));
        font_files.push(system.face_file(&listing.label));
    }

    let font_index = match &args.font {
        Some(explicit) => {
            let canonical = explicit.canonicalize().unwrap_or_else(|_| explicit.clone());
            font_files
                .iter()
                .position(|path| path.as_ref().is_some_and(|p| *p == canonical))
                .unwrap_or_else(|| {
                    font_choices.push((explicit.display().to_string(), false));
                    font_files.push(Some(canonical));
                    font_choices.len() - 1
                })
        }
        None => 0,
    };
    let style_index = match &args.style {
        Some(name) => style_names
            .iter()
            .position(|candidate| candidate == name)
            .with_context(|| format!("style preset not found: {name}"))?,
        None => default_style_index(&style_names),
    };

    let session = Session {
        stages: StageTimes::default(),
        fit: parse_fit(&args.fit),
        display_width: args.width,
        headless: args.headless,
        driver,
        cache_mib: args.cache_mib,
        cache: None,
        cache_generation: 0,
        fps_ema: 0.0,
        frames_shown: 0,
        playback_started: None,
        last_present: None,
        welcome_until: Instant::now() + std::time::Duration::from_secs(6),
        help_visible: false,
        player: Player::spawn(
            Path::new("ffmpeg"),
            Path::new(&format!("assets/{}.mp4", videos[0].stem)),
        )?,
        videos,
        style_names,
        font_choices,
        font_files,
        font_index,
        style_index,
        text: DEFAULT_TEXT.to_string(),
        tier: Tier::Fast,
        inspect: false,
        layer_toggles: Vec::new(),
        video_index: 0,
        bake: None,
        bake_note: String::new(),
        mode: Mode::Viewing,
        saved_picks: (font_index, style_index),
        pending_save: None,
        pending_screenshot: None,
        toast: String::new(),
    };
    run_session(session, args.smoke)
}

/// Sensible first impression: a calm preset, never the arc-heavy opener.
fn default_style_index(style_names: &[String]) -> usize {
    style_names
        .iter()
        .position(|name| name == "classic-glow")
        .or_else(|| style_names.iter().position(|name| name != "art-deco-arc"))
        .unwrap_or(0)
}

fn parse_fit(name: &str) -> plaque_forge::application::FitMode {
    match name {
        "maximize" => plaque_forge::application::FitMode::Maximize,
        "balanced" => plaque_forge::application::FitMode::Balanced,
        "fixed" => plaque_forge::application::FitMode::Fixed,
        _ => plaque_forge::application::FitMode::Artistic,
    }
}

fn run_session(mut session: Session, smoke: Option<usize>) -> Result<()> {
    if smoke.is_some() {
        // Bounded self-check: the artistic fit search is far too heavy to be
        // a quick gate on constrained machines.
        session.fit = plaque_forge::application::FitMode::Balanced;
    }
    eprintln!("baking title (fit={:?})...", session.fit);
    session.rebake()?;
    if let Some(frames) = smoke {
        return smoke_run(&mut session, frames);
    }
    HEADLESS.store(session.headless, std::sync::atomic::Ordering::Relaxed);
    if !headless() {
        opencv::highgui::named_window(WINDOW, opencv::highgui::WINDOW_AUTOSIZE)
            .context("failed to create showcase window")?;
    }
    let result = event_loop(&mut session);
    if !headless() {
        let _ = opencv::highgui::destroy_all_windows();
    }
    result
}

fn smoke_run(session: &mut Session, frames: usize) -> Result<()> {
    let mut composed = 0usize;
    for index in 0..frames {
        let Some(mut frame) = session.player.advance()? else {
            break;
        };
        if let Some(bake) = session.bake.as_mut() {
            bake.compositor.composite(&mut frame, index)?;
        }
        composed += 1;
    }
    println!(
        "smoke: composited {composed} frames of {}",
        session.asset().stem
    );
    Ok(())
}

/// Destination display dimensions preserving aspect.
fn scaled_dims(source_w: u32, source_h: u32, display_width: u32) -> (u32, u32) {
    if source_w == 0 || source_h == 0 {
        return (display_width.max(1), display_width.max(1));
    }
    let width = display_width.max(16).min(source_w);
    let height = ((width as f64 / source_w as f64) * source_h as f64)
        .round()
        .max(1.0) as u32;
    (width, height)
}

unsafe fn mat_view_rgba(bytes: &mut [u8], width: u32, height: u32) -> Mat {
    unsafe {
        Mat::new_rows_cols_with_data_unsafe_def(
            height as i32,
            width as i32,
            CV_8UC4,
            bytes.as_mut_ptr() as *mut core::ffi::c_void,
        )
        .expect("mat view over contiguous rgba buffer")
    }
}

unsafe fn mat_view_bgra(bytes: &[u8], width: u32, height: u32) -> Mat {
    unsafe {
        Mat::new_rows_cols_with_data_unsafe_def(
            height as i32,
            width as i32,
            CV_8UC4,
            bytes.as_ptr() as *mut core::ffi::c_void,
        )
        .expect("mat view over contiguous bgra buffer")
    }
}

fn mat_to_vec(mat: &Mat) -> Result<Vec<u8>> {
    Ok(mat.data_bytes()?.to_vec())
}

/// RGBA surface -> owned BGRA buffer at display scale (OpenCV convert+resize).
fn scaled_bgra(frame: Surface, display_width: u32) -> Result<(Vec<u8>, u32, u32)> {
    let width = frame.width();
    let height = frame.height();
    let mut pixels = frame.into_pixels();
    let (dst_w, dst_h) = scaled_dims(width, height, display_width);
    unsafe {
        let view = mat_view_rgba(&mut pixels, width, height);
        // Downscale first so the channel conversion touches fewer pixels.
        let mut resized = Mat::default();
        opencv::imgproc::resize(
            &view,
            &mut resized,
            opencv::core::Size_::new(dst_w as i32, dst_h as i32),
            0.0,
            0.0,
            opencv::imgproc::INTER_LINEAR,
        )?;
        let mut converted = Mat::default();
        opencv::imgproc::cvt_color_def(&resized, &mut converted, opencv::imgproc::COLOR_RGBA2BGRA)?;
        Ok((mat_to_vec(&converted)?, dst_w, dst_h))
    }
}

fn event_loop(session: &mut Session) -> Result<()> {
    loop {
        let iteration_start = Instant::now();

        // Scripted presses land before rendering so their effect is visible
        // in the very same frame.
        let mut injected: Option<Key> = None;
        if let Some(driver) = session.driver.as_mut() {
            injected = driver.poll();
            if driver.finished() {
                session.pending_screenshot = driver.pending_shot();
                report_fps(session);
                return Ok(());
            }
        }

        // Cache hit: no decode, no composite — pure presentation.
        let cached = if session.inspect {
            None
        } else {
            session.cache.as_ref().and_then(|cache| {
                if cache.video_index != session.video_index {
                    return None;
                }
                cache.frame(session.player.index).cloned()
            })
        };

        let (bytes, width, height) = if let Some(bytes) = cached {
            (bytes, cache_width(session), cache_height(session))
        } else {
            match session.player.advance() {
                Ok(Some(frame)) => {
                    let decode_stage = Instant::now();
                    let frame_index = session.player.index.saturating_sub(1);
                    let mut prepared = frame;
                    session.stages.decode += decode_stage.elapsed().as_nanos();
                    if let Some(bake) = session.bake.as_mut() {
                        let clamped =
                            frame_index.min(bake.compositor.pack().motion.len().saturating_sub(1));
                        let composite_stage = Instant::now();
                        bake.compositor.composite(&mut prepared, clamped)?;
                        session.stages.composite += composite_stage.elapsed().as_nanos();
                    }
                    if session.inspect {
                        draw_inspect_overlays(session, &mut prepared, clamped_index(session))?;
                    }
                    let scale_stage = Instant::now();
                    let (bytes, w, h) = scaled_bgra(prepared, session.display_width)?;
                    session.stages.scale += scale_stage.elapsed().as_nanos();
                    store_in_cache(session, frame_index, &bytes);
                    (bytes, w, h)
                }
                Ok(None) => {
                    // End of stream: next asset; demo re-rolls its picks.
                    let demo = matches!(session.mode, Mode::Demo(_));
                    session.cycle_video(1)?;
                    if demo {
                        start_demo(session)?;
                    }
                    continue;
                }
                Err(error) => return Err(error),
            }
        };

        let present_stage = Instant::now();
        show_frame(session, &bytes, width, height)?;
        session.stages.present += present_stage.elapsed().as_nanos();
        update_fps(session, iteration_start);

        // Pacing + input acquisition.
        let typing = matches!(session.mode, Mode::EnteringText(_) | Mode::SavingName(_));
        let modal = typing
            || matches!(session.mode, Mode::PickingFont(_) | Mode::Composing(_))
            || session.player.paused
            || session.driver.is_some();
        let delay_ms = if modal {
            16
        } else {
            (1000.0 / session.player.info.fps.max(1.0)).clamp(8.0, 40.0) as i32
        };

        let key = if let Some(key) = injected {
            Some(key)
        } else if session.driver.is_some() || session.headless {
            // The driver is polled exactly once per iteration (top of loop);
            // idle here so wait deadlines still elapse.
            std::thread::sleep(std::time::Duration::from_millis(4));
            None
        } else if session.headless {
            std::thread::sleep(std::time::Duration::from_millis(8));
            None
        } else {
            let raw = opencv::highgui::wait_key(delay_ms)?;
            (raw >= 0).then(|| normalize(raw))
        };

        if let Some(key) = key
            && dispatch_key(session, key)?
        {
            report_fps(session);
            return Ok(());
        }
    }
}

fn clamped_index(session: &Session) -> usize {
    session
        .player
        .index
        .saturating_sub(1)
        .min(source_frame_budget(session))
}

fn cache_width(session: &Session) -> u32 {
    session
        .cache
        .as_ref()
        .map(|cache| cache.width)
        .unwrap_or(session.display_width)
}

fn cache_height(session: &Session) -> u32 {
    session
        .cache
        .as_ref()
        .map(|cache| cache.height)
        .unwrap_or(1)
}

fn store_in_cache(session: &mut Session, index: usize, bytes: &[u8]) {
    let generation = session.cache_generation;
    let video_index = session.video_index;
    if let Some(cache) = session.cache.as_mut()
        && cache.generation == generation
        && cache.video_index == video_index
        && let Some(slot) = cache.frames.get_mut(index)
    {
        *slot = Some(bytes.to_vec());
    }
}

fn update_fps(session: &mut Session, started: Instant) {
    if session.playback_started.is_none() {
        session.playback_started = Some(started);
    }
    let elapsed = started.elapsed().as_secs_f64().max(1.0 / 1000.0);
    let instant_fps = 1.0 / elapsed;
    session.fps_ema = if session.frames_shown == 0 {
        instant_fps
    } else {
        session.fps_ema * 0.9 + instant_fps * 0.1
    };
    session.frames_shown += 1;
    session.last_present = Some(started);
}

fn report_fps(session: &Session) {
    let overall = session
        .playback_started
        .map(|started| {
            let secs = started.elapsed().as_secs_f64().max(1e-3);
            session.frames_shown as f64 / secs
        })
        .unwrap_or(0.0);
    println!(
        "showcase: displayed {} frames, responsive {:.1} fps, overall {:.1} fps",
        session.frames_shown, session.fps_ema, overall
    );
    session.stages.report(session.frames_shown);
}

fn source_frame_budget(session: &Session) -> usize {
    session
        .bake
        .as_ref()
        .map(|bake| bake.compositor.pack().motion.len())
        .unwrap_or(usize::MAX)
        .saturating_sub(1)
}

fn dispatch_key(session: &mut Session, key: Key) -> Result<bool> {
    if let Key::Char('q') = key {
        return Ok(true);
    }
    if let Key::Esc = key {
        match std::mem::replace(&mut session.mode, Mode::Viewing) {
            Mode::PickingFont(picker) => session.apply_font_choice(picker.cancel())?,
            Mode::Demo(_) => {
                let (font, style) = session.saved_picks;
                session.font_index = font;
                session.style_index = style;
                session.rebake()?;
            }
            Mode::Composing(_) => session.rebake()?,
            other => {
                session.mode = other;
            }
        }
        return Ok(false);
    }

    if let Some(buffer) = session.mode.text_buffer_mut() {
        match key {
            Key::Char(c) => buffer.push(c),
            Key::Backspace => {
                buffer.pop();
            }
            _ => {}
        }
        if let Key::Enter = key {
            finish_entry(session)?;
        }
        return Ok(false);
    }

    if let Mode::PickingFont(picker) = &mut session.mode {
        match key {
            Key::Up => {
                if let Some(choice) = picker.move_cursor(-1) {
                    apply_labelled_font(session, &choice.label)?;
                }
            }
            Key::Down => {
                if let Some(choice) = picker.move_cursor(1) {
                    apply_labelled_font(session, &choice.label)?;
                }
            }
            Key::Backspace => {
                picker.edit_query(false);
            }
            Key::Delete => {
                picker.edit_query(true);
            }
            Key::Enter => {
                let chosen = picker.commit();
                session.mode = Mode::Viewing;
                session.apply_font_choice(chosen)?;
            }
            Key::Char(c) => picker.push_char(c),
            _ => {}
        }
        return Ok(false);
    }

    if let Mode::Composing(model) = &mut session.mode {
        match key {
            Key::Up => model.move_cursor(-1),
            Key::Down => model.move_cursor(1),
            Key::Left => model.adjust_selected(EditDirection::Previous),
            Key::Right => model.adjust_selected(EditDirection::Next),
            Key::Enter => {
                model.press_enter();
            }
            Key::Char('w') => {
                // Save flow starts from the live document.
                session.pending_save = Some(model.to_toml());
                session.mode = Mode::SavingName(suggested_save_name(&session.style_names));
            }
            _ => {}
        }
        session.rebake()?;
        return Ok(false);
    }

    match key {
        Key::PageUp => session.cycle_video(-1)?,
        Key::PageDown => session.cycle_video(1)?,
        Key::Up => session.cycle_style(-1)?,
        Key::Down => session.cycle_style(1)?,
        Key::Enter => session.mode = Mode::EnteringText(session.text.clone()),
        Key::Char('/') => {
            let listing: Vec<FontListing> = session
                .font_choices
                .iter()
                .map(|(label, curated)| FontListing {
                    label: label.clone(),
                    curated: *curated,
                })
                .collect();
            session.mode = Mode::PickingFont(FontPicker::open(&listing, session.font_index));
        }
        Key::Char('d') => {
            session.saved_picks = (session.font_index, session.style_index);
            session.player.paused = false;
            start_demo(session)?;
        }
        Key::Char('i') => {
            session.inspect = !session.inspect;
        }
        Key::Char('f') => {
            session.tier = session.tier.toggled();
            session.rebake()?;
        }
        Key::Char('e') => {
            let source = std::fs::read_to_string(session.style_path())?;
            let model = EditModel::open(
                &source,
                Path::new("styles").parent().unwrap_or(Path::new(".")),
            )
            .map_err(|error| anyhow::anyhow!("invalid style TOML: {error}"))?;
            session.mode = Mode::Composing(Box::new(model));
        }
        Key::Char(' ') => session.player.paused = !session.player.paused,
        Key::Home => {
            if session.cache_active() {
                session.move_to(0);
            } else {
                session.player.restart()?;
            }
        }
        Key::Left => {
            let step = (session.player.info.fps.round() as i64 * 5).max(1);
            if session.cache_active() {
                session.move_index(-step);
            } else {
                session.player.seek_by(-step)?;
            }
        }
        Key::Right => {
            let step = (session.player.info.fps.round() as i64 * 5).max(1);
            if session.cache_active() {
                session.move_index(step);
            } else {
                session.player.seek_by(step)?;
            }
        }
        Key::Char(',') => {
            if session.cache_active() {
                session.move_index(-1);
            } else {
                session.player.step_back();
            }
        }
        Key::Char('.') => {
            if session.cache_active() {
                session.move_index(1);
            } else {
                session.player.step_forward()?;
            }
        }
        Key::Char('?') => session.help_visible = !session.help_visible,
        _ => {}
    }
    Ok(false)
}

fn apply_labelled_font(session: &mut Session, label: &str) -> Result<()> {
    if let Some(index) = session
        .font_choices
        .iter()
        .position(|(candidate, _)| candidate == label)
    {
        session.apply_font_choice(index)?;
    }
    Ok(())
}

fn suggested_save_name(existing: &[String]) -> String {
    let base = "my-style";
    let mut candidate = base.to_string();
    let mut suffix = 2;
    while existing.contains(&candidate) {
        candidate = format!("{base}-{suffix}");
        suffix += 1;
    }
    candidate
}

fn start_demo(session: &mut Session) -> Result<()> {
    let fonts: Vec<String> = session
        .font_choices
        .iter()
        .filter(|(_, curated)| *curated)
        .map(|(label, _)| label.clone())
        .collect();
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64 ^ d.as_secs())
        .unwrap_or(1);
    let mut demo = DemoState::start(seed.max(1));
    if let Some(combo) = demo.next_combo(&fonts, &session.style_names) {
        apply_labelled_font(session, &combo.font_label)?;
        if let Some(index) = session
            .style_names
            .iter()
            .position(|n| *n == combo.style_name)
        {
            session.style_index = index;
            session.rebake()?;
        }
    }
    session.mode = Mode::Demo(demo);
    Ok(())
}

fn finish_entry(session: &mut Session) -> Result<()> {
    match std::mem::replace(&mut session.mode, Mode::Viewing) {
        Mode::EnteringText(text) => {
            if !text.trim().is_empty() {
                session.text = text;
                session.rebake()?;
            }
        }
        Mode::SavingName(name) => {
            let name = name.trim().trim_end_matches(".toml").to_string();
            if name.is_empty() {
                return Ok(());
            }
            if let Some(document) = session.pending_save.take() {
                let destination = PathBuf::from("styles").join(format!("{name}.toml"));
                std::fs::write(&destination, document)
                    .with_context(|| format!("failed to write {}", destination.display()))?;
                if !session.style_names.contains(&name) {
                    session.style_names.push(name.clone());
                }
                session.style_index = session
                    .style_names
                    .iter()
                    .position(|candidate| *candidate == name)
                    .unwrap_or(session.style_index);
                session.toast = format!("saved {destination:?}");
                session.rebake()?;
            }
        }
        other => {
            session.mode = other;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Presentation

fn draw_inspect_overlays(session: &Session, frame: &mut Surface, frame_index: usize) -> Result<()> {
    let Some(bake) = session.bake.as_ref() else {
        return Ok(());
    };
    let pack = bake.compositor.pack();
    // Tracked surface quad.
    let quad = bake.compositor.plaque_quad(frame_index)?;
    draw_quad(frame, quad, YELLOW);
    // Foreground occluders, filled solid green.
    if bake.compositor.uses_analysis_occluders() {
        let path = pack
            .root
            .join(plaque_forge::analysis::OCCLUDER_DIR)
            .join(format!("{frame_index:06}.png"));
        if path.is_file() {
            let mask = load_full_luma(
                &path,
                pack.manifest.source.width,
                pack.manifest.source.height,
            )?;
            fill_mask(frame, &mask, GREEN_FILL, 96);
        }
    }
    // Declared layers, toggleable with number keys.
    for (slot, layer) in pack.manifest.layers.iter().enumerate() {
        let Some(true) = session.layer_toggles.get(slot) else {
            continue;
        };
        if let Some((mask, width, height)) = layer_mask(pack, layer, frame_index)? {
            outline_mask(frame, &mask, width, height, (0.0, 255.0, 255.0));
        }
    }
    Ok(())
}

fn layer_mask(
    pack: &Analysis,
    layer: &LayerAsset,
    frame_index: usize,
) -> Result<Option<(Vec<u8>, u32, u32)>> {
    let dims = match layer.coordinates {
        LayerCoordinates::PlaqueCanonical => (
            pack.manifest.canonical_width,
            pack.manifest.canonical_height,
        ),
        LayerCoordinates::SourcePixels => (pack.manifest.source.width, pack.manifest.source.height),
    };
    let path = match layer.kind {
        LayerArtifactKind::AlphaImage => resolve_asset(&pack.root, layer.path.as_path())?,
        LayerArtifactKind::AlphaSequence => {
            if !plaque_forge::layers::frame_in_layer(layer, frame_index) {
                return Ok(None);
            }
            resolve_asset(
                &pack.root,
                &sequence_path(layer.path.as_path(), frame_index),
            )?
        }
    };
    let mask = plaque_forge::image_io::load_luma(&path, dims.0, dims.1)?;
    Ok(Some((mask, dims.0, dims.1)))
}

fn fill_mask(surface: &mut Surface, mask: &[u8], color: [u8; 3], threshold: u8) {
    let width = surface.width();
    for (index, &alpha) in mask.iter().enumerate() {
        if alpha >= threshold {
            let x = (index % width as usize) as u32;
            let y = (index / width as usize) as u32;
            surface.set_pixel(
                x,
                y,
                plaque_forge::color::Rgba::new(color[0], color[1], color[2], 255),
            );
        }
    }
}

fn outline_mask(
    surface: &mut Surface,
    mask: &[u8],
    width: u32,
    height: u32,
    color: (f64, f64, f64),
) {
    let rgba = plaque_forge::color::Rgba::new(color.0 as u8, color.1 as u8, color.2 as u8, 255);
    let at = |x: i64, y: i64| -> u8 {
        if x < 0 || y < 0 || x >= width as i64 || y >= height as i64 {
            0
        } else {
            mask[(y as usize) * width as usize + x as usize]
        }
    };
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let here = at(x, y);
            if here == 0 {
                continue;
            }
            let edge =
                at(x - 1, y) == 0 || at(x + 1, y) == 0 || at(x, y - 1) == 0 || at(x, y + 1) == 0;
            if edge {
                surface.set_pixel(x as u32, y as u32, rgba);
            }
        }
    }
}

/// Quad outline in screen pixels (Bresenham segments).
fn draw_quad(surface: &mut Surface, quad: plaque_forge::geometry::Quad, color: Scalar) {
    let corners = [quad.tl, quad.tr, quad.br, quad.bl, quad.tl];
    let points: Vec<(f64, f64)> = corners.iter().map(|point| (point.x, point.y)).collect();
    let rgba =
        plaque_forge::color::Rgba::new(color.0[2] as u8, color.0[1] as u8, color.0[0] as u8, 255);
    for pair in points.windows(2) {
        let (x0, y0) = (pair[0].0.round() as i64, pair[0].1.round() as i64);
        let (x1, y1) = (pair[1].0.round() as i64, pair[1].1.round() as i64);
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = i64::from(x1 > x0) * 2 - 1;
        let sy = i64::from(y1 > y0) * 2 - 1;
        let mut err = dx + dy;
        let (mut x, mut y) = (x0, y0);
        loop {
            for ox in -1..=1 {
                for oy in -1..=1 {
                    let (px, py) = (x + ox, y + oy);
                    if px >= 0
                        && py >= 0
                        && px < surface.width() as i64
                        && py < surface.height() as i64
                    {
                        surface.set_pixel(px as u32, py as u32, rgba);
                    }
                }
            }
            if x == x1 && y == y1 {
                break;
            }
            let twice = 2 * err;
            if twice >= dy {
                err += dy;
                x += sx;
            }
            if twice <= dx {
                err += dx;
                y += sy;
            }
        }
    }
}

/// Draw every overlay onto one displayed frame and show/screenshot it.
fn show_frame(session: &mut Session, bytes: &[u8], width: u32, height: u32) -> Result<()> {
    let owned = bytes.to_vec();
    let view = unsafe { mat_view_bgra(&owned, width, height) };
    let mut shadowed = view.clone();

    draw_hud_bottom(session, &mut shadowed)?;
    if !session.inspect
        && Instant::now() < session.welcome_until
        && matches!(session.mode, Mode::Viewing)
    {
        draw_help_card(&mut shadowed, true);
    }
    if session.help_visible {
        draw_help_card(&mut shadowed, false);
    }
    match &session.mode {
        Mode::EnteringText(buffer) => draw_entry_echo(&mut shadowed, "new title", buffer),
        Mode::SavingName(buffer) => draw_entry_echo(&mut shadowed, "save as", buffer),
        Mode::PickingFont(picker) => draw_font_popup(&mut shadowed, picker),
        Mode::Composing(model) => draw_composer_panel(&mut shadowed, model),
        _ => {}
    }

    // Screenshot requests from the driver capture the final composited UI.
    let shot = session
        .driver
        .as_mut()
        .and_then(Driver::pending_shot)
        .or_else(|| session.pending_screenshot.take());
    if let Some(path) = shot {
        let mut png = Vector::<u8>::new();
        imencode(".png", &shadowed, &mut png, &Vector::<i32>::new())
            .with_context(|| format!("failed to encode screenshot {}", path))?;
        std::fs::write(&path, png.to_vec())
            .with_context(|| format!("failed to write screenshot {}", path))?;
    }

    if !headless() {
        opencv::highgui::imshow(WINDOW, &shadowed)?;
    }
    Ok(())
}

const ZONE_LINE_HEIGHT: i32 = 20;

/// Bottom tri-zone HUD: left status, center transient, right hints.
fn draw_hud_bottom(session: &Session, mat: &mut Mat) -> Result<()> {
    let rows = mat.rows();
    let cols = mat.cols();
    let scale = (cols as f64 / 1280.0).clamp(0.42, 0.8);
    let baseline_y = rows - 12;
    let strip_top = rows - ZONE_LINE_HEIGHT * 2 - 14;
    rectangle_points(
        mat,
        CvPoint::new(0, strip_top),
        CvPoint::new(cols, rows),
        BLACK,
        -1,
        LINE_8,
        0,
    )?;

    // Left: identity + quality.
    let stale_note = if session.bake.is_none() {
        "no analysis"
    } else if session.bake.as_ref().is_some_and(|bake| bake.stale) {
        "stale cache"
    } else {
        ""
    };
    let left1 = format!(
        "{} {}/{}  [{}]",
        session.asset().stem,
        session.video_index + 1,
        session.videos.len(),
        session.tier.label()
    );
    let left2 = if stale_note.is_empty() {
        format!(
            "style {} · font {}",
            session
                .style_names
                .get(session.style_index)
                .cloned()
                .unwrap_or_default(),
            session
                .font_choices
                .get(session.font_index)
                .map(|(l, _)| l.clone())
                .unwrap_or_default(),
        )
    } else {
        stale_note.to_string()
    };
    put_text(
        mat,
        &left1,
        CvPoint::new(10, baseline_y - ZONE_LINE_HEIGHT),
        FONT_HERSHEY_SIMPLEX,
        scale,
        WHITE,
        1,
        LINE_8,
        false,
    )?;
    put_text(
        mat,
        &left2,
        CvPoint::new(10, baseline_y),
        FONT_HERSHEY_SIMPLEX,
        scale,
        WHITE,
        1,
        LINE_8,
        false,
    )?;

    // Center: transient state.
    let center = match (&session.mode, &session.toast) {
        (_, toast) if !toast.is_empty() => toast.clone(),
        (Mode::Viewing, _) => String::new(),
        (mode, _) => format!("mode: {}", mode.label()),
    };
    if !center.is_empty() {
        let width_px = (center.len() as f64 * scale * 13.0) as i32;
        put_text(
            mat,
            &center,
            CvPoint::new((cols - width_px) / 2, baseline_y - ZONE_LINE_HEIGHT),
            FONT_HERSHEY_SIMPLEX,
            scale,
            YELLOW,
            1,
            LINE_8,
            false,
        )?;
    }

    // Right: condensed hints (two short lines).
    const RIGHT_HINT_1: &str = "PgUp/Dn vid · Up/Dn style · / fonts";
    const RIGHT_HINT_2: &str = "e edit · s save · d demo · i inspect · f tier · ? help · q quit";
    let right1 = RIGHT_HINT_1;
    let right2 = RIGHT_HINT_2;
    let width1 = (RIGHT_HINT_1.len() as f64 * scale * 12.5) as i32;
    let width2 = (RIGHT_HINT_2.len() as f64 * scale * 12.5) as i32;
    put_text(
        mat,
        right1,
        CvPoint::new(cols - width1 - 10, baseline_y - ZONE_LINE_HEIGHT),
        FONT_HERSHEY_SIMPLEX,
        scale,
        WHITE,
        1,
        LINE_8,
        false,
    )?;
    put_text(
        mat,
        right2,
        CvPoint::new(cols - width2 - 10, baseline_y),
        FONT_HERSHEY_SIMPLEX,
        scale,
        WHITE,
        1,
        LINE_8,
        false,
    )?;
    Ok(())
}

/// Big centered echo so typed text is immediately visible (issue #6).
fn draw_entry_echo(mat: &mut Mat, prompt: &str, buffer: &str) {
    let rows = mat.rows();
    let cols = mat.cols();
    rectangle_points(
        mat,
        CvPoint::new(cols / 8, rows / 3),
        CvPoint::new(cols * 7 / 8, rows / 3 + 90),
        BLACK,
        -1,
        LINE_8,
        0,
    )
    .ok();
    put_text(
        mat,
        prompt,
        CvPoint::new(cols / 8 + 16, rows / 3 + 34),
        FONT_HERSHEY_SIMPLEX,
        0.6,
        YELLOW,
        1,
        LINE_8,
        false,
    )
    .ok();
    let shown: String = buffer
        .chars()
        .rev()
        .take(38)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    put_text(
        mat,
        &format!("{shown}_"),
        CvPoint::new(cols / 8 + 16, rows / 3 + 70),
        FONT_HERSHEY_SIMPLEX,
        0.8,
        WHITE,
        1,
        LINE_8,
        false,
    )
    .ok();
}

/// Centered key reference; `welcome` variant notes it disappears on its own.
fn draw_help_card(mat: &mut Mat, welcome: bool) {
    let lines: [&str; 9] = [
        "PgUp/PgDn video   Up/Down style   Enter text",
        "/ fonts (type to search)   e composer   s save",
        "d demo   i inspect   f FAST/FINE   Space pause",
        "Left/Right +-5s   Home restart",
        ", . step back/forward   q quit   Esc cancel",
        "",
        "composer: Up/Dn row, Lt/Rt adjust, Enter add/remove, w save",
        "",
        if welcome {
            "any key dismisses this card"
        } else {
            "? closes help"
        },
    ];
    let rows = mat.rows();
    let cols = mat.cols();
    let card_h = 40 + lines.len() as i32 * 26 + 20;
    rectangle_points(
        mat,
        CvPoint::new(cols / 10, rows / 8),
        CvPoint::new(cols * 9 / 10, rows / 8 + card_h),
        Scalar::new(0.0, 0.0, 0.0, 255.0),
        -1,
        LINE_8,
        0,
    )
    .ok();
    for (offset, line) in lines.iter().enumerate() {
        put_text(
            mat,
            line,
            CvPoint::new(cols / 10 + 24, rows / 8 + 46 + offset as i32 * 26),
            FONT_HERSHEY_SIMPLEX,
            0.52,
            WHITE,
            1,
            LINE_8,
            false,
        )
        .ok();
    }
}

fn draw_font_popup(mat: &mut Mat, picker: &FontPicker) {
    let cols = mat.cols();
    let query_line = match picker.query() {
        Some(query) => format!("search: {query}_"),
        None => "curated+system fonts  (type to search)".to_string(),
    };
    let entries: Vec<_> = picker.rows().collect();
    let visible_height = entries.len().min(14) as i32 * 22 + 56;
    rectangle_points(
        mat,
        CvPoint::new(20, 20),
        CvPoint::new(cols - 20, 20 + visible_height),
        BLACK,
        -1,
        LINE_8,
        0,
    )
    .ok();
    put_text(
        mat,
        &query_line,
        CvPoint::new(30, 44),
        FONT_HERSHEY_SIMPLEX,
        0.55,
        WHITE,
        1,
        LINE_8,
        false,
    )
    .ok();
    // Scroll window around the cursor.
    let cursor_position = entries
        .iter()
        .position(|(index, _)| *index == picker.cursor())
        .unwrap_or(0);
    let start = cursor_position.saturating_sub(6);
    for (row_offset, (entry_index, choice)) in entries.iter().skip(start).take(14).enumerate() {
        let marker = if choice.curated { "*" } else { " " };
        let selected = *entry_index == picker.cursor();
        let color = if selected { YELLOW } else { WHITE };
        let text = format!("{marker} {}", choice.label);
        let org = CvPoint::new(34, 68 + row_offset as i32 * 22);
        put_text(
            mat,
            &text,
            org,
            FONT_HERSHEY_SIMPLEX,
            0.55,
            color,
            if selected { 2 } else { 1 },
            LINE_8,
            false,
        )
        .ok();
    }
}

fn draw_composer_panel(mat: &mut Mat, model: &EditModel) {
    let rows_count = mat.rows();
    let cols = mat.cols();
    let panel_width = 380;
    let panel_left = cols - panel_width - 12;

    rectangle_points(
        mat,
        CvPoint::new(panel_left, 12),
        CvPoint::new(cols - 12, rows_count - 60),
        BLACK,
        -1,
        LINE_8,
        0,
    )
    .ok();

    let mut y = 38;
    put_text(
        mat,
        "STYLE COMPOSER",
        CvPoint::new(panel_left + 8, y),
        FONT_HERSHEY_SIMPLEX,
        0.55,
        YELLOW,
        1,
        LINE_8,
        false,
    )
    .ok();
    y += 28;

    let rows = model.rows();
    let cursor = model.cursor();
    let start = cursor.saturating_sub(9);
    for (offset, row) in rows.iter().skip(start).take(13).enumerate() {
        let index = start + offset;
        let selected = index == cursor;
        let value = model.row_value(index);
        let color = if selected { YELLOW } else { WHITE };
        let label: String = if row.label.chars().count() > 22 {
            format!("{}\u{2026}", row.label.chars().take(21).collect::<String>())
        } else {
            row.label.clone()
        };
        let text = format!(
            "{} {:<22} {}",
            if selected { '>' } else { ' ' },
            label,
            value
        );
        put_text(
            mat,
            &text,
            CvPoint::new(panel_left + 8, y),
            FONT_HERSHEY_SIMPLEX,
            0.42,
            color,
            1,
            LINE_8,
            false,
        )
        .ok();
        y += 20;
        if y > rows_count - 92 {
            break;
        }
    }

    if let Some(error) = model.error() {
        let clipped: String = error.chars().take(56).collect();
        put_text(
            mat,
            &clipped,
            CvPoint::new(panel_left + 8, rows_count - 74),
            FONT_HERSHEY_SIMPLEX,
            0.36,
            Scalar::new(90.0, 90.0, 255.0, 255.0),
            1,
            LINE_8,
            false,
        )
        .ok();
    }
    put_text(
        mat,
        "Up/Dn row - Lt/Rt adjust - Enter add - w save - Esc close",
        CvPoint::new(panel_left + 8, rows_count - 50),
        FONT_HERSHEY_SIMPLEX,
        0.36,
        WHITE,
        1,
        LINE_8,
        false,
    )
    .ok();
}
