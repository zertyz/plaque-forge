//! Content digests used for cache/provenance identities.
//!
//! Hashing is deliberately independent of video, scene, or CLI layers. File hashing
//! streams data so large media files are not loaded into memory merely to identify them.

use std::{
    fs::File,
    io::{BufReader, Read},
    path::{Component, Path},
};

use anyhow::{Context, Result, bail};
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

/// Hash a portable set of files as one identity. Both relative names and bytes are
/// covered, and caller order is deliberately irrelevant.
pub fn relative_files_sha256<'a>(
    root: &Path,
    paths: impl IntoIterator<Item = &'a std::path::PathBuf>,
) -> Result<String> {
    let mut paths = paths.into_iter().cloned().collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    let mut digest = Sha256::new();
    digest.update(b"plaque-forge.relative-files/1\0");
    for relative in paths {
        if relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("bundle digest path is not relative: {}", relative.display());
        }
        let name = relative
            .to_str()
            .with_context(|| format!("bundle digest path is not UTF-8: {}", relative.display()))?;
        let name = name.as_bytes();
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name);
        let path = root.join(&relative);
        let file = File::open(&path)
            .with_context(|| format!("failed to open render input: {}", path.display()))?;
        let mut reader = BufReader::new(file);
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = reader
                .read(&mut buffer)
                .with_context(|| format!("failed to hash render input {}", path.display()))?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::relative_files_sha256;
    use std::{fs, path::PathBuf};

    #[test]
    fn relative_file_bundle_identity_covers_names_and_contents() {
        let root = std::env::temp_dir().join(format!(
            "plaque-forge-bundle-digest-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("masks")).unwrap();
        fs::write(root.join("manifest.toml"), b"manifest").unwrap();
        fs::write(root.join("masks/000000.png"), b"first").unwrap();

        let paths = [
            PathBuf::from("masks/000000.png"),
            PathBuf::from("manifest.toml"),
        ];
        let first = relative_files_sha256(&root, paths.iter()).unwrap();
        let reordered = relative_files_sha256(&root, paths.iter().rev()).unwrap();
        assert_eq!(first, reordered, "caller ordering must not affect identity");

        fs::write(root.join("masks/000000.png"), b"second").unwrap();
        assert_ne!(first, relative_files_sha256(&root, paths.iter()).unwrap());

        fs::remove_dir_all(root).unwrap();
    }
}
