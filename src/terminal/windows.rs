use super::*;
use std::{os::windows::io::AsRawHandle, sync::mpsc::TrySendError};
use windows_sys::Win32::{Foundation::ERROR_BROKEN_PIPE, System::IO::CancelSynchronousIo};

const INPUT_CHUNKS: usize = 32;

fn cancel_and_join(worker: JoinHandle<()>) {
    // Cancellation is not sticky: the worker may be between checking stop
    // and entering ReadFile/WriteFile. Retain its handle and retry until it
    // exits, rather than cancel once and race an unconditional join.
    while !worker.is_finished() {
        unsafe { CancelSynchronousIo(worker.as_raw_handle()) };
        worker.thread().unpark();
        thread::sleep(Duration::from_millis(1));
    }
    let _ = worker.join();
}

pub(super) struct Writer {
    input: Option<mpsc::SyncSender<Vec<u8>>>,
    errors: Receiver<io::Error>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Writer {
    pub(super) fn new(mut writer: Box<dyn Write + Send>) -> Result<Self> {
        let (input, receive) = mpsc::sync_channel::<Vec<u8>>(INPUT_CHUNKS);
        let (errors, receive_errors) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = thread::Builder::new()
            .name("rockdown-pty-input".into())
            .spawn(move || {
                while let Ok(bytes) = receive.recv() {
                    let mut offset = 0;
                    while offset < bytes.len() {
                        if worker_stop.load(Ordering::Acquire) {
                            return;
                        }
                        match writer.write(&bytes[offset..]) {
                            Ok(0) => {
                                let _ = errors.try_send(io::ErrorKind::WriteZero.into());
                                return;
                            }
                            Ok(count) => offset += count,
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                            Err(error) => {
                                let _ = errors.try_send(error);
                                return;
                            }
                        }
                    }
                }
            })
            .context("starting terminal writer thread")?;
        Ok(Self {
            input: Some(input),
            errors: receive_errors,
            stop,
            worker: Some(worker),
        })
    }
}

impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.flush()?;
        let count = bytes.len().min(READ_SIZE);
        if count == 0 {
            return Ok(0);
        }
        // At most INPUT_CHUNKS queued chunks plus one in-flight chunk,
        // independent of the parser's separately bounded input/reply queue.
        match self
            .input
            .as_ref()
            .unwrap()
            .try_send(bytes[..count].to_vec())
        {
            Ok(()) => Ok(count),
            Err(TrySendError::Full(_)) => Err(io::ErrorKind::WouldBlock.into()),
            Err(TrySendError::Disconnected(_)) => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.errors.try_recv() {
            Ok(error) => Err(error),
            Err(TryRecvError::Empty) => Ok(()),
            Err(TryRecvError::Disconnected) => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.input.take();
        if let Some(worker) = self.worker.take() {
            cancel_and_join(worker);
        }
    }
}

pub(super) struct Pty {
    pub(super) master: Option<Box<dyn MasterPty + Send>>,
    pub(super) output: Option<Receiver<io::Result<Vec<u8>>>>,
    reader_start: Option<mpsc::SyncSender<Box<dyn Read + Send>>>,
    close_start: Option<mpsc::SyncSender<Box<dyn MasterPty + Send>>>,
    reader: Option<JoinHandle<()>>,
    closer: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl Pty {
    pub(super) fn new() -> Result<Self> {
        let (sender, output) = mpsc::sync_channel(OUTPUT_CHUNKS);
        let (reader_start, receive_reader) = mpsc::sync_channel::<Box<dyn Read + Send>>(1);
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let reader = thread::Builder::new()
            .name("rockdown-pty-output".into())
            .spawn(move || {
                let Ok(mut reader) = receive_reader.recv() else {
                    return;
                };
                let mut bytes = [0; READ_SIZE];
                let mut deliver = true;
                while !worker_stop.load(Ordering::Acquire) {
                    match reader.read(&mut bytes) {
                        Ok(0) => break,
                        Ok(count) => {
                            if deliver {
                                deliver = sender.send(Ok(bytes[..count].to_vec())).is_ok();
                            }
                            // A disconnected UI means teardown, not stop:
                            // ClosePseudoConsole still needs a pipe drainer.
                        }
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Err(error) if error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => {
                            break;
                        }
                        Err(error) => {
                            let _ = sender.send(Err(error));
                            break;
                        }
                    }
                }
            })
            .context("starting terminal reader thread")?;
        let mut pty = Self {
            master: None,
            output: Some(output),
            reader_start: Some(reader_start),
            close_start: None,
            reader: Some(reader),
            closer: None,
            stop,
        };
        let (close_start, receive_master) = mpsc::sync_channel::<Box<dyn MasterPty + Send>>(1);
        pty.closer = Some(
            thread::Builder::new()
                .name("rockdown-pty-close".into())
                .spawn(move || {
                    if let Ok(master) = receive_master.recv() {
                        drop(master);
                    }
                })
                .context("starting terminal close thread")?,
        );
        pty.close_start = Some(close_start);
        Ok(pty)
    }

    pub(super) fn start_reader(&mut self, reader: Box<dyn Read + Send>) -> Result<()> {
        self.reader_start
            .take()
            .unwrap()
            .send(reader)
            .map_err(|_| anyhow::anyhow!("terminal reader thread stopped"))
    }

    pub(super) fn close(&mut self) {
        if let Some(master) = self.master.take() {
            // The prestarted closer receives exactly one console. The
            // capacity-one channel cannot block the UI.
            if let Some(sender) = self.close_start.take() {
                let _ = sender.send(master);
            }
        }
        self.close_start.take();
    }

    pub(super) fn shutdown(&mut self) {
        self.output.take();
        self.reader_start.take();
        self.close();
        if let Some(closer) = self.closer.take() {
            let _ = closer.join();
        }
        // Only cancel after closing ConPTY, never while it needs draining.
        self.stop.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            cancel_and_join(reader);
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs::File, os::windows::io::FromRawHandle};
    use windows_sys::Win32::System::Pipes::CreatePipe;

    fn pipe() -> (File, File) {
        let mut read = std::ptr::null_mut();
        let mut write = std::ptr::null_mut();
        assert_ne!(
            unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) },
            0
        );
        // Each successful CreatePipe handle is transferred to one owner.
        unsafe { (File::from_raw_handle(read), File::from_raw_handle(write)) }
    }

    #[test]
    fn full_input_queue_and_blocked_pipe_write_are_cancellable() {
        super::super::tests::windows_deadline(|| {
            for iteration in 0..16 {
                let (_read, write) = pipe();
                let mut writer = Writer::new(Box::new(write)).unwrap();
                let mut full = false;
                for _ in 0..INPUT_CHUNKS + 2 {
                    match writer.write(&[b'x'; READ_SIZE]) {
                        Ok(count) => assert_eq!(count, READ_SIZE),
                        Err(error) => {
                            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                            full = true;
                            break;
                        }
                    }
                }
                assert!(full, "writer queue was not bounded");
                if iteration % 2 == 0 {
                    thread::sleep(Duration::from_millis(10));
                }
                // Keep the unread pipe open: only cancellation, not EOF or
                // draining, can release a worker already inside WriteFile.
                drop(writer);
            }
        });
    }

    #[test]
    fn idle_pipe_read_is_cancellable_including_startup_races() {
        super::super::tests::windows_deadline(|| {
            for iteration in 0..16 {
                let (read, _write) = pipe();
                let mut pty = Pty::new().unwrap();
                pty.start_reader(Box::new(read)).unwrap();
                if iteration % 2 == 0 {
                    thread::sleep(Duration::from_millis(10));
                }
                drop(pty);
            }
        });
    }

    #[test]
    fn asynchronous_partial_writes_preserve_order_and_report_errors() {
        struct PartialWriter(mpsc::SyncSender<Vec<u8>>, usize);
        impl Write for PartialWriter {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.1 == 0 {
                    return Err(io::ErrorKind::PermissionDenied.into());
                }
                self.1 -= 1;
                let count = bytes.len().min(3);
                self.0.send(bytes[..count].to_vec()).unwrap();
                Ok(count)
            }

            fn flush(&mut self) -> io::Result<()> {
                panic!("the worker must not flush a synchronous pipe");
            }
        }

        super::super::tests::windows_deadline(|| {
            let (sent, received) = mpsc::sync_channel(2);
            let mut writer = Writer::new(Box::new(PartialWriter(sent, 2))).unwrap();
            assert_eq!(writer.write(b"abcdef!").unwrap(), 7);
            assert_eq!(received.recv().unwrap(), b"abc");
            assert_eq!(received.recv().unwrap(), b"def");
            loop {
                match writer.flush() {
                    Ok(()) => thread::sleep(Duration::from_millis(1)),
                    Err(error) => {
                        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                        break;
                    }
                }
            }
        });
    }
}
