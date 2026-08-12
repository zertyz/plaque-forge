/// Cache compatibility identifier for generated analysis data.
///
/// Change this only when `analyze` changes the meaning or layout of data consumed by
/// later runs. Renderer, CLI, documentation, or unrelated refactors must not invalidate
/// analysis caches.
pub const ANALYZER_CACHE_VERSION: &str = "analysis-v9";

/// Identifies the renderer in generated render manifests without introducing a custom
/// build script. Package releases are the unit of renderer provenance.
pub const RENDERER_BUILD_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const LONG_VERSION: &str = env!("CARGO_PKG_VERSION");
