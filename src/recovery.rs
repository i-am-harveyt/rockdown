use crate::{
    document::{Document, RecoveryContents},
    documents::{Documents, OpenDocument, Viewport},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const VERSION: u32 = 1;

/// Per-user storage, deliberately outside the workspace and its source files.
pub fn default_root() -> Result<PathBuf> {
    let get = |name| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let base = if cfg!(windows) {
        get("LOCALAPPDATA").or_else(|| get("USERPROFILE").map(|home| home.join("AppData/Local")))
    } else if cfg!(target_os = "macos") {
        get("HOME").map(|home| home.join("Library/Application Support"))
    } else {
        get("XDG_STATE_HOME").or_else(|| get("HOME").map(|home| home.join(".local/state")))
    };
    Ok(base
        .context("No user recovery storage directory is available")?
        .join("rockdown/recovery"))
}

/// The file handle owns the OS lock. It must not be unlinked, including on exit:
/// unlinking a locked inode would let another process lock a different inode.
pub struct RecoveryStore {
    directory: PathBuf,
    workspace: PathBuf,
    _lock: File,
    writable: Cell<bool>,
    checkpoint: RefCell<Option<Checkpoint>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    version: u32,
    workspace: StoredPath,
    active: usize,
    documents: Vec<StoredDocument>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDocument {
    path: Option<StoredPath>,
    contents: Option<RecoveryContents>,
    row: usize,
    col: usize,
    top: usize,
    offset: f32,
}

// Keep non-Unicode native filenames losslessly without unsafe OsString decoding.
#[derive(Serialize, Deserialize)]
#[serde(tag = "platform", content = "units")]
enum StoredPath {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

impl StoredPath {
    fn new(path: &Path) -> Self {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            Self::Unix(path.as_os_str().as_bytes().to_vec())
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            Self::Windows(path.as_os_str().encode_wide().collect())
        }
    }

    fn into_path(self) -> Result<PathBuf> {
        match self {
            #[cfg(unix)]
            Self::Unix(bytes) => {
                use std::os::unix::ffi::OsStringExt;
                Ok(std::ffi::OsString::from_vec(bytes).into())
            }
            #[cfg(windows)]
            Self::Windows(units) => {
                use std::os::windows::ffi::OsStringExt;
                Ok(std::ffi::OsString::from_wide(&units).into())
            }
            _ => bail!("Recovery snapshot uses a different operating system's paths"),
        }
    }
}

struct Checkpoint {
    active: u64,
    documents: Vec<DocumentStamp>,
}

struct DocumentStamp {
    id: u64,
    token: (u64, u64),
    path: Option<PathBuf>,
    row: usize,
    col: usize,
    top: usize,
    offset: u32,
}

impl DocumentStamp {
    fn new(entry: &OpenDocument) -> Self {
        Self {
            id: entry.id,
            token: entry.document.recovery_token(),
            path: entry.document.path.clone(),
            row: entry.document.buffer.row,
            col: entry.document.buffer.col,
            top: entry.viewport.top,
            offset: entry.viewport.offset.to_bits(),
        }
    }

    fn matches(&self, entry: &OpenDocument) -> bool {
        self.id == entry.id
            && self.token == entry.document.recovery_token()
            && self.path == entry.document.path
            && self.row == entry.document.buffer.row
            && self.col == entry.document.buffer.col
            && self.top == entry.viewport.top
            && self.offset == entry.viewport.offset.to_bits()
    }
}

impl RecoveryStore {
    pub fn acquire(root: &Path) -> Result<Self> {
        Self::acquire_in(&default_root()?, root)
    }

    /// Explicit-file invocations must not erase a directory's ordinary session.
    /// Callers pass the ordered absolute file identities resolved by Documents.
    pub fn acquire_explicit<'a>(
        root: &Path,
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Result<Self> {
        let mut hash = Sha256::new();
        for path in paths {
            ensure!(
                path.is_absolute(),
                "Explicit recovery paths must be absolute"
            );
            let bytes = path.as_os_str().as_encoded_bytes();
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        let storage = default_root()?
            .join("explicit")
            .join(format!("{:x}", hash.finalize()));
        Self::acquire_in(&storage, root)
    }

    /// Explicit storage is useful for isolated embedding and tests, without
    /// changing process-global HOME/XDG variables.
    pub fn acquire_in(storage_root: &Path, workspace_root: &Path) -> Result<Self> {
        let workspace = fs::canonicalize(workspace_root)
            .with_context(|| format!("Locating recovery workspace {}", workspace_root.display()))?;
        ensure!(workspace.is_dir(), "Recovery workspace must be a directory");
        private_directory(storage_root)?;
        let key = format!(
            "{:x}",
            Sha256::digest(workspace.as_os_str().as_encoded_bytes())
        );
        let directory = storage_root.join(key);
        private_directory(&directory)?;
        let lock = private_file(&directory.join("owner.lock"), false)?;
        lock_file(&lock).with_context(|| {
            format!(
                "Another Rockdown instance owns recovery for {}",
                workspace.display()
            )
        })?;
        let snapshot = directory.join("session.json");
        let writable = match fs::symlink_metadata(&snapshot) {
            Ok(_) => false,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Err(error) => return Err(error.into()),
        };
        Ok(Self {
            directory,
            workspace,
            _lock: lock,
            writable: Cell::new(writable),
            checkpoint: RefCell::new(None),
        })
    }

    /// Dirty/never-saved buffers come from recovery; clean saved files come from
    /// disk. Missing or unreadable clean files are omitted, not recreated.
    /// Snapshot errors disable writes until a subsequent successful load, so
    /// callers cannot replace the only recovery evidence with a fresh tab.
    pub fn load(&self) -> Result<Option<Documents>> {
        self.writable.set(false);
        self.checkpoint.borrow_mut().take();
        let result = self.load_snapshot();
        if result.is_ok() {
            self.writable.set(true);
        }
        result.with_context(|| format!("Loading recovery {}", self.directory.display()))
    }

    fn load_snapshot(&self) -> Result<Option<Documents>> {
        let path = self.directory.join("session.json");
        let file = match read_file(&path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let snapshot: Snapshot = serde_json::from_reader(BufReader::new(file))
            .context("Recovery snapshot is corrupt; it has been left untouched")?;
        ensure!(
            snapshot.version == VERSION,
            "Unsupported recovery snapshot version {}",
            snapshot.version
        );
        ensure!(
            snapshot.workspace.into_path()? == self.workspace,
            "Recovery workspace identity mismatch"
        );
        let mut entries: Vec<OpenDocument> = Vec::with_capacity(snapshot.documents.len());
        let mut active = 0;
        for (index, stored) in snapshot.documents.into_iter().enumerate() {
            let path = stored.path.map(StoredPath::into_path).transpose()?;
            ensure!(
                path.as_ref().is_none_or(|path| path.is_absolute()),
                "Recovery document path must be absolute"
            );
            let mut document = match stored.contents {
                Some(contents) => Document::from_recovery(path, contents),
                None => {
                    let path = path.context("Recovery document has neither a file nor contents")?;
                    match Document::open(&path) {
                        Ok(document) => document,
                        Err(_) => continue,
                    }
                }
            };
            // Symlinks may have changed to make formerly distinct clean tabs
            // aliases. Retain dirty copies rather than silently losing edits.
            if !document.buffer.dirty()
                && document.path.is_some()
                && entries
                    .iter()
                    .any(|entry| entry.document.path == document.path)
            {
                continue;
            }
            document.buffer.restore_cursor(stored.row, stored.col);
            let top = stored.top.min(document.buffer.lines.len() - 1);
            let offset = if top == stored.top && stored.offset.is_finite() {
                stored.offset.max(0.)
            } else {
                0.
            };
            if index <= snapshot.active {
                active = entries.len();
            }
            entries.push(OpenDocument {
                id: index as u64 + 1,
                document,
                viewport: Viewport { top, offset },
            });
        }
        Ok(Documents::from_recovery(entries, active))
    }

    /// No source files are written. Unchanged timer ticks compare only small
    /// tokens/positions; they do not scan or copy the document text or serialize.
    pub fn checkpoint(&self, documents: &Documents) -> Result<()> {
        self.ensure_writable()?;
        if self.checkpoint.borrow().as_ref().is_some_and(|previous| {
            previous.active == documents.active_id()
                && previous.documents.len() == documents.entries().len()
                && previous
                    .documents
                    .iter()
                    .zip(documents.entries())
                    .all(|(a, b)| a.matches(b))
        }) {
            return Ok(());
        }
        self.persist(documents, false)?;
        *self.checkpoint.borrow_mut() = Some(Checkpoint {
            active: documents.active_id(),
            documents: documents.entries().iter().map(DocumentStamp::new).collect(),
        });
        Ok(())
    }

    /// Call only after close/discard was accepted. Never flush dirty buffers to
    /// Markdown; retain only file identities for saved tabs, not discarded text.
    pub fn finish(&self, documents: &Documents) -> Result<()> {
        self.ensure_writable()?;
        self.persist(documents, true)?;
        self.checkpoint.borrow_mut().take();
        Ok(())
    }

    fn ensure_writable(&self) -> Result<()> {
        ensure!(
            self.writable.get(),
            "Recovery snapshot must load successfully before it can be replaced"
        );
        Ok(())
    }

    fn persist(&self, documents: &Documents, clean_exit: bool) -> Result<()> {
        let mut active = 0;
        let mut stored = Vec::with_capacity(documents.entries().len());
        for entry in documents.entries() {
            if clean_exit && !entry.document.has_saved_file() {
                continue;
            }
            if entry.id == documents.active_id() {
                active = stored.len();
            }
            stored.push(StoredDocument {
                path: entry.document.path.as_deref().map(StoredPath::new),
                contents: if clean_exit {
                    None
                } else {
                    entry.document.recovery_contents()
                },
                row: entry.document.buffer.row,
                col: entry.document.buffer.col,
                top: entry.viewport.top,
                offset: if entry.viewport.offset.is_finite() {
                    entry.viewport.offset.max(0.)
                } else {
                    0.
                },
            });
        }
        let snapshot = Snapshot {
            version: VERSION,
            workspace: StoredPath::new(&self.workspace),
            active,
            documents: stored,
        };
        self.replace(&snapshot)
            .with_context(|| format!("Writing recovery {}", self.directory.display()))
    }

    fn replace(&self, snapshot: &Snapshot) -> Result<()> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let temporary = self.directory.join(format!(
            ".session-{}-{}.tmp",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let file = private_file(&temporary, true)?;
        let result = (|| -> Result<()> {
            let mut writer = BufWriter::new(file);
            serde_json::to_writer(&mut writer, snapshot)?;
            writer.flush()?;
            writer.get_ref().sync_all()?;
            drop(writer);
            replace_file(&temporary, &self.directory.join("session.json"))?;
            sync_directory(&self.directory)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Recovery storage must be a real directory"
        );
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "Recovery storage belongs to another user"
        );
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
            sync_directory(parent)?;
        }
    }
    #[cfg(windows)]
    {
        fs::create_dir_all(path)?;
        let metadata = fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Recovery storage must be a real directory"
        );
        windows_private_directory(path)?;
    }
    Ok(())
}

fn private_file(path: &Path, exclusive: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    if exclusive {
        options.create_new(true);
    } else {
        options.create(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Recovery storage must contain regular files"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "Recovery file belongs to another user"
        );
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

fn read_file(path: &Path) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "Recovery snapshot must be a regular file",
        ));
    }
    Ok(file)
}

#[cfg(unix)]
fn lock_file(file: &File) -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == -1 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
fn lock_file(file: &File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY, LockFileEx,
    };
    let mut overlapped = unsafe { std::mem::zeroed() };
    if unsafe {
        LockFileEx(
            file.as_raw_handle(),
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(unix)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    if unsafe {
        MoveFileExW(
            wide(from).as_ptr(),
            wide(to).as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    // Windows uses MOVEFILE_WRITE_THROUGH for the replacement instead.
    #[cfg(windows)]
    let _ = path;
    Ok(())
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

#[cfg(windows)]
fn windows_private_directory(path: &Path) -> Result<()> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, SetFileSecurityW,
        },
    };
    // Protected inheritable DACL: only the object owner and LocalSystem. New
    // lock/snapshot files inherit this before any sensitive bytes are written.
    let descriptor = windows_sys::core::w!("D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)");
    let mut security = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            descriptor,
            SDDL_REVISION_1,
            &mut security,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let result = unsafe {
        SetFileSecurityW(
            wide(path).as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            security,
        )
    };
    let error = (result == 0).then(std::io::Error::last_os_error);
    unsafe { LocalFree(security) };
    if let Some(error) = error {
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crash_restores_unsaved_tabs_baselines_and_active_viewport_without_writing_sources() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = root.path().join("named.md");
        fs::write(&path, "\u{feff}one\r\ntwo\r\n").unwrap();
        let mut documents = Documents::new(Document::untitled("initial"));
        documents.current_mut().buffer.insert_text("unsaved ");
        documents.open(&path).unwrap();
        documents.current_mut().buffer.key("A");
        documents.current_mut().buffer.insert_text(" edited");
        documents.current_mut().buffer.key("escape");
        *documents.viewport_mut() = Viewport {
            top: 1,
            offset: 7.5,
        };
        let cursor = (
            documents.current().buffer.row,
            documents.current().buffer.col,
        );
        {
            let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
            store.checkpoint(&documents).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), "\u{feff}one\r\ntwo\r\n");
        }
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        let mut recovered = store.load().unwrap().unwrap();
        assert_eq!(recovered.entries().len(), 2);
        assert_eq!(recovered.current().buffer.text(), "one edited\ntwo\n");
        assert!(recovered.current().buffer.dirty());
        assert_eq!(
            (
                recovered.current().buffer.row,
                recovered.current().buffer.col
            ),
            cursor
        );
        assert_eq!(recovered.viewport().top, 1);
        assert_eq!(recovered.viewport().offset, 7.5);
        recovered.save(None, false).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "\u{feff}one edited\r\ntwo\r\n"
        );
        recovered.previous();
        assert!(recovered.current().path.is_none());
        assert_eq!(recovered.current().buffer.text(), "unsaved initial");
        assert!(recovered.current().buffer.dirty());
        recovered.current_mut().buffer.key("0");
        recovered.current_mut().buffer.key("d");
        recovered.current_mut().buffer.key("w");
        assert_eq!(recovered.current().buffer.text(), "initial");
        assert!(!recovered.current().buffer.dirty());
    }

    #[test]
    fn recovered_named_edits_keep_pre_crash_conflicts_including_removed_files() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = root.path().join("named.md");
        fs::write(&path, "original\r\n").unwrap();
        let mut documents = Documents::new(Document::open(&path).unwrap());
        documents.current_mut().buffer.insert_text("local ");
        {
            let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
            store.checkpoint(&documents).unwrap();
        }
        fs::write(&path, "original\n").unwrap();
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        let mut recovered = store.load().unwrap().unwrap();
        assert!(recovered.save(None, false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "original\n");
        assert_eq!(recovered.current().buffer.text(), "local original\n");
        fs::remove_file(&path).unwrap();
        assert!(recovered.save(None, false).is_err());
        assert!(!path.exists());
        recovered.save(None, true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "local original\r\n");
    }

    #[test]
    fn clean_exit_discards_edits_and_untitled_tabs_but_reopens_saved_files_from_disk() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let clean = root.path().join("clean.md");
        let dirty = root.path().join("dirty.md");
        fs::write(&clean, "one\ntwo").unwrap();
        fs::write(&dirty, "original").unwrap();
        let mut documents = Documents::new(Document::open(&clean).unwrap());
        documents.current_mut().buffer.key("j");
        documents.viewport_mut().top = 1;
        documents.open(&dirty).unwrap();
        documents.current_mut().buffer.insert_text("discard ");
        documents.new_document();
        documents
            .current_mut()
            .buffer
            .insert_text("discard untitled");
        {
            let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
            store.checkpoint(&documents).unwrap();
            store.finish(&documents).unwrap();
        }
        assert_eq!(fs::read_to_string(&dirty).unwrap(), "original");
        fs::write(&clean, "new\ncontent").unwrap();
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        let recovered = store.load().unwrap().unwrap();
        assert_eq!(recovered.entries().len(), 2);
        assert_eq!(recovered.entries()[1].document.buffer.text(), "original");
        assert!(!recovered.entries()[1].document.buffer.dirty());
        assert_eq!(recovered.current().buffer.text(), "new\ncontent");
        assert!(!recovered.current().buffer.dirty());
        assert_eq!(recovered.current().buffer.row, 1);
        assert_eq!(recovered.viewport().top, 1);
    }

    #[test]
    fn clean_crash_tabs_reload_disk_and_clamp_cursor_and_viewport() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = root.path().join("named.md");
        fs::write(&path, "first\nsecond line").unwrap();
        let mut documents = Documents::new(Document::open(&path).unwrap());
        documents.current_mut().buffer.row = usize::MAX;
        documents.current_mut().buffer.col = usize::MAX;
        *documents.viewport_mut() = Viewport {
            top: usize::MAX,
            offset: f32::INFINITY,
        };
        {
            let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
            store.checkpoint(&documents).unwrap();
        }
        fs::write(&path, "a\u{301}\u{1f469}\u{200d}\u{1f4bb}").unwrap();
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        let recovered = store.load().unwrap().unwrap();
        assert_eq!(
            recovered.current().buffer.text(),
            "a\u{301}\u{1f469}\u{200d}\u{1f4bb}"
        );
        assert_eq!(recovered.current().buffer.row, 0);
        assert_eq!(recovered.current().buffer.col, "a\u{301}".len());
        assert!(!recovered.current().buffer.dirty());
        assert_eq!(recovered.viewport().top, 0);
        assert_eq!(recovered.viewport().offset, 0.);
    }

    #[test]
    fn corrupt_and_future_snapshots_are_preserved_and_block_writes() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        let documents = Documents::new(Document::untitled("precious"));
        store.checkpoint(&documents).unwrap();
        let snapshot_path = store.directory.join("session.json");
        let mut future: serde_json::Value =
            serde_json::from_slice(&fs::read(&snapshot_path).unwrap()).unwrap();
        future["version"] = serde_json::json!(VERSION + 1);
        for bytes in [b"{truncated".to_vec(), serde_json::to_vec(&future).unwrap()] {
            fs::write(&snapshot_path, &bytes).unwrap();
            assert!(store.load().is_err());
            assert!(store.checkpoint(&documents).is_err());
            assert!(store.finish(&documents).is_err());
            assert_eq!(fs::read(&snapshot_path).unwrap(), bytes);
        }
        let bytes = fs::read(&snapshot_path).unwrap();
        drop(store);
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        assert!(store.checkpoint(&documents).is_err());
        assert_eq!(fs::read(&snapshot_path).unwrap(), bytes);
    }

    #[test]
    fn ownership_is_exclusive_until_drop_and_workspace_aliases_share_it() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        assert!(RecoveryStore::acquire_in(storage.path(), &root.path().join(".")).is_err());
        let other_store = RecoveryStore::acquire_in(storage.path(), other.path()).unwrap();
        let documents = Documents::new(Document::untitled("recover me"));
        store.checkpoint(&documents).unwrap();
        assert!(other_store.load().unwrap().is_none());
        drop(store);
        let reopened = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        assert_eq!(
            reopened.load().unwrap().unwrap().current().buffer.text(),
            "recover me"
        );
    }

    #[test]
    fn saves_and_reloads_invalidate_the_unchanged_checkpoint_cache() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = root.path().join("named.md");
        fs::write(&path, "original").unwrap();
        let mut documents = Documents::new(Document::open(&path).unwrap());
        documents.current_mut().buffer.insert_text("local ");
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        store.checkpoint(&documents).unwrap();
        let snapshot_path = store.directory.join("session.json");
        let modified = fs::metadata(&snapshot_path).unwrap().modified().unwrap();
        store.checkpoint(&documents).unwrap();
        assert_eq!(
            fs::metadata(&snapshot_path).unwrap().modified().unwrap(),
            modified
        );
        documents.save(None, false).unwrap();
        store.checkpoint(&documents).unwrap();
        fs::write(&path, "external").unwrap();
        assert_eq!(
            store.load().unwrap().unwrap().current().buffer.text(),
            "external"
        );
        documents.reload(false).unwrap();
        documents.current_mut().buffer.insert_text("new ");
        store.checkpoint(&documents).unwrap();
        fs::write(&path, "replacement").unwrap();
        documents.reload(true).unwrap();
        documents.current_mut().buffer.insert_text("new ");
        store.checkpoint(&documents).unwrap();
        assert_eq!(
            store.load().unwrap().unwrap().current().buffer.text(),
            "new replacement"
        );
    }

    #[test]
    fn never_saved_named_contents_survive_without_creating_or_replacing_files() {
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = root.path().join("new.md");
        let mut documents = Documents::new(Document::untitled(""));
        documents.open(&path).unwrap();
        documents.current_mut().buffer.insert_text("unwritten");
        {
            let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
            store.checkpoint(&documents).unwrap();
        }
        assert!(!path.exists());
        fs::write(&path, "created externally").unwrap();
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        let mut recovered = store.load().unwrap().unwrap();
        assert_eq!(recovered.current().buffer.text(), "unwritten");
        assert!(recovered.current().buffer.dirty());
        assert!(recovered.save(None, false).is_err());
        store.finish(&recovered).unwrap();
        assert!(store.load().unwrap().is_none());
        assert_eq!(fs::read_to_string(path).unwrap(), "created externally");
    }

    #[cfg(unix)]
    #[test]
    fn snapshots_are_private_and_native_non_unicode_paths_round_trip() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().unwrap();
        let storage = tempfile::tempdir().unwrap();
        let path = root.path().join("中文 note.md");
        fs::write(&path, "original").unwrap();
        let mut documents = Documents::new(Document::open(&path).unwrap());
        documents.current_mut().buffer.insert_text("local ");
        let store = RecoveryStore::acquire_in(storage.path(), root.path()).unwrap();
        store.checkpoint(&documents).unwrap();
        assert_eq!(
            fs::metadata(&store.directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(store.directory.join("session.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let recovered = store.load().unwrap().unwrap();
        assert_eq!(
            recovered.current().path.as_ref(),
            documents.current().path.as_ref()
        );
        assert_eq!(recovered.current().buffer.text(), "local original");
        // macOS filesystems reject invalid UTF-8 names. Test the lossless native
        // path codec separately without requiring such a file to exist.
        use std::os::unix::ffi::OsStringExt;
        let native = PathBuf::from(std::ffi::OsString::from_vec(b"note-\xff.md".to_vec()));
        let serialized = serde_json::to_vec(&StoredPath::new(&native)).unwrap();
        let decoded: StoredPath = serde_json::from_slice(&serialized).unwrap();
        assert_eq!(decoded.into_path().unwrap(), native);
    }
}
