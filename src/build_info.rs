pub const SOURCE_FINGERPRINT: &str = env!("PLAQUE_FORGE_SOURCE_FINGERPRINT");
// Bump only when analysis output compatibility changes.
pub const ANALYZER_CACHE_VERSION: &str = "cceaae40697f5e50";
pub const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (source ",
    env!("PLAQUE_FORGE_SOURCE_FINGERPRINT"),
    ")"
);
