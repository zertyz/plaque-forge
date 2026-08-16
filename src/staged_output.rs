use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, File, OpenOptions, TryLockError},
    hash::{Hash, Hasher},
    io::ErrorKind,
    path::{Component, Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

const STALE_WORK_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const DEAD_OWNER_GRACE: Duration = Duration::from_secs(5);
const FAILURE_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const FAILURE_MAX_PER_TARGET: usize = 3;
const LEASE_FILE: &str = ".lease";

pub struct StagedOutput {
    path: PathBuf,
    target: PathBuf,
    work_lease: Option<File>,
    locked_targets: Vec<PathBuf>,
    locks: Vec<PathBuf>,
    lock_leases: Vec<File>,
    committed: bool,
}

impl StagedOutput {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn commit(mut self, replace: bool) -> Result<()> {
        let parent = self.target.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create output parent {}", parent.display()))?;

        let incoming = sibling(&self.target, "incoming");
        if incoming.exists() {
            remove(&incoming).with_context(|| {
                format!(
                    "failed to remove abandoned incoming output {}",
                    incoming.display()
                )
            })?;
        }
        match fs::rename(&self.path, &incoming) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::CrossesDevices => {
                if let Err(error) = copy_tree(&self.path, &incoming) {
                    let _ = remove_if_exists(&incoming);
                    return Err(error)
                        .context("failed to copy staged output onto the destination filesystem");
                }
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!(
                        "failed to hand staged output to destination filesystem: {}",
                        incoming.display()
                    )
                });
            }
        }
        // The directory is no longer visible to the work reaper. Release its
        // lease and remove every transaction-private file before publication.
        // This is also deliberately tolerant of work made by pre-lease builds.
        self.work_lease.take();
        if let Err(error) = strip_private_bookkeeping(&incoming) {
            let _ = remove_if_exists(&incoming);
            return Err(error).context("failed to strip private staging bookkeeping");
        }

        if self.target.exists() && !replace {
            remove_if_exists(&incoming)?;
            bail!("output already exists: {}", self.target.display());
        }

        let previous = sibling(&self.target, "replaced");
        if previous.exists() {
            remove_if_exists(&incoming)?;
            bail!(
                "replacement backup already exists: {}\nhelp: inspect or delete that path explicitly",
                previous.display()
            );
        }
        if self.target.exists() {
            fs::rename(&self.target, &previous).with_context(|| {
                format!(
                    "failed to preserve existing output {} as {}",
                    self.target.display(),
                    previous.display()
                )
            })?;
        }

        if let Err(error) = fs::rename(&incoming, &self.target) {
            if previous.exists()
                && let Err(restore_error) = fs::rename(&previous, &self.target)
            {
                return Err(error).with_context(|| {
                    format!(
                        "failed to install {} and failed to restore {}: {restore_error}",
                        incoming.display(),
                        previous.display()
                    )
                });
            }
            return Err(error).with_context(|| {
                format!(
                    "failed to install {}; the previous output was restored",
                    self.target.display()
                )
            });
        }

        if previous.exists() {
            remove(&previous).with_context(|| {
                format!(
                    "new output installed; failed to delete {}",
                    previous.display()
                )
            })?;
        }
        remove_if_exists(&self.path)?;
        purge_failures(&self.target)?;
        self.committed = true;
        self.release_locks()?;
        Ok(())
    }

    /// Publish a set of regular files as one recoverable bundle. Members are
    /// installed in the supplied order, so callers can put their manifest last
    /// and use it as the bundle's commit marker.
    pub fn commit_files(mut self, members: &[(PathBuf, PathBuf)], replace: bool) -> Result<()> {
        if members.is_empty() {
            bail!("cannot commit an empty output bundle");
        }

        let stage_root = absolute_lexical(&self.path)?;
        let mut prepared = Vec::with_capacity(members.len());
        for (staged, target) in members {
            let staged = absolute_lexical(staged)?;
            if staged == stage_root || !staged.starts_with(&stage_root) {
                bail!(
                    "bundle member is outside its owned stage {}: {}",
                    stage_root.display(),
                    staged.display()
                );
            }
            let metadata = fs::symlink_metadata(&staged).with_context(|| {
                format!(
                    "staged bundle member was not produced: {}",
                    staged.display()
                )
            })?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                bail!(
                    "staged bundle member is not a regular file: {}",
                    staged.display()
                );
            }
            let target = absolute_lexical(target)?;
            self.lock_target(&target)?;
            if prepared
                .iter()
                .any(|member: &PreparedFile| member.target == target)
            {
                bail!("output bundle repeats target {}", target.display());
            }
            let parent = target.parent().unwrap_or_else(|| Path::new("."));
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create output parent {}", parent.display()))?;
            if target.exists() && !replace {
                bail!("output already exists: {}", target.display());
            }
            let incoming = sibling(&target, "incoming");
            let previous = sibling(&target, "replaced");
            if previous.exists() {
                bail!(
                    "replacement backup already exists: {}\nhelp: inspect or delete that path explicitly",
                    previous.display()
                );
            }
            remove_if_exists(&incoming)?;
            prepared.push(PreparedFile {
                staged,
                target,
                incoming,
                previous,
                had_previous: false,
                installed: false,
            });
        }

        for member in &prepared {
            match fs::rename(&member.staged, &member.incoming) {
                Ok(()) => {}
                Err(error) if error.kind() == ErrorKind::CrossesDevices => {
                    if let Err(copy_error) = fs::copy(&member.staged, &member.incoming) {
                        cleanup_prepared(&prepared);
                        return Err(copy_error).with_context(|| {
                            format!(
                                "failed to stage bundle member on the destination filesystem: {}",
                                member.target.display()
                            )
                        });
                    }
                }
                Err(error) => {
                    cleanup_prepared(&prepared);
                    return Err(error).with_context(|| {
                        format!(
                            "failed to stage bundle member for publication: {}",
                            member.target.display()
                        )
                    });
                }
            }
        }

        for index in 0..prepared.len() {
            if prepared[index].target.exists() {
                if let Err(error) = fs::rename(&prepared[index].target, &prepared[index].previous) {
                    rollback_prepared(&mut prepared);
                    return Err(error).with_context(|| {
                        format!(
                            "failed to preserve existing bundle member {}",
                            prepared[index].target.display()
                        )
                    });
                }
                prepared[index].had_previous = true;
            }
        }

        for index in 0..prepared.len() {
            if let Err(error) = fs::rename(&prepared[index].incoming, &prepared[index].target) {
                rollback_prepared(&mut prepared);
                return Err(error).with_context(|| {
                    format!(
                        "failed to publish bundle member {}; previous bundle was restored",
                        prepared[index].target.display()
                    )
                });
            }
            prepared[index].installed = true;
        }

        self.work_lease.take();
        self.committed = true;
        remove_if_exists(&self.path)?;
        purge_failures(&self.target)?;
        for member in &prepared {
            remove_if_exists(&member.previous).with_context(|| {
                format!(
                    "new bundle installed; failed to remove backup {}",
                    member.previous.display()
                )
            })?;
        }
        self.release_locks()?;
        Ok(())
    }

    fn lock_target(&mut self, target: &Path) -> Result<()> {
        if self.locked_targets.iter().any(|locked| locked == target) {
            return Ok(());
        }
        let lock = lock_path(target);
        let lease = acquire_lock(&lock, target)?;
        if let Err(error) = recover_interrupted_publication(target) {
            drop(lease);
            let _ = remove_if_exists(&lock);
            return Err(error);
        }
        self.locked_targets.push(target.to_path_buf());
        self.locks.push(lock);
        self.lock_leases.push(lease);
        Ok(())
    }

    fn release_locks(&mut self) -> Result<()> {
        self.lock_leases.clear();
        let mut first_error = None;
        for lock in self.locks.drain(..) {
            if let Err(error) = remove_if_exists(&lock)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            Err(error).context("output was published, but its temporary lock could not be removed")
        } else {
            Ok(())
        }
    }
}

struct PreparedFile {
    staged: PathBuf,
    target: PathBuf,
    incoming: PathBuf,
    previous: PathBuf,
    had_previous: bool,
    installed: bool,
}

fn cleanup_prepared(members: &[PreparedFile]) {
    for member in members {
        let _ = remove_if_exists(&member.incoming);
    }
}

fn rollback_prepared(members: &mut [PreparedFile]) {
    for member in members.iter_mut().rev() {
        if member.installed {
            let _ = remove_if_exists(&member.target);
            member.installed = false;
        }
        if member.had_previous && member.previous.exists() {
            let _ = fs::rename(&member.previous, &member.target);
            member.had_previous = false;
        }
        let _ = remove_if_exists(&member.incoming);
    }
}

impl Drop for StagedOutput {
    fn drop(&mut self) {
        self.work_lease.take();
        if !self.committed {
            let _ = remove_if_exists(&self.path);
        }
        self.lock_leases.clear();
        for lock in &self.locks {
            let _ = remove_if_exists(lock);
        }
    }
}

pub fn create(target: &Path) -> Result<StagedOutput> {
    let root = temporary_root();
    let work_root = root.join("work");
    let lock_root = root.join("locks");
    let failure_root = root.join("failures");
    fs::create_dir_all(&work_root)?;
    fs::create_dir_all(&lock_root)?;
    fs::create_dir_all(&failure_root)?;
    reap_stale_work(&work_root)?;
    reap_failures(&failure_root)?;

    let target = absolute_lexical(target)?;
    let lock = lock_root.join(format!("{:016x}", path_hash(&target)));
    let lock_lease = acquire_lock(&lock, &target)?;
    if let Err(error) = recover_interrupted_publication(&target) {
        drop(lock_lease);
        let _ = remove_if_exists(&lock);
        return Err(error);
    }

    let name = safe_name(
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("output"),
    );
    static STAGE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let count = STAGE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = work_root.join(format!("{name}-{}-{nonce}-{count}", std::process::id()));
    if let Err(error) = fs::create_dir(&path) {
        drop(lock_lease);
        let _ = remove_if_exists(&lock);
        return Err(error)
            .with_context(|| format!("failed to create staged output {}", path.display()));
    }
    let work_lease = match create_lease(&path.join(LEASE_FILE)) {
        Ok(lease) => lease,
        Err(error) => {
            let _ = remove_if_exists(&path);
            drop(lock_lease);
            let _ = remove_if_exists(&lock);
            return Err(error).context("failed to lease staged output");
        }
    };
    Ok(StagedOutput {
        path,
        target: target.clone(),
        work_lease: Some(work_lease),
        locked_targets: vec![target],
        locks: vec![lock],
        lock_leases: vec![lock_lease],
        committed: false,
    })
}

pub fn write_file(target: &Path, contents: &[u8], replace: bool) -> Result<()> {
    let staged = create(target)?;
    let name = target.file_name().context("output file has no name")?;
    let staged_file = staged.path().join(name);
    fs::write(&staged_file, contents)
        .with_context(|| format!("failed to write staged file {}", staged_file.display()))?;
    staged.commit_files(&[(staged_file, target.to_path_buf())], replace)
}

pub fn retain_failure(stage: &Path, target: &Path) -> Result<Option<PathBuf>> {
    let diagnostics = stage.join("diagnostics");
    let summary = stage.join("analysis-summary.json");
    if !diagnostics.is_dir() && !summary.is_file() {
        return Ok(None);
    }

    let target_name = safe_name(
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("analysis"),
    );
    let root = temporary_root().join("failures").join(target_name);
    fs::create_dir_all(&root)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let retained = root.join(format!("{timestamp}-{}", std::process::id()));
    fs::create_dir(&retained)?;
    let result = (|| -> Result<()> {
        if diagnostics.is_dir() {
            copy_tree(&diagnostics, &retained.join("diagnostics"))?;
        }
        for name in ["analysis-summary.json", "trajectory.json"] {
            let source = stage.join(name);
            if source.is_file() {
                fs::copy(&source, retained.join(name))?;
            }
        }
        prune_failures(&root, Some(&retained))
    })();
    if let Err(error) = result {
        let _ = remove_if_exists(&retained);
        return Err(error);
    }
    Ok(Some(retained))
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

fn acquire_lock(lock: &Path, target: &Path) -> Result<File> {
    for attempt in 0..2 {
        match fs::create_dir(lock) {
            Ok(()) => match create_lease(&lock.join(LEASE_FILE)) {
                Ok(lease) => return Ok(lease),
                Err(error) => {
                    let _ = remove_if_exists(lock);
                    return Err(error).with_context(|| {
                        format!("failed to lease output lock for {}", target.display())
                    });
                }
            },
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempt == 0 => {
                if lease_is_abandoned(lock, DEAD_OWNER_GRACE) {
                    remove(lock)?;
                    continue;
                }
                bail!(
                    "output is locked by another Plaque Forge process: {}",
                    target.display()
                );
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to lock output {}", target.display()));
            }
        }
    }
    bail!("failed to acquire output lock for {}", target.display())
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(source)
        .with_context(|| format!("failed to inspect staged path {}", source.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "staged output contains a symbolic link: {}",
            source.display()
        );
    }
    if metadata.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!("unsupported staged output type: {}", source.display());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}

fn reap_stale_work(root: &Path) -> Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if lease_is_abandoned(&entry.path(), STALE_WORK_AGE) {
            remove(&entry.path())?;
        }
    }
    Ok(())
}

fn lease_is_abandoned(path: &Path, missing_lease_age: Duration) -> bool {
    let Some(age) = fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
    else {
        return false;
    };
    let lease = match OpenOptions::new()
        .read(true)
        .write(true)
        .open(path.join(LEASE_FILE))
    {
        Ok(lease) => lease,
        Err(error) if error.kind() == ErrorKind::NotFound => return age > missing_lease_age,
        Err(_) => return false,
    };
    match lease.try_lock() {
        Ok(()) => age > DEAD_OWNER_GRACE,
        Err(TryLockError::WouldBlock) => false,
        Err(TryLockError::Error(_)) => false,
    }
}

fn create_lease(path: &Path) -> Result<File> {
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)?;
    lease
        .try_lock()
        .map_err(std::io::Error::from)
        .context("failed to acquire filesystem lease")?;
    Ok(lease)
}

fn strip_private_bookkeeping(root: &Path) -> Result<()> {
    for name in [LEASE_FILE, "owner"] {
        remove_if_exists(&root.join(name))?;
    }
    Ok(())
}

fn reap_failures(root: &Path) -> Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if file_type.is_dir() {
            prune_failures(&path, None)?;
        } else {
            remove_if_exists(&path)?;
        }
    }
    Ok(())
}

fn prune_failures(root: &Path, keep: Option<&Path>) -> Result<()> {
    let directory = match fs::read_dir(root) {
        Ok(directory) => directory,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    let mut entries = directory
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|entry| keep.is_none_or(|keep| entry.path() != keep))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        std::cmp::Reverse(
            entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or(UNIX_EPOCH),
        )
    });
    for (index, entry) in entries.into_iter().enumerate() {
        let expired = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > FAILURE_MAX_AGE);
        let available_slots = if keep.is_some() {
            FAILURE_MAX_PER_TARGET.saturating_sub(1)
        } else {
            FAILURE_MAX_PER_TARGET
        };
        if index >= available_slots || expired {
            remove_if_exists(&entry.path())?;
        }
    }
    Ok(())
}

fn purge_failures(target: &Path) -> Result<()> {
    let name = safe_name(
        target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("analysis"),
    );
    remove_if_exists(&temporary_root().join("failures").join(name))
}

fn temporary_root() -> PathBuf {
    std::env::temp_dir().join("plaque-forge")
}

fn lock_path(target: &Path) -> PathBuf {
    temporary_root()
        .join("locks")
        .join(format!("{:016x}", path_hash(target)))
}

fn recover_interrupted_publication(target: &Path) -> Result<()> {
    let incoming = sibling(target, "incoming");
    let previous = sibling(target, "replaced");
    if previous.exists() {
        if target.exists() {
            remove(&previous).with_context(|| {
                format!(
                    "failed to remove obsolete publication backup {}",
                    previous.display()
                )
            })?;
        } else {
            fs::rename(&previous, target).with_context(|| {
                format!(
                    "failed to restore interrupted publication backup {}",
                    previous.display()
                )
            })?;
        }
    }
    remove_if_exists(&incoming).with_context(|| {
        format!(
            "failed to remove interrupted incoming output {}",
            incoming.display()
        )
    })
}

fn sibling(target: &Path, label: &str) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("output");
    target.with_file_name(format!(".{name}.{label}"))
}

fn path_hash(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn absolute_lexical(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    Ok(normalized)
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => remove(path),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn remove(path: &Path) -> Result<()> {
    let file_type = fs::symlink_metadata(path)?.file_type();
    if file_type.is_dir() && !file_type.is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(name: &str) -> PathBuf {
        static TEST_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let count = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "plaque-forge-staged-output-{name}-{}-{nonce}-{count}",
            std::process::id()
        ))
    }

    #[test]
    fn replacement_keeps_the_old_output_until_commit() {
        let root = test_root("replace");
        let target = root.join("analysis");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("value"), "old").unwrap();

        let staged = create(&target).unwrap();
        assert!(staged.path().starts_with(temporary_root().join("work")));
        fs::write(staged.path().join("value"), "new").unwrap();
        assert_eq!(fs::read_to_string(target.join("value")).unwrap(), "old");

        staged.commit(true).unwrap();
        assert_eq!(fs::read_to_string(target.join("value")).unwrap(), "new");

        let outside = root.join("outside");
        fs::write(&outside, "keep").unwrap();
        assert!(remove_child(&target, &outside).is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "keep");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_stage_is_removed_on_drop() {
        let root = test_root("drop");
        let target = root.join("analysis");
        let staged = create(&target).unwrap();
        let work = staged.path().to_path_buf();
        drop(staged);
        assert!(!work.exists());
    }

    #[test]
    fn active_work_is_protected_by_an_os_held_lease() {
        let root = test_root("active-lease");
        let target = root.join("analysis");
        let staged = create(&target).unwrap();
        assert!(!lease_is_abandoned(staged.path(), Duration::ZERO));
        assert!(staged.path().join(LEASE_FILE).is_file());
        drop(staged);
    }

    #[test]
    fn committed_directory_never_publishes_private_bookkeeping() {
        let root = test_root("private-bookkeeping");
        let target = root.join("analysis");
        let staged = create(&target).unwrap();
        fs::write(staged.path().join("value"), "new").unwrap();
        fs::write(staged.path().join("owner"), "legacy private path").unwrap();
        staged.commit(true).unwrap();
        assert_eq!(fs::read_to_string(target.join("value")).unwrap(), "new");
        assert!(!target.join(LEASE_FILE).exists());
        assert!(!target.join("owner").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn global_failure_reaper_enforces_the_run_cap() {
        let root = test_root("failure-reaper");
        let asset = root.join("asset");
        fs::create_dir_all(&asset).unwrap();
        for index in 0..5 {
            fs::create_dir(asset.join(format!("run-{index}"))).unwrap();
        }

        reap_failures(&root).unwrap();
        assert_eq!(
            fs::read_dir(&asset).unwrap().count(),
            FAILURE_MAX_PER_TARGET
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_recovers_an_interrupted_destination_publication() {
        let root = test_root("recover-publication");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("analysis");
        let previous = sibling(&target, "replaced");
        let incoming = sibling(&target, "incoming");
        fs::create_dir_all(&previous).unwrap();
        fs::write(previous.join("value"), "old").unwrap();
        fs::create_dir_all(&incoming).unwrap();
        fs::write(incoming.join("value"), "unfinished-new").unwrap();

        let staged = create(&target).unwrap();
        assert_eq!(fs::read_to_string(target.join("value")).unwrap(), "old");
        assert!(!previous.exists());
        assert!(!incoming.exists());
        drop(staged);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn create_discards_a_backup_after_the_new_target_was_installed() {
        let root = test_root("finish-publication-recovery");
        let target = root.join("analysis");
        let previous = sibling(&target, "replaced");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("value"), "new").unwrap();
        fs::create_dir_all(&previous).unwrap();
        fs::write(previous.join("value"), "old").unwrap();

        let staged = create(&target).unwrap();
        assert_eq!(fs::read_to_string(target.join("value")).unwrap(), "new");
        assert!(!previous.exists());
        drop(staged);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_bundle_publishes_manifest_last_and_replaces_every_member() {
        let root = test_root("file-bundle");
        fs::create_dir_all(&root).unwrap();
        let video = root.join("title.mkv");
        let mask = root.join("title.text-mask.png");
        let manifest = root.join("title.render-manifest.json");
        fs::write(&video, "old-video").unwrap();
        fs::write(&mask, "old-mask").unwrap();
        fs::write(&manifest, "old-manifest").unwrap();

        let staged = create(&video).unwrap();
        let staged_video = staged.path().join("title.mkv");
        let staged_mask = staged.path().join("title.text-mask.png");
        let staged_manifest = staged.path().join("title.render-manifest.json");
        fs::write(&staged_video, "new-video").unwrap();
        fs::write(&staged_mask, "new-mask").unwrap();
        fs::write(&staged_manifest, "new-manifest").unwrap();
        staged
            .commit_files(
                &[
                    (staged_mask, mask.clone()),
                    (staged_video, video.clone()),
                    (staged_manifest, manifest.clone()),
                ],
                true,
            )
            .unwrap();

        assert_eq!(fs::read_to_string(video).unwrap(), "new-video");
        assert_eq!(fs::read_to_string(mask).unwrap(), "new-mask");
        assert_eq!(fs::read_to_string(manifest).unwrap(), "new-manifest");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_bundle_rejects_members_outside_its_stage() {
        let root = test_root("outside-bundle-member");
        fs::create_dir_all(&root).unwrap();
        let target = root.join("title.mkv");
        let outside = root.join("outside.mkv");
        fs::write(&outside, "outside").unwrap();
        let staged = create(&target).unwrap();
        assert!(
            staged
                .commit_files(&[(outside.clone(), target)], true)
                .is_err()
        );
        assert_eq!(fs::read_to_string(outside).unwrap(), "outside");
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn failed_diagnostic_copy_does_not_leave_a_partial_retained_failure() {
        use std::os::unix::fs::symlink;

        let root = test_root("failed-retention");
        let target = root.join(format!(
            "analysis-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let stage = root.join("stage");
        fs::create_dir_all(stage.join("diagnostics")).unwrap();
        symlink(&target, stage.join("diagnostics/unsafe-link")).unwrap();

        assert!(retain_failure(&stage, &target).is_err());
        let retained_root = temporary_root()
            .join("failures")
            .join(safe_name(target.file_name().unwrap().to_str().unwrap()));
        assert!(
            !retained_root.exists() || fs::read_dir(&retained_root).unwrap().next().is_none(),
            "failed retention left a partial run"
        );
        let _ = remove_if_exists(&retained_root);
        fs::remove_dir_all(root).unwrap();
    }
}
