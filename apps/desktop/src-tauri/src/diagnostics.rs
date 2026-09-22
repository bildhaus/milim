use milim_core::paths::Paths;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

const LOG_LIMIT_BYTES: u64 = 5 * 1024 * 1024;
const MESSAGE_LIMIT_BYTES: usize = 2 * 1024;
const DETAIL_LIMIT_BYTES: usize = 8 * 1024;
const TRUNCATED: &str = "...[truncated]";

pub fn log_dir() -> PathBuf {
    Paths::resolve().root().join("logs")
}

pub fn init() -> std::result::Result<PathBuf, String> {
    let dir = log_dir();
    let writer = RotatingWriter::new(&dir, LOG_LIMIT_BYTES).map_err(|e| e.to_string())?;
    let filter = tracing_subscriber::filter::filter_fn(|metadata| {
        metadata.target().starts_with("milim_desktop")
            && matches!(
                *metadata.level(),
                tracing::Level::ERROR | tracing::Level::WARN | tracing::Level::INFO
            )
    });
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .with_filter(filter);
    tracing_subscriber::registry()
        .with(file_layer)
        .try_init()
        .map_err(|e| e.to_string())?;

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(location) = info.location() {
            tracing::error!(
                target: "milim_desktop::panic",
                file = location.file(),
                line = location.line(),
                column = location.column(),
                "desktop panic"
            );
        } else {
            tracing::error!(target: "milim_desktop::panic", "desktop panic");
        }
        previous(info);
    }));
    tracing::info!(target: "milim_desktop", "desktop diagnostics initialized");
    Ok(dir)
}

pub fn record_frontend_error(message: &str, detail: Option<&str>) {
    let message = capped(message, MESSAGE_LIMIT_BYTES);
    let detail = detail.map(|value| capped(value, DETAIL_LIMIT_BYTES));
    tracing::error!(
        target: "milim_desktop::renderer",
        message = %message,
        detail = detail.as_deref().unwrap_or(""),
        "frontend error"
    );
}

const STARTUP_ERROR_TITLE: &str = "milim could not start";

/// Record a fatal startup error, show it in a native alert, and exit.
///
/// Startup runs before Tauri owns a window, so the dialog plugin is not
/// available yet. The alert uses tools that ship with the OS, and the message
/// is passed as an argument or environment value rather than spliced into a
/// script.
pub fn fatal_startup_error(message: &str) -> ! {
    let message = capped(message, MESSAGE_LIMIT_BYTES);
    tracing::error!(target: "milim_desktop::startup", error = %message, "desktop startup failed");
    eprintln!("{STARTUP_ERROR_TITLE}: {message}");
    let body = format!(
        "{message}\n\nDetails are in the diagnostics log at {}.",
        log_dir().display()
    );
    show_native_alert(&body);
    std::process::exit(1);
}

fn show_native_alert(body: &str) {
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("osascript");
        command.args([
            "-e",
            "on run argv",
            "-e",
            &format!(
                "display alert \"{STARTUP_ERROR_TITLE}\" message (item 1 of argv) as critical"
            ),
            "-e",
            "end run",
            body,
        ]);
        command
    } else if cfg!(windows) {
        let mut command = std::process::Command::new("powershell.exe");
        command
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &format!(
                    "Add-Type -AssemblyName PresentationFramework; [void][System.Windows.MessageBox]::Show($env:MILIM_STARTUP_ERROR, '{STARTUP_ERROR_TITLE}', 'OK', 'Error')"
                ),
            ])
            .env("MILIM_STARTUP_ERROR", body);
        milim_core::proc::hide_console(&mut command);
        command
    } else {
        let mut command = std::process::Command::new("zenity");
        command.args(["--error", "--title", STARTUP_ERROR_TITLE, "--text", body]);
        command
    };
    if let Err(error) = command.status() {
        eprintln!("could not show the startup error dialog: {error}");
    }
}

fn capped(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_string();
    }
    if limit <= TRUNCATED.len() {
        let mut end = limit.min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        return value[..end].to_string();
    }
    let budget = limit.saturating_sub(TRUNCATED.len());
    let mut end = budget.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], TRUNCATED)
}

#[derive(Clone)]
struct RotatingWriter {
    state: Arc<Mutex<RotatingState>>,
}

struct RotatingState {
    current: PathBuf,
    previous: PathBuf,
    file: Option<File>,
    bytes: u64,
    limit: u64,
}

impl RotatingWriter {
    fn new(dir: &Path, limit: u64) -> io::Result<Self> {
        fs::create_dir_all(dir)?;
        let current = dir.join("desktop.log");
        let previous = dir.join("desktop.previous.log");
        let bytes = fs::metadata(&current).map(|value| value.len()).unwrap_or(0);
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&current)?;
        Ok(Self {
            state: Arc::new(Mutex::new(RotatingState {
                current,
                previous,
                file: Some(file),
                bytes,
                limit,
            })),
        })
    }
}

impl RotatingState {
    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            let _ = file.flush();
        }
        let rotation = (|| {
            match fs::remove_file(&self.previous) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if self.current.exists() {
                fs::rename(&self.current, &self.previous)?;
            }
            Ok(())
        })();
        self.file = Some(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.current)?,
        );
        self.bytes = fs::metadata(&self.current)
            .map(|value| value.len())
            .unwrap_or(0);
        rotation
    }
}

impl Write for RotatingWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("diagnostics log lock poisoned"))?;
        if state.bytes > 0 && state.bytes.saturating_add(buffer.len() as u64) > state.limit {
            state.rotate()?;
        }
        // ponytail: a single oversized tracing write is truncated; add structured chunking only if real logs need it.
        let allowed = buffer.len().min(state.limit as usize);
        state
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("diagnostics log unavailable"))?
            .write_all(&buffer[..allowed])?;
        state.bytes = state.bytes.saturating_add(allowed as u64);
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("diagnostics log lock poisoned"))?;
        state
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("diagnostics log unavailable"))?
            .flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "milim-diagnostics-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ))
    }

    #[test]
    fn diagnostics_rotation_keeps_two_bounded_files() {
        let dir = temp_dir("rotation");
        let mut writer = RotatingWriter::new(&dir, 16).expect("writer");
        writer.write_all(b"first-entry\n").expect("first");
        writer.write_all(b"second-entry\n").expect("second");
        writer.write_all(b"third-entry\n").expect("third");
        writer.flush().expect("flush");

        let files = fs::read_dir(&dir).expect("read logs").count();
        assert_eq!(files, 2);
        assert!(fs::metadata(dir.join("desktop.log")).unwrap().len() <= 16);
        assert!(
            fs::metadata(dir.join("desktop.previous.log"))
                .unwrap()
                .len()
                <= 16
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn diagnostics_frontend_fields_are_capped() {
        assert!(
            capped(&"x".repeat(MESSAGE_LIMIT_BYTES + 100), MESSAGE_LIMIT_BYTES).len()
                <= MESSAGE_LIMIT_BYTES
        );
        assert!(capped("hello עוÜÝ", 10).len() <= 10);
    }

    #[test]
    fn diagnostics_writer_creation_failure_is_recoverable() {
        let root = temp_dir("failure");
        fs::write(&root, b"not a directory").expect("fixture");
        assert!(RotatingWriter::new(&root.join("logs"), 16).is_err());
        let _ = fs::remove_file(root);
    }
}
