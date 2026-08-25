//! Interactive text-style showcase.
//!
//! Logic lives here so it stays unit-testable without a display: key
//! normalization, font-picker filtering, quality tiers, the style-composer
//! editing model, and demo scheduling. The `showcase` binary only wires these
//! onto an OpenCV highgui loop.

pub mod composer;
pub mod fonts;
pub mod keys;
pub mod quality;
pub mod state;
