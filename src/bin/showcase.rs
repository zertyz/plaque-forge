//! plaque-forge-showcase — interactive stage for text styles over live video.
//!
//! Plays every analyzed asset in a loop and composites a live-editable title
//! through the same pipeline the CLI renders with. Interaction logic lives in
//! `plaque_forge::showcase` modules; this binary owns decoding, scaling,
//! OpenCV highgui plumbing, and key dispatch.

use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use image::{ImageBuffer, RgbaImage, imageops::FilterType};
use opencv::core::Point as CvPoint;
use opencv::core::{Mat, Scalar, Vector};
use opencv::imgcodecs::{IMREAD_COLOR, imdecode};
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
use plaque_forge::showcase::fonts::FontPicker;
use plaque_forge::showcase::keys::{Key, normalize};
use plaque_forge::showcase::quality::Tier;
use plaque_forge::showcase::state::{DemoState, Mode};
use plaque_forge::surface::Surface;
use plaque_forge::video::{self, Decoder};
use plaque_forge::workspace;

const WINDOW: &str = "plaque-forge-showcase";
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
    decoder: Option<Decoder>,
    index: usize,
    paused: bool,
    previous: Option<Surface>,
}

impl Player {
    fn spawn(ffmpeg: &Path, input: &Path) -> Result<Self> {
        let info = video::probe(Path::new("ffprobe"), input)?;
        info.ensure_supported_compositing_color()?;
        let decoder = Some(Decoder::spawn_from(ffmpeg, input, &info, 0)?);
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
        self.decoder = Some(Decoder::spawn_from(
            &self.ffmpeg,
            &self.input.clone(),
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

struct Session {
    fit: plaque_forge::application::FitMode,
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
    toast: String,
}

impl Session {
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
        Ok(())
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
        None => 0,
    };

    let session = Session {
        fit: parse_fit(&args.fit),
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
        toast: String::new(),
    };
    run_session(session, args.width, args.smoke)
}

fn parse_fit(name: &str) -> plaque_forge::application::FitMode {
    match name {
        "maximize" => plaque_forge::application::FitMode::Maximize,
        "balanced" => plaque_forge::application::FitMode::Balanced,
        "fixed" => plaque_forge::application::FitMode::Fixed,
        _ => plaque_forge::application::FitMode::Artistic,
    }
}

fn run_session(mut session: Session, display_width: u32, smoke: Option<usize>) -> Result<()> {
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
    opencv::highgui::named_window(WINDOW, opencv::highgui::WINDOW_AUTOSIZE)
        .context("failed to create showcase window")?;
    let result = event_loop(&mut session, display_width);
    let _ = opencv::highgui::destroy_all_windows();
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

fn event_loop(session: &mut Session, display_width: u32) -> Result<()> {
    loop {
        let decoded = if !session.player.paused {
            session.player.advance()?
        } else {
            None
        };
        if decoded.is_none() && !session.player.paused {
            // End of stream: advance to the next asset (demo re-rolls picks).
            let demo = matches!(session.mode, Mode::Demo(_));
            session.cycle_video(1)?;
            if demo {
                start_demo(session)?;
                continue;
            }
            continue;
        }
        let shown = decoded.clone().or_else(|| session.player.previous.clone());
        let Some(source) = shown else { continue };

        let frame_index = session
            .player
            .index
            .saturating_sub(1)
            .min(source_frame_budget(session));
        let mut prepared = source;
        if let Some(bake) = session.bake.as_mut() {
            bake.compositor.composite(&mut prepared, frame_index)?;
        }
        if session.inspect {
            draw_inspect_overlays(session, &mut prepared, frame_index)?;
        }
        present(session, prepared, display_width)?;

        let typing = matches!(session.mode, Mode::EnteringText(_) | Mode::SavingName(_));
        let delay_ms = if session.player.paused
            || typing
            || matches!(session.mode, Mode::PickingFont(_) | Mode::Composing(_))
        {
            30
        } else {
            (1000.0 / session.player.info.fps.max(1.0)).clamp(15.0, 60.0) as i32
        };
        let raw = opencv::highgui::wait_key(delay_ms)?;
        if raw < 0 {
            continue;
        }
        if dispatch_key(session, normalize(raw))? {
            return Ok(());
        }
    }
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
            session.player.restart()?;
        }
        Key::Left => {
            let step = (session.player.info.fps.round() as i64 * 5).max(1);
            session.player.seek_by(-step)?;
        }
        Key::Right => {
            let step = (session.player.info.fps.round() as i64 * 5).max(1);
            session.player.seek_by(step)?;
        }
        Key::Char(',') => {
            session.player.step_back();
        }
        Key::Char('.') => {
            session.player.step_forward()?;
        }
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

fn hud_lines(session: &Session) -> Vec<String> {
    let mut top = vec![format!(
        "[{}] {} ({}/{})  font: {}{}",
        session.tier.label(),
        session.asset().stem,
        session.video_index + 1,
        session.videos.len(),
        session
            .font_choices
            .get(session.font_index)
            .map(|(l, _)| l.clone())
            .unwrap_or_default(),
        if session
            .font_choices
            .get(session.font_index)
            .is_some_and(|(_, curated)| *curated)
        {
            "*"
        } else {
            ""
        },
    )];
    top.push(format!(
        "style: {}   PgUp/PgDn video · Up/Dn style · '/' fonts · Enter text · e edit · s save · d demo · i inspect · f tier · q quit",
        session.style_names.get(session.style_index).cloned().unwrap_or_default(),
    ));
    let mut notes = Vec::new();
    if session.bake.is_none() {
        notes.push(session.bake_note.clone());
    } else if session.bake.as_ref().is_some_and(|bake| bake.stale) {
        notes.push("analysis cache is stale (render anyway)".into());
    }
    if !matches!(session.mode, Mode::Viewing) {
        notes.push(format!("mode: {}", session.mode.label()));
    }
    if !session.toast.is_empty() {
        notes.push(session.toast.clone());
    }
    if let Some(error) = match &session.mode {
        Mode::Composing(model) => model.error().map(str::to_string),
        _ => None,
    } {
        notes.push(error);
    }
    top.extend(notes.into_iter().take(2));
    top
}

fn present(session: &mut Session, frame: Surface, display_width: u32) -> Result<()> {
    let scale = display_width as f32 / frame.width().max(1) as f32;
    let scaled_w = (frame.width() as f32 * scale).round().max(1.0) as u32;
    let scaled_h = (frame.height() as f32 * scale).round().max(1.0) as u32;
    let view: RgbaImage =
        ImageBuffer::from_raw(frame.width(), frame.height(), frame.pixels().to_vec())
            .context("frame buffer mismatch")?;
    let scaled = image::imageops::resize(&view, scaled_w, scaled_h, FilterType::Triangle);

    let mut png = Vec::with_capacity((scaled_w * scaled_h / 2) as usize);
    image::DynamicImage::ImageRgba8(scaled)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)?;
    let vector: Vector<u8> = png.into_iter().collect();
    let mat = imdecode(&vector, IMREAD_COLOR)?;
    anyhow::ensure!(!mat.empty(), "decoded preview frame is empty");

    // HUD.
    let mut shadowed = mat.clone();
    let lines = hud_lines(session);
    for (row, line) in lines.iter().enumerate() {
        let org = CvPoint::new(10, 26 + row as i32 * 22);
        put_text(
            &mut shadowed,
            line,
            org,
            FONT_HERSHEY_SIMPLEX,
            0.55,
            BLACK,
            3,
            LINE_8,
            false,
        )?;
        put_text(
            &mut shadowed,
            line,
            org,
            FONT_HERSHEY_SIMPLEX,
            0.55,
            WHITE,
            1,
            LINE_8,
            false,
        )?;
    }
    // Popups drawn last so they sit above the HUD.
    match &session.mode {
        Mode::EnteringText(buffer) => draw_entry_bar(&shadowed, "new title:", buffer),
        Mode::SavingName(buffer) => draw_entry_bar(&shadowed, "save as:", buffer),
        Mode::PickingFont(picker) => draw_font_popup(&shadowed, picker),
        Mode::Composing(model) => draw_composer_panel(&shadowed, model),
        _ => {}
    }

    opencv::highgui::imshow(WINDOW, &shadowed)?;
    Ok(())
}

fn dim_strip(mat: &mut Mat, height: i32) {
    rectangle_points(
        mat,
        CvPoint::new(0, mat.rows() - height),
        CvPoint::new(mat.cols(), mat.rows()),
        BLACK,
        -1,
        LINE_8,
        0,
    )
    .ok();
}

fn draw_entry_bar(mat: &Mat, prompt: &str, buffer: &str) {
    let mut copy = mat.clone();
    dim_strip(&mut copy, 46);
    let text = format!("{prompt} {buffer}_");
    let org = CvPoint::new(12, copy.rows() - 16);
    put_text(
        &mut copy,
        &text,
        org,
        FONT_HERSHEY_SIMPLEX,
        0.65,
        WHITE,
        1,
        LINE_8,
        false,
    )
    .ok();
    imshow_replace(copy);
}

fn draw_font_popup(mat: &Mat, picker: &FontPicker) {
    let mut copy = mat.clone();
    let query_line = match picker.query() {
        Some(query) => format!("search: {query}_"),
        None => "curated+system fonts  (type to search)".to_string(),
    };
    let rows: Vec<_> = picker.rows().collect();
    let visible_height = rows.len().min(14) as i32 * 22 + 56;
    rectangle_points(
        &mut copy,
        CvPoint::new(20, 20),
        CvPoint::new(mat.cols() - 20, 20 + visible_height),
        BLACK,
        -1,
        LINE_8,
        0,
    )
    .ok();
    put_text(
        &mut copy,
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
    let cursor_position = rows
        .iter()
        .position(|(index, _)| *index == picker.cursor())
        .unwrap_or(0);
    let start = cursor_position.saturating_sub(6);
    for (row_offset, (entry_index, choice)) in rows.iter().skip(start).take(14).enumerate() {
        let marker = if choice.curated { "*" } else { " " };
        let selected = *entry_index == picker.cursor();
        let color = if selected { YELLOW } else { WHITE };
        let text = format!("{marker} {}", choice.label);
        let org = CvPoint::new(34, 68 + row_offset as i32 * 22);
        put_text(
            &mut copy,
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
    imshow_replace(copy);
}

fn draw_composer_panel(mat: &Mat, model: &EditModel) {
    let mut copy = mat.clone();
    let panel_width = 380;
    rectangle_points(
        &mut copy,
        CvPoint::new(mat.cols() - panel_width - 12, 12),
        CvPoint::new(mat.cols() - 12, mat.rows() - 60),
        BLACK,
        -1,
        LINE_8,
        0,
    )
    .ok();
    let mut y = 36;
    put_text(
        &mut copy,
        "STYLE COMPOSER",
        CvPoint::new(mat.cols() - panel_width, y),
        FONT_HERSHEY_SIMPLEX,
        0.55,
        YELLOW,
        1,
        LINE_8,
        false,
    )
    .ok();
    y += 26;
    let rows = model.rows();
    let cursor = model.cursor();
    let start = cursor.saturating_sub(8);
    for (offset, row) in rows.iter().skip(start).take(14).enumerate() {
        let index = start + offset;
        let selected = index == cursor;
        let value = model.row_value(index);
        let color = if selected { YELLOW } else { WHITE };
        let label = if row.label.chars().count() > 24 {
            format!("{}…", row.label.chars().take(23).collect::<String>())
        } else {
            row.label.clone()
        };
        let text = format!(
            "{} {:<24} {}",
            if selected { '>' } else { ' ' },
            label,
            value
        );
        put_text(
            &mut copy,
            &text,
            CvPoint::new(mat.cols() - panel_width, y),
            FONT_HERSHEY_SIMPLEX,
            0.42,
            color,
            1,
            LINE_8,
            false,
        )
        .ok();
        y += 20;
        if y > mat.rows() - 90 {
            break;
        }
    }
    if let Some(error) = model.error() {
        let clipped: String = error.chars().take(58).collect();
        put_text(
            &mut copy,
            &clipped,
            CvPoint::new(mat.cols() - panel_width, mat.rows() - 72),
            FONT_HERSHEY_SIMPLEX,
            0.38,
            Scalar::new(80.0, 80.0, 255.0, 255.0),
            1,
            LINE_8,
            false,
        )
        .ok();
    }
    put_text(
        &mut copy,
        "Up/Dn pick · Lt/Rt adjust · Enter add · w save · Esc close",
        CvPoint::new(mat.cols() - panel_width, mat.rows() - 48),
        FONT_HERSHEY_SIMPLEX,
        0.38,
        WHITE,
        1,
        LINE_8,
        false,
    )
    .ok();
    imshow_replace(copy);
}

/// Swap the freshly annotated frame into the window.
fn imshow_replace(mat: Mat) {
    opencv::highgui::imshow(WINDOW, &mat).ok();
}
