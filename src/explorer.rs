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
    #[cfg(windows)]
    identity: (u32, u64),
    #[cfg(windows)]
    creation_time: u64,
    #[cfg(windows)]
    attributes: u32,
}

impl Fingerprint {
    fn read(path: &Path) -> Result<Self> {
        #[cfg(not(windows))]
        let metadata = fs::symlink_metadata(path)?;
        #[cfg(windows)]
        let (metadata, info) = {
            use std::os::windows::fs::OpenOptionsExt;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS,
                FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
                GetFileInformationByHandle,
            };

            // Query the entry itself, including directory junctions, without opening its target.
            let file = fs::OpenOptions::new()
                .access_mode(0)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)?;
            let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
            if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            (file.metadata()?, unsafe { info.assume_init() })
        };
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            Kind::Symlink
        } else if is_reparse_point(&metadata) {
            Kind::Other
        } else if file_type.is_dir() {
            Kind::Directory
        } else if file_type.is_file() {
            Kind::File
        } else {
            Kind::Other
        };
        Ok(Self {
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
            #[cfg(windows)]
            identity: (
                info.dwVolumeSerialNumber,
                (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            ),
            #[cfg(windows)]
            creation_time: (u64::from(info.ftCreationTime.dwHighDateTime) << 32)
                | u64::from(info.ftCreationTime.dwLowDateTime),
            #[cfg(windows)]
            attributes: info.dwFileAttributes,
        })
    }
}

fn is_reparse_point(metadata: &Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
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
        let actual = Fingerprint::read(&path)
            .with_context(|| format!("Cannot inspect {}", path.display()))?;
        if actual != entry.fingerprint {
            bail!(
                "{} changed externally; reload the explorer first",
                path.display()
            );
        }
        if actual.kind == Kind::Directory || (actual.kind == Kind::Symlink && path.is_dir()) {
            self.navigate(&path)?;
            Ok(None)
        } else if matches!(actual.kind, Kind::File | Kind::Symlink) {
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
        let mut names: HashSet<String> = HashSet::new();
        let mut ids = HashSet::new();
        for line in &self.buffer.lines {
            if !ids.insert(line.id) {
                bail!("Explorer contains a duplicate line identity; reload before editing");
            }
            if line.text.is_empty() {
                continue;
            }
            let (name, directory) = parse_name(&line.text)?;
            #[cfg(windows)]
            for previous in &names {
                if windows_names_equal(name, previous)? {
                    bail!("Duplicate explorer destination: {name}");
                }
            }
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
        #[cfg(windows)]
        {
            before.identity == after.identity
                && before.creation_time == after.creation_time
                && before.attributes == after.attributes
        }
        #[cfg(not(any(unix, windows)))]
        {
            true
        }
    }
}

fn reserved_name(name: &str) -> Result<bool> {
    #[cfg(windows)]
    {
        let prefix: String = name.chars().take(".rockdown-stage".len()).collect();
        Ok(windows_names_equal(name, TRASH)? || windows_names_equal(&prefix, ".rockdown-stage")?)
    }
    #[cfg(not(windows))]
    {
        Ok(name == TRASH)
    }
}

#[cfg(windows)]
fn windows_names_equal(left: &str, right: &str) -> Result<bool> {
    use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};
    let left: Vec<u16> = left.encode_utf16().collect();
    let right: Vec<u16> = right.encode_utf16().collect();
    // Use Windows' ordinal case mapping, not Unicode's expanding lowercase mappings.
    let result = unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len().try_into()?,
            right.as_ptr(),
            right.len().try_into()?,
            1,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(result == CSTR_EQUAL)
}

#[cfg(windows)]
fn invalid_windows_name(name: &str) -> bool {
    if name.ends_with(['.', ' '])
        || name
            .chars()
            .any(|c| c <= '\u{1f}' || "\\:<>\"|?*".contains(c))
        || name.encode_utf16().count() > 255
    {
        return true;
    }
    // Device names remain reserved with extensions, including the superscript digits.
    let stem = name.split('.').next().unwrap_or(name).trim_end_matches(' ');
    let stem = stem.to_uppercase();
    matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$" | "CLOCK$"
    ) || stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2"
                    | "3"
                    | "4"
                    | "5"
                    | "6"
                    | "7"
                    | "8"
                    | "9"
                    | "\u{b9}"
                    | "\u{b2}"
                    | "\u{b3}"
            )
        })
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
        || reserved_name(name)?
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
    #[cfg(windows)]
    if invalid_windows_name(name) {
        bail!("Invalid Windows explorer filename {text:?}");
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
        if reserved_name(name)? {
            continue;
        }
        parse_name(name)
            .with_context(|| format!("Cannot represent filename {name:?} in explorer"))?;
        let fingerprint = Fingerprint::read(&item.path())?;
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
            if !metadata.is_dir()
                || metadata.file_type().is_symlink()
                || is_reparse_point(&metadata)
            {
                bail!(
                    "Refusing unsafe trash location {}; it must be a real directory, not a symlink or reparse point",
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
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
        let from = windows_path(source)?;
        let to = windows_path(destination)?;
        // No REPLACE_EXISTING or COPY_ALLOWED: never clobber or fall back to copy/delete.
        if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
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
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        let _ = (source, destination);
        bail!("Safe explorer commits require macOS, Linux, or Windows")
    }
}

#[cfg(windows)]
fn windows_path(path: &Path) -> Result<Vec<u16>> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        bail!("Windows explorer path contains a NUL: {}", path.display());
    }
    wide.push(0);
    Ok(wide)
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
        assert!(parse_name("two\nlines").is_err());
        // Win32 rejects this name before the explorer can enumerate it.
        #[cfg(not(windows))]
        {
            let sandbox = Sandbox::new();
            fs::write(sandbox.0.join("two\nlines"), "").unwrap();
            assert!(Explorer::open(&sandbox.0).is_err());
        }
    }

    #[test]
    fn no_replace_moves_preserve_existing_files_and_directories() {
        let sandbox = Sandbox::new();
        for directory in [false, true] {
            let source = sandbox
                .0
                .join(if directory { "source-dir" } else { "source" });
            let destination = sandbox
                .0
                .join(if directory { "target-dir" } else { "target" });
            if directory {
                fs::create_dir(&source).unwrap();
                fs::create_dir(&destination).unwrap();
                fs::write(source.join("contents"), "source").unwrap();
            } else {
                fs::write(&source, "source").unwrap();
                fs::write(&destination, "external").unwrap();
            }
            assert!(move_without_overwrite(&source, &destination).is_err());
            if directory {
                assert_eq!(
                    fs::read_to_string(source.join("contents")).unwrap(),
                    "source"
                );
                assert_eq!(fs::read_dir(&destination).unwrap().count(), 0);
                fs::remove_dir(&destination).unwrap();
            } else {
                assert_eq!(fs::read_to_string(&source).unwrap(), "source");
                assert_eq!(fs::read_to_string(&destination).unwrap(), "external");
                fs::remove_file(&destination).unwrap();
            }
            move_without_overwrite(&source, &destination).unwrap();
            assert!(!source.exists());
            let contents = if directory {
                destination.join("contents")
            } else {
                destination
            };
            assert_eq!(fs::read_to_string(contents).unwrap(), "source");
        }
    }

    #[test]
    fn case_only_renames_use_staging_for_files_and_directories() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("file"), "contents").unwrap();
        fs::create_dir(sandbox.0.join("folder")).unwrap();
        fs::write(sandbox.0.join("folder/child"), "child").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        rename_line(&mut explorer, "file", "FILE");
        rename_line(&mut explorer, "folder/", "FOLDER/");
        let report = explorer.commit().unwrap();
        assert_eq!(report.renamed.len(), 2);
        assert_eq!(explorer.buffer.text(), "FOLDER/\nFILE");
        assert_eq!(
            fs::read_to_string(sandbox.0.join("FILE")).unwrap(),
            "contents"
        );
        assert_eq!(
            fs::read_to_string(sandbox.0.join("FOLDER/child")).unwrap(),
            "child"
        );
        assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 2);
    }

    #[test]
    fn rollback_preserves_external_obstacles_and_staged_recovery_data() {
        let sandbox = Sandbox::new();
        let staged = sandbox.0.join("staged");
        let source = sandbox.0.join("source");
        fs::write(&staged, "original").unwrap();
        fs::write(&source, "external").unwrap();
        let mut operations = [Operation {
            source: Some(source.clone()),
            staged: staged.clone(),
            destination: sandbox.0.join("destination"),
            directory: false,
            location: Location::Staged,
        }];
        assert_eq!(rollback(&mut operations).len(), 1);
        assert_eq!(operations[0].location, Location::Staged);
        assert_eq!(fs::read_to_string(&source).unwrap(), "external");
        assert_eq!(fs::read_to_string(&staged).unwrap(), "original");
    }

    #[test]
    fn rejects_nul_paths_without_moving_the_source() {
        let sandbox = Sandbox::new();
        let source = sandbox.0.join("source");
        let destination = sandbox.0.join("destination");
        fs::write(&source, "safe").unwrap();
        assert!(parse_name("destination\0ignored").is_err());
        assert!(move_without_overwrite(&source, &sandbox.0.join("destination\0ignored")).is_err());
        assert!(move_without_overwrite(&sandbox.0.join("source\0ignored"), &destination).is_err());
        assert_eq!(fs::read_to_string(source).unwrap(), "safe");
        assert!(!destination.exists());
    }

    #[cfg(unix)]
    #[test]
    fn unix_filename_validation_remains_case_sensitive_and_permissive() {
        for name in [
            "CON",
            "nul.txt",
            "name:stream",
            "back\\slash",
            "trailing.",
            "trailing ",
            ".ROCKDOWN-TRASH",
            ".rockdown-stage-old",
        ] {
            assert!(parse_name(name).is_ok(), "rejected {name:?}");
        }
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("a"), "A").unwrap();
        fs::write(sandbox.0.join("b"), "B").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        rename_line(&mut explorer, "b", "A");
        assert!(explorer.desired().is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_invalid_and_reserved_names_before_mutation() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("original"), "safe").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        for name in [
            "CON",
            "con.txt",
            "PrN.log",
            "AUX",
            "nul.tar.gz",
            "COM1",
            "lpt9.txt",
            "COM\u{b9}.txt",
            "LPT\u{b2}",
            "COM\u{b3}",
            "CONIN$",
            "conout$.txt",
            "CLOCK$",
            "CON .txt",
            "name:stream",
            "C:relative",
            "back\\slash",
            "a<b",
            "a>b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "control\u{1f}",
            "trailing.",
            "trailing ",
            ".ROCKDOWN-TRASH",
            ".Rockdown-Stage",
            ".ROCKDOWN-STAGE-recovery",
        ] {
            for text in [name.to_owned(), format!("{name}/")] {
                assert!(parse_name(&text).is_err(), "accepted {text:?}");
            }
            explorer.buffer.lines[0].text = name.to_owned();
            assert!(explorer.commit().is_err(), "committed {name:?}");
            assert_eq!(
                fs::read_to_string(sandbox.0.join("original")).unwrap(),
                "safe"
            );
            assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 1);
        }
        assert!(parse_name(&"x".repeat(256)).is_err());
        assert!(parse_name(&"\u{1f600}".repeat(128)).is_err());
        for name in [
            "COM0",
            "COM10",
            "LPT0",
            "console.txt",
            "auxiliary",
            "normal name.txt",
            "\u{e9}.txt",
        ] {
            assert!(parse_name(name).is_ok(), "rejected {name:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_rejects_case_insensitive_destinations_before_mutation() {
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("first"), "one").unwrap();
        fs::write(sandbox.0.join("second"), "two").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        for (left, right) in [
            ("same", "SAME"),
            ("\u{e9}.txt", "\u{c9}.txt"),
            ("first", "FIRST"),
        ] {
            explorer.buffer.lines[0].text = left.to_owned();
            explorer.buffer.lines[1].text = right.to_owned();
            let error = explorer.commit().unwrap_err();
            assert!(error.to_string().contains("Duplicate explorer destination"));
            assert_eq!(fs::read_to_string(sandbox.0.join("first")).unwrap(), "one");
            assert_eq!(fs::read_to_string(sandbox.0.join("second")).unwrap(), "two");
            assert_eq!(fs::read_dir(&sandbox.0).unwrap().count(), 2);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_no_replace_preserves_a_case_variant_destination() {
        let sandbox = Sandbox::new();
        let source = sandbox.0.join("source");
        fs::write(&source, "original").unwrap();
        fs::write(sandbox.0.join("DESTINATION"), "external").unwrap();
        assert!(move_without_overwrite(&source, &sandbox.0.join("destination")).is_err());
        assert_eq!(fs::read_to_string(&source).unwrap(), "original");
        assert_eq!(
            fs::read_to_string(sandbox.0.join("DESTINATION")).unwrap(),
            "external"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_hides_and_protects_case_variants_of_recovery_names() {
        let sandbox = Sandbox::new();
        fs::create_dir(sandbox.0.join(".ROCKDOWN-TRASH")).unwrap();
        fs::create_dir(sandbox.0.join(".Rockdown-Stage-recovery")).unwrap();
        fs::write(sandbox.0.join(".Rockdown-Stage-recovery/precious"), "safe").unwrap();
        fs::write(sandbox.0.join("visible"), "delete me").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        assert_eq!(explorer.buffer.text(), "visible");
        explorer.buffer.lines[0].text.clear();
        explorer.commit().unwrap();
        assert_eq!(
            fs::read_to_string(sandbox.0.join(".Rockdown-Stage-recovery/precious")).unwrap(),
            "safe"
        );
        assert_eq!(explorer.buffer.text(), "");
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_preserve_utf16_and_unicode_renames() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let raw = OsString::from_wide(&[b'x' as u16, 0xd800]);
        assert_eq!(
            windows_path(Path::new(&raw)).unwrap(),
            [b'x' as u16, 0xd800, 0]
        );
        let sandbox = Sandbox::new();
        fs::write(sandbox.0.join("\u{e9}-\u{1f600}"), "contents").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        rename_line(&mut explorer, "\u{e9}-\u{1f600}", "\u{c9}-\u{1f600}");
        explorer.commit().unwrap();
        assert_eq!(explorer.buffer.text(), "\u{c9}-\u{1f600}");
        assert_eq!(
            fs::read_to_string(sandbox.0.join("\u{c9}-\u{1f600}")).unwrap(),
            "contents"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_fingerprint_detects_same_length_and_mtime_replacement() {
        let sandbox = Sandbox::new();
        let retained = Sandbox::new();
        let path = sandbox.0.join("original");
        fs::write(&path, "before").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        let before = explorer.snapshot["original"].fingerprint.clone();
        fs::rename(&path, retained.0.join("original")).unwrap();
        fs::write(&path, "after!").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(before.modified.unwrap()))
            .unwrap();
        let mut after = Fingerprint::read(&path).unwrap();
        assert_eq!(before.len, after.len);
        assert_eq!(before.modified, after.modified);
        assert_ne!(before.identity, after.identity);
        // Identity still detects replacement if creation timestamps happen to match.
        after.creation_time = before.creation_time;
        assert!(!same_restored_entry(&before, &after));
        explorer.adopt_rolled_back_snapshot();
        assert_eq!(explorer.snapshot["original"].fingerprint, before);
        assert!(explorer.enter().is_err());
        rename_line(&mut explorer, "original", "renamed");
        assert!(explorer.commit().is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "after!");
        assert!(!sandbox.0.join("renamed").exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_reparse_fingerprints_and_trash_never_follow_targets() {
        let sandbox = Sandbox::new();
        let target = Sandbox::new();
        let link = sandbox.0.join(".ROCKDOWN-TRASH");
        if let Err(error) = std::os::windows::fs::symlink_dir(&target.0, &link) {
            if error.raw_os_error() == Some(1314) {
                eprintln!(
                    "Skipping symlink test: enable Windows Developer Mode or symlink privilege"
                );
                return;
            }
            panic!("Cannot create test symlink: {error}");
        }
        let before = Fingerprint::read(&link).unwrap();
        assert_eq!(before.kind, Kind::Symlink);
        fs::write(target.0.join("precious"), "safe").unwrap();
        assert_eq!(Fingerprint::read(&link).unwrap(), before);
        assert!(ensure_trash_directory(&link).is_err());
        fs::write(sandbox.0.join("original"), "safe").unwrap();
        let mut explorer = Explorer::open(&sandbox.0).unwrap();
        explorer.buffer.lines[0].text.clear();
        assert!(explorer.commit().is_err());
        assert_eq!(
            fs::read_to_string(sandbox.0.join("original")).unwrap(),
            "safe"
        );
        assert_eq!(
            fs::read_to_string(target.0.join("precious")).unwrap(),
            "safe"
        );
    }

    #[test]
    fn failed_install_rolls_back_a_partially_completed_swap() {
        let sandbox = Sandbox::new();
        let staging = unique_directory(&sandbox.0, ".stage").unwrap();
        fs::write(sandbox.0.join("a"), "A").unwrap();
        fs::write(sandbox.0.join("b"), "B").unwrap();
        let before = Fingerprint::read(&sandbox.0.join("a")).unwrap();
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
        assert!(same_restored_entry(
            &before,
            &Fingerprint::read(&sandbox.0.join("a")).unwrap()
        ));
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
