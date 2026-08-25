/// Cache compatibility identifier for generated analysis data.
///
/// Change this only when `analyze` changes the meaning or layout of data consumed by
/// later runs. Renderer, CLI, documentation, or unrelated refactors must not invalidate
/// analysis caches.
pub const ANALYZER_CACHE_VERSION: &str = "surface-analysis-v12-transitional-semantic-occlusion-bounded-semantic-material-depth-reviewed-dynamics";

/// Human-facing renderer release.
pub const RENDERER_BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Exact identity of the Rust renderer implementation and resolved build inputs.
pub const RENDERER_SOURCE_SHA256: &str = env!("PLAQUE_FORGE_RENDERER_SOURCE_SHA256");

pub const LONG_VERSION: &str = env!("CARGO_PKG_VERSION");
