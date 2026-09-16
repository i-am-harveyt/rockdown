use super::*;

#[cfg(windows)]
pub(super) fn windows_deadline(test: impl FnOnce() + Send + 'static) {
    let (done, completion) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        test();
        let _ = done.send(());
    });
    completion
        .recv_timeout(Duration::from_secs(20))
        .expect("Windows terminal operation blocked or panicked");
    worker.join().unwrap();
}

#[cfg(windows)]
fn windows_ready(terminal: &mut Terminal) {
    terminal
        .send(b"@echo off\rset RD_LEFT=ROCKDOWN_\rset RD_RIGHT=READY\recho %RD_LEFT%%RD_RIGHT%\r")
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !terminal.screen().contents().contains("ROCKDOWN_READY") {
        terminal.poll();
        assert!(
            std::time::Instant::now() < deadline,
            "cmd did not consume input: {:?}",
            terminal.error()
        );
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(windows)]
#[test]
fn windows_conpty_spawn_input_resize_and_exit_drain_final_output() {
    windows_deadline(|| {
        let mut terminal = Terminal::spawn(&std::env::temp_dir(), "cmd.exe", 24, 80).unwrap();
        windows_ready(&mut terminal);
        assert!(terminal.send(&vec![b'x'; INPUT_LIMIT + 1]).is_err());
        terminal.resize(31, 97).unwrap();
        assert_eq!(terminal.screen().size(), (31, 97));
        let size = terminal
            .windows
            .master
            .as_ref()
            .unwrap()
            .get_size()
            .unwrap();
        assert_eq!((size.rows, size.cols), (31, 97));
        assert!(terminal.resize(1, 32768).is_err());
        terminal.send(b"echo %RD_LEFT%FINAL\rexit\r").unwrap();
        while !terminal.exited() {
            terminal.poll();
            thread::sleep(Duration::from_millis(5));
        }
        assert!(terminal.screen().contents().contains("ROCKDOWN_FINAL"));
        assert!(terminal.send(b"late input").is_err());
    });
}

#[cfg(windows)]
#[test]
fn windows_conpty_idle_and_failed_spawn_teardown() {
    windows_deadline(|| {
        assert!(
            Terminal::spawn(
                &std::env::temp_dir(),
                "rockdown-nonexistent-shell.exe",
                24,
                80
            )
            .is_err()
        );
        for _ in 0..4 {
            // Also close before polling the initial cursor-inheritance
            // query. Closing input must release ConPTY's pending query.
            drop(Terminal::spawn(&std::env::temp_dir(), "cmd.exe", 24, 80).unwrap());
            let mut terminal = Terminal::spawn(&std::env::temp_dir(), "cmd.exe", 24, 80).unwrap();
            windows_ready(&mut terminal);
            drop(terminal);
        }
    });
}

#[cfg(windows)]
#[test]
fn windows_conpty_teardown_under_input_and_output_backpressure() {
    windows_deadline(|| {
        use std::os::windows::io::{AsRawHandle, BorrowedHandle};
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject,
        };

        let mut terminal = Terminal::spawn(&std::env::temp_dir(), "cmd.exe", 24, 80).unwrap();
        // Hold a separate process handle so teardown must demonstrably
        // terminate the shell, not merely release our handle to it.
        let process =
            unsafe { BorrowedHandle::borrow_raw(terminal.child.as_raw_handle().unwrap()) }
                .try_clone_to_owned()
                .unwrap();
        windows_ready(&mut terminal);
        terminal.send(b"cmd /d /q /c \"for /L %i in (1,1,1000000) do @echo ROCKDOWN_FLOOD_abcdefghijklmnopqrstuvwxyz0123456789\"\r").unwrap();
        while !terminal.screen().contents().contains("ROCKDOWN_FLOOD") {
            terminal.poll();
            thread::sleep(Duration::from_millis(5));
        }
        // Stop polling so both the bounded output channel and ConPTY's
        // output pipe fill. Input must remain bounded and nonblocking too.
        thread::sleep(Duration::from_millis(250));
        let mut full = false;
        for _ in 0..4096 {
            if let Err(error) = terminal.send(&[b'x'; READ_SIZE]) {
                assert!(error.to_string().contains("queue is full"), "{error:#}");
                full = true;
                break;
            }
        }
        assert!(full, "terminal input was not backpressured");
        drop(terminal);
        assert_eq!(
            unsafe { WaitForSingleObject(process.as_raw_handle(), 0) },
            WAIT_OBJECT_0
        );
    });
}

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
