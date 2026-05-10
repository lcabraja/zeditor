use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

pub fn next_operation_id() -> u64 {
    NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed)
}

pub fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

pub fn log_path() -> &'static PathBuf {
    static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
    LOG_PATH.get_or_init(|| {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Zeditor")
            .join("zeditor.log")
    })
}

pub fn event(scope: &str, details: impl AsRef<str>) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };

    let _ = writeln!(
        file,
        "{} thread={:?} scope={} {}",
        unix_millis(),
        std::thread::current().id(),
        scope,
        details.as_ref()
    );
}
