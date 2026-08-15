//! Content digests used for cache/provenance identities.
//!
//! Hashing is deliberately independent of video, scene, or CLI layers. File hashing
//! streams data so large media files are not loaded into memory merely to identify them.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::Path,
};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

pub fn file_sha256(path: &Path) -> Result<String> {
    let file = File::open(path)
        .with_context(|| format!("failed to open file for SHA-256: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .with_context(|| format!("failed to hash {}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>())
}

pub fn bytes_sha256(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}
