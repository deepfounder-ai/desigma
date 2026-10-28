//! Crash reports. Every panic is appended to `crash.log` in the per-user data folder, so a user
//! whose window just vanished (no console on Windows) has something to attach to a bug report.

use std::any::Any;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The log is trimmed to its newest reports past this size.
const MAX_LOG_BYTES: u64 = 512 * 1024;

pub fn log_path() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("desigma").join("crash.log")
}

/// Chains a hook that writes the report before the default one prints it.
pub fn install() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()));
        let text = report(&payload_message(info.payload()), location.as_deref(), &std::backtrace::Backtrace::force_capture().to_string());
        write_report(&log_path(), &text);
        prev(info);
    }));
}

/// The message a panic was raised with, for a hook payload or a `catch_unwind` error.
pub fn payload_message(payload: &(dyn Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "unknown panic".to_string())
}

fn report(message: &str, location: Option<&str>, backtrace: &str) -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let thread = std::thread::current();
    format!(
        "=== desigma {} crash (unix time {secs}) ===\nos: {} {}\nthread: {}\npanic: {message}\nat: {}\nbacktrace:\n{backtrace}\n",
        crate::VERSION,
        std::env::consts::OS,
        std::env::consts::ARCH,
        thread.name().unwrap_or("unnamed"),
        location.unwrap_or("unknown"),
    )
}

/// Appends `text`, keeping the file under [`MAX_LOG_BYTES`]. Never panics: this runs inside the
/// panic hook.
fn write_report(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(path).is_ok_and(|m| m.len() + text.len() as u64 > MAX_LOG_BYTES) {
        let _ = std::fs::write(path, text);
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_messages_are_readable() {
        let err = std::panic::catch_unwind(|| panic!("boom {}", 7)).unwrap_err();
        assert_eq!(payload_message(err.as_ref()), "boom 7");
        let err = std::panic::catch_unwind(|| std::panic::panic_any(5u8)).unwrap_err();
        assert_eq!(payload_message(err.as_ref()), "unknown panic");
    }

    #[test]
    fn reports_append_and_stay_bounded() {
        let dir = std::env::temp_dir().join(format!("desigma_crash_test_{}", std::process::id()));
        let path = dir.join("crash.log");
        let first = report("first", Some("a.rs:1:1"), "bt");
        assert!(first.contains("panic: first") && first.contains("at: a.rs:1:1") && first.contains(crate::VERSION));
        write_report(&path, &first);
        write_report(&path, &report("second", None, "bt"));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("panic: first") && text.contains("panic: second"));
        // A report that would push the log past the cap starts it over.
        write_report(&path, &"x".repeat(MAX_LOG_BYTES as usize));
        assert!(!std::fs::read_to_string(&path).unwrap().contains("panic: first"));
        std::fs::remove_dir_all(dir).ok();
    }
}
