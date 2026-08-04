pub const SOURCE_FINGERPRINT: &str = env!("PLAQUE_FORGE_SOURCE_FINGERPRINT");
pub const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (source ",
    env!("PLAQUE_FORGE_SOURCE_FINGERPRINT"),
    ")"
);
