use std::sync::Mutex;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::config::Config;

#[derive(Debug, Clone)]
pub struct Status {
    pub unlocked: bool,
    pub holder_id: String,
    pub holder_name: String,
    pub until: Option<Instant>,
    pub detail: String,
    pub link: String,
}

impl Status {
    pub fn locked(detail: impl Into<String>) -> Self {
        Self {
            unlocked: false,
            holder_id: String::new(),
            holder_name: String::new(),
            until: None,
            detail: detail.into(),
            link: String::new(),
        }
    }

    pub fn summary(&self, laser_name: &str) -> String {
        if self.unlocked {
            let left = self
                .until
                .map(|until| until.saturating_duration_since(Instant::now()))
                .unwrap_or(Duration::ZERO);
            let mins = left.as_secs() / 60;
            let secs = left.as_secs() % 60;
            format!(
                "{laser_name} enabled — {} — {mins}m {secs:02}s",
                self.holder_name
            )
        } else if !self.detail.is_empty() {
            format!("{laser_name} locked — {}", self.detail)
        } else if !self.link.is_empty() {
            format!("{laser_name} locked — {}", self.link)
        } else {
            format!("{laser_name} locked — badge in to cut")
        }
    }
}

#[derive(Debug, Clone)]
pub struct Balloon {
    pub title: String,
    pub body: String,
}

pub struct App {
    pub cfg: Config,
    status: Mutex<Status>,
    balloon: Mutex<Option<Balloon>>,
    stop_gen: watch::Sender<u64>,
    pub stop_rx: watch::Receiver<u64>,
}

impl App {
    pub fn new(cfg: Config) -> Self {
        let (stop_gen, stop_rx) = watch::channel(0u64);
        Self {
            cfg,
            status: Mutex::new(Status::locked("waiting for a badge")),
            balloon: Mutex::new(None),
            stop_gen,
            stop_rx,
        }
    }

    pub fn is_unlocked(&self) -> bool {
        self.status.lock().expect("status").unlocked
    }

    pub fn status_snapshot(&self) -> Status {
        self.status.lock().expect("status").clone()
    }

    pub fn set_link(&self, link: impl Into<String>) {
        self.status.lock().expect("status").link = link.into();
    }

    pub fn set_auth_note(&self, detail: impl Into<String>) {
        let mut status = self.status.lock().expect("status");
        if !status.unlocked {
            status.detail = detail.into();
        }
    }

    pub fn unlock(&self, id: &str, name: &str, minutes: u64) {
        let until = Instant::now() + Duration::from_secs(minutes.saturating_mul(60));
        let mut status = self.status.lock().expect("status");
        status.unlocked = true;
        status.holder_id = id.to_string();
        status.holder_name = name.to_string();
        status.until = Some(until);
        status.detail.clear();
    }

    /// Lock the tube. When a session was open, tell the proxy to stop the machine.
    pub fn lock(&self, detail: impl Into<String>) {
        let mut status = self.status.lock().expect("status");
        let was = status.unlocked;
        let link = status.link.clone();
        *status = Status::locked(detail);
        status.link = link;
        drop(status);
        if was {
            self.stop_gen.send_modify(|n| *n = n.saturating_add(1));
        }
    }

    pub fn balloon(&self, title: impl Into<String>, body: impl Into<String>) {
        *self.balloon.lock().expect("balloon") = Some(Balloon {
            title: title.into(),
            body: body.into(),
        });
    }

    pub fn take_balloon(&self) -> Option<Balloon> {
        self.balloon.lock().expect("balloon").take()
    }
}
