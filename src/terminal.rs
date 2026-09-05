use std::{
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use unicode_segmentation::UnicodeSegmentation;

const READ_SIZE: usize = 8192;
const OUTPUT_CHUNKS: usize = 32;
const INPUT_LIMIT: usize = 1024 * 1024;
const REPLY_RESERVE: usize = READ_SIZE * 8;
const OSC_LIMIT: usize = 16 * 1024;

/// The reader applies backpressure instead of dropping terminal output. Both
/// master handles are nonblocking, so teardown never depends on a descendant
/// closing its inherited slave descriptor.
pub struct Terminal {
    master: Option<Box<dyn MasterPty + Send>>,
    writer: Option<Box<dyn Write + Send>>,
    child: Box<dyn Child + Send + Sync>,
    reader: Option<JoinHandle<()>>,
    output: Option<Receiver<io::Result<Vec<u8>>>>,
    stop: Arc<AtomicBool>,
    parser: vt100::Parser<Responses>,
    input_offset: usize,
    child_exited: bool,
    output_closed: bool,
    error: Option<String>,
    osc_len: Option<usize>,
    escaped: bool,
    dropping_osc: bool,
}

#[derive(Default)]
struct Responses {
    pending: Vec<u8>,
}

impl vt100::Callbacks for Responses {
    fn unhandled_csi(
        &mut self,
        screen: &mut vt100::Screen,
        intermediate: Option<u8>,
        second: Option<u8>,
        params: &[&[u16]],
        command: char,
    ) {
        if second.is_some() || params.len() > 1 || params.iter().any(|p| p.len() > 1) {
            return;
        }
        let parameter = params.first().and_then(|p| p.first()).copied().unwrap_or(0);
        match (intermediate, parameter, command) {
            (None, 5, 'n') => self.pending.extend_from_slice(b"\x1b[0n"),
            (None | Some(b'?'), 6, 'n') => {
                let (row, col) = screen.cursor_position();
                let private = if intermediate.is_some() { "?" } else { "" };
                // Vec's Write implementation cannot fail.
                let _ = write!(self.pending, "\x1b[{private}{};{}R", row + 1, col + 1);
            }
            (None, 0, 'c') => self.pending.extend_from_slice(b"\x1b[?1;2c"),
            (Some(b'>'), 0, 'c') => self.pending.extend_from_slice(b"\x1b[>0;1;0c"),
            (None, 18, 't') => {
                let (rows, cols) = screen.size();
                let _ = write!(self.pending, "\x1b[8;{rows};{cols}t");
            }
            _ => {}
        }
    }

    fn unhandled_escape(
        &mut self,
        _: &mut vt100::Screen,
        first: Option<u8>,
        second: Option<u8>,
        byte: u8,
    ) {
        if first.is_none() && second.is_none() && byte == b'Z' {
            self.pending.extend_from_slice(b"\x1b[?1;2c");
        }
    }
}

fn pty_size(rows: u16, cols: u16) -> Result<PtySize> {
    ensure!(rows > 0 && cols > 0, "terminal dimensions must be nonzero");
    ensure!(
        u32::from(rows) * u32::from(cols) <= 1_048_576,
        "terminal dimensions are too large"
    );
    Ok(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    })
}

impl Terminal {
    pub fn spawn(cwd: &Path, shell: &str, rows: u16, cols: u16) -> Result<Self> {
        ensure!(!shell.is_empty(), "terminal shell is empty");
        let pair = native_pty_system()
            .openpty(pty_size(rows, cols)?)
            .context("opening terminal PTY")?;
        set_nonblocking(pair.master.as_ref())?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .context("opening terminal reader")?;
        let writer = pair
            .master
            .take_writer()
            .context("opening terminal writer")?;
        let mut command = CommandBuilder::new(shell);
        command.cwd(cwd);
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "Rockdown");
        command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        // Inherited overrides otherwise defeat TIOCSWINSZ in many applications.
        command.env_remove("LINES");
        command.env_remove("COLUMNS");
        command.env_remove("TERMINFO");
        command.env_remove("TERMINFO_DIRS");
        let child = pair
            .slave
            .spawn_command(command)
            .with_context(|| format!("starting terminal shell {shell}"))?;
        drop(pair.slave);
        let (sender, output) = mpsc::sync_channel(OUTPUT_CHUNKS);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        // Construct the owner before spawning the worker: any subsequent error
        // must still terminate and reap the already-started shell.
        let mut terminal = Self {
            master: Some(pair.master),
            writer: Some(writer),
            child,
            reader: None,
            output: Some(output),
            stop,
            parser: vt100::Parser::new_with_callbacks(rows, cols, 2000, Responses::default()),
            input_offset: 0,
            child_exited: false,
            output_closed: false,
            error: None,
            osc_len: None,
            escaped: false,
            dropping_osc: false,
        };
        terminal.reader = Some(
            thread::Builder::new()
                .name("rockdown-pty".into())
                .spawn(move || {
                    let mut bytes = [0_u8; READ_SIZE];
                    while !worker_stop.load(Ordering::Acquire) {
                        match reader.read(&mut bytes) {
                            Ok(0) => break,
                            Ok(count) => {
                                if sender.send(Ok(bytes[..count].to_vec())).is_err() {
                                    break;
                                }
                            }
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                thread::park_timeout(Duration::from_millis(5));
                            }
                            Err(error) => {
                                let _ = sender.send(Err(error));
                                break;
                            }
                        }
                    }
                })
                .context("starting terminal reader thread")?,
        );
        Ok(terminal)
    }

    /// Enqueues input atomically. A full queue returns an explicit error rather
    /// than blocking the UI or silently injecting only part of a paste.
    pub fn send(&mut self, bytes: &[u8]) -> Result<()> {
        if let Some(error) = &self.error {
            bail!("terminal I/O failed: {error}");
        }
        ensure!(
            !self.child_exited && !self.output_closed,
            "terminal shell has exited"
        );
        self.flush_input()?;
        let pending = &mut self.parser.callbacks_mut().pending;
        ensure!(
            bytes.len() <= INPUT_LIMIT - (pending.len() - self.input_offset),
            "terminal input queue is full; wait for the shell to consume input"
        );
        if self.input_offset != 0 {
            pending.drain(..self.input_offset);
            self.input_offset = 0;
        }
        pending.extend_from_slice(bytes);
        self.flush_input()
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> Result<()> {
        let size = pty_size(rows, cols)?;
        if self.parser.screen().size() != (rows, cols) {
            self.master
                .as_ref()
                .context("terminal PTY is closed")?
                .resize(size)
                .context("resizing terminal PTY")?;
            self.parser.screen_mut().set_size(rows, cols);
        }
        Ok(())
    }

    fn flush_input(&mut self) -> Result<()> {
        let Some(writer) = &mut self.writer else {
            return Ok(());
        };
        let pending = &mut self.parser.callbacks_mut().pending;
        // Bound write work as well as read work per UI frame.
        for _ in 0..16 {
            if self.input_offset == pending.len() {
                pending.clear();
                self.input_offset = 0;
                break;
            }
            let end = pending.len().min(self.input_offset + READ_SIZE);
            match writer.write(&pending[self.input_offset..end]) {
                Ok(0) => bail!("terminal writer closed"),
                Ok(count) => self.input_offset += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error).context("writing terminal input"),
            }
        }
        Ok(())
    }

    /// Nonblocking, bounded work; returns true when visible state/status changed.
    /// Keep polling until exited(), including after the shell process terminates,
    /// so the last buffered output is not lost.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Err(error) = self.flush_input() {
            changed |= self.record_error(error.to_string());
        }
        if !self.child_exited {
            match self.child.try_wait() {
                Ok(Some(_)) => {
                    self.child_exited = true;
                    changed = true;
                }
                Ok(None) => {}
                Err(error) => {
                    changed |= self.record_error(format!("waiting for terminal shell: {error}"))
                }
            }
        }
        for _ in 0..16 {
            // Every possible response from one chunk fits this reservation.
            // Backpressure pauses parsing, not the UI, and preserves replies.
            if self.parser.callbacks().pending.len() > INPUT_LIMIT - REPLY_RESERVE {
                break;
            }
            let Some(output) = &self.output else { break };
            match output.try_recv() {
                Ok(Ok(bytes)) => {
                    self.process_output(&bytes);
                    changed = true;
                    if let Err(error) = self.flush_input() {
                        changed |= self.record_error(error.to_string());
                    }
                }
                Ok(Err(error)) => {
                    changed |= self.record_error(format!("reading terminal output: {error}"))
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    changed |= !self.output_closed;
                    self.output_closed = true;
                    self.output.take();
                    break;
                }
            }
        }
        changed
    }

    fn record_error(&mut self, error: String) -> bool {
        if self.error.is_some() {
            return false;
        }
        self.error = Some(error);
        // Continuing to buffer replies for a broken writer would stop output
        // draining forever. Preserve the failure for the GUI, but drain output.
        self.writer.take();
        self.parser.callbacks_mut().pending.clear();
        self.input_offset = 0;
        true
    }

    fn process_output(&mut self, bytes: &[u8]) {
        // vte's std-enabled OSC buffer is otherwise unbounded. Abort oversized
        // control strings and discard their payload through the terminator,
        // without treating the remaining payload as visible terminal text.
        let mut start = 0;
        for (index, &byte) in bytes.iter().enumerate() {
            let terminates = matches!(byte, 7 | 0x18 | 0x1a | 0x1b);
            if self.dropping_osc {
                start = index + 1;
                if terminates {
                    self.dropping_osc = false;
                    self.escaped = byte == 0x1b;
                    if self.escaped {
                        start = index;
                    }
                }
                continue;
            }
            if let Some(length) = self.osc_len.as_mut() {
                if terminates {
                    self.osc_len = None;
                } else {
                    *length += 1;
                    if *length > OSC_LIMIT {
                        self.parser.process(&bytes[start..index]);
                        self.parser.process(b"\x18");
                        self.osc_len = None;
                        self.dropping_osc = true;
                        start = index + 1;
                    }
                }
            } else if self.escaped && byte == b']' {
                self.osc_len = Some(0);
            }
            self.escaped = byte == 0x1b;
        }
        self.parser.process(&bytes[start..]);
        if self.writer.is_none() {
            self.parser.callbacks_mut().pending.clear();
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.parser.screen()
    }

    pub fn exited(&self) -> bool {
        self.child_exited && self.output_closed
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(unix)]
fn set_nonblocking(master: &dyn MasterPty) -> Result<()> {
    let fd = master
        .as_raw_fd()
        .context("PTY does not expose a native descriptor")?;
    // Duplicated PTY handles share this open-file description and its flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error()).context("making terminal I/O nonblocking");
    }
    Ok(())
}

#[cfg(not(unix))]
fn set_nonblocking(_: &dyn MasterPty) -> Result<()> {
    bail!("the embedded terminal currently requires a Unix PTY")
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Dropping the receiver releases a reader blocked by backpressure.
        self.output.take();
        if let Some(reader) = self.reader.take() {
            reader.thread().unpark();
            let _ = reader.join();
        }
        #[cfg(unix)]
        if let Some(group) = self
            .master
            .as_ref()
            .and_then(|master| master.process_group_leader())
        {
            // Only signal the foreground group belonging to this PTY, never
            // our own group. This also closes a running foreground program.
            if group > 0 && group != unsafe { libc::getpgrp() } {
                unsafe {
                    libc::kill(-group, libc::SIGKILL);
                }
            }
        }
        // macOS can leave a dying foreground shell in wait4 until the final
        // master descriptor closes; field drop order would close it too late.
        self.writer.take();
        self.master.take();
        if !self.child_exited {
            // portable-pty starts with SIGHUP, which an interactive shell can
            // defer while waiting on a foreground job. Teardown must not wait
            // for shell traps or programs that ignore hangup.
            #[cfg(unix)]
            if let Some(pid) = self.child.process_id() {
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGKILL);
                }
            }
            #[cfg(not(unix))]
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Encode GPUI key names as xterm input. Printable Unicode is kept as UTF-8;
/// named non-text keys are never accidentally sent as their names.
pub fn key_bytes(key: &str, ctrl: bool, alt: bool, application_cursor: bool) -> Option<Vec<u8>> {
    let fixed: Option<&[u8]> = match key {
        "enter" | "return" => Some(b"\r"),
        "backspace" if ctrl => Some(b"\x08"),
        "backspace" => Some(b"\x7f"),
        "tab" => Some(b"\t"),
        "backtab" | "shift-tab" => Some(b"\x1b[Z"),
        "escape" | "esc" => Some(b"\x1b"),
        "delete" => Some(if ctrl { b"\x1b[3;5~" } else { b"\x1b[3~" }),
        "insert" => Some(b"\x1b[2~"),
        "pageup" => Some(b"\x1b[5~"),
        "pagedown" => Some(b"\x1b[6~"),
        "f1" => Some(b"\x1bOP"),
        "f2" => Some(b"\x1bOQ"),
        "f3" => Some(b"\x1bOR"),
        "f4" => Some(b"\x1bOS"),
        "f5" => Some(b"\x1b[15~"),
        "f6" => Some(b"\x1b[17~"),
        "f7" => Some(b"\x1b[18~"),
        "f8" => Some(b"\x1b[19~"),
        "f9" => Some(b"\x1b[20~"),
        "f10" => Some(b"\x1b[21~"),
        "f11" => Some(b"\x1b[23~"),
        "f12" => Some(b"\x1b[24~"),
        _ => None,
    };
    let mut result = Vec::with_capacity(key.len().max(8) + usize::from(alt));
    if alt {
        result.push(0x1b);
    }
    if let Some(bytes) = fixed {
        result.extend_from_slice(bytes);
        return Some(result);
    }
    let cursor = match key {
        "up" => Some(b'A'),
        "down" => Some(b'B'),
        "right" => Some(b'C'),
        "left" => Some(b'D'),
        "home" => Some(b'H'),
        "end" => Some(b'F'),
        _ => None,
    };
    if let Some(suffix) = cursor {
        result.extend_from_slice(if ctrl {
            b"\x1b[1;5"
        } else if application_cursor {
            b"\x1bO"
        } else {
            b"\x1b["
        });
        result.push(suffix);
        return Some(result);
    }
    let text = if key == "space" { " " } else { key };
    let mut graphemes = text.graphemes(true);
    let grapheme = graphemes.next()?;
    if graphemes.next().is_some() {
        return None;
    }
    let character = grapheme.chars().next()?;
    if ctrl {
        if !grapheme.is_ascii() {
            return None;
        }
        let control = match character {
            'a'..='z' => character as u8 - b'a' + 1,
            '@'..='_' => character as u8 & 0x1f,
            ' ' | '2' => 0,
            '3' => 0x1b,
            '4' => 0x1c,
            '5' => 0x1d,
            '6' => 0x1e,
            '7' | '/' => 0x1f,
            '8' | '?' => 0x7f,
            _ => return None,
        };
        result.push(control);
    } else {
        result.extend_from_slice(text.as_bytes());
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_alt_and_application_keys_preserve_terminal_protocol() {
        assert_eq!(key_bytes("c", true, false, false), Some(vec![3]));
        assert_eq!(key_bytes("[", true, false, false), Some(vec![27]));
        assert_eq!(key_bytes("space", true, true, false), Some(vec![27, 0]));
        assert_eq!(
            key_bytes("é", false, true, false),
            Some(b"\x1b\xc3\xa9".to_vec())
        );
        assert_eq!(
            key_bytes("e\u{301}", false, false, false),
            Some("e\u{301}".as_bytes().to_vec())
        );
        assert_eq!(
            key_bytes("up", false, false, true),
            Some(b"\x1bOA".to_vec())
        );
        assert_eq!(
            key_bytes("left", true, false, true),
            Some(b"\x1b[1;5D".to_vec())
        );
        assert_eq!(key_bytes("shift", false, false, false), None);
    }

    #[test]
    fn split_queries_reply_at_the_cursor_when_received() {
        let mut parser = vt100::Parser::new_with_callbacks(24, 80, 0, Responses::default());
        parser.process(b"\x1b[4;9H\x1b[");
        parser.process(b"6n\x1b[2;3H\x1b[?6n\x1b[5n");
        assert_eq!(parser.callbacks().pending, b"\x1b[4;9R\x1b[?2;3R\x1b[0n");
    }

    #[cfg(unix)]
    #[test]
    fn closing_a_foreground_job_closes_the_pty_before_reaping() {
        let mut terminal = Terminal::spawn(&std::env::temp_dir(), "/bin/sh", 24, 80).unwrap();
        let pid = terminal.child.process_id().unwrap() as libc::pid_t;
        terminal.send(b"/bin/sleep 60\n").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            terminal.poll();
            if terminal
                .master
                .as_ref()
                .and_then(|master| master.process_group_leader())
                .is_some_and(|group| group != pid)
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "shell did not start a foreground job"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let (done, completion) = mpsc::sync_channel(1);
        let closer = thread::spawn(move || {
            drop(terminal);
            done.send(()).unwrap();
        });
        completion
            .recv_timeout(Duration::from_secs(3))
            .expect("PTY teardown blocked");
        closer.join().unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1, "shell was not reaped");
        assert_eq!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH));
    }
}
