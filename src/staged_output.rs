use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};

pub fn create(target: &Path) -> Result<PathBuf> {
    let partial = sibling(target, "partial");
    if partial.exists() {
        bail!(
            "staged output already exists: {}\nhelp: inspect or delete that path explicitly",
            partial.display()
        );
    }
    fs::create_dir_all(&partial)
        .with_context(|| format!("failed to create staged output {}", partial.display()))?;
    Ok(partial)
}

pub fn commit(partial: &Path, target: &Path, replace: bool) -> Result<()> {
    if !target.exists() {
        return fs::rename(partial, target).with_context(|| {
            format!(
                "failed to commit staged output {} to {}",
                partial.display(),
                target.display()
            )
        });
    }
    if !replace {
        bail!("output already exists: {}", target.display());
    }

    let previous = sibling(target, "replaced");
    if previous.exists() {
        bail!(
            "replacement backup already exists: {}\nhelp: inspect or delete that path explicitly",
            previous.display()
        );
    }
    fs::rename(target, &previous).with_context(|| {
        format!(
            "failed to preserve existing output {} as {}",
            target.display(),
            previous.display()
        )
    })?;
    if let Err(error) = fs::rename(partial, target) {
        if let Err(restore_error) = fs::rename(&previous, target) {
            return Err(error).with_context(|| {
                format!(
                    "failed to install {} and failed to restore {}: {restore_error}",
                    partial.display(),
                    previous.display()
                )
            });
        }
        return Err(error).with_context(|| {
            format!(
                "failed to install {}; the previous output was restored",
                target.display()
            )
        });
    }
    remove(&previous).with_context(|| {
        format!(
            "new output installed; failed to delete {}",
            previous.display()
        )
    })
}

pub fn remove_child(root: &Path, child: &Path) -> Result<()> {
    let root = fs::canonicalize(root)
        .with_context(|| format!("failed to resolve owned output {}", root.display()))?;
    let child = fs::canonicalize(child)
        .with_context(|| format!("failed to resolve owned child {}", child.display()))?;
    if child == root || !child.starts_with(&root) {
        bail!(
            "refusing to delete path outside owned output {}: {}",
            root.display(),
            child.display()
        );
    }
    remove(&child)
}

fn sibling(target: &Path, label: &str) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    target.with_file_name(format!("{name}.{label}-{}", std::process::id()))
}

fn remove(path: &Path) -> Result<()> {
    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_dir() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn replacement_keeps_the_old_output_until_commit() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "plaque-forge-staged-output-{}-{nonce}",
            std::process::id()
        ));
        let target = root.join("analysis");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("value"), "old").unwrap();

        let partial = create(&target).unwrap();
        fs::write(partial.join("value"), "new").unwrap();
        assert_eq!(fs::read_to_string(target.join("value")).unwrap(), "old");

        commit(&partial, &target, true).unwrap();
        assert_eq!(fs::read_to_string(target.join("value")).unwrap(), "new");

        let outside = root.join("outside");
        fs::write(&outside, "keep").unwrap();
        assert!(remove_child(&target, &outside).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
        fs::remove_dir_all(root).unwrap();
    }
}
