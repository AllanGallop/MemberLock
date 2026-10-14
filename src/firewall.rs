use std::process::Command;

use crate::config::Config;
use crate::log;

pub fn ensure(cfg: &Config) {
    if !cfg.firewall.enforce {
        return;
    }
    if !is_admin() {
        let msg = "Firewall enforcement needs Administrator. Laser stays locked in MemberLock, but LightBurn can still be pointed straight at the controller.";
        log::write("error", msg);
        return;
    }
    for (index, program) in cfg.firewall.deny_programs.iter().enumerate() {
        if !program.exists() {
            log::write(
                "info",
                &format!("firewall skip, not installed: {}", program.display()),
            );
            continue;
        }
        for port in [cfg.laser.job_port, cfg.laser.panel_port] {
            let name = format!("MemberLock block {index} port {port}");
            let _ = netsh(&["advfirewall", "firewall", "delete", "rule", &format!("name={name}")]);
            let program = program.display().to_string();
            let remote_ip = cfg.laser.address.to_string();
            let remote_port = port.to_string();
            let result = netsh(&[
                "advfirewall",
                "firewall",
                "add",
                "rule",
                &format!("name={name}"),
                "dir=out",
                "action=block",
                &format!("program={program}"),
                "protocol=UDP",
                &format!("remoteip={remote_ip}"),
                &format!("remoteport={remote_port}"),
                "enable=yes",
            ]);
            match result {
                Ok(()) => log::write(
                    "info",
                    &format!("firewall blocks {program} from reaching {remote_ip}:{remote_port}"),
                ),
                Err(err) => log::write("error", &format!("firewall rule failed: {err}")),
            }
        }
    }
}

fn is_admin() -> bool {
    unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().as_bool() }
}

fn netsh(args: &[&str]) -> Result<(), String> {
    let output = Command::new("netsh")
        .args(args)
        .output()
        .map_err(|err| err.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        Err(format!("{stdout} {stderr}").trim().to_string())
    }
}

