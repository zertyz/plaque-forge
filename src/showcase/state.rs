//! Pure showcase state machine – testable without egui or video decode.

use super::fonts::CURATED_FONTS;

/// Overlay kinds that `i` cycles through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMode {
    None,
    PlaqueBounds,
    Foreground,
    WritableMask,
    StructuralMask,
    Occluder,
    AllDiagnostics,
}

impl OverlayMode {
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::PlaqueBounds,
            Self::PlaqueBounds => Self::Foreground,
            Self::Foreground => Self::WritableMask,
            Self::WritableMask => Self::StructuralMask,
            Self::StructuralMask => Self::Occluder,
            Self::Occluder => Self::AllDiagnostics,
            Self::AllDiagnostics => Self::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::PlaqueBounds => "Plaque Bounds (yellow)",
            Self::Foreground => "Foreground Objects (green)",
            Self::WritableMask => "Writable Mask (blue)",
            Self::StructuralMask => "Structural Mask (magenta)",
            Self::Occluder => "Occluder Coverage (orange)",
            Self::AllDiagnostics => "All Overlays",
        }
    }

    pub fn color(self) -> [u8; 4] {
        match self {
            Self::None => [0, 0, 0, 0],
            Self::PlaqueBounds => [255, 255, 0, 255],
            Self::Foreground => [0, 255, 0, 180],
            Self::WritableMask => [80, 140, 255, 140],
            Self::StructuralMask => [255, 0, 255, 140],
            Self::Occluder => [255, 165, 0, 140],
            Self::AllDiagnostics => [255, 255, 255, 100],
        }
    }

    pub const ALL: &'static [OverlayMode] = &[
        Self::None,
        Self::PlaqueBounds,
        Self::Foreground,
        Self::WritableMask,
        Self::StructuralMask,
        Self::Occluder,
        Self::AllDiagnostics,
    ];
}

#[derive(Debug, Clone)]
pub struct FontPicker {
    pub open: bool,
    pub mode: FontPickerMode,
    pub query: String,
    pub filtered: Vec<String>,
    pub selected: usize,
    pub saved_font: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FontPickerMode {
    Curated,
    Search,
}

impl FontPicker {
    pub fn new(current_font: String) -> Self {
        Self {
            open: false,
            mode: FontPickerMode::Curated,
            query: String::new(),
            filtered: CURATED_FONTS.iter().map(|s| s.to_string()).collect(),
            selected: 0,
            saved_font: current_font,
        }
    }

    pub fn open(&mut self, current_font: String) {
        self.open = true;
        self.mode = FontPickerMode::Curated;
        self.query.clear();
        self.filtered = CURATED_FONTS.iter().map(|s| s.to_string()).collect();
        // select current font if present
        if let Some(idx) = self.filtered.iter().position(|f| f == &current_font) {
            self.selected = idx;
        } else {
            self.selected = 0;
        }
        self.saved_font = current_font;
    }

    pub fn update_search(&mut self, system_fonts: &[String]) {
        if self.mode == FontPickerMode::Search {
            let q = self.query.to_lowercase();
            if q.is_empty() {
                self.filtered = system_fonts.to_vec();
            } else {
                self.filtered = system_fonts
                    .iter()
                    .filter(|name| name.to_lowercase().contains(&q))
                    .cloned()
                    .collect();
            }
            self.selected = 0;
        }
    }

    pub fn handle_char(&mut self, c: char, system_fonts: &[String]) -> bool {
        // spec: if typing any letter or number, revert to full system fonts search mode
        if c.is_ascii_alphanumeric() || c == ' ' || c == '-' {
            if self.mode == FontPickerMode::Curated {
                self.mode = FontPickerMode::Search;
                self.query.clear();
            }
            self.query.push(c);
            self.update_search(system_fonts);
            true
        } else {
            false
        }
    }

    pub fn backspace(&mut self, system_fonts: &[String]) {
        if self.mode == FontPickerMode::Search {
            self.query.pop();
            self.update_search(system_fonts);
        }
    }

    pub fn current_selection(&self) -> Option<&str> {
        self.filtered.get(self.selected).map(|s| s.as_str())
    }
}

#[derive(Debug, Clone)]
pub struct ShowcaseState {
    pub videos: Vec<String>,
    pub current_video: usize,
    pub text: String,
    pub font: String,
    pub font_picker: FontPicker,
    pub text_edit_open: bool,
    pub text_edit_buffer: String,
    pub text_edit_saved: String,
    pub styles: Vec<String>,
    pub current_style: Option<usize>,
    pub style_draft_dirty: bool,
    pub demo_mode: bool,
    pub demo_saved_font: Option<String>,
    pub demo_saved_style: Option<usize>,
    pub overlay: OverlayMode,
    pub overlay_multi: Vec<OverlayMode>,
    pub multi_overlay_mode: bool,
    pub save_dialog_open: bool,
    pub save_name: String,
    pub save_saved_style: Option<String>,
}

impl ShowcaseState {
    pub fn new(videos: Vec<String>, styles: Vec<String>) -> Self {
        let font = CURATED_FONTS
            .first()
            .copied()
            .unwrap_or("Noto Serif")
            .to_string();
        Self {
            videos,
            current_video: 0,
            text: "Press ENTER to change this text".to_string(),
            font: font.clone(),
            font_picker: FontPicker::new(font),
            text_edit_open: false,
            text_edit_buffer: String::new(),
            text_edit_saved: String::new(),
            styles,
            current_style: None,
            style_draft_dirty: false,
            demo_mode: false,
            demo_saved_font: None,
            demo_saved_style: None,
            overlay: OverlayMode::None,
            overlay_multi: Vec::new(),
            multi_overlay_mode: false,
            save_dialog_open: false,
            save_name: String::new(),
            save_saved_style: None,
        }
    }

    pub fn current_video_stem(&self) -> Option<&str> {
        self.videos.get(self.current_video).map(|s| s.as_str())
    }

    pub fn next_video(&mut self) {
        if self.videos.is_empty() {
            return;
        }
        self.current_video = (self.current_video + 1) % self.videos.len();
    }

    pub fn prev_video(&mut self) {
        if self.videos.is_empty() {
            return;
        }
        if self.current_video == 0 {
            self.current_video = self.videos.len() - 1;
        } else {
            self.current_video -= 1;
        }
    }

    // Text editing (#2.1)
    pub fn open_text_edit(&mut self) {
        self.text_edit_open = true;
        self.text_edit_buffer = self.text.clone();
        self.text_edit_saved = self.text.clone();
    }
    pub fn commit_text_edit(&mut self) {
        if !self.text_edit_buffer.trim().is_empty() {
            self.text = self.text_edit_buffer.clone();
        }
        self.text_edit_open = false;
    }
    pub fn cancel_text_edit(&mut self) {
        self.text_edit_open = false;
        // revert
        self.text_edit_buffer = self.text_edit_saved.clone();
    }

    // Font picker (#2.2)
    pub fn open_font_picker(&mut self) {
        self.font_picker.open(self.font.clone());
    }
    pub fn close_font_picker_commit(&mut self) {
        if let Some(sel) = self.font_picker.current_selection() {
            self.font = sel.to_string();
        }
        self.font_picker.open = false;
        self.font_picker.mode = FontPickerMode::Curated;
        self.font_picker.query.clear();
    }
    pub fn cancel_font_picker(&mut self) {
        self.font = self.font_picker.saved_font.clone();
        self.font_picker.open = false;
        self.font_picker.mode = FontPickerMode::Curated;
        self.font_picker.query.clear();
    }
    pub fn font_picker_up(&mut self) {
        if self.font_picker.filtered.is_empty() {
            return;
        }
        if self.font_picker.selected == 0 {
            self.font_picker.selected = self.font_picker.filtered.len() - 1;
        } else {
            self.font_picker.selected -= 1;
        }
        if let Some(sel) = self.font_picker.current_selection() {
            self.font = sel.to_string();
        }
    }
    pub fn font_picker_down(&mut self) {
        if self.font_picker.filtered.is_empty() {
            return;
        }
        self.font_picker.selected =
            (self.font_picker.selected + 1) % self.font_picker.filtered.len();
        if let Some(sel) = self.font_picker.current_selection() {
            self.font = sel.to_string();
        }
    }

    // Style (#2.3)
    pub fn style_up(&mut self) {
        if self.styles.is_empty() {
            return;
        }
        self.style_draft_dirty = false;
        match self.current_style {
            None => self.current_style = Some(self.styles.len() - 1),
            Some(0) => self.current_style = Some(self.styles.len() - 1),
            Some(idx) => self.current_style = Some(idx - 1),
        }
    }
    pub fn style_down(&mut self) {
        if self.styles.is_empty() {
            return;
        }
        self.style_draft_dirty = false;
        match self.current_style {
            None => self.current_style = Some(0),
            Some(idx) => self.current_style = Some((idx + 1) % self.styles.len()),
        }
    }
    pub fn current_style_name(&self) -> Option<&str> {
        self.current_style
            .and_then(|idx| self.styles.get(idx).map(|s| s.as_str()))
    }

    // Demo (#5)
    pub fn enter_demo(&mut self) {
        if self.demo_mode {
            return;
        }
        self.demo_mode = true;
        self.demo_saved_font = Some(self.font.clone());
        self.demo_saved_style = self.current_style;
    }
    pub fn exit_demo(&mut self) {
        if !self.demo_mode {
            return;
        }
        self.demo_mode = false;
        if let Some(saved) = self.demo_saved_font.take() {
            self.font = saved;
        }
        self.current_style = self.demo_saved_style.take();
        self.style_draft_dirty = false;
    }
    pub fn demo_randomize(&mut self, font: String, style_idx: Option<usize>) {
        if !self.demo_mode {
            return;
        }
        self.font = font;
        self.current_style = style_idx;
    }

    // Inspect (#6)
    pub fn cycle_overlay(&mut self) {
        self.overlay = self.overlay.next();
        // exiting multi mode when cycling
        self.multi_overlay_mode = false;
    }
    pub fn toggle_multi_overlay(&mut self, mode: OverlayMode) {
        if self.overlay_multi.contains(&mode) {
            self.overlay_multi.retain(|m| *m != mode);
        } else {
            self.overlay_multi.push(mode);
        }
        self.multi_overlay_mode = !self.overlay_multi.is_empty();
    }

    // Save (#4)
    pub fn open_save_dialog(&mut self) {
        self.save_dialog_open = true;
        let base = self.current_style_name().unwrap_or("custom");
        self.save_name = format!("{base}_custom");
        self.save_saved_style = self.current_style_name().map(|s| s.to_string());
    }
    pub fn commit_save(&mut self) -> Option<String> {
        if self.save_name.trim().is_empty() {
            return None;
        }
        let name = self.save_name.trim().to_string();
        self.save_dialog_open = false;
        Some(name)
    }
    pub fn cancel_save(&mut self) {
        self.save_dialog_open = false;
    }

    pub fn font_style_label(&self) -> String {
        format!(
            "Font: {} | Style: {}",
            self.font,
            self.current_style_name().unwrap_or("direct")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ShowcaseState {
        ShowcaseState::new(
            vec!["a".into(), "b".into(), "c".into()],
            vec![
                "gold-shine".into(),
                "classic-glow".into(),
                "chrome-shine".into(),
            ],
        )
    }

    #[test]
    fn default_text_is_enter_prompt() {
        let s = state();
        assert_eq!(s.text, "Press ENTER to change this text");
    }

    #[test]
    fn video_navigation_wraps() {
        let mut s = state();
        assert_eq!(s.current_video, 0);
        s.next_video();
        assert_eq!(s.current_video, 1);
        s.next_video();
        s.next_video();
        assert_eq!(s.current_video, 0);
        s.prev_video();
        assert_eq!(s.current_video, 2);
        s.prev_video();
        assert_eq!(s.current_video, 1);
    }

    #[test]
    fn pgup_pgdown_empty_videos_is_noop() {
        let mut s = ShowcaseState::new(vec![], vec![]);
        s.next_video();
        s.prev_video();
        assert_eq!(s.current_video, 0);
    }

    #[test]
    fn text_edit_commit_and_cancel() {
        let mut s = state();
        s.open_text_edit();
        assert!(s.text_edit_open);
        s.text_edit_buffer = "Hello".into();
        s.commit_text_edit();
        assert_eq!(s.text, "Hello");
        assert!(!s.text_edit_open);

        s.open_text_edit();
        s.text_edit_buffer = "World".into();
        s.cancel_text_edit();
        assert_eq!(s.text, "Hello");
        assert!(!s.text_edit_open);
    }

    #[test]
    fn font_picker_curated_up_down_immediate_change() {
        let mut s = state();
        s.open_font_picker();
        let initial_font = s.font.clone();
        assert!(s.font_picker.open);
        assert_eq!(s.font_picker.mode, FontPickerMode::Curated);
        s.font_picker_down();
        assert_ne!(s.font, initial_font);
        assert_eq!(s.font, s.font_picker.current_selection().unwrap());
        let _after_down = s.font.clone();
        s.font_picker_up();
        assert_eq!(s.font, initial_font);
        // wrap
        s.font_picker_up();
        assert_eq!(s.font, CURATED_FONTS.last().copied().unwrap());
        s.cancel_font_picker();
        assert_eq!(s.font, initial_font);
        assert!(!s.font_picker.open);
    }

    #[test]
    fn font_picker_typing_switches_to_search() {
        let mut s = state();
        s.open_font_picker();
        let system = vec![
            "Noto Serif".into(),
            "DejaVu Sans".into(),
            "Arial".into(),
            "Noto Sans".into(),
        ];
        assert_eq!(s.font_picker.mode, FontPickerMode::Curated);
        // typing letter should switch
        let changed = s.font_picker.handle_char('a', &system);
        assert!(changed);
        assert_eq!(s.font_picker.mode, FontPickerMode::Search);
        assert_eq!(s.font_picker.query, "a");
        // filtered contains 'a' case insensitive
        assert!(
            s.font_picker
                .filtered
                .iter()
                .any(|f| f.to_lowercase().contains('a'))
        );
        // backspace works
        s.font_picker.handle_char('r', &system);
        assert_eq!(s.font_picker.query, "ar");
        s.font_picker.backspace(&system);
        assert_eq!(s.font_picker.query, "a");
        // delete? backspace is tested; ESC reverts
        let saved = s.font_picker.saved_font.clone();
        s.cancel_font_picker();
        assert_eq!(s.font, saved);
    }

    #[test]
    fn font_picker_search_up_down_selection() {
        let mut s = state();
        s.open_font_picker();
        let system = vec!["Alpha".into(), "Beta".into(), "Gamma".into()];
        s.font_picker.handle_char('a', &system);
        assert_eq!(s.font_picker.filtered.len(), 3); // all contain 'a'
        let first = s.font.clone();
        s.font_picker_down();
        assert_ne!(s.font, first);
        s.close_font_picker_commit();
        assert!(!s.font_picker.open);
        assert_eq!(
            s.font,
            s.font_picker.filtered[s.font_picker.selected].clone()
        );
    }

    #[test]
    fn style_up_down_discards_changes() {
        let mut s = state();
        s.current_style = Some(1);
        s.style_draft_dirty = true;
        s.style_down();
        assert_eq!(s.current_style, Some(2));
        assert!(!s.style_draft_dirty);
        s.style_up();
        assert_eq!(s.current_style, Some(1));
        s.style_up();
        assert_eq!(s.current_style, Some(0));
        s.style_up();
        assert_eq!(s.current_style, Some(2)); // wrap
    }

    #[test]
    fn style_down_from_none_goes_to_zero() {
        let mut s = state();
        s.current_style = None;
        s.style_down();
        assert_eq!(s.current_style, Some(0));
        s.current_style = None;
        s.style_up();
        assert_eq!(s.current_style, Some(2));
    }

    #[test]
    fn demo_enter_exit_restores() {
        let mut s = state();
        s.font = "MyFont".into();
        s.current_style = Some(1);
        s.enter_demo();
        assert!(s.demo_mode);
        s.demo_randomize("RandomFont".into(), Some(0));
        assert_eq!(s.font, "RandomFont");
        s.exit_demo();
        assert!(!s.demo_mode);
        assert_eq!(s.font, "MyFont");
        assert_eq!(s.current_style, Some(1));
    }

    #[test]
    fn demo_esc_exits_even_when_other_keys_work() {
        let mut s = state();
        s.enter_demo();
        // other keys still work – e.g., text edit
        s.open_text_edit();
        assert!(s.text_edit_open);
        s.exit_demo();
        assert!(!s.demo_mode);
    }

    #[test]
    fn overlay_cycle_wraps() {
        let mut s = state();
        assert_eq!(s.overlay, OverlayMode::None);
        for _ in 0..OverlayMode::ALL.len() {
            s.cycle_overlay();
        }
        assert_eq!(s.overlay, OverlayMode::None);
        s.cycle_overlay();
        assert_eq!(s.overlay, OverlayMode::PlaqueBounds);
    }

    #[test]
    fn overlay_colors_distinct() {
        let colors: std::collections::HashSet<_> =
            OverlayMode::ALL.iter().map(|m| m.color()).collect();
        // all except None should be distinct; None is transparent
        assert!(colors.len() >= 6);
    }

    #[test]
    fn multi_overlay_toggle() {
        let mut s = state();
        assert!(!s.multi_overlay_mode);
        s.toggle_multi_overlay(OverlayMode::Foreground);
        assert!(s.multi_overlay_mode);
        assert!(s.overlay_multi.contains(&OverlayMode::Foreground));
        s.toggle_multi_overlay(OverlayMode::Foreground);
        assert!(!s.overlay_multi.contains(&OverlayMode::Foreground));
    }

    #[test]
    fn save_dialog_prefill_and_commit() {
        let mut s = state();
        s.current_style = Some(0);
        s.open_save_dialog();
        assert!(s.save_dialog_open);
        assert_eq!(s.save_name, "gold-shine_custom");
        s.save_name = "my_style".into();
        let name = s.commit_save().unwrap();
        assert_eq!(name, "my_style");
        assert!(!s.save_dialog_open);
    }

    #[test]
    fn save_dialog_no_style_uses_custom() {
        let mut s = state();
        s.current_style = None;
        s.open_save_dialog();
        assert_eq!(s.save_name, "custom_custom");
    }

    #[test]
    fn font_style_label_always_present() {
        let s = state();
        let label = s.font_style_label();
        assert!(label.contains("Font:"));
        assert!(label.contains("Style:"));
    }
}
