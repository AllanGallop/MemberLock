use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Config {
    pub laser: LaserConfig,
    pub session: SessionConfig,
    pub rfid: RfidConfig,
    pub unifi: UnifiConfig,
    pub firewall: FirewallConfig,
}

#[derive(Debug, Clone)]
pub struct LaserConfig {
    pub name: String,
    pub address: Ipv4Addr,
    /// Local address used as the UDP source when talking to the controller.
    /// `None` means "whichever address routes to the controller".
    pub source_address: Option<Ipv4Addr>,
    pub listen: Ipv4Addr,
    pub job_port: u16,
    pub panel_port: u16,
    pub job_source_port: u16,
    pub panel_source_port: u16,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub minutes: u64,
}

#[derive(Debug, Clone)]
pub struct RfidConfig {
    /// `"auto"` selects a USB serial reader. Otherwise a port name such as `COM3`.
    pub port: String,
    pub baud: u32,
}

#[derive(Clone)]
pub struct UnifiConfig {
    pub base_url: String,
    pub api_token: String,
    /// Access policy name or id. Empty allows every active user whose card matches.
    pub policy: String,
    pub insecure_tls: bool,
    pub refresh: Duration,
    pub max_stale: Duration,
}

impl std::fmt::Debug for UnifiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnifiConfig")
            .field("base_url", &self.base_url)
            .field("api_token", &"<redacted>")
            .field("policy", &self.policy)
            .field("insecure_tls", &self.insecure_tls)
            .field("refresh", &self.refresh)
            .field("max_stale", &self.max_stale)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct FirewallConfig {
    pub enforce: bool,
    pub deny_programs: Vec<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct FileConfig {
    #[serde(default)]
    laser: LaserFile,
    #[serde(default)]
    session: SessionFile,
    #[serde(default)]
    rfid: RfidFile,
    #[serde(default)]
    unifi: UnifiFile,
    #[serde(default)]
    firewall: FirewallFile,
}

#[derive(Debug, Deserialize)]
struct LaserFile {
    #[serde(default = "default_name")]
    name: String,
    #[serde(default = "default_laser_ip")]
    address: String,
    #[serde(default)]
    source_address: String,
    #[serde(default = "default_listen")]
    listen: String,
    #[serde(default = "default_job_port")]
    job_port: u16,
    #[serde(default = "default_panel_port")]
    panel_port: u16,
    #[serde(default = "default_job_source")]
    job_source_port: u16,
    #[serde(default = "default_panel_source")]
    panel_source_port: u16,
}

#[derive(Debug, Deserialize)]
struct SessionFile {
    #[serde(default = "default_minutes")]
    minutes: u64,
}

#[derive(Debug, Deserialize)]
struct RfidFile {
    #[serde(default = "default_port")]
    port: String,
    #[serde(default = "default_baud")]
    baud: u32,
}

#[derive(Debug, Deserialize)]
struct UnifiFile {
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_token: String,
    #[serde(default = "default_policy")]
    policy: String,
    #[serde(default = "default_true")]
    insecure_tls: bool,
    #[serde(default = "default_refresh")]
    refresh_seconds: u64,
    #[serde(default = "default_stale")]
    max_stale_seconds: u64,
}

#[derive(Debug, Deserialize)]
struct FirewallFile {
    #[serde(default)]
    enforce: bool,
    #[serde(default = "default_deny_programs")]
    deny_programs: Vec<String>,
}

impl Default for LaserFile {
    fn default() -> Self {
        Self {
            name: default_name(),
            address: default_laser_ip(),
            source_address: String::new(),
            listen: default_listen(),
            job_port: default_job_port(),
            panel_port: default_panel_port(),
            job_source_port: default_job_source(),
            panel_source_port: default_panel_source(),
        }
    }
}

impl Default for SessionFile {
    fn default() -> Self {
        Self {
            minutes: default_minutes(),
        }
    }
}

impl Default for RfidFile {
    fn default() -> Self {
        Self {
            port: default_port(),
            baud: default_baud(),
        }
    }
}

impl Default for UnifiFile {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_token: String::new(),
            policy: default_policy(),
            insecure_tls: true,
            refresh_seconds: default_refresh(),
            max_stale_seconds: default_stale(),
        }
    }
}

impl Default for FirewallFile {
    fn default() -> Self {
        Self {
            enforce: false,
            deny_programs: default_deny_programs(),
        }
    }
}

fn default_name() -> String {
    "Laser".into()
}
fn default_laser_ip() -> String {
    "192.168.1.50".into()
}
fn default_listen() -> String {
    "0.0.0.0".into()
}
fn default_job_port() -> u16 {
    50200
}
fn default_panel_port() -> u16 {
    50207
}
fn default_job_source() -> u16 {
    40200
}
fn default_panel_source() -> u16 {
    40207
}
fn default_minutes() -> u64 {
    30
}
fn default_port() -> String {
    "auto".into()
}
fn default_baud() -> u32 {
    9600
}
fn default_policy() -> String {
    "Laser".into()
}
fn default_true() -> bool {
    true
}
fn default_refresh() -> u64 {
    45
}
fn default_stale() -> u64 {
    900
}
fn default_deny_programs() -> Vec<String> {
    vec![
        r"C:\Program Files\LightBurn\LightBurn.exe".into(),
        r"C:\Program Files (x86)\LightBurn\LightBurn.exe".into(),
    ]
}

impl Config {
    pub fn from_toml(text: &str) -> anyhow::Result<Self> {
        let file: FileConfig = toml::from_str(text)?;
        let address: Ipv4Addr = file.laser.address.parse().map_err(|_| {
            anyhow::anyhow!(
                "laser.address `{}` is not an IPv4 address",
                file.laser.address
            )
        })?;
        let listen: Ipv4Addr = file.laser.listen.parse().map_err(|_| {
            anyhow::anyhow!("laser.listen `{}` is not an IPv4 address", file.laser.listen)
        })?;
        let source_address = if file.laser.source_address.trim().is_empty() {
            None
        } else {
            Some(file.laser.source_address.parse().map_err(|_| {
                anyhow::anyhow!(
                    "laser.source_address `{}` is not an IPv4 address",
                    file.laser.source_address
                )
            })?)
        };
        let minutes = file.session.minutes.max(1);
        let refresh = Duration::from_secs(file.unifi.refresh_seconds.max(15));
        let max_stale = Duration::from_secs(file.unifi.max_stale_seconds.max(refresh.as_secs()));
        Ok(Self {
            laser: LaserConfig {
                name: file.laser.name,
                address,
                source_address,
                listen,
                job_port: file.laser.job_port,
                panel_port: file.laser.panel_port,
                job_source_port: file.laser.job_source_port,
                panel_source_port: file.laser.panel_source_port,
            },
            session: SessionConfig { minutes },
            rfid: RfidConfig {
                port: file.rfid.port,
                baud: file.rfid.baud.max(300),
            },
            unifi: UnifiConfig {
                base_url: file.unifi.base_url.trim_end_matches('/').to_string(),
                api_token: file.unifi.api_token.trim().to_string(),
                policy: file.unifi.policy.trim().to_string(),
                insecure_tls: file.unifi.insecure_tls,
                refresh,
                max_stale,
            },
            firewall: FirewallConfig {
                enforce: file.firewall.enforce,
                deny_programs: file
                    .firewall
                    .deny_programs
                    .into_iter()
                    .map(PathBuf::from)
                    .collect(),
            },
        })
    }
}

pub fn example_toml() -> &'static str {
    r#"# MemberLock — point LightBurn at this PC, not at the Ruida controller.
# The controller address below is where MemberLock forwards allowed packets.

[laser]
name = "Laser"
address = "192.168.1.50"
# Leave blank to use the address this PC uses to reach the controller.
source_address = ""
listen = "0.0.0.0"
job_port = 50200
panel_port = 50207
job_source_port = 40200
panel_source_port = 40207

[session]
# How long a valid badge enables the tube. Scanning again sooner toggles the session off.
minutes = 30

[rfid]
# "auto" uses the only USB serial device, or set "COM3".
port = "auto"
baud = 9600

[unifi]
# UniFi Access developer API. Console → Access → Settings → General → Advanced → API Token.
base_url = "https://192.168.1.1:12445"
api_token = ""
# Policy name or id. The user must be ACTIVE and assigned this policy.
# Leave empty to accept any active user whose card matches.
policy = "Laser"
# UniFi consoles ship with a private certificate.
insecure_tls = true
refresh_seconds = 45
# If Access cannot be reached for this long, the laser stays locked.
max_stale_seconds = 900

[firewall]
# Requires Administrator. Adds persistent outbound blocks so LightBurn.exe
# cannot talk to the controller directly and skip the badge check.
enforce = false
deny_programs = [
    'C:\Program Files\LightBurn\LightBurn.exe',
    'C:\Program Files (x86)\LightBurn\LightBurn.exe',
]
"#
}
