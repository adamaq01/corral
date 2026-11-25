//! OpenSSH's client on a real pty, as a person runs it, with the screen it shows read through a
//! terminal emulator.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};

pub struct Pty {
    master: Box<dyn MasterPty + Send>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn Child + Send + Sync>,
    parser: Arc<Mutex<vt100::Parser>>,
    /// Every byte the terminal received.
    raw: Arc<Mutex<Vec<u8>>>,
}

fn size(rows: u16, cols: u16) -> PtySize {
    PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}

impl Pty {
    /// Runs `ssh` with `args` on a terminal of `rows` by `cols` that calls itself `term`.
    pub fn ssh(args: &[String], term: &str, rows: u16, cols: u16, home: &std::path::Path) -> Self {
        let pair = native_pty_system().openpty(size(rows, cols)).unwrap();
        let mut command = CommandBuilder::new("ssh");
        command.args(args);
        command.env("TERM", term);
        command.env("HOME", home);
        command.env_remove("SSH_AUTH_SOCK");
        command.env_remove("NO_COLOR");
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().unwrap();
        let writer = pair.master.take_writer().unwrap();
        let parser = Arc::new(Mutex::new(vt100::Parser::new(
            nonzero(rows),
            nonzero(cols),
            0,
        )));
        let raw = Arc::new(Mutex::new(Vec::new()));
        let (parsed, kept) = (parser.clone(), raw.clone());
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                lock(&parsed).process(&buffer[..read]);
                lock(&kept).extend_from_slice(&buffer[..read]);
            }
        });
        Self {
            master: pair.master,
            writer,
            child,
            parser,
            raw,
        }
    }

    /// Types `keys` as the terminal sends them.
    pub fn send(&mut self, keys: &[u8]) {
        self.writer.write_all(keys).unwrap();
        self.writer.flush().unwrap();
    }

    pub fn screen(&self) -> String {
        lock(&self.parser).screen().contents()
    }

    /// One row of the screen, as shown.
    pub fn row(&self, row: u16) -> String {
        let parser = lock(&self.parser);
        let cols = parser.screen().size().1.get();
        parser
            .screen()
            .rows(0, cols)
            .nth(usize::from(row))
            .unwrap_or_default()
    }

    /// Whether the alternate screen shows, as a full-screen program draws on.
    pub fn full_screen(&self) -> bool {
        lock(&self.parser).screen().alternate_screen()
    }

    pub fn raw(&self) -> String {
        String::from_utf8_lossy(&lock(&self.raw)).into_owned()
    }

    /// Whether any cell on the screen has a color of its own.
    pub fn colored(&self) -> bool {
        let parser = lock(&self.parser);
        let screen = parser.screen();
        let (rows, cols) = (screen.size().0.get(), screen.size().1.get());
        (0..rows).any(|row| {
            (0..cols).any(|col| {
                screen.cell(row, col).is_some_and(|cell| {
                    cell.fgcolor() != vt100::Color::Default
                        || cell.bgcolor() != vt100::Color::Default
                })
            })
        })
    }

    /// Waits until the screen shows `wanted`, and returns it.
    pub async fn until(&self, wanted: &str) -> String {
        let began = Instant::now();
        loop {
            let screen = self.screen();
            if screen.contains(wanted) {
                return screen;
            }
            assert!(
                began.elapsed() < Duration::from_secs(30),
                "no {wanted:?} on the screen:\n{screen}\nraw: {:?}",
                self.raw()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) {
        self.master.resize(size(rows, cols)).unwrap();
        lock(&self.parser).set_size(nonzero(rows), nonzero(cols));
    }

    /// Waits for ssh to exit, and returns its exit code.
    pub async fn exited(&mut self) -> u32 {
        let began = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status.exit_code();
            }
            assert!(
                began.elapsed() < Duration::from_secs(30),
                "ssh is still running:\n{}",
                self.screen()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

fn nonzero(n: u16) -> std::num::NonZeroU16 {
    std::num::NonZeroU16::new(n).expect("a terminal has rows and columns")
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Keys as a terminal sends them.
pub mod keys {
    pub const ENTER: &[u8] = b"\r";
    pub const ESC: &[u8] = b"\x1b";
    pub const UP: &[u8] = b"\x1b[A";
    pub const DOWN: &[u8] = b"\x1b[B";
    pub const LEFT: &[u8] = b"\x1b[D";
    pub const SPACE: &[u8] = b" ";
}
