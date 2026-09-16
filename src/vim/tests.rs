use super::{Buffer, MAX_UNDO_DEPTH, Mode, ViewportMotion};

mod editing;
mod motions;
mod selection;
mod state;

fn keys(buffer: &mut Buffer, keys: &[&str]) {
    for key in keys {
        assert!(buffer.key(key), "unconsumed key: {key}");
    }
}
