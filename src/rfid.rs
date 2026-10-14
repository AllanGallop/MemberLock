use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::runtime::Handle;
use tokio::sync::RwLock;

use crate::identity::{Directory, Grant};
use crate::log;
use crate::state::App;
use crate::unifi;

struct Cache {
    directory: Directory,
    fetched_at: Option<Instant>,
    last_error: Option<String>,
}

pub fn spawn(app: Arc<App>) {
    let cache = Arc::new(RwLock::new(Cache {
        directory: Directory::default(),
        fetched_at: None,
        last_error: None,
    }));
    let refresher = Arc::clone(&app);
    let refresh_cache = Arc::clone(&cache);
    tokio::spawn(async move {
        loop {
            refresh(&refresher, &refresh_cache).await;
            tokio::time::sleep(refresher.cfg.unifi.refresh).await;
        }
    });

    let handle = Handle::current();
    std::thread::spawn(move || reader_loop(app, cache, handle));
}

async fn refresh(app: &App, cache: &RwLock<Cache>) {
    match unifi::fetch_directory(&app.cfg.unifi).await {
        Ok(directory) => {
            let n = directory.len();
            log::write("info", &format!("UniFi Access directory has {n} users"));
            let mut guard = cache.write().await;
            guard.directory = directory;
            guard.fetched_at = Some(Instant::now());
            guard.last_error = None;
            if !app.is_unlocked() {
                app.set_auth_note(format!("{n} members loaded — badge in to cut"));
            }
        }
        Err(err) => {
            let msg = err.to_string();
            log::write("error", &format!("UniFi Access refresh failed: {msg}"));
            cache.write().await.last_error = Some(msg);
            if !app.is_unlocked() {
                app.set_auth_note("UniFi Access unreachable — laser stays locked");
            }
        }
    }
}

fn reader_loop(app: Arc<App>, cache: Arc<RwLock<Cache>>, handle: Handle) {
    let mut announced = String::new();
    loop {
        let port = match resolve_port(&app.cfg.rfid.port) {
            Ok(port) => port,
            Err(err) => {
                log::write("error", &err);
                if err != announced {
                    app.balloon("RFID reader", &err);
                    announced = err;
                }
                std::thread::sleep(Duration::from_secs(5));
                continue;
            }
        };
        announced.clear();
        log::write(
            "info",
            &format!("RFID reader {port} at {} baud", app.cfg.rfid.baud),
        );
        if let Err(err) = read_port(&app, &cache, &handle, &port) {
            log::write("error", &format!("RFID {port} closed: {err}"));
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

fn resolve_port(configured: &str) -> Result<String, String> {
    if !configured.trim().is_empty() && !configured.eq_ignore_ascii_case("auto") {
        return Ok(configured.to_string());
    }
    let ports = serialport::available_ports().map_err(|err| err.to_string())?;
    let mut usb = Vec::new();
    for port in ports {
        let name = port.port_name;
        let usb_port = matches!(port.port_type, serialport::SerialPortType::UsbPort(_));
        let looks_serial = name.to_ascii_uppercase().starts_with("COM");
        if usb_port || looks_serial {
            usb.push(name);
        }
    }
    match usb.len() {
        0 => Err("No USB serial RFID reader found. Set rfid.port in config.toml.".into()),
        1 => Ok(usb.remove(0)),
        _ => Err(format!(
            "Several serial ports ({}) — set rfid.port to the reader.",
            usb.join(", ")
        )),
    }
}

fn read_port(
    app: &Arc<App>,
    cache: &Arc<RwLock<Cache>>,
    handle: &Handle,
    port_name: &str,
) -> Result<(), String> {
    let mut port = serialport::new(port_name, app.cfg.rfid.baud)
        .timeout(Duration::from_millis(400))
        .open()
        .map_err(|err| err.to_string())?;
    let mut pending = Vec::new();
    let mut buf = [0u8; 128];
    let mut last_token = String::new();
    let mut last_at = Instant::now() - Duration::from_secs(10);

    loop {
        match port.read(&mut buf) {
            Ok(0) => {}
            Ok(n) => {
                pending.extend_from_slice(&buf[..n]);
                while let Some(split) = pending.iter().position(|b| *b == b'\n' || *b == b'\r') {
                    let line: Vec<u8> = pending.drain(..=split).collect();
                    let text = String::from_utf8_lossy(&line);
                    let Some(token) = crate::identity::extract_token(&text) else {
                        continue;
                    };
                    if token == last_token && last_at.elapsed() < Duration::from_secs(2) {
                        continue;
                    }
                    last_token.clone_from(&token);
                    last_at = Instant::now();
                    let app = Arc::clone(app);
                    let cache = Arc::clone(cache);
                    let token = token.clone();
                    handle.spawn(async move {
                        on_scan(app, cache, &token).await;
                    });
                }
                if pending.len() > 512 {
                    pending.clear();
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::TimedOut => {}
            Err(err) => return Err(err.to_string()),
        }
    }
}

async fn on_scan(app: Arc<App>, cache: Arc<RwLock<Cache>>, token: &str) {
    log::write("info", &format!("card scanned ({})", mask(token)));
    if cache.read().await.fetched_at.map(|t| t.elapsed() > app.cfg.unifi.refresh).unwrap_or(true) {
        refresh(&app, &cache).await;
    }

    let guard = cache.read().await;
    let fresh = guard
        .fetched_at
        .map(|t| t.elapsed() <= app.cfg.unifi.max_stale)
        .unwrap_or(false);
    if !fresh || guard.directory.is_empty() {
        let why = guard
            .last_error
            .clone()
            .unwrap_or_else(|| "UniFi Access has not returned a member list.".into());
        drop(guard);
        log::write("error", &format!("scan refused, directory unavailable: {why}"));
        app.balloon("Laser stays locked", "UniFi Access could not be checked.");
        return;
    }

    match guard.directory.authorize(token, &app.cfg.unifi.policy) {
        Ok(grant) => {
            drop(guard);
            apply_grant(&app, grant);
        }
        Err(deny) => {
            drop(guard);
            let message = deny.message(&app.cfg.unifi.policy);
            log::write("info", &format!("card refused: {message}"));
            app.balloon("Not authorized", &message);
        }
    }
}

fn apply_grant(app: &App, grant: Grant) {
    let status = app.status_snapshot();
    if status.unlocked && status.holder_id == grant.id {
        log::write("info", &format!("{} signed out", grant.name));
        app.lock(format!("{} signed out — badge in to cut", grant.name));
        app.balloon("Laser locked", "Session ended. The tube is off.");
        return;
    }
    let minutes = app.cfg.session.minutes;
    log::write(
        "info",
        &format!("laser enabled for {} ({minutes} min)", grant.name),
    );
    app.unlock(&grant.id, &grant.name, minutes);
    app.balloon(
        "Laser enabled",
        &format!("{name} can cut for {minutes} minutes. Scan again to end early.", name = grant.name),
    );
}

fn mask(token: &str) -> String {
    let n = token.chars().count();
    if n <= 4 {
        return "****".into();
    }
    let tail: String = token.chars().skip(n - 4).collect();
    format!("…{tail}")
}
