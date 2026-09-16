use super::{Buffer, MAX_UNDO_DEPTH, Mode, Snapshot};

impl Buffer {
    fn push_undo(&mut self, snapshot: Snapshot) {
        if self.undo_stack.len() >= MAX_UNDO_DEPTH {
            self.undo_stack.remove(0);
        }
        self.undo_stack.push(snapshot);
    }

    fn push_redo(&mut self, snapshot: Snapshot) {
        if self.redo_stack.len() >= MAX_UNDO_DEPTH {
            self.redo_stack.remove(0);
        }
        self.redo_stack.push(snapshot);
    }

    pub fn undo(&mut self) {
        self.finish_insert();
        self.reset_command();
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        if let Some(previous) = self.undo_stack.pop() {
            let current = self.snapshot();
            self.restore(previous);
            self.push_redo(current);
        }
        self.clamp();
    }

    pub fn redo(&mut self) {
        self.finish_insert();
        self.reset_command();
        self.mode = Mode::Normal;
        self.visual_anchor = None;
        if let Some(next) = self.redo_stack.pop() {
            let current = self.snapshot();
            self.restore(next);
            self.push_undo(current);
        }
        self.clamp();
    }

    pub(super) fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor: self.position(),
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        if !self
            .lines
            .iter()
            .map(|line| line.text.as_str())
            .eq(snapshot.lines.iter().map(|line| line.text.as_str()))
        {
            self.bump_revision();
        }
        self.lines = snapshot.lines;
        self.row = snapshot.cursor.row;
        self.col = snapshot.cursor.col;
        self.preferred_column = None;
    }

    pub(super) fn record(&mut self, before: Snapshot) {
        if before.lines != self.lines {
            self.push_undo(before);
            self.redo_stack.clear();
        }
    }

    pub(super) fn ensure_insert_start(&mut self) {
        if self.insert_start.is_none() {
            self.insert_start = Some(self.snapshot());
        }
    }

    pub(super) fn finish_insert(&mut self) {
        if let Some(before) = self.insert_start.take() {
            self.record(before);
        }
    }
}
