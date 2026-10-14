use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn init(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let _ = rotate_if_big(path);
    let file = OpenOptions::new().create(true).append(true).open(path).ok();
    *LOG.lock().expect("log lock") = file;
    write("info", "MemberLock starting");
}

fn rotate_if_big(path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.len() < 2_000_000 {
        return;
    }
    let mut old = path.to_path_buf();
    old.set_extension("log.old");
    let _ = fs::rename(path, old);
}

pub fn write(level: &str, message: &str) {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("{secs} {level} {message}\n");
    if let Ok(mut slot) = LOG.lock() {
        if let Some(file) = slot.as_mut() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }
    if cfg!(debug_assertions) {
        eprint!("{line}");
    }
}
