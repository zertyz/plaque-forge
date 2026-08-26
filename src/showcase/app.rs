//! Egui application for the showcase.

use std::path::{Path, PathBuf};
use std::collections::BTreeSet;

use anyhow::Result;
use egui::{Color32, RichText, TextureHandle, TextureOptions, Vec2};

use crate::{
    media::{MediaCatalog, FilesystemCatalog},
    surface::Surface,
    color::Rgba,
};

use super::{
    diagnostics::{draw_quad_border, fill_mask_overlay, to_greyscale},
    fonts::CURATED_FONTS,
    preview::PreviewCache,
    state::{OverlayMode, ShowcaseState},
    styles::StyleDraft,
    video::VideoPlayer,
};

pub struct ShowcaseApp {
    state: ShowcaseState,
    preview: PreviewCache,
    player: Option<VideoPlayer>,
    system_fonts: Vec<String>,
    texture: Option<TextureHandle>,
    last_frame: Option<Surface>,
    style_draft: StyleDraft,
    error_message: Option<String>,
    // UI toggles
    show_style_editor: bool,
    show_inspect_window: bool,
    prev_frame_idx: usize,
}

impl ShowcaseApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        // Load catalog
        let videos = Self::load_videos();
        let styles = Self::load_styles();
        let system_fonts = Self::load_system_fonts();
        let mut state = ShowcaseState::new(videos.clone(), styles.clone());
        let mut preview = PreviewCache::new();
        // try to set font file based on chosen font label
        let font_path = Self::resolve_font(&state.font);
        preview.set_font(font_path.clone());
        preview.set_text(state.text.clone());
        let style_draft = StyleDraft::default();
        // if has initial style, load it
        if let Some(name) = state.current_style_name() {
            let p = PathBuf::from(format!("styles/{name}.toml"));
            if let Ok(d) = StyleDraft::from_style_file(&p) {
                preview.set_style(d.clone());
            }
        } else {
            preview.set_style(style_draft.clone());
        }

        let mut app = Self {
            state,
            preview,
            player: None,
            system_fonts,
            texture: None,
            last_frame: None,
            style_draft,
            error_message: None,
            show_style_editor: true,
            show_inspect_window: false,
            prev_frame_idx: 0,
        };
        app.open_current_video();
        app
    }

    fn load_videos() -> Vec<String> {
        let catalog = FilesystemCatalog::production().ok();
        if let Some(cat) = catalog {
            if let Ok(v) = cat.videos() {
                return v.into_iter().map(|x| x.stem).collect();
            }
        }
        // fallback scan assets/*.mp4
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir("assets") {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("mp4") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        out.push(stem.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    fn load_styles() -> Vec<String> {
        let catalog = FilesystemCatalog::production().ok();
        if let Some(cat) = catalog {
            if let Ok(s) = cat.styles() {
                return s.into_iter().map(|x| x.name).collect();
            }
        }
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir("styles") {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) == Some("toml") {
                    if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                        out.push(stem.to_string());
                    }
                }
            }
        }
        out.sort();
        out
    }

    fn load_system_fonts() -> Vec<String> {
        use crate::media::fonts::{SystemFonts, FamilyIndex};
        let sys = SystemFonts::load();
        // get curated exclude
        let curated_lower: BTreeSet<_> = CURATED_FONTS.iter().map(|s| s.to_lowercase()).collect();
        let mut fonts = CURATED_FONTS.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let mut sys_families = sys.families_excluding(&curated_lower);
        sys_families.sort();
        fonts.extend(sys_families);
        fonts
    }

    fn resolve_font(label: &str) -> PathBuf {
        // Try curated file first
        let curated_path = PathBuf::from(format!("fonts/{label}.ttf"));
        if curated_path.is_file() {
            return curated_path;
        }
        // Try fonts/NotoSerif-Regular.ttf for label NotoSerif-Regular
        let alt = PathBuf::from(format!("fonts/{label}"));
        if alt.is_file() {
            return alt;
        }
        if PathBuf::from("fonts/NotoSerif-Regular.ttf").is_file() {
            return PathBuf::from("fonts/NotoSerif-Regular.ttf");
        }
        // System font via fontdb: find file path via SystemFonts candidate? Fallback to curated
        // Use fc-match via SystemFonts::match_pattern then lookup file? Simpler: return fonts/NotoSerif-Regular.ttf
        PathBuf::from("fonts/NotoSerif-Regular.ttf")
    }

    fn open_current_video(&mut self) {
        if let Some(stem) = self.state.current_video_stem().map(|s| s.to_string()) {
            let path = PathBuf::from(format!("assets/{stem}.mp4"));
            match VideoPlayer::open(&path) {
                Ok(p) => {
                    self.player = Some(p);
                    // Try to set analysis root for preview
                    let analysis_path = crate::workspace::analysis_path(&path).unwrap_or(PathBuf::from(format!("assets/analysis/{stem}")));
                    if analysis_path.is_dir() {
                        self.preview.set_analysis(Some(analysis_path));
                    } else {
                        self.preview.set_analysis(None);
                    }
                    self.error_message = None;
                }
                Err(e) => {
                    self.error_message = Some(format!("Failed to open {}: {e}", path.display()));
                    self.player = None;
                }
            }
        }
    }

    fn current_analysis(&self) -> Option<crate::analysis::Analysis> {
        if let Some(stem) = self.state.current_video_stem() {
            let path = PathBuf::from(format!("assets/{stem}.mp4"));
            if let Ok(ap) = crate::workspace::analysis_path(&path) {
                if let Ok(a) = crate::analysis::Analysis::open(&ap) {
                    return Some(a);
                }
            }
        }
        None
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        let input = ctx.input(|i| {
            let mut keys = Vec::new();
            for ev in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = ev {
                    keys.push((*key, *modifiers));
                }
            }
            keys
        });
        for (key, mods) in input {
            // Prioritize modal handling
            if self.state.text_edit_open {
                if key == egui::Key::Escape {
                    self.state.cancel_text_edit();
                    ctx.request_repaint();
                } else if key == egui::Key::Enter {
                    self.state.commit_text_edit();
                    self.preview.set_text(self.state.text.clone());
                    ctx.request_repaint();
                }
                continue;
            }
            if self.state.save_dialog_open {
                if key == egui::Key::Escape {
                    self.state.cancel_save();
                } else if key == egui::Key::Enter {
                    if let Some(name) = self.state.commit_save() {
                        let draft = self.style_draft.clone();
                        let dest = PathBuf::from(format!("styles/{name}.toml"));
                        match draft.save_to_file(&dest) {
                            Ok(_) => {
                                self.state.styles.push(name.clone());
                                self.state.styles.sort();
                                self.state.current_style = self.state.styles.iter().position(|s| s == &name);
                                self.error_message = Some(format!("Saved style to {}", dest.display()));
                            }
                            Err(e) => self.error_message = Some(format!("Save failed: {e}")),
                        }
                    }
                }
                continue;
            }
            if self.state.font_picker.open {
                match key {
                    egui::Key::Escape => {
                        self.state.cancel_font_picker();
                        let p = Self::resolve_font(&self.state.font);
                        self.preview.set_font(p);
                    }
                    egui::Key::Enter => {
                        self.state.close_font_picker_commit();
                        let p = Self::resolve_font(&self.state.font);
                        self.preview.set_font(p);
                    }
                    egui::Key::ArrowUp => {
                        self.state.font_picker_up();
                        let p = Self::resolve_font(&self.state.font);
                        self.preview.set_font(p);
                    }
                    egui::Key::ArrowDown => {
                        self.state.font_picker_down();
                        let p = Self::resolve_font(&self.state.font);
                        self.preview.set_font(p);
                    }
                    egui::Key::Backspace => {
                        self.state.font_picker.backspace(&self.system_fonts);
                        if let Some(sel) = self.state.font_picker.current_selection().map(|s| s.to_string()) {
                            self.state.font = sel.clone();
                            let p = Self::resolve_font(&sel);
                            self.preview.set_font(p);
                        }
                    }
                    _ => {}
                }
                continue;
            }
            // Global keys when no modal
            match key {
                egui::Key::PageDown => {
                    self.state.next_video();
                    self.open_current_video();
                }
                egui::Key::PageUp => {
                    self.state.prev_video();
                    self.open_current_video();
                }
                egui::Key::Enter => {
                    self.state.open_text_edit();
                }
                egui::Key::ArrowUp => {
                    self.state.style_up();
                    self.apply_current_style();
                }
                egui::Key::ArrowDown => {
                    self.state.style_down();
                    self.apply_current_style();
                }
                egui::Key::Escape => {
                    if self.state.demo_mode {
                        self.state.exit_demo();
                        self.apply_current_style();
                    }
                }
                _ => {
                    // Check char '/' 'd' 'i'
                    // Need to handle via text events separately; use key mapping for slash
                }
            }
        }
        // Text events for '/' 'd' 'i' etc when no modal
        ctx.input(|i| {
            for ev in &i.events {
                if let egui::Event::Text(text) = ev {
                    if self.state.text_edit_open || self.state.save_dialog_open || self.state.font_picker.open {
                        //Handled differently: font picker typing
                        if self.state.font_picker.open {
                            for c in text.chars() {
                                if c.is_ascii_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                                    let handled = self.state.font_picker.handle_char(c, &self.system_fonts);
                                    if handled {
                                        if let Some(sel) = self.state.font_picker.current_selection().map(|s| s.to_string()) {
                                            self.state.font = sel.clone();
                                            let p = Self::resolve_font(&sel);
                                            self.preview.set_font(p);
                                        }
                                    }
                                }
                            }
                        }
                        continue;
                    }
                    for c in text.chars() {
                        match c {
                            '/' => {
                                self.state.open_font_picker();
                            }
                            'd' | 'D' => {
                                if !self.state.demo_mode {
                                    self.state.enter_demo();
                                    self.randomize_demo();
                                } else {
                                    self.state.exit_demo();
                                    self.apply_current_style();
                                }
                            }
                            'i' | 'I' => {
                                // Check shift: capital I indicates Shift held; but we also want Shift+I window
                                if c == 'I' {
                                    self.show_inspect_window = !self.show_inspect_window;
                                } else {
                                    self.state.cycle_overlay();
                                }
                            }
                            's' | 'S' => {
                                // open save dialog with S?
                            }
                            _ => {}
                        }
                    }
                }
            }
        });
        // Also handle single key 'd'/'i' via Key events if text not triggered (e.g., non-printable)
        // Already above via Text; ensure PageUp/PageDown also trigger demo randomize?
    }

    fn apply_current_style(&mut self) {
        if let Some(name) = self.state.current_style_name().map(|s| s.to_string()) {
            let p = PathBuf::from(format!("styles/{name}.toml"));
            if let Ok(d) = StyleDraft::from_style_file(&p) {
                self.style_draft = d.clone();
                self.preview.set_style(d);
            } else {
                // fallback default
                let d = StyleDraft::default();
                self.style_draft = d.clone();
                self.preview.set_style(d);
            }
        } else {
            let d = StyleDraft::default();
            self.style_draft = d.clone();
            self.preview.set_style(d);
        }
        self.state.style_draft_dirty = false;
    }

    fn randomize_demo(&mut self) {
        use rand::seq::SliceRandom;
        let mut rng = rand::thread_rng();
        // Spec: random combos of fonts from hard-coded list
        let font = CURATED_FONTS.choose(&mut rng).cloned().unwrap_or(CURATED_FONTS[0]).to_string();
        let style_idx = if self.state.styles.is_empty() {
            None
        } else {
            Some(rand::random::<usize>() % self.state.styles.len())
        };
        self.state.demo_randomize(font.clone(), style_idx);
        let p = Self::resolve_font(&font);
        self.preview.set_font(p);
        self.apply_current_style();
    }

    fn draw_inspect_overlays(&self, surface: &mut Surface, analysis: Option<&crate::analysis::Analysis>) {
        if self.state.overlay == super::state::OverlayMode::None && self.state.overlay_multi.is_empty() && !self.state.demo_mode {
            return;
        }
        let modes: Vec<OverlayMode> = if self.state.multi_overlay_mode {
            self.state.overlay_multi.clone()
        } else if self.state.overlay != OverlayMode::None {
            vec![self.state.overlay]
        } else {
            vec![]
        };
        let Some(pack) = analysis else { return };
        for mode in modes {
            match mode {
                OverlayMode::PlaqueBounds => {
                    // Use current motion sample; approximate with first frame transform
                    if let Some(sample) = pack.motion.first() {
                        let quad = crate::analyze::extraction::transformed_rect(pack.manifest.source_plaque_rect, sample.transform);
                        draw_quad_border(surface, quad, Rgba::new(255,255,0,255), 2);
                    }
                }
                OverlayMode::Foreground => {
                    // Try to load foreground masks: for demo, load first occluder frame
                    let path = pack.root.join(crate::analysis::OCCLUDER_DIR).join("000000.png");
                    if path.is_file() {
                        if let Ok(img) = image::open(&path) {
                            let luma = img.to_luma8();
                            let mask = luma.into_raw();
                            if mask.len() == surface.width() as usize * surface.height() as usize {
                                fill_mask_overlay(surface, &mask, Rgba::new(0,255,0,180));
                            }
                        }
                    }
                }
                OverlayMode::WritableMask => {
                    let p = pack.root.join(crate::analysis::CONTENT_MASK_FILE);
                    if let Ok(img) = image::open(&p) {
                        let mask = img.to_luma8().into_raw();
                        fill_mask_overlay(surface, &mask, Rgba::new(80,140,255,140));
                    }
                }
                OverlayMode::StructuralMask => {
                    let p = pack.root.join(crate::analysis::STRUCTURAL_MASK_FILE);
                    if let Ok(img) = image::open(&p) {
                        let mask = img.to_luma8().into_raw();
                        fill_mask_overlay(surface, &mask, Rgba::new(255,0,255,140));
                    }
                }
                OverlayMode::Occluder => {
                    let dir = pack.root.join(crate::analysis::OCCLUDER_DIR);
                    if dir.is_dir() {
                        if let Ok(entries) = std::fs::read_dir(&dir) {
                            for e in entries.flatten().take(1) {
                                if let Ok(img) = image::open(e.path()) {
                                    let mask = img.to_luma8().into_raw();
                                    fill_mask_overlay(surface, &mask, Rgba::new(255,165,0,140));
                                }
                            }
                        }
                    }
                }
                OverlayMode::AllDiagnostics | OverlayMode::None => {}
            }
        }
    }
}

impl eframe::App for ShowcaseApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.handle_keys(ctx);

        // Demo mode tick: randomize each time video loops? For now randomize every 5 seconds
        if self.state.demo_mode {
            ctx.request_repaint_after(std::time::Duration::from_secs(5));
            // Simple time-based randomization via repaint; actual per-video randomization handled by player loop detection
        }

        // Top bar
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("Plaque Forge Showcase");
                ui.separator();
                if let Some(stem) = self.state.current_video_stem() {
                    ui.label(RichText::new(format!("Video: {stem}")).strong());
                }
                if let Some(p) = &self.player {
                    ui.label(format!("{}x{} @ {:.1}fps", p.width, p.height, p.fps));
                    if !p.has_analysis {
                        ui.colored_label(Color32::YELLOW, "No analysis");
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(self.state.font_style_label()).monospace().small());
                    if self.state.demo_mode {
                        ui.colored_label(Color32::from_rgb(255, 180, 0), "DEMO");
                    }
                    if self.state.overlay != OverlayMode::None {
                        ui.colored_label(Color32::YELLOW, format!("Inspect: {}", self.state.overlay.label()));
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.label("PgUp/PgDown: video • Enter: text • /: fonts • Up/Down: style • i: inspect • Shift+I: multi • d: demo • s: save");
                if ui.button("Style Editor").clicked() {
                    self.show_style_editor = !self.show_style_editor;
                }
                if ui.button("Save Style").clicked() {
                    self.state.open_save_dialog();
                }
            });
            if let Some(err) = &self.error_message {
                ui.colored_label(Color32::LIGHT_RED, err);
            }
        });

        // Style editor side panel (full parametric)
        if self.show_style_editor {
            egui::SidePanel::right("style_editor").min_width(340.0).show(ctx, |ui| {
                ui.heading("Style Lab");
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.collapsing("Typography", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Weight");
                            let mut w = self.style_draft.font_weight as i32;
                            if ui.add(egui::Slider::new(&mut w, 1..=1000)).changed() {
                                self.style_draft.font_weight = w as u16;
                                self.preview.set_style(self.style_draft.clone());
                                self.state.style_draft_dirty = true;
                            }
                        });
                    });
                    ui.collapsing("Fill / Material", |ui| {
                        ui.horizontal(|ui| {
                            ui.label("Kind");
                            egui::ComboBox::from_id_salt("fill_kind")
                                .selected_text(format!("{:?}", self.style_draft.fill_kind).split('(').next().unwrap_or(""))
                                .show_ui(ui, |ui| {
                                    let kinds = [
                                        ("Flat", super::styles::FillKind::Flat("#FFFFFFFF".into())),
                                        ("Linear Gradient", super::styles::FillKind::LinearGradient { top: "#FF0000FF".into(), bottom: "#0000FFFF".into() }),
                                        ("Gold", super::styles::FillKind::Gold { dark: "#5B3210FF".into(), mid: "#C98B3CFF".into(), light: "#F3D38AFF".into(), highlight: "#FFF1C4FF".into() }),
                                        ("Chrome", super::styles::FillKind::Chrome { dark: "#182436FF".into(), mid: "#8EA9C7FF".into(), light: "#F7FBFFFF".into() }),
                                        ("Holographic", super::styles::FillKind::Holographic),
                                        ("Fire", super::styles::FillKind::Fire { dark: "#380707FF".into(), mid: "#D84A0FFF".into(), light: "#FFE066FF".into() }),
                                        ("Ice", super::styles::FillKind::Ice { dark: "#0B2447FF".into(), mid: "#408EE0FF".into(), light: "#EBF6FFFF".into() }),
                                        ("Nebula", super::styles::FillKind::Nebula { dark: "#140728FF".into(), mid: "#7622A8FF".into(), light: "#F18AEBFF".into() }),
                                        ("Liquid", super::styles::FillKind::Liquid { first: "#184E77FF".into(), second: "#52B69AFF".into(), frequency: 4.0 }),
                                        ("Halftone", super::styles::FillKind::Halftone { foreground: "#111827FF".into(), background: "#F3F4F6FF".into(), cell: 6 }),
                                        ("Blueprint", super::styles::FillKind::Blueprint { dark: "#082E5EFF".into(), light: "#5FD8FFFF".into(), grid: "#D7F6FFB8".into(), cell: 8 }),
                                        ("Paper", super::styles::FillKind::Paper { light: "#FFF3D2FF".into(), mid: "#D6B988FF".into(), dark: "#7B5B38FF".into(), seed: 0x5041_5045 }),
                                    ];
                                    for (label, kind) in kinds {
                                        if ui.selectable_label(false, label).clicked() {
                                            self.style_draft.fill_kind = kind;
                                            self.preview.set_style(self.style_draft.clone());
                                            self.state.style_draft_dirty = true;
                                        }
                                    }
                                });
                        });
                        // Color pickers for current fill
                        match &mut self.style_draft.fill_kind.clone() {
                            super::styles::FillKind::Flat(c) => {
                                let mut col = parse_color(c);
                                if ui_color_picker(ui, &mut col) {
                                    self.style_draft.fill_kind = super::styles::FillKind::Flat(format_color(col));
                                    self.preview.set_style(self.style_draft.clone());
                                }
                            }
                            _ => {
                                ui.label("Select parameters via specific material UI (colors editable via pickers in detailed view).");
                            }
                        }
                    });
                    ui.collapsing("Layouts (Arc / Orbital)", |ui| {
                        let mut changed = false;
                        let len = self.style_draft.layouts.len();
                        for idx in 0..len {
                            ui.horizontal(|ui| {
                                ui.label(format!("Arc {idx}"));
                                let mut sweep = self.style_draft.layouts[idx].sweep_degrees;
                                if ui.add(egui::Slider::new(&mut sweep, -330.0..=330.0).text("sweep")).changed() {
                                    self.style_draft.layouts[idx].sweep_degrees = sweep;
                                    changed = true;
                                }
                                let mut scale = self.style_draft.layouts[idx].radius_scale;
                                if ui.add(egui::Slider::new(&mut scale, 0.2..=5.0).text("radius")).changed() {
                                    self.style_draft.layouts[idx].radius_scale = scale;
                                    changed = true;
                                }
                                if ui.button("✕").clicked() {
                                    // defer removal
                                }
                            });
                        }
                        if changed {
                            self.preview.set_style(self.style_draft.clone());
                        }
                        if ui.button("+ Add Arc").clicked() {
                            self.style_draft.layouts.push(super::styles::LayoutDraft { sweep_degrees: 58.0, radius_scale: 1.0 });
                            self.preview.set_style(self.style_draft.clone());
                        }
                        if ui.button("+ Add Orbit (Arc + Animation)").clicked() {
                            self.style_draft.layouts.push(super::styles::LayoutDraft { sweep_degrees: 60.0, radius_scale: 1.0 });
                            self.style_draft.animations.push(super::styles::AnimationDraft::Orbit { period: 8.0, degrees: 360.0, phase: 0.0 });
                            self.preview.set_style(self.style_draft.clone());
                        }
                    });
                    ui.collapsing("Underlays (Stroke/Glow/Shadow/Extrude/Chromatic/Trail)", |ui| {
                        for idx in 0..self.style_draft.underlays.len() {
                            let mut remove = false;
                            ui.horizontal(|ui| {
                                ui.label(format!("Underlay {idx}"));
                                if ui.button("✕").clicked() { remove = true; }
                            });
                            if remove {
                                self.style_draft.underlays.remove(idx);
                                self.preview.set_style(self.style_draft.clone());
                                break;
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.button("+ Stroke").clicked() { self.style_draft.underlays.push(super::styles::UnderlayDraft::Stroke { width: 0.05, color: "#03181ED2".into() }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Glow").clicked() { self.style_draft.underlays.push(super::styles::UnderlayDraft::Glow { radius: 10, color: "#69F2FA90".into() }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Shadow").clicked() { self.style_draft.underlays.push(super::styles::UnderlayDraft::Shadow { offset_x: 0.03, offset_y: 0.04, blur: 6, color: "#000000A0".into() }); self.preview.set_style(self.style_draft.clone()); }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("+ Extrude").clicked() { self.style_draft.underlays.push(super::styles::UnderlayDraft::Extrude { depth: 0.1, angle: 55.0, color: "#2A1608D8".into() }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Chromatic").clicked() { self.style_draft.underlays.push(super::styles::UnderlayDraft::ChromaticSplit { offset: 0.025, red: "#FF2A55CC".into(), cyan: "#2AD5FFCC".into() }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Trail").clicked() { self.style_draft.underlays.push(super::styles::UnderlayDraft::Trail { distance: 0.16, copies: 4, angle: 180.0, color: "#FFB70366".into() }); self.preview.set_style(self.style_draft.clone()); }
                        });
                    });
                    ui.collapsing("Overlays (Bevel/Letterpress)", |ui| {
                        for idx in 0..self.style_draft.overlays.len() {
                            ui.label(format!("Overlay {idx}: {:?}", self.style_draft.overlays[idx]));
                        }
                        if ui.button("+ Bevel").clicked() { self.style_draft.overlays.push(super::styles::OverlayDraft::Bevel { width: 0.05, highlight: "#FFF1C0B8".into(), shadow: "#321B08B8".into() }); self.preview.set_style(self.style_draft.clone()); }
                        if ui.button("+ Letterpress").clicked() { self.style_draft.overlays.push(super::styles::OverlayDraft::Letterpress { width: 0.04, highlight: "#FFFFFF55".into(), shadow: "#00000077".into() }); self.preview.set_style(self.style_draft.clone()); }
                    });
                    ui.collapsing("Surface Effects (Laser/Emboss)", |ui| {
                        for idx in 0..self.style_draft.surface_effects.len() {
                            ui.label(format!("Surface {idx}: {:?}", self.style_draft.surface_effects[idx]));
                        }
                        if ui.button("+ Laser Burn").clicked() { self.style_draft.surface_effects.push(super::styles::SurfaceEffectDraft::LaserBurn { depth: 0.72, warmth: 0.65, edge: 2, seed: 0x4255_524E }); self.preview.set_style(self.style_draft.clone()); }
                        if ui.button("+ Emboss").clicked() { self.style_draft.surface_effects.push(super::styles::SurfaceEffectDraft::Emboss { depth: 0.65, highlight: 0.72, shadow: 0.68, light_angle: None, cast: 2 }); self.preview.set_style(self.style_draft.clone()); }
                    });
                    ui.collapsing("Animations", |ui| {
                        for idx in 0..self.style_draft.animations.len() {
                            ui.label(format!("Anim {idx}: {:?}", self.style_draft.animations[idx]));
                        }
                        egui::Grid::new("anim_buttons").show(ui, |ui| {
                            if ui.button("+ Pulse").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Pulse { period: 2.4, min: 0.82, max: 1.0, phase: 0.0 }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Shine").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Shine { period: 2.8, width: 0.18, angle: 35.0, color: "#FFFFFFB8".into() }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Flicker").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Flicker { period: 1.6, min: 0.65, strength: 0.32, phase: 0.0 }); self.preview.set_style(self.style_draft.clone()); }
                            ui.end_row();
                            if ui.button("+ Wave").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Wave { period: 2.8, amp: 0.035, wave: 0.42, phase: 0.0 }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Typewriter").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Typewriter { period: 4.0, hold: 0.35 }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Dissolve").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Dissolve { period: 4.0, hold: 0.35, seed: 0x504C_4151 }); self.preview.set_style(self.style_draft.clone()); }
                            ui.end_row();
                            if ui.button("+ Scramble").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Scramble { period: 3.8, hold: 0.30, steps: 15.0, seed: 0x5343_524D }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ SplitFlap").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::SplitFlap { period: 4.2, hold: 0.30, steps: 16.0 }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Confetti").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Confetti { period: 4.4, hold: 0.35, pieces: 720, spread: 0.48, seed: 0x434F_4E46 }); self.preview.set_style(self.style_draft.clone()); }
                            ui.end_row();
                            if ui.button("+ Glitch").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Glitch { period: 2.6, ripple: 0.018, slice: 0.085, burst: 0.20, seed: 0x474C_4954 }); self.preview.set_style(self.style_draft.clone()); }
                            if ui.button("+ Orbit").clicked() { self.style_draft.animations.push(super::styles::AnimationDraft::Orbit { period: 8.0, degrees: 360.0, phase: 0.0 }); self.preview.set_style(self.style_draft.clone()); }
                            ui.end_row();
                        });
                        if !self.style_draft.animations.is_empty() && ui.button("Clear Animations").clicked() {
                            self.style_draft.animations.clear();
                            self.preview.set_style(self.style_draft.clone());
                        }
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Reset Draft").clicked() {
                            self.style_draft = StyleDraft::default();
                            self.preview.set_style(self.style_draft.clone());
                        }
                        if ui.button("Save…").clicked() {
                            self.state.open_save_dialog();
                        }
                    });
                });
            });
        }

        // Central video
        egui::CentralPanel::default().show(ctx, |ui| {
            // Poll video frame
            let mut surface_opt: Option<Surface> = None;
            let mut has_analysis = false;
            let analysis = self.current_analysis();
            has_analysis = analysis.is_some();
            let mut demo_wrapped = false;
            if let Some(player) = &mut self.player {
                match player.next_surface() {
                    Ok(Some(s)) => surface_opt = Some(s),
                    Ok(None) => {
                        // loop already handled in next_surface
                        surface_opt = player.next_surface().ok().flatten();
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Decode error: {e}"));
                    }
                }
                // FPS-aware repaint
                let dur = player.frame_duration();
                ctx.request_repaint_after(dur);
                let cur = player.current_frame;
                demo_wrapped = self.state.demo_mode && cur < self.prev_frame_idx && cur < 5 && self.prev_frame_idx > 10;
                self.prev_frame_idx = cur;
            }
            if demo_wrapped {
                self.state.next_video();
                self.open_current_video();
                self.randomize_demo();
                has_analysis = self.current_analysis().is_some();
                // Fetch first frame of new video immediately
                if let Some(player) = &mut self.player {
                    surface_opt = player.next_surface().ok().flatten();
                    let dur = player.frame_duration();
                    ctx.request_repaint_after(dur);
                    self.prev_frame_idx = player.current_frame;
                } else {
                    surface_opt = None;
                }
            }

            if let Some(mut surface) = surface_opt {
                let time = self.player.as_ref().map(|p| p.time_seconds()).unwrap_or(0.0);
                // Analysis missing -> greyscale + dark bar (diagnostics handles bar; we do greyscale here)
                if !has_analysis {
                    to_greyscale(&mut surface);
                } else {
                    // overlays before text? Draw behind text but after frame? We'll draw under text for plaque bounds etc via diagnostics
                    self.draw_inspect_overlays(&mut surface, analysis.as_ref());
                }

                // Render text preview onto surface
                let analysis_ref = analysis.as_ref();
                let rendered = self.preview.render_frame(&surface, time, analysis_ref);
                let final_surface = match rendered {
                    Ok(s) => s,
                    Err(e) => {
                        self.error_message = Some(format!("Render error: {e}"));
                        surface
                    }
                };

                // Handle demo analysis missing: if no analysis, final_surface is already greyscale + centered text; we still want text visible
                // Also handle overlay green etc already drawn before render? Our preview renders text after overlays; good.

                // Convert to egui texture
                let w = final_surface.width() as usize;
                let h = final_surface.height() as usize;
                let pixels = final_surface.pixels();
                // pixels are RGBA
                let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], pixels);
                let tex = self.texture.get_or_insert_with(|| {
                    ctx.load_texture("video", color_image.clone(), TextureOptions::LINEAR)
                });
                tex.set(color_image, TextureOptions::LINEAR);

                let available = ui.available_size();
                let img_size = Vec2::new(w as f32, h as f32);
                let scale = (available.x / img_size.x).min(available.y / img_size.y).min(1.5);
                let display_size = img_size * scale;
                let rect = egui::Rect::from_center_size(ui.available_rect_before_wrap().center(), display_size);
                ui.put(rect, egui::Image::from_texture((tex.id(), display_size)));

                // Overlay missing analysis notice as egui text (centered)
                if !has_analysis {
                    let center = rect.center();
                    egui::Area::new(egui::Id::new("missing_notice"))
                        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                        .show(ctx, |ui| {
                            egui::Frame::new().fill(Color32::from_black_alpha(180)).corner_radius(6).inner_margin(12).show(ui, |ui| {
                                ui.colored_label(Color32::WHITE, RichText::new("No analysis data for this video").heading().strong());
                                ui.colored_label(Color32::LIGHT_GRAY, "Consult the documentation on how to generate it");
                            });
                        });
                }

                // Demo label
                if self.state.demo_mode {
                    let combo = self.state.font_style_label();
                    egui::Area::new(egui::Id::new("demo_label"))
                        .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -20.0))
                        .show(ctx, |ui| {
                            egui::Frame::new().fill(Color32::from_black_alpha(150)).corner_radius(4).inner_margin(6).show(ui, |ui| {
                                ui.colored_label(Color32::YELLOW, combo);
                            });
                        });
                }
                // Inspect badge already in top bar; also show overlay label near bottom
                if self.state.overlay != OverlayMode::None && !has_analysis {
                    // Still show badge even without analysis – but overlays are ineffective
                }
                self.last_frame = Some(final_surface);
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label("No video loaded");
                });
                if self.player.is_none() && !self.state.videos.is_empty() {
                    if ui.button("Retry open").clicked() {
                        self.open_current_video();
                    }
                }
            }
        });

        // Font picker popup
        if self.state.font_picker.open {
            egui::Window::new("Fonts – Up/Down to preview, type to search, Enter confirm, Esc cancel")
                .collapsible(false)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.label(format!("Mode: {:?}  Query: '{}'  Selected: {}", self.state.font_picker.mode, self.state.font_picker.query, self.state.font));
                    ui.separator();
                    let filter_text = if self.state.font_picker.mode == super::state::FontPickerMode::Search {
                        format!("Filter: {}", self.state.font_picker.query)
                    } else {
                        "Curated list – start typing to search all system fonts".to_string()
                    };
                    ui.label(filter_text);
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        for (idx, font) in self.state.font_picker.filtered.iter().enumerate() {
                            let selected = idx == self.state.font_picker.selected;
                            if ui.selectable_label(selected, font).clicked() {
                                self.state.font_picker.selected = idx;
                                self.state.font = font.clone();
                                let p = Self::resolve_font(font);
                                self.preview.set_font(p);
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        if ui.button("Up").clicked() { self.state.font_picker_up(); let p = Self::resolve_font(&self.state.font); self.preview.set_font(p); }
                        if ui.button("Down").clicked() { self.state.font_picker_down(); let p = Self::resolve_font(&self.state.font); self.preview.set_font(p); }
                        if ui.button("Enter – Confirm").clicked() { self.state.close_font_picker_commit(); let p = Self::resolve_font(&self.state.font); self.preview.set_font(p); }
                        if ui.button("Esc – Cancel").clicked() { self.state.cancel_font_picker(); let p = Self::resolve_font(&self.state.font); self.preview.set_font(p); }
                    });
                    // Mouse support: Up/Down already, plus typing handled via ctx.input above
                });
        }

        // Text edit modal
        if self.state.text_edit_open {
            egui::Window::new("Edit Text – Enter to confirm, Esc to cancel")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.text_edit_singleline(&mut self.state.text_edit_buffer);
                    ui.horizontal(|ui| {
                        if ui.button("OK (Enter)").clicked() {
                            self.state.commit_text_edit();
                            self.preview.set_text(self.state.text.clone());
                        }
                        if ui.button("Cancel (Esc)").clicked() {
                            self.state.cancel_text_edit();
                        }
                    });
                });
        }

        // Save dialog
        if self.state.save_dialog_open {
            egui::Window::new("Save Style – Enter to confirm")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Style name (without .toml):");
                    ui.text_edit_singleline(&mut self.state.save_name);
                    ui.horizontal(|ui| {
                        if ui.button("Save (Enter)").clicked() {
                            if let Some(name) = self.state.commit_save() {
                                let draft = self.style_draft.clone();
                                let dest = PathBuf::from(format!("styles/{name}.toml"));
                                match draft.save_to_file(&dest) {
                                    Ok(_) => {
                                        self.state.styles.push(name.clone());
                                        self.state.styles.sort();
                                        self.state.current_style = self.state.styles.iter().position(|s| s == &name);
                                        self.error_message = Some(format!("Saved to {}", dest.display()));
                                    }
                                    Err(e) => self.error_message = Some(format!("Save failed: {e}")),
                                }
                            }
                        }
                        if ui.button("Cancel (Esc)").clicked() {
                            self.state.cancel_save();
                        }
                    });
                });
        }

        // Inspect multi window Shift+I
        if self.show_inspect_window {
            egui::Window::new("Inspect Overlays – click to toggle, i cycles")
                .collapsible(false)
                .show(ctx, |ui| {
                    ui.label("Yellow: plaque bounds • Green: foreground • Blue: writable • Magenta: structural • Orange: occluder");
                    for &mode in super::state::OverlayMode::ALL {
                        if mode == OverlayMode::None { continue; }
                        let mut checked = self.state.overlay_multi.contains(&mode);
                        if mode == OverlayMode::AllDiagnostics {
                            checked = self.state.overlay_multi.len() >= 5;
                        }
                        if ui.checkbox(&mut checked, format!("{} {:?}", mode.label(), mode.color())).changed() {
                            if mode == OverlayMode::AllDiagnostics {
                                if checked {
                                    self.state.overlay_multi = vec![OverlayMode::PlaqueBounds, OverlayMode::Foreground, OverlayMode::WritableMask, OverlayMode::StructuralMask, OverlayMode::Occluder];
                                } else {
                                    self.state.overlay_multi.clear();
                                }
                                self.state.multi_overlay_mode = !self.state.overlay_multi.is_empty();
                            } else {
                                self.state.toggle_multi_overlay(mode);
                            }
                        }
                    }
                    if ui.button("Clear All").clicked() {
                        self.state.overlay_multi.clear();
                        self.state.multi_overlay_mode = false;
                        self.state.overlay = OverlayMode::None;
                    }
                    // Also show analysis info if available
                    if let Some(pack) = self.current_analysis() {
                        ui.separator();
                        ui.label(format!("Analysis: {} | Trajectory: {} | Has occluder: {} | FPS {:.1}", pack.manifest.analyzer_build, pack.manifest.trajectory_model, pack.manifest.has_occluder, pack.manifest.source.fps));
                    } else {
                        ui.colored_label(Color32::YELLOW, "No analysis cache for current video");
                    }
                });
        }
    }
}

fn parse_color(s: &str) -> Color32 {
    if let Ok(rgba) = crate::color::Rgba::parse(s) {
        Color32::from_rgba_unmultiplied(rgba.r, rgba.g, rgba.b, rgba.a)
    } else {
        Color32::WHITE
    }
}
fn format_color(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}{:02X}", c.r(), c.g(), c.b(), c.a())
}
fn ui_color_picker(ui: &mut egui::Ui, col: &mut Color32) -> bool {
    let mut changed = false;
    let mut hsva = egui::color_picker::color_edit_button_srgba(ui, col, egui::color_picker::Alpha::BlendOrAdditive);
    // simpler: use color_edit_button_srgba
    // The above already handles picking; detect change via comparison
    // For simplicity, use button
    changed
}
