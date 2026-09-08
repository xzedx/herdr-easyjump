//! Raw-mode terminal input and sizing via libc, no TUI framework.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

pub static INTERRUPTED: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_sig: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// Make SIGHUP/SIGTERM/SIGINT interrupt the blocking read so cleanup runs.
pub fn install_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = on_signal as extern "C" fn(libc::c_int) as *const () as usize;
        sa.sa_flags = 0; // no SA_RESTART: read() returns EINTR
        libc::sigemptyset(&mut sa.sa_mask);
        for sig in [libc::SIGHUP, libc::SIGTERM, libc::SIGINT] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
    }
}

pub struct RawMode {
    orig: libc::termios,
}

impl RawMode {
    pub fn enable() -> Option<Self> {
        unsafe {
            let mut orig: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(0, &mut orig) != 0 {
                return None;
            }
            let mut raw = orig;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(0, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            Some(RawMode { orig })
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        // TCSANOW, not TCSADRAIN: draining blocks forever when nobody reads
        // the pty master (test harnesses, a dying parent).
        unsafe {
            libc::tcsetattr(0, libc::TCSANOW, &self.orig);
        }
    }
}

pub fn size() -> (usize, usize) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            return (ws.ws_col as usize, ws.ws_row as usize);
        }
    }
    let env = |k: &str, d: usize| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    (env("COLUMNS", 120), env("LINES", 30))
}

#[derive(Debug, PartialEq)]
pub enum Key {
    Char(char),
    Esc,
    Enter,
    Backspace,
    CtrlC,
    Seq,
    Eof,
}

fn read_byte() -> Option<u8> {
    let mut b = [0u8; 1];
    loop {
        let n = unsafe { libc::read(0, b.as_mut_ptr() as *mut libc::c_void, 1) };
        if n == 1 {
            return Some(b[0]);
        }
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            if INTERRUPTED.load(Ordering::SeqCst) {
                return None;
            }
            continue;
        }
        return None;
    }
}

fn readable_within(ms: i32) -> bool {
    let mut pfd = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut pfd, 1, ms) > 0 }
}

/// Wait up to `ms` for a key; None on timeout.
pub fn read_key_timeout(ms: i32) -> Option<Key> {
    if !readable_within(ms) {
        return if INTERRUPTED.load(Ordering::SeqCst) {
            Some(Key::Eof)
        } else {
            None
        };
    }
    Some(read_key())
}

pub fn read_key() -> Key {
    let Some(b) = read_byte() else {
        return Key::Eof;
    };
    match b {
        0x1b => {
            // Swallow escape sequences (arrows etc.), treat a lone ESC as esc.
            if readable_within(30) {
                while readable_within(10) {
                    let mut buf = [0u8; 32];
                    unsafe { libc::read(0, buf.as_mut_ptr() as *mut libc::c_void, 32) };
                }
                Key::Seq
            } else {
                Key::Esc
            }
        }
        0x03 => Key::CtrlC,
        0x0d | 0x0a => Key::Enter,
        0x7f | 0x08 => Key::Backspace,
        b if b < 0x80 => Key::Char(b as char),
        _ => {
            // Multi-byte UTF-8: drain and ignore.
            while readable_within(0) {
                let mut buf = [0u8; 8];
                unsafe { libc::read(0, buf.as_mut_ptr() as *mut libc::c_void, 8) };
            }
            Key::Seq
        }
    }
}

pub fn draw(lines: &[String]) {
    let (cols, rows) = size();
    let mut out = String::from("\x1b[?25l\x1b[H");
    let frame: Vec<String> = lines
        .iter()
        .take(rows)
        .map(|l| crate::render::clip_ansi(l, cols))
        .collect();
    for (i, line) in frame.iter().enumerate() {
        out.push_str(line);
        out.push_str("\x1b[K");
        if i + 1 < frame.len() {
            out.push_str("\r\n");
        }
    }
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(out.as_bytes());
    let _ = so.flush();
}

pub fn enter_alt_screen() {
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(b"\x1b[?1049h");
    let _ = so.flush();
}

pub fn leave_alt_screen() {
    let mut so = std::io::stdout().lock();
    let _ = so.write_all(b"\x1b[?25h\x1b[?1049l");
    let _ = so.flush();
}
