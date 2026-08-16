//! Shared test infrastructure used by integration test files.
#![allow(dead_code)]

use std::path::Path;

pub mod synthetic;

/// Root of the repository, resolved from `CARGO_MANIFEST_DIR`.
pub fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}
