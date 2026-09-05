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
    saved: Option<String>,
}

impl Document {
    pub fn untitled(text: &str) -> Self {
        Self {
            buffer: Buffer::new(text),
            path: None,
            saved: None,
        }
    }
    pub fn open(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path).with_context(|| format!("Opening {}", path.display()))?;
        if !fs::metadata(&path)?.is_file() {
            bail!("Only regular text files can be edited");
        }
        let text = fs::read_to_string(&path).context("Documents must be UTF-8 text")?;
        Ok(Self {
            buffer: Buffer::new(&text),
            path: Some(path),
            saved: Some(text),
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
        let text = self.buffer.text();
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
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("Saving {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
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
