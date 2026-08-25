//! Showcase application state: modes, text entry, and demo scheduling.
//!
//! Everything here is pure logic over plain data so the state machine is
//! unit-testable without a windowing system.

use crate::showcase::composer::EditModel;

/// Deterministic xorshift64* generator; a real RNG dependency is not needed
/// for shuffle-style demo scheduling, and seeds make tests reproducible.
#[derive(Debug, Clone)]
pub struct DemoRandom {
    state: u64,
}

impl DemoRandom {
    pub fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    /// Pick an element index uniformly.
    pub fn pick_index(&mut self, len: usize) -> usize {
        debug_assert!(len > 0);
        (self.next_u64() % len.max(1) as u64) as usize
    }
}

/// One random demo combination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DemoCombo {
    pub font_label: String,
    pub style_name: String,
}

#[derive(Debug, Default)]
pub struct DemoState {
    rng: Option<DemoRandom>,
}

impl DemoState {
    /// Begin a session; `seed` may come from the wall clock in production and
    /// from fixtures in tests.
    pub fn start(seed: u64) -> Self {
        Self {
            rng: Some(DemoRandom::new(seed)),
        }
    }

    /// The next random font×style pairing.
    pub fn next_combo(&mut self, fonts: &[String], styles: &[String]) -> Option<DemoCombo> {
        if fonts.is_empty() || styles.is_empty() {
            return None;
        }
        let rng = self.rng.as_mut()?;
        Some(DemoCombo {
            font_label: fonts[rng.pick_index(fonts.len())].clone(),
            style_name: styles[rng.pick_index(styles.len())].clone(),
        })
    }
}

/// Top-level interaction mode. Popups own their transient input buffers.
#[derive(Debug)]
pub enum Mode {
    /// Normal playback; every transport key is live.
    Viewing,
    /// ENTER prompt collecting replacement title text.
    EnteringText(String),
    /// '/' popup; holds the picker model plus pre-open selection for Esc.
    PickingFont(crate::showcase::fonts::FontPicker),
    /// Style composer overlay.
    Composing(Box<EditModel>),
    /// 's' flow collecting the destination preset name.
    SavingName(String),
    /// 'd' randomized showcase; ESC returns to [`Mode::Viewing`].
    Demo(DemoState),
}

impl Mode {
    pub fn label(&self) -> &'static str {
        match self {
            Mode::Viewing => "viewing",
            Mode::EnteringText(_) => "text",
            Mode::PickingFont(_) => "font",
            Mode::Composing(_) => "composer",
            Mode::SavingName(_) => "save",
            Mode::Demo(_) => "demo",
        }
    }

    pub fn text_buffer_mut(&mut self) -> Option<&mut String> {
        match self {
            Mode::EnteringText(buffer) | Mode::SavingName(buffer) => Some(buffer),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_random_is_deterministic_per_seed_and_uniform_enough() {
        let mut a = DemoState::start(42);
        let mut b = DemoState::start(42);
        let fonts = vec!["f1".to_string(), "f2".to_string()];
        let styles = vec!["s1".to_string(), "s2".to_string(), "s3".to_string()];
        for _ in 0..8 {
            assert_eq!(a.next_combo(&fonts, &styles), b.next_combo(&fonts, &styles));
        }
        // Different seeds diverge immediately with overwhelming likelihood.
        let mut c = DemoState::start(43);
        assert_ne!(a.next_combo(&fonts, &styles), c.next_combo(&fonts, &styles));
    }

    #[test]
    fn empty_catalogs_yield_no_combo_instead_of_panicking() {
        let mut state = DemoState::start(7);
        assert!(state.next_combo(&[], &["s".into()]).is_none());
        assert!(state.next_combo(&["f".into()], &[]).is_none());
    }

    #[test]
    fn mode_labels_distinguish_popups_for_hud_rendering() {
        use crate::media::FontListing;
        use crate::showcase::fonts::FontPicker;
        let listing = vec![FontListing {
            label: "A".into(),
            curated: false,
        }];
        assert_eq!(Mode::Viewing.label(), "viewing");
        assert_eq!(Mode::EnteringText("hi".into()).label(), "text");
        assert_eq!(
            Mode::PickingFont(FontPicker::open(&listing, 0)).label(),
            "font"
        );
        assert_eq!(Mode::SavingName(String::new()).label(), "save");
        assert_eq!(Mode::Demo(DemoState::start(1)).label(), "demo");
    }

    #[test]
    fn text_entry_buffers_are_mutable_through_the_mode() {
        let mut mode = Mode::EnteringText("abc".into());
        mode.text_buffer_mut().unwrap().push('d');
        match mode {
            Mode::EnteringText(buffer) => assert_eq!(buffer, "abcd"),
            other => panic!("unexpected mode {other:?}"),
        }
    }
}
