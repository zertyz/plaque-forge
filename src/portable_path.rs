//! Portable paths stored in generated project artifacts.
//!
//! Serialized paths are always relative, use `/` separators, and cannot carry a
//! workstation root. Callers additionally reject parent components for paths that
//! must stay inside a generated bundle.

use std::{
    fmt,
    path::{Component, Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PortablePath(PathBuf);

impl PortablePath {
    pub fn project(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        validate(path, true)?;
        Ok(Self(path.to_path_buf()))
    }

    pub fn bundle(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        validate(path, false)?;
        Ok(Self(path.to_path_buf()))
    }

    pub fn as_path(&self) -> &Path {
        &self.0
    }

    pub fn resolve_from(&self, owner: &Path) -> PathBuf {
        owner
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(&self.0)
    }

    pub fn has_parent_component(&self) -> bool {
        self.0
            .components()
            .any(|component| component == Component::ParentDir)
    }

    fn serialized(&self) -> String {
        self.0
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/")
    }
}

impl AsRef<Path> for PortablePath {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl fmt::Display for PortablePath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.serialized())
    }
}

impl Serialize for PortablePath {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.serialized())
    }
}

impl<'de> Deserialize<'de> for PortablePath {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::project(&value).map_err(de::Error::custom)
    }
}

pub fn relative_reference(owner: &Path, target: &Path) -> Result<PortablePath> {
    let owner_parent = owner.parent().unwrap_or_else(|| Path::new("."));
    if owner_parent == target.parent().unwrap_or_else(|| Path::new(".")) {
        let name = target.file_name().context("target path has no file name")?;
        return PortablePath::project(PathBuf::from(name));
    }

    let current = std::env::current_dir().context("failed to resolve current directory")?;
    let absolute = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            current.join(path)
        }
    };
    let lexical_owner = normalize_lexically(&absolute(owner_parent));
    let lexical_target = normalize_lexically(&absolute(target));
    // Resolve symlinks only as a pair. Canonicalizing just the target can turn a
    // repository-relative reference into a path through the host's real mount
    // point when the checkout itself was reached through a symlink.
    let (owner, target) = match (owner_parent.canonicalize(), target.canonicalize()) {
        (Ok(owner), Ok(target)) => (owner, target),
        _ => (lexical_owner, lexical_target),
    };
    let owner_components = owner.components().collect::<Vec<_>>();
    let target_components = target.components().collect::<Vec<_>>();
    let common = owner_components
        .iter()
        .zip(&target_components)
        .take_while(|(left, right)| left == right)
        .count();
    if common == 0 {
        bail!("artifact and referenced file do not share a filesystem root");
    }
    let mut relative = PathBuf::new();
    for _ in common..owner_components.len() {
        relative.push("..");
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }
    PortablePath::project(relative)
}

fn validate(path: &Path, allow_parent: bool) -> Result<()> {
    let raw = path.to_string_lossy();
    if raw.is_empty() {
        bail!("portable path is empty");
    }
    if raw.contains('\\')
        || raw.starts_with("//")
        || raw
            .as_bytes()
            .get(1)
            .is_some_and(|character| *character == b':')
    {
        bail!("portable path uses a platform-specific or absolute form: {raw}");
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            Component::ParentDir if allow_parent => {}
            Component::ParentDir => bail!("bundle path escapes its owner: {raw}"),
            Component::CurDir => bail!("portable path contains a redundant '.' component: {raw}"),
            Component::RootDir | Component::Prefix(_) => {
                bail!("portable path is absolute: {raw}")
            }
        }
    }
    Ok(())
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut output = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                output.pop();
            }
            other => output.push(other.as_os_str()),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_absolute_and_windows_paths() {
        assert!(PortablePath::project("/home/user/file").is_err());
        assert!(PortablePath::project(r"C:\work\file").is_err());
        assert!(PortablePath::project(r"dir\file").is_err());
    }

    #[test]
    fn bundle_path_cannot_escape() {
        assert!(PortablePath::bundle("../mask.png").is_err());
        assert_eq!(
            PortablePath::bundle("masks/000001.png")
                .unwrap()
                .to_string(),
            "masks/000001.png"
        );
    }

    #[test]
    fn reference_is_relative_to_its_document() {
        let owner = Path::new("/work/assets/analysis/a/manifest.toml");
        let target = Path::new("/work/assets/a.mp4");
        assert_eq!(
            relative_reference(owner, target).unwrap().to_string(),
            "../../a.mp4"
        );
    }

    #[cfg(unix)]
    #[test]
    fn reference_resolves_both_sides_through_a_symlink() {
        use std::fs;
        use std::os::unix::fs::symlink;
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "plaque-forge-portable-path-{}-{nonce}",
            std::process::id()
        ));
        let checkout = root.join("real-checkout");
        fs::create_dir_all(checkout.join("assets/analysis/example")).unwrap();
        fs::write(checkout.join("assets/example.mp4"), []).unwrap();
        let linked = root.join("linked-checkout");
        symlink(&checkout, &linked).unwrap();

        let owner = linked.join("assets/analysis/example/manifest.toml");
        let target = linked.join("assets/example.mp4");
        assert_eq!(
            relative_reference(&owner, &target).unwrap().to_string(),
            "../../example.mp4"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
