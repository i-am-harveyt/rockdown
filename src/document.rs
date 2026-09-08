use crate::vim::Buffer;
use anyhow::{Context, Result, bail};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Document {
    pub buffer: Buffer,
    pub path: Option<PathBuf>,
    line_ending: &'static str,
    bom: bool,
    // Keep the exact on-disk text, not the normalized buffer, for conflict checks.
    saved: Option<String>,
}

impl Document {
    pub fn untitled(text: &str) -> Self {
        // The first newline determines the style; without one, use the platform default.
        let line_ending = match text.find('\n') {
            Some(index) if text[..index].ends_with('\r') => "\r\n",
            Some(_) => "\n",
            None if cfg!(windows) => "\r\n",
            None => "\n",
        };
        let bom = text.starts_with('\u{feff}');
        Self {
            buffer: Buffer::new(text.strip_prefix('\u{feff}').unwrap_or(text)),
            path: None,
            line_ending,
            bom,
            saved: None,
        }
    }
    pub fn open(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path).with_context(|| format!("Opening {}", path.display()))?;
        if !fs::metadata(&path)?.is_file() {
            bail!("Only regular text files can be edited");
        }
        let text = fs::read_to_string(&path).context("Documents must be UTF-8 text")?;
        let mut document = Self::untitled(&text);
        document.path = Some(path);
        document.saved = Some(text);
        Ok(document)
    }
    pub fn is_markdown(&self) -> bool {
        self.path.as_ref().is_none_or(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        })
    }
    pub fn save(&mut self, target: Option<&Path>, force: bool) -> Result<()> {
        let path = target
            .map(Path::to_path_buf)
            .or_else(|| self.path.clone())
            .context("Untitled document: use :w filename.md")?;
        let path = if path.exists() {
            fs::canonicalize(&path)?
        } else {
            fs::canonicalize(
                path.parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .join(path.file_name().context("Missing filename")?)
        };
        if !force {
            match fs::read_to_string(&path) {
                Ok(current) => {
                    if self.path.as_ref() != Some(&path) {
                        bail!("File already exists; use :w! to replace it");
                    }
                    if self.saved.as_ref() != Some(&current) {
                        bail!("File changed on disk; use :w! to overwrite or :e! to reload");
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if self.path.as_ref() == Some(&path) && self.saved.is_some() {
                        bail!("File was removed on disk; use :w! to recreate it");
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        let mut text = self.buffer.text();
        if self.line_ending != "\n" {
            text = text.replace('\n', self.line_ending);
        }
        if self.bom {
            text.insert(0, '\u{feff}');
        }
        atomic_write(&path, text.as_bytes())?;
        self.saved = Some(text);
        self.path = Some(path);
        self.buffer.mark_saved();
        Ok(())
    }
    pub fn reconcile(&mut self, report: &crate::explorer::CommitReport) {
        let Some(path) = self.path.as_ref() else {
            return;
        };
        // Compare against the original path once, so a rename cycle doesn't apply twice.
        if report
            .deleted
            .iter()
            .any(|deleted| path.starts_with(deleted))
        {
            self.path = None;
            self.saved = None;
        } else if let Some((old, new)) =
            report.renamed.iter().find(|(old, _)| path.starts_with(old))
        {
            self.path = Some(new.join(path.strip_prefix(old).expect("matched prefix")));
        }
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_file_name(format!(
        ".rockdown-save-{}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        if let Ok(metadata) = fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    // Windows cannot reliably rename or remove an open temporary file.
    #[cfg(windows)]
    drop(file);
    let result = result.and_then(|()| fs::rename(&temporary, path).map_err(Into::into));
    if result.is_err() {
        // Copied read-only attributes must not prevent cleanup after a failed save.
        #[cfg(windows)]
        #[allow(clippy::permissions_set_readonly_false)]
        if let Ok(metadata) = fs::metadata(&temporary) {
            let mut permissions = metadata.permissions();
            if permissions.readonly() {
                permissions.set_readonly(false);
                let _ = fs::set_permissions(&temporary, permissions);
            }
        }
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Saving {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_endings_and_bom_survive_edit_save_reload_and_save_as() {
        for ending in ["\n", "\r\n"] {
            for bom in ["", "\u{feff}"] {
                for trailing in ["", ending] {
                    let dir = tempfile::tempdir().unwrap();
                    let path = dir.path().join("note.md");
                    let original = format!("{bom}one{ending}two{trailing}");
                    fs::write(&path, &original).unwrap();
                    let mut doc = Document::open(&path).unwrap();
                    assert_eq!(doc.buffer.lines[0].text, "one");
                    assert_eq!(doc.saved.as_ref(), Some(&original));
                    assert!(!doc.buffer.dirty());

                    doc.buffer.key("A");
                    doc.buffer.insert_text(" edited");
                    doc.buffer.key("enter");
                    doc.buffer.insert_text("new");
                    doc.buffer.key("escape");
                    let expected = format!("{bom}one edited{ending}new{ending}two{trailing}");
                    assert!(doc.buffer.dirty());
                    doc.save(None, false).unwrap();
                    assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
                    assert_eq!(doc.saved.as_ref(), Some(&expected));
                    assert!(!doc.buffer.dirty());
                    // The saved snapshot must remain raw for the next conflict check.
                    doc.save(None, false).unwrap();

                    let mut reloaded = Document::open(&path).unwrap();
                    assert_eq!(reloaded.buffer.text(), doc.buffer.text());
                    assert!(!reloaded.buffer.dirty());
                    reloaded.save(None, false).unwrap();
                    let other = dir.path().join("copy.md");
                    reloaded.save(Some(&other), false).unwrap();
                    assert_eq!(fs::read(&other).unwrap(), expected.as_bytes());
                    reloaded.save(None, false).unwrap();
                }
            }
        }
    }

    #[test]
    fn untouched_bom_and_lone_carriage_returns_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        for text in [
            "\u{feff}",
            "\u{feff}plain",
            "\u{feff}one\r\n\r\ntwo\r\n",
            "\u{feff}one\ntwo\n",
            "one\rtwo\r",
            "\u{feff}one\rtwo\r\nlast\r",
            "\u{feff}\u{feff}content\r\n",
        ] {
            fs::write(&path, text).unwrap();
            let mut doc = Document::open(&path).unwrap();
            assert!(!doc.buffer.dirty());
            doc.save(None, false).unwrap();
            assert_eq!(fs::read(&path).unwrap(), text.as_bytes());
        }
    }

    #[test]
    fn newline_or_bom_only_external_changes_are_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        let original = "\u{feff}one\r\ntwo\r\n";
        for external in ["\u{feff}one\ntwo\n", "one\r\ntwo\r\n"] {
            fs::write(&path, original).unwrap();
            let mut doc = Document::open(&path).unwrap();
            fs::write(&path, external).unwrap();
            let error = doc.save(None, false).unwrap_err();
            assert!(error.to_string().contains("File changed on disk"));
            assert_eq!(fs::read(&path).unwrap(), external.as_bytes());
            assert_eq!(doc.saved.as_deref(), Some(original));
            doc.save(None, true).unwrap();
            assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
            doc.save(None, false).unwrap();

            fs::write(&path, external).unwrap();
            let mut reloaded = Document::open(&path).unwrap();
            reloaded.save(None, false).unwrap();
            assert_eq!(fs::read(&path).unwrap(), external.as_bytes());
        }
    }

    #[test]
    fn new_documents_use_provided_style_or_platform_default() {
        let default = if cfg!(windows) { "\r\n" } else { "\n" };
        for (initial, ending) in [
            ("", default),
            ("plain", default),
            ("one\ntwo", "\n"),
            ("one\r\ntwo", "\r\n"),
            ("\u{feff}one\ntwo", "\n"),
            ("\u{feff}one\r\ntwo", "\r\n"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("new.md");
            let mut doc = Document::untitled(initial);
            doc.buffer.key("G");
            doc.buffer.key("A");
            doc.buffer.insert_text("\r\npasted\r\ntext");
            doc.buffer.key("escape");
            doc.save(Some(&path), false).unwrap();
            let expected = format!("{initial}{ending}pasted{ending}text");
            assert_eq!(fs::read(&path).unwrap(), expected.as_bytes());
            assert!(!doc.buffer.text().contains('\r'));
        }
    }

    #[test]
    fn failed_rename_cleans_up_temporary_and_keeps_document_state() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("directory");
        fs::create_dir(&target).unwrap();
        let mut doc = Document::untitled("\u{feff}one\r\ntwo");
        doc.buffer.insert_text("local ");
        let error = doc.save(Some(&target), true).unwrap_err();
        assert!(error.to_string().contains("Saving"));
        assert!(target.is_dir());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        assert!(doc.path.is_none());
        assert!(doc.saved.is_none());
        assert!(doc.buffer.dirty());
    }

    #[cfg(unix)]
    #[test]
    fn atomic_save_preserves_unix_permissions_including_readonly() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "original").unwrap();
        for mode in [0o640, 0o444] {
            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            atomic_write(&path, b"replacement").unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"replacement");
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                mode
            );
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[cfg(windows)]
    #[test]
    #[allow(clippy::permissions_set_readonly_false)]
    fn failed_readonly_save_removes_readonly_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readonly.md");
        fs::write(&path, "original").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions.clone()).unwrap();
        let result = atomic_write(&path, b"replacement");
        // Restore the destination even if an assertion fails, so TempDir can remove it.
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), b"original");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn external_edits_and_save_as_collisions_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "original").unwrap();
        let mut doc = Document::open(&path).unwrap();
        doc.buffer.key("A");
        doc.buffer.insert_text(" local");
        doc.buffer.key("escape");
        fs::write(&path, "external").unwrap();
        assert!(doc.save(None, false).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "external");
        doc.save(None, true).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "original local");
        let other = dir.path().join("other.md");
        fs::write(&other, "protected").unwrap();
        assert!(doc.save(Some(&other), false).is_err());
        assert_eq!(fs::read_to_string(other).unwrap(), "protected");
    }
}
