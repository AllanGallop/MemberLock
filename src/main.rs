#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod firewall;
mod identity;
mod log;
mod rfid;
mod ruida;
mod state;
mod tray;
mod unifi;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::example_toml;
use crate::state::App;

fn main() {
    let (cfg, config_path) = match load_config() {
        Ok(loaded) => loaded,
        Err(err) => {
            message_box("MemberLock", &err.to_string());
            return;
        }
    };
    let log_path = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .join("memberlock.log");
    log::init(&log_path);
    log::write(
        "info",
        &format!(
            "config {} — {} at {}",
            config_path.display(),
            cfg.laser.name,
            cfg.laser.address
        ),
    );

    let app = Arc::new(App::new(cfg));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _guard = runtime.enter();

    firewall::ensure(&app.cfg);
    ruida::spawn(Arc::clone(&app), app.stop_rx.clone());
    rfid::spawn(Arc::clone(&app));

    let timer_app = Arc::clone(&app);
    runtime.spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let status = timer_app.status_snapshot();
            if status.unlocked {
                if let Some(until) = status.until {
                    if Instant::now() >= until {
                        log::write("info", "session timed out");
                        timer_app.lock("session ended — badge in to cut");
                        timer_app.balloon(
                            "Laser locked",
                            "The session timed out. Jog and the red-dot frame still work.",
                        );
                    }
                }
            }
        }
    });

    if let Err(err) = tray::run(Arc::clone(&app), &config_path, &log_path) {
        log::write("error", &format!("tray exited: {err}"));
    }
    runtime.shutdown_background();
}

fn load_config() -> anyhow::Result<(crate::config::Config, PathBuf)> {
    let path = config_path();
    if !path.exists() {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(&path, example_toml())?;
    }
    let text = fs::read_to_string(&path)?;
    let cfg = crate::config::Config::from_toml(&text)?;
    Ok((cfg, path))
}

fn config_path() -> PathBuf {
    if let Ok(cwd) = std::env::current_dir() {
        let here = cwd.join("config.toml");
        if here.exists() {
            return here;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let beside = dir.join("config.toml");
            if beside.exists() {
                return beside;
            }
        }
    }
    appdata_config()
}

fn appdata_config() -> PathBuf {
    if let Ok(root) = std::env::var("APPDATA") {
        return PathBuf::from(root).join("MemberLock").join("config.toml");
    }
    PathBuf::from("config.toml")
}

fn message_box(title: &str, body: &str) {
    use windows::core::PCWSTR;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR};
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let body: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = MessageBoxW(
            None,
            PCWSTR(body.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_ICONERROR,
        );
    }
}
