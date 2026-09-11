use crate::{document::Document, explorer::CommitReport};
use anyhow::{Context, Result, bail};
use std::{fs, path::Path};

#[derive(Clone, Copy, Debug, Default)]
pub struct Viewport {
    pub top: usize,
    pub offset: f32,
}

pub struct OpenDocument {
    pub id: u64,
    pub document: Document,
    pub viewport: Viewport,
}

pub struct Documents {
    entries: Vec<OpenDocument>,
    active: usize,
    next_id: u64,
}

impl Documents {
    pub fn new(document: Document) -> Self {
        Self {
            entries: vec![OpenDocument {
                id: 1,
                document,
                viewport: Viewport::default(),
            }],
            active: 0,
            next_id: 2,
        }
    }

    pub fn current(&self) -> &Document {
        &self.entries[self.active].document
    }

    pub fn current_mut(&mut self) -> &mut Document {
        &mut self.entries[self.active].document
    }

    pub fn active_id(&self) -> u64 {
        self.entries[self.active].id
    }

    pub fn entries(&self) -> &[OpenDocument] {
        &self.entries
    }

    pub fn viewport(&self) -> &Viewport {
        &self.entries[self.active].viewport
    }

    pub fn viewport_mut(&mut self) -> &mut Viewport {
        &mut self.entries[self.active].viewport
    }

    pub fn select(&mut self, id: u64) -> Result<()> {
        self.active = self
            .entries
            .iter()
            .position(|entry| entry.id == id)
            .context("No such buffer")?;
        Ok(())
    }

    pub fn new_document(&mut self) {
        let entry = self.entry(Document::untitled(""));
        self.entries.push(entry);
        self.active = self.entries.len() - 1;
    }

    pub fn get(&self, id: u64) -> Option<&Document> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| &entry.document)
    }

    /// Dialog completions belong to the initiating buffer, even after a tab switch.
    pub fn save_id(&mut self, id: u64, target: Option<&Path>) -> Result<()> {
        let previous = self.active_id();
        self.select(id)?;
        let result = self.save(target, false);
        self.select(previous)?;
        result
    }

    pub fn next(&mut self) {
        self.active = (self.active + 1) % self.entries.len();
    }

    pub fn previous(&mut self) {
        self.active = if self.active == 0 {
            self.entries.len() - 1
        } else {
            self.active - 1
        };
    }

    pub fn delete(&mut self, force: bool) -> Result<()> {
        if !force && self.current().buffer.dirty() {
            bail!("Buffer has unsaved changes; use :bd! to discard them");
        }
        if self.entries.len() == 1 {
            let replacement = self.entry(Document::untitled(""));
            self.entries[0] = replacement;
        } else {
            self.entries.remove(self.active);
            self.active = self.active.min(self.entries.len() - 1);
        }
        Ok(())
    }

    pub fn open(&mut self, path: &Path) -> Result<()> {
        let path = fs::canonicalize(path).with_context(|| format!("Opening {}", path.display()))?;
        if let Some(index) = self
            .entries
            .iter()
            .position(|entry| entry.document.path.as_ref() == Some(&path))
        {
            self.active = index;
            return Ok(());
        }
        let document = Document::open(&path)?;
        let entry = self.entry(document);
        self.entries.push(entry);
        self.active = self.entries.len() - 1;
        Ok(())
    }

    pub fn reload(&mut self, force: bool) -> Result<()> {
        if !force && self.current().buffer.dirty() {
            bail!("Buffer has unsaved changes; use :e! to discard them");
        }
        let path = self
            .current()
            .path
            .as_deref()
            .context("Untitled document has no file to reload")?;
        let document = Document::open(path)?;
        self.entries[self.active].document = document;
        self.entries[self.active].viewport = Viewport::default();
        Ok(())
    }

    pub fn save(&mut self, target: Option<&Path>, force: bool) -> Result<()> {
        let path = target
            .or(self.current().path.as_deref())
            .context("Untitled document: use :w filename.md")?;
        // Match Document::save's identity for both existing files and new names.
        let path = if path.exists() {
            fs::canonicalize(path)?
        } else {
            fs::canonicalize(
                path.parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or(Path::new(".")),
            )?
            .join(path.file_name().context("Missing filename")?)
        };
        if self.entries.iter().enumerate().any(|(index, entry)| {
            index != self.active && entry.document.path.as_ref() == Some(&path)
        }) {
            bail!("File is already open in another buffer");
        }
        self.current_mut().save(Some(&path), force)
    }

    pub fn dirty(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.document.buffer.dirty())
    }

    pub fn reconcile(&mut self, report: &CommitReport) {
        // Each document compares the complete report against its original path once.
        for entry in &mut self.entries {
            entry.document.reconcile(report);
        }
    }

    fn entry(&mut self, document: Document) -> OpenDocument {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).expect("Buffer IDs exhausted");
        OpenDocument {
            id,
            document,
            viewport: Viewport::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vim::Mode;

    #[test]
    fn switching_preserves_edits_cursor_mode_viewport_and_independent_undo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.md");
        fs::write(&path, "other").unwrap();
        let mut docs = Documents::new(Document::untitled("first\nsecond"));
        let first = docs.active_id();
        docs.current_mut().buffer.key("j");
        docs.current_mut().buffer.key("A");
        docs.current_mut().buffer.insert_text(" edited");
        let cursor = (docs.current().buffer.row, docs.current().buffer.col);
        *docs.viewport_mut() = Viewport {
            top: 1,
            offset: 7.5,
        };
        docs.open(&path).unwrap();
        let second = docs.active_id();
        assert!(!docs.current().buffer.dirty());
        assert!(docs.dirty());
        docs.current_mut().buffer.insert_text("another ");
        docs.viewport_mut().offset = 3.0;
        docs.select(first).unwrap();
        assert_eq!(docs.current().buffer.text(), "first\nsecond edited");
        assert_eq!(
            (docs.current().buffer.row, docs.current().buffer.col),
            cursor
        );
        assert_eq!(docs.current().buffer.mode, Mode::Insert);
        assert_eq!((docs.viewport().top, docs.viewport().offset), (1, 7.5));
        docs.current_mut().buffer.undo();
        assert_eq!(docs.current().buffer.text(), "first\nsecond");
        assert!(docs.dirty());
        docs.next();
        assert_eq!(docs.active_id(), second);
        assert_eq!(docs.current().buffer.text(), "another other");
        assert_eq!(docs.viewport().offset, 3.0);
        docs.current_mut().buffer.undo();
        assert_eq!(docs.current().buffer.text(), "other");
        assert!(!docs.dirty());
        docs.next();
        assert_eq!(docs.active_id(), first);
        docs.previous();
        assert_eq!(docs.active_id(), second);
        docs.previous();
        assert_eq!(docs.active_id(), first);
        docs.current_mut().buffer.redo();
        assert_eq!(docs.current().buffer.text(), "first\nsecond edited");
    }

    #[test]
    fn opening_canonical_alias_activates_existing_edits_without_reading_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        fs::write(&path, "disk").unwrap();
        let mut docs = Documents::new(Document::open(&path).unwrap());
        let id = docs.active_id();
        docs.current_mut().buffer.insert_text("local ");
        fs::write(&path, [0xff]).unwrap();
        docs.open(&dir.path().join(".").join("note.md")).unwrap();
        assert_eq!(docs.active_id(), id);
        assert_eq!(docs.entries().len(), 1);
        assert_eq!(docs.current().buffer.text(), "local disk");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_aliases_deduplicate_and_cannot_bypass_save_collision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        let alias = dir.path().join("alias.md");
        fs::write(&path, "disk").unwrap();
        std::os::unix::fs::symlink(&path, &alias).unwrap();
        let mut docs = Documents::new(Document::untitled(""));
        let untitled = docs.active_id();
        docs.open(&path).unwrap();
        let file = docs.active_id();
        docs.current_mut().buffer.insert_text("local ");
        docs.select(untitled).unwrap();
        docs.current_mut().buffer.insert_text("replacement");
        assert!(docs.save(Some(&alias), true).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "disk");
        assert_eq!(docs.current().buffer.text(), "replacement");
        assert!(docs.current().path.is_none());
        docs.open(&alias).unwrap();
        assert_eq!(docs.active_id(), file);
        assert_eq!(docs.entries().len(), 2);
        assert_eq!(docs.current().buffer.text(), "local disk");
    }

    #[test]
    fn deleting_dirty_buffers_requires_force_and_last_buffer_gets_fresh_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("other.md");
        fs::write(&path, "other").unwrap();
        let mut docs = Documents::new(Document::untitled(""));
        let first = docs.active_id();
        docs.current_mut().buffer.insert_text("unsaved");
        docs.open(&path).unwrap();
        let second = docs.active_id();
        docs.select(first).unwrap();
        assert!(docs.delete(false).is_err());
        assert_eq!(docs.active_id(), first);
        assert_eq!(docs.current().buffer.text(), "unsaved");
        docs.delete(true).unwrap();
        assert_eq!(docs.active_id(), second);
        assert!(docs.select(first).is_err());
        assert_eq!(docs.active_id(), second);
        docs.current_mut().buffer.insert_text("unsaved ");
        assert!(docs.delete(false).is_err());
        docs.delete(true).unwrap();
        let fresh = docs.active_id();
        assert_ne!(fresh, first);
        assert_ne!(fresh, second);
        assert_eq!(docs.entries().len(), 1);
        assert!(docs.current().path.is_none());
        assert_eq!(docs.current().buffer.text(), "");
        assert!(!docs.dirty());
        docs.current_mut().buffer.undo();
        assert_eq!(docs.current().buffer.text(), "");
        docs.next();
        docs.previous();
        assert_eq!(docs.active_id(), fresh);
    }

    #[test]
    fn failed_io_preserves_active_identity_text_cursor_mode_viewport_and_undo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        let invalid = dir.path().join("invalid.md");
        fs::write(&path, "disk").unwrap();
        fs::write(&invalid, [0xff]).unwrap();
        let mut docs = Documents::new(Document::open(&path).unwrap());
        let id = docs.active_id();
        docs.current_mut().buffer.key("A");
        docs.current_mut().buffer.insert_text(" local");
        docs.viewport_mut().offset = 8.0;
        let col = docs.current().buffer.col;
        assert!(docs.open(&dir.path().join("missing.md")).is_err());
        assert!(docs.open(&invalid).is_err());
        assert!(docs.open(dir.path()).is_err());
        fs::write(&path, "external").unwrap();
        assert!(docs.reload(false).is_err());
        assert!(docs.save(None, false).is_err());
        assert!(
            docs.save(Some(&dir.path().join("missing").join("note.md")), true)
                .is_err()
        );
        fs::write(&path, [0xff]).unwrap();
        assert!(docs.reload(true).is_err());
        assert_eq!(docs.active_id(), id);
        assert_eq!(docs.entries().len(), 1);
        assert_eq!(
            docs.current().path.as_ref(),
            Some(&fs::canonicalize(&path).unwrap())
        );
        assert_eq!(docs.current().buffer.text(), "disk local");
        assert_eq!(docs.current().buffer.mode, Mode::Insert);
        assert_eq!(docs.current().buffer.col, col);
        assert_eq!(docs.viewport().offset, 8.0);
        docs.current_mut().buffer.undo();
        assert_eq!(docs.current().buffer.text(), "disk");
        fs::write(&path, "reloaded").unwrap();
        docs.reload(true).unwrap();
        assert_eq!(docs.active_id(), id);
        assert_eq!(docs.current().buffer.text(), "reloaded");
        assert!(!docs.dirty());
        docs.current_mut().buffer.undo();
        assert_eq!(docs.current().buffer.text(), "reloaded");
    }

    #[test]
    fn force_cannot_save_over_another_buffers_existing_or_removed_path() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.md");
        let second = dir.path().join("second.md");
        fs::write(&first, "first").unwrap();
        fs::write(&second, "second").unwrap();
        let mut docs = Documents::new(Document::open(&first).unwrap());
        let original = docs.active_id();
        docs.open(&second).unwrap();
        docs.current_mut().buffer.insert_text("local ");
        let active = docs.active_id();
        assert!(docs.save(Some(&first), true).is_err());
        assert_eq!(fs::read_to_string(&first).unwrap(), "first");
        fs::remove_file(&first).unwrap();
        assert!(docs.save(Some(&first), true).is_err());
        assert!(!first.exists());
        assert_eq!(docs.active_id(), active);
        assert_eq!(docs.current().buffer.text(), "local second");
        assert_eq!(
            docs.current().path.as_ref(),
            Some(&fs::canonicalize(&second).unwrap())
        );
        docs.select(original).unwrap();
        assert_eq!(docs.current().buffer.text(), "first");
    }

    #[test]
    fn reconcile_applies_directory_rename_cycles_and_deletions_to_all_original_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        for name in ["a", "b", "gone"] {
            fs::create_dir(root.join(name)).unwrap();
            fs::write(root.join(name).join("note.md"), name).unwrap();
        }
        let mut docs = Documents::new(Document::open(&root.join("a/note.md")).unwrap());
        let first = docs.active_id();
        docs.open(&root.join("b/note.md")).unwrap();
        let second = docs.active_id();
        docs.open(&root.join("gone/note.md")).unwrap();
        let deleted = docs.active_id();
        docs.current_mut().buffer.insert_text("local ");
        docs.reconcile(&CommitReport {
            renamed: vec![
                (root.join("a"), root.join("b")),
                (root.join("b"), root.join("a")),
            ],
            deleted: vec![root.join("gone")],
            created: vec![],
        });
        assert_eq!(docs.active_id(), deleted);
        assert!(docs.current().path.is_none());
        assert_eq!(docs.current().buffer.text(), "local gone");
        docs.current_mut().buffer.undo();
        assert_eq!(docs.current().buffer.text(), "gone");
        docs.select(first).unwrap();
        assert_eq!(docs.current().path.as_ref(), Some(&root.join("b/note.md")));
        assert_eq!(docs.current().buffer.text(), "a");
        docs.select(second).unwrap();
        assert_eq!(docs.current().path.as_ref(), Some(&root.join("a/note.md")));
        assert_eq!(docs.current().buffer.text(), "b");
    }
}
