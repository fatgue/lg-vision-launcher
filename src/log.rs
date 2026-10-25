//! Minimal logger: stderr, plus /tmp/vision.log on the TV (where stderr is discarded).
use std::io::Write;
use std::sync::{Mutex, OnceLock};

static FILE: OnceLock<Mutex<std::fs::File>> = OnceLock::new();

pub fn init_file(path: &str) {
    if let Ok(file) = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(path) {
        let _ = FILE.set(Mutex::new(file));
    }
}

pub fn write(line: std::fmt::Arguments) {
    eprintln!("{line}");
    if let Some(file) = FILE.get() {
        if let Ok(mut file) = file.lock() {
            let _ = writeln!(file, "{line}");
        }
    }
}

macro_rules! log {
    ($($arg:tt)*) => { $crate::log::write(format_args!($($arg)*)) };
}
pub(crate) use log;
