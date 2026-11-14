//! Лог в файл `bot.log` с ротацией: при превышении размера файл становится `bot.log.1`
//! (хранятся две старые копии).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

const KEEP: u32 = 2;

pub struct RotatingLogger {
    path: PathBuf,
    max_bytes: u64,
    file: Mutex<Option<File>>,
}

impl RotatingLogger {
    pub fn init(path: impl Into<PathBuf>, max_bytes: u64) {
        let logger = RotatingLogger { path: path.into(), max_bytes, file: Mutex::new(None) };
        let level = match std::env::var("RUST_LOG").unwrap_or_default().to_lowercase().as_str() {
            "debug" => LevelFilter::Debug,
            "trace" => LevelFilter::Trace,
            "warn" => LevelFilter::Warn,
            "error" => LevelFilter::Error,
            _ => LevelFilter::Info,
        };
        log::set_max_level(level);
        let _ = log::set_boxed_logger(Box::new(logger));
    }

    fn rotate(&self) {
        for i in (1..KEEP).rev() {
            let _ = fs::rename(self.numbered(i), self.numbered(i + 1));
        }
        let _ = fs::rename(&self.path, self.numbered(1));
    }

    fn numbered(&self, i: u32) -> PathBuf {
        let mut s = self.path.clone().into_os_string();
        s.push(format!(".{i}"));
        s.into()
    }
}

impl Log for RotatingLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        // Подробности библиотек нужны только при отладке
        m.target().starts_with("file_receiver") || m.level() <= Level::Warn || log::max_level() >= LevelFilter::Debug
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => " WARN",
            Level::Info => " INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };
        let target = record.target().split("::").next().unwrap_or("");
        let line = format!("{time} {level} {target} > {}\n", record.args());

        let mut guard = self.file.lock().unwrap();
        let too_big = fs::metadata(&self.path).map(|m| m.len() >= self.max_bytes).unwrap_or(false);
        if too_big {
            *guard = None;
            self.rotate();
        }
        if guard.is_none() {
            use std::os::unix::fs::OpenOptionsExt;
            *guard = OpenOptions::new().create(true).append(true).mode(0o600).open(&self.path).ok();
        }
        if let Some(f) = guard.as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(f) = self.file.lock().unwrap().as_mut() {
            let _ = f.flush();
        }
    }
}
