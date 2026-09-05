use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs::{self, Metadata};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};

use crate::vim::Buffer;

const TRASH: &str = ".rockdown-trash";
static NEXT_TRANSACTION: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default)]
pub struct CommitReport {
    pub renamed: Vec<(PathBuf, PathBuf)>,
    pub deleted: Vec<PathBuf>,
    pub created: Vec<PathBuf>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Fingerprint {
    kind: Kind,
    len: u64,
    modified: Option<SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64, u32),
}

impl Fingerprint {
    fn new(metadata: &Metadata) -> Self {
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            Kind::Symlink
        } else if file_type.is_dir() {
            Kind::Directory
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        Self {
            kind,
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                    metadata.mode(),
                )
            },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Entry {
    name: String,
    fingerprint: Fingerprint,
}

impl Entry {
    fn display_name(&self) -> String {
        if self.fingerprint.kind == Kind::Directory {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}

pub struct Explorer {
    pub buffer: Buffer,
    pub directory: PathBuf,
    workspace: PathBuf,
    entries: HashMap<u64, Entry>,
    snapshot: BTreeMap<String, Entry>,
}

#[derive(Debug)]
struct Desired {
    id: u64,
    name: String,
    directory: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Location {
    Original,
    Staged,
    Destination,
}

struct Operation {
    source: Option<PathBuf>,
    staged: PathBuf,
    destination: PathBuf,
    directory: bool,
    location: Location,
}

impl Explorer {
    pub fn open(directory: &Path) -> Result<Self> {
        let directory = fs::canonicalize(directory)
            .with_context(|| format!("Cannot open directory {}", directory.display()))?;
        let snapshot = read_directory(&directory, None)?;
        let (buffer, entries) = make_buffer(&snapshot);
        Ok(Self {
            workspace: directory.clone(),
            directory,
            buffer,
            entries,
            snapshot,
        })
    }

    /// Reload never silently discards a pending filesystem edit.
    pub fn reload(&mut self) -> Result<()> {
        self.require_clean()?;
        self.refresh()
    }

    pub fn enter(&mut self) -> Result<Option<PathBuf>> {
        self.require_clean()?;
        let Some(line) = self.buffer.lines.get(self.buffer.row) else {
            return Ok(None);
        };
        let Some(entry) = self.entries.get(&line.id) else {
            return Ok(None);
        };
        let path = self.directory.join(&entry.name);
        let actual = fs::symlink_metadata(&path)
            .with_context(|| format!("Cannot inspect {}", path.display()))?;
        if Fingerprint::new(&actual) != entry.fingerprint {
            bail!(
                "{} changed externally; reload the explorer first",
                path.display()
            );
        }
        if actual.is_dir() || (actual.file_type().is_symlink() && path.is_dir()) {
            self.navigate(&path)?;
            Ok(None)
        } else if actual.is_file() || actual.file_type().is_symlink() {
            Ok(Some(path))
        } else {
            bail!("{} is not a regular file or directory", path.display());
        }
    }

    pub fn parent(&mut self) -> Result<()> {
        self.require_clean()?;
        if let Some(parent) = self.directory.parent() {
            let parent = parent.to_path_buf();
            self.navigate(&parent)?;
        }
        Ok(())
    }

    pub fn commit(&mut self) -> Result<CommitReport> {
        let desired = self.desired()?;
        self.check_snapshot(None)?;

        let desired_ids: HashSet<u64> = desired.iter().map(|entry| entry.id).collect();
        let mut report = CommitReport::default();
        for entry in &desired {
            let destination = self.directory.join(&entry.name);
            match self.entries.get(&entry.id) {
                Some(original) if original.name != entry.name => {
                    report
                        .renamed
                        .push((self.directory.join(&original.name), destination));
                }
                None => report.created.push(destination),
                _ => {}
            }
        }
        for (id, original) in &self.entries {
            if !desired_ids.contains(id) {
                report.deleted.push(self.directory.join(&original.name));
            }
        }
        report.deleted.sort();
        if report.renamed.is_empty() && report.deleted.is_empty() && report.created.is_empty() {
            self.refresh()?;
            return Ok(report);
        }

        // All affected sources leave the namespace before any destination is installed.
        // That makes swaps, longer rename cycles, and replacement of deleted names safe.
        let staging = unique_directory(&self.directory, ".rockdown-stage")?;
        let mut trash = None;
        let mut operations = Vec::new();
        let result = (|| -> Result<()> {
            if !report.deleted.is_empty() {
                let root = self.workspace.join(TRASH);
                ensure_trash_directory(&root)?;
                trash = Some(unique_directory(&root, "transaction")?);
            }
            for entry in &desired {
                let original = self.entries.get(&entry.id);
                if original.is_some_and(|original| original.name == entry.name) {
                    continue;
                }
                operations.push(Operation {
                    source: original.map(|original| self.directory.join(&original.name)),
                    staged: staging.join(format!("entry-{}", operations.len())),
                    destination: self.directory.join(&entry.name),
                    directory: entry.directory,
                    location: Location::Original,
                });
            }
            for deleted in &report.deleted {
                let trash = trash.as_ref().expect("deletions have a trash transaction");
                operations.push(Operation {
                    source: Some(deleted.clone()),
                    staged: staging.join(format!("entry-{}", operations.len())),
                    destination: trash.join(deleted.file_name().expect("entry has a filename")),
                    directory: false,
                    location: Location::Original,
                });
            }

            // Prepare new entries before touching any user data.
            for operation in &mut operations {
                if operation.source.is_none() {
                    if operation.directory {
                        fs::create_dir(&operation.staged)?;
                    } else {
                        fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&operation.staged)?;
                    }
                    operation.location = Location::Staged;
                }
            }
            self.check_snapshot(Some(&staging))?;
            for operation in &mut operations {
                if let Some(source) = &operation.source {
                    fs::rename(source, &operation.staged).with_context(|| {
                        format!("Cannot stage {} for explorer commit", source.display())
                    })?;
                    operation.location = Location::Staged;
                }
            }
            for operation in &mut operations {
                move_without_overwrite(&operation.staged, &operation.destination)?;
                operation.location = Location::Destination;
            }
            Ok(())
        })();

        if let Err(error) = result {
            let mut failures = rollback(&mut operations);
            if let Err(error) = fs::remove_dir(&staging) {
                failures.push(format!(
                    "staging retained at {}: {error}",
                    staging.display()
                ));
            }
            if let Some(trash) = &trash
                && let Err(error) = fs::remove_dir(trash)
            {
                failures.push(format!(
                    "recovery data retained at {}: {error}",
                    trash.display()
                ));
            }
            if failures.is_empty() {
                // Renaming back changes ctime even though the user's changes were rolled
                // back. Keep the pending buffer, but refresh fingerprints for a retry.
                self.adopt_rolled_back_snapshot();
                return Err(
                    error.context("Explorer commit failed; filesystem changes were rolled back")
                );
            }
            bail!(
                "Explorer commit failed: {error:#}. Rollback was incomplete; do not retry before recovering files: {}",
                failures.join("; ")
            );
        }

        fs::remove_dir(&staging).with_context(|| {
            format!(
                "Filesystem changes committed, but staging cleanup failed at {}; reload before further edits",
                staging.display()
            )
        })?;
        self.refresh().with_context(|| {
            format!(
                "Filesystem changes committed ({} renamed, {} deleted to {}, {} created), but explorer refresh failed",
                report.renamed.len(),
                report.deleted.len(),
                trash.as_deref().unwrap_or(&self.workspace).display(),
                report.created.len()
            )
        })?;
        Ok(report)
    }

    pub fn dirty(&self) -> bool {
        let visible = self
            .buffer
            .lines
            .iter()
            .filter(|line| !line.text.is_empty());
        let identities_changed = visible.clone().count() != self.entries.len()
            || visible.into_iter().any(|line| {
                self.entries.get(&line.id).is_none_or(|entry| {
                    line.text.strip_suffix('/').unwrap_or(&line.text) != entry.name
                })
            });
        self.buffer.dirty() || identities_changed
    }

    fn require_clean(&self) -> Result<()> {
        if self.dirty() {
            bail!(
                "Explorer has pending changes; commit or undo them before navigating or reloading"
            );
        }
        Ok(())
    }

    fn navigate(&mut self, directory: &Path) -> Result<()> {
        let directory = fs::canonicalize(directory)
            .with_context(|| format!("Cannot open {}", directory.display()))?;
        let snapshot = read_directory(&directory, None)?;
        let (buffer, entries) = make_buffer(&snapshot);
        // Moving above the workspace (or following a directory symlink outside it)
        // establishes a new trash root outside every entry we may subsequently move.
        if !directory.starts_with(&self.workspace) {
            self.workspace = directory.clone();
        }
        self.directory = directory;
        self.snapshot = snapshot;
        self.buffer = buffer;
        self.entries = entries;
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        let snapshot = read_directory(&self.directory, None)?;
        let (buffer, entries) = make_buffer(&snapshot);
        self.snapshot = snapshot;
        self.buffer = buffer;
        self.entries = entries;
        Ok(())
    }

    fn desired(&self) -> Result<Vec<Desired>> {
        let mut desired = Vec::new();
        let mut names = HashSet::new();
        let mut ids = HashSet::new();
        for line in &self.buffer.lines {
            if !ids.insert(line.id) {
                bail!("Explorer contains a duplicate line identity; reload before editing");
            }
            if line.text.is_empty() {
                continue;
            }
            let (name, directory) = parse_name(&line.text)?;
            if !names.insert(name.to_owned()) {
                bail!("Duplicate explorer destination: {name}");
            }
            if let Some(original) = self.entries.get(&line.id)
                && directory != (original.fingerprint.kind == Kind::Directory)
            {
                bail!(
                    "Cannot change the entry type of {}; use a new line instead",
                    original.name
                );
            }
            desired.push(Desired {
                id: line.id,
                name: name.to_owned(),
                directory,
            });
        }
        Ok(desired)
    }

    fn check_snapshot(&self, ignore: Option<&Path>) -> Result<()> {
        let current = read_directory(&self.directory, ignore)?;
        if current != self.snapshot {
            let changed = self
                .snapshot
                .keys()
                .chain(current.keys())
                .find(|name| self.snapshot.get(*name) != current.get(*name));
            bail!(
                "Directory changed externally at {}; commit aborted without overwriting external changes",
                changed
                    .map(|name| self.directory.join(name))
                    .unwrap_or_else(|| self.directory.clone())
                    .display()
            );
        }
        Ok(())
    }

    fn adopt_rolled_back_snapshot(&mut self) {
        let Ok(current) = read_directory(&self.directory, None) else {
            return;
        };
        // Only adopt when all original names and stable identities are back. Content
        // changes from another process must continue to cause a conflict.
        if current.len() != self.snapshot.len()
            || self.snapshot.iter().any(|(name, old)| {
                current
                    .get(name)
                    .is_none_or(|new| !same_restored_entry(&old.fingerprint, &new.fingerprint))
            })
        {
            return;
        }
        for entry in self.entries.values_mut() {
            if let Some(current) = current.get(&entry.name) {
                *entry = current.clone();
            }
        }
        self.snapshot = current;
    }
}

fn same_restored_entry(before: &Fingerprint, after: &Fingerprint) -> bool {
    before.kind == after.kind && before.len == after.len && before.modified == after.modified && {
        #[cfg(unix)]
        {
            before.identity.0 == after.identity.0
                && before.identity.1 == after.identity.1
                && before.identity.4 == after.identity.4
        }
        #[cfg(not(unix))]
        {
            true
        }
    }
}

fn parse_name(text: &str) -> Result<(&str, bool)> {
    let directory = text.ends_with('/');
    let name = if directory {
        &text[..text.len() - 1]
    } else {
        text
    };
    if name.is_empty()
        || name == "."
        || name == ".."
        || name == TRASH
        || name.contains(['/', '\0', '\n', '\r'])
        || Path::new(name).components().count() != 1
        || !matches!(
            Path::new(name).components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        bail!(
            "Invalid explorer filename {text:?}; use a single name, with an optional trailing slash for a directory"
        );
    }
    Ok((name, directory))
}

fn read_directory(directory: &Path, ignore: Option<&Path>) -> Result<BTreeMap<String, Entry>> {
    let mut entries = BTreeMap::new();
    for item in fs::read_dir(directory)
        .with_context(|| format!("Cannot read directory {}", directory.display()))?
    {
        let item = item?;
        if ignore.is_some_and(|ignore| item.path() == ignore) {
            continue;
        }
        let filename = item.file_name();
        let name = filename.to_str().ok_or_else(|| {
            anyhow!(
                "Cannot represent non-UTF-8 filename {:?} in explorer {}",
                filename,
                directory.display()
            )
        })?;
        if name == TRASH {
            continue;
        }
        parse_name(name)
            .with_context(|| format!("Cannot represent filename {name:?} in explorer"))?;
        let fingerprint = Fingerprint::new(&fs::symlink_metadata(item.path())?);
        entries.insert(
            name.to_owned(),
            Entry {
                name: name.to_owned(),
                fingerprint,
            },
        );
    }
    Ok(entries)
}

fn make_buffer(snapshot: &BTreeMap<String, Entry>) -> (Buffer, HashMap<u64, Entry>) {
    let mut sorted: Vec<&Entry> = snapshot.values().collect();
    sorted.sort_by(|a, b| {
        (a.fingerprint.kind != Kind::Directory)
            .cmp(&(b.fingerprint.kind != Kind::Directory))
            .then_with(|| a.name.cmp(&b.name))
    });
    let text = sorted
        .iter()
        .map(|entry| entry.display_name())
        .collect::<Vec<_>>()
        .join("\n");
    let mut buffer = Buffer::new(&text);
    buffer.mark_saved();
    let entries = buffer
        .lines
        .iter()
        .zip(sorted)
        .map(|(line, entry)| (line.id, entry.clone()))
        .collect();
    (buffer, entries)
}

fn ensure_trash_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                bail!(
                    "Refusing unsafe trash location {}; it must be a real directory, not a symlink",
                    path.display()
                );
            }
            Ok(())
        }
        Err(error) => {
            Err(error).with_context(|| format!("Cannot create trash directory {}", path.display()))
        }
    }
}

fn unique_directory(parent: &Path, prefix: &str) -> Result<PathBuf> {
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    for _ in 0..128 {
        let serial = NEXT_TRANSACTION.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!(
            "{prefix}-{}-{timestamp}-{serial}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("Cannot create staging directory {}", path.display())
                });
            }
        }
    }
    bail!(
        "Cannot allocate a unique staging directory in {}",
        parent.display()
    )
}

fn move_without_overwrite(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;
        let from = CString::new(source.as_os_str().as_bytes())?;
        let to = CString::new(destination.as_os_str().as_bytes())?;
        // The existence check and rename must be one kernel operation.
        #[cfg(target_os = "macos")]
        let status = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let status = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                from.as_ptr(),
                libc::AT_FDCWD,
                to.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if status != 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "Cannot move {} to {} without overwriting",
                    source.display(),
                    destination.display()
                )
            });
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (source, destination);
        bail!("Safe explorer commits require macOS or Linux")
    }
}

fn rollback(operations: &mut [Operation]) -> Vec<String> {
    let mut failures = Vec::new();
    // Vacate *all* final names before restoring originals, including rename cycles.
    for operation in operations.iter_mut().rev() {
        if operation.location == Location::Destination {
            match move_without_overwrite(&operation.destination, &operation.staged) {
                Ok(()) => operation.location = Location::Staged,
                Err(error) => failures.push(format!("{error:#}")),
            }
        }
    }
    for operation in operations.iter_mut().rev() {
        if operation.location != Location::Staged {
            continue;
        }
        let result = if let Some(source) = &operation.source {
            move_without_overwrite(&operation.staged, source)
        } else if operation.directory {
            // Never recursively delete: unexpected contents are preserved for recovery.
            fs::remove_dir(&operation.staged).map_err(anyhow::Error::from)
        } else {
            fs::remove_file(&operation.staged).map_err(anyhow::Error::from)
        };
        match result {
            Ok(()) => operation.location = Location::Original,
            Err(error) => failures.push(format!(
                "Cannot restore {}: {error:#}",
                operation.staged.display()
            )),
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sandbox(PathBuf);

    impl Sandbox {
        fn new() -> Self {
            Self(unique_directory(&std::env::temp_dir(), "rockdown-explorer-test").unwrap())
        }
    }

    impl Drop for Sandbox {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn rename_line(explorer: &mut Explorer, old: &str, new: &str) {
        explorer
            .buffer
            .lines
            .iter_mut()
            .find(|line| line.text == old)
            .unwrap()
            .text = new.to_owned();
    }

    #[test]
    fn creates_renames_and_recovers_deleted_entries() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("old"), "keep contents").unwrap();
        fs::create_dir(sandbox.0.join("removed")).unwrap();
        fs::write(sandbox.0.join("removed/child"), "recover me").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        rename_line(&mut explorer, "old", "renamed");
        explorer.buffer.lines.retain(|line| line.text != "removed/");
        explorer.buffer.row = 0;
        explorer.buffer.key("A");
        explorer.buffer.insert_text("\nnew-file\nnew-directory/");
        let report = explorer.commit().unwrap();
        assert_eq!(
            fs::read_to_string(sandbox.0.join("renamed")).unwrap(),
            "keep contents"
        );
        assert!(sandbox.0.join("new-file").is_file());
        assert!(sandbox.0.join("new-directory").is_dir());
        assert!(!sandbox.0.join("old").exists());
        assert!(!sandbox.0.join("removed").exists());
        assert_eq!(
            report.renamed,
            vec![(
                explorer.directory.join("old"),
                explorer.directory.join("renamed")
            )]
        );
        assert_eq!(report.deleted, vec![explorer.directory.join("removed")]);
        assert_eq!(
            report.created,
            vec![
                explorer.directory.join("new-file"),
                explorer.directory.join("new-directory")
            ]
        );
        let transaction = fs::read_dir(sandbox.0.join(TRASH))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(
            fs::read_to_string(transaction.join("removed/child")).unwrap(),
            "recover me"
        );
        assert!(!explorer.buffer.dirty());
    }

    #[test]
    fn rejects_traversal_duplicates_and_type_changes_before_mutation() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("original"), "untouched").unwrap();
        fs::write(sandbox.0.join("other"), "other contents").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        for invalid in [
            "..",
            ".",
            "../escape",
            "/absolute",
            "nested/name",
            TRASH,
            "other",
            "original/",
        ] {
            let line = explorer
                .buffer
                .lines
                .iter_mut()
                .find(|line| line.text != "other")
                .unwrap();
            line.text = invalid.to_owned();
            assert!(explorer.commit().is_err(), "accepted {invalid:?}");
            assert_eq!(
                fs::read_to_string(sandbox.0.join("original")).unwrap(),
                "untouched"
            );
            assert_eq!(
                fs::read_to_string(sandbox.0.join("other")).unwrap(),
                "other contents"
            );
            explorer.buffer.lines[0].text = "original".to_owned();
        }
    }

    #[test]
    fn rename_swap_preserves_both_files() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("a"), "A").unwrap();
        fs::write(sandbox.0.join("b"), "B").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        explorer.buffer.lines[0].text = "b".into();
        explorer.buffer.lines[1].text = "a".into();
        let report = explorer.commit().unwrap();
        assert_eq!(fs::read_to_string(sandbox.0.join("a")).unwrap(), "B");
        assert_eq!(fs::read_to_string(sandbox.0.join("b")).unwrap(), "A");
        assert_eq!(report.renamed.len(), 2);
    }

    #[test]
    fn external_collision_and_content_change_abort_without_mutation() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("source"), "original").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        rename_line(&mut explorer, "source", "destination");
        fs::write(sandbox.0.join("destination"), "external").unwrap();
        assert!(explorer.commit().is_err());
        assert_eq!(
            fs::read_to_string(sandbox.0.join("destination")).unwrap(),
            "external"
        );
        assert_eq!(
            fs::read_to_string(sandbox.0.join("source")).unwrap(),
            "original"
        );
        fs::remove_file(sandbox.0.join("destination")).unwrap();
        fs::write(sandbox.0.join("source"), "externally modified contents").unwrap();
        assert!(explorer.commit().is_err());
        assert!(!sandbox.0.join("destination").exists());
    }

    #[test]
    fn lists_hidden_files_and_refuses_dirty_navigation() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join(".hidden"), "").unwrap();
        fs::create_dir(sandbox.0.join("z-dir")).unwrap();
        fs::create_dir(sandbox.0.join(TRASH)).unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        assert_eq!(explorer.buffer.text(), "z-dir/\n.hidden");
        explorer.buffer.row = 1;
        explorer.buffer.col = 0;
        explorer.buffer.insert_text("changed-");
        assert!(explorer.enter().is_err());
        assert!(explorer.parent().is_err());
        assert!(explorer.reload().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn deleting_directory_symlink_never_touches_its_target() {
        let sandbox = Sandbox::new();
        let target = Sandbox::new();
        fs::write(target.0.join("precious"), "safe").unwrap();
        std::os::unix::fs::symlink(&target.0, sandbox.0.join("link")).unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        explorer.buffer.lines[0].text.clear();
        explorer.commit().unwrap();
        assert_eq!(
            fs::read_to_string(target.0.join("precious")).unwrap(),
            "safe"
        );
        let transaction = fs::read_dir(sandbox.0.join(TRASH))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(
            fs::symlink_metadata(transaction.join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn rejects_non_utf8_names() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let sandbox = Sandbox::new();
        let invalid = sandbox.0.join(OsString::from_vec(vec![0xff]));
        fs::write(&invalid, "").unwrap();
        assert!(Explorer::open(&sandbox.0).is_err());
    }

    #[test]
    fn rejects_multiline_names() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("two\nlines"), "").unwrap();
        assert!(Explorer::open(&sandbox.0).is_err());
    }

    #[test]
    fn failed_install_rolls_back_a_partially_completed_swap() {
        let sandbox = Sandbox::new();
        let staging = unique_directory(&sandbox.0, ".stage").unwrap();
        fs::write(sandbox.0.join("a"), "A").unwrap();
        fs::write(sandbox.0.join("b"), "B").unwrap();
        fs::rename(sandbox.0.join("a"), staging.join("0")).unwrap();
        fs::rename(sandbox.0.join("b"), staging.join("1")).unwrap();
        fs::rename(staging.join("0"), sandbox.0.join("b")).unwrap();
        let mut operations = vec![
            Operation {
                source: Some(sandbox.0.join("a")),
                staged: staging.join("0"),
                destination: sandbox.0.join("b"),
                directory: false,
                location: Location::Destination,
            },
            Operation {
                source: Some(sandbox.0.join("b")),
                staged: staging.join("1"),
                destination: sandbox.0.join("a"),
                directory: false,
                location: Location::Staged,
            },
        ];
        assert!(rollback(&mut operations).is_empty());
        assert_eq!(fs::read_to_string(sandbox.0.join("a")).unwrap(), "A");
        assert_eq!(fs::read_to_string(sandbox.0.join("b")).unwrap(), "B");
    }

    #[test]
    fn replacing_a_line_with_identical_text_is_still_a_pending_filesystem_edit() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("original"), "preserve until committed").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        for key in ["y", "y", "p", "k", "d", "d"] {
            explorer.buffer.key(key);
        }
        assert_eq!(explorer.buffer.text(), "original");
        assert!(explorer.parent().is_err());
        assert!(explorer.reload().is_err());
        assert_eq!(
            fs::read_to_string(sandbox.0.join("original")).unwrap(),
            "preserve until committed"
        );
    }
}
