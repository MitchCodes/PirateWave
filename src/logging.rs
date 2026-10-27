//! Bounded asynchronous diagnostics. No callback or normal per-sample DSP logging.
//! Rare render-worker failure paths enqueue events; only the writer touches disk.
use std::{
    fmt,
    fs::{File, OpenOptions},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, OnceLock,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
const CAPACITY: usize = 1024;
const MAX_BYTES: u64 = 5 * 1024 * 1024;
static LOGGER: OnceLock<Logger> = OnceLock::new();
static DROPPED: AtomicU64 = AtomicU64::new(0);
struct Logger {
    tx: mpsc::SyncSender<Event>,
    path: PathBuf,
}
enum Event {
    Line {
        time: u128,
        level: &'static str,
        text: String,
    },
    Stop,
}
pub struct Guard {
    tx: mpsc::SyncSender<Event>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.tx.send(Event::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
pub fn path() -> PathBuf {
    LOGGER.get().map_or_else(
        || {
            crate::storage::default_path()
                .with_file_name("logs")
                .join("piratewave.log")
        },
        |l| l.path.clone(),
    )
}
pub fn init() -> io::Result<Guard> {
    let mut path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = match open_log(&path) {
        Ok(file) => file,
        Err(error) if writer_conflict(&error) => {
            // Native scanners/diagnostics can run beside the UI. Each writer owns
            // its file, so rotation cannot race another process's buffered output.
            path = path.with_file_name(format!("piratewave-{}.log", std::process::id()));
            open_log(&path)?
        }
        Err(error) => return Err(error),
    };
    let (tx, rx) = mpsc::sync_channel(CAPACITY);
    let writer_path = path.clone();
    let thread = thread::Builder::new()
        .name("piratewave-log-writer".into())
        .spawn(move || {
            if let Err(error) = write_loop(file, &writer_path, rx, MAX_BYTES) {
                eprintln!("PirateWave log writer failed: {error}");
            }
        })?;
    let _ = LOGGER.set(Logger {
        tx: tx.clone(),
        path,
    });
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic| {
        error(format_args!("Rust panic: {panic}"));
        previous(panic);
    }));
    Ok(Guard {
        tx,
        thread: Some(thread),
    })
}
pub fn info(args: fmt::Arguments<'_>) {
    emit("INFO", args);
}
pub fn error(args: fmt::Arguments<'_>) {
    emit("ERROR", args);
}
pub fn warn(args: fmt::Arguments<'_>) {
    emit("WARN", args);
}
fn emit(level: &'static str, args: fmt::Arguments<'_>) {
    if let Some(logger) = LOGGER.get() {
        let event = Event::Line {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            level,
            text: args.to_string(),
        };
        if logger.tx.try_send(event).is_err() {
            DROPPED.fetch_add(1, Ordering::Relaxed);
        }
    }
}
fn timestamp(ms: u128) -> String {
    let seconds = (ms / 1000) as i64;
    let z = seconds / 86400 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        ms % 1000
    )
}
fn writer_conflict(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied || matches!(error.raw_os_error(), Some(32 | 33))
}
fn open_log(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // Other readers are allowed; only this process writes/rotates.
    }
    options.open(path)
}
fn write_loop(
    file: File,
    path: &Path,
    rx: mpsc::Receiver<Event>,
    max_bytes: u64,
) -> io::Result<()> {
    let mut size = file.metadata()?.len();
    let mut writer = Some(BufWriter::new(file));
    loop {
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Event::Line { time, level, text }) => {
                if size >= max_bytes {
                    if let Some(mut old) = writer.take() {
                        old.flush()?;
                    }
                    for n in (1..=3).rev() {
                        let from = if n == 1 {
                            path.to_path_buf()
                        } else {
                            path.with_extension(format!("{}.log", n - 1))
                        };
                        let to = path.with_extension(format!("{n}.log"));
                        if to.exists() {
                            std::fs::remove_file(&to)?;
                        }
                        if from.exists() {
                            std::fs::rename(from, to)?;
                        }
                    }
                    writer = Some(BufWriter::new(open_log(path)?));
                    size = 0;
                }
                let dropped = DROPPED.swap(0, Ordering::Relaxed);
                let output = writer.as_mut().unwrap();
                if dropped > 0 {
                    writeln!(
                        output,
                        "{} [WARN] pid={} dropped {dropped} log events (queue full)",
                        timestamp(time),
                        std::process::id()
                    )?;
                }
                let clean = text.replace(['\r', '\n'], " ");
                let line = format!(
                    "{} [{level}] pid={} {clean}\n",
                    timestamp(time),
                    std::process::id()
                );
                output.write_all(line.as_bytes())?;
                size += line.len() as u64;
                if level == "ERROR" {
                    output.flush()?;
                }
            }
            Ok(Event::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(mut output) = writer.take() {
                    output.flush()?;
                }
                return Ok(());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(output) = &mut writer {
                    output.flush()?;
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_sharing_violations_select_a_process_specific_log() {
        assert!(writer_conflict(&io::Error::from_raw_os_error(32)));
        assert!(writer_conflict(&io::Error::from_raw_os_error(33)));
        assert!(!writer_conflict(&io::Error::new(
            io::ErrorKind::NotFound,
            "missing"
        )));
    }
    #[test]
    fn timestamp_and_writer_rotate_flush_and_preserve_errors() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(timestamp(1792540800123), "2026-10-21T00:00:00.123Z");
        let dir = std::env::temp_dir().join(format!("piratewave-log-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("piratewave.log");
        let file = File::create(&path).unwrap();
        let (tx, rx) = mpsc::sync_channel(10);
        for i in 0..5 {
            tx.send(Event::Line {
                time: 0,
                level: "ERROR",
                text: format!("failure {i}\nsecond line"),
            })
            .unwrap();
        }
        tx.send(Event::Stop).unwrap();
        write_loop(file, &path, rx, 100).unwrap();
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("failure 4 second line"));
        assert!(path.with_extension("1.log").exists());
        for name in [
            "piratewave.log",
            "piratewave.1.log",
            "piratewave.2.log",
            "piratewave.3.log",
        ] {
            let _ = std::fs::remove_file(dir.join(name));
        }
        std::fs::remove_dir(dir).unwrap();
    }
}
