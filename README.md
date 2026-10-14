# MemberLock

MemberLock is a Windows tray app that sits between LightBurn and an Ethernet Ruida controller. Jogging and the red-dot frame keep working. Cutting, pulse, and job start are forwarded only after a USB serial RFID badge matches an active UniFi Access user.

The tube stays off until that check succeeds. If UniFi Access cannot be reached, it stays off.

## What is blocked while locked

On the keypad channel (UDP 50207):

- Pulse
- Start / pause
- Laser gate

On the job channel (UDP 50200), after the Ruida swizzle is removed:

- Cut moves (`0xA8`–`0xAB`), which are the moves that fire the tube
- Process start (`0xD8 0x00`) and resume (`0xD8 0x03`)

Travel moves, status polls, jog keys, origin, stop, and frame (`0xA5 0x53 0x00`) are forwarded either way. When a session ends, MemberLock sends stop to the controller.

## What this does not cover

The controller keypad can still pulse or start a job that is already stored on the machine. A USB cable from LightBurn to the controller also skips this app. This is an access gate for the Ethernet path, not a substitute for the lid interlock, training, or a hardware e-stop.

Pointing LightBurn at the controller's own IP skips MemberLock unless the firewall rules below are installed.

## LightBurn

1. Leave the controller's IP as `laser.address` in `config.toml`. Do not give that address to LightBurn.
2. In LightBurn, set the Ruida device address to `127.0.0.1` when LightBurn is on this PC, or to this PC's LAN address when LightBurn is on another computer. Type that address in the device settings.

MemberLock listens on UDP 50200 and 50207 and talks to the controller from UDP 40200 and 40207, which is what the controller expects. The source port is bound on the LAN address so it can coexist with LightBurn on the same PC.

## UniFi Access

Create an API token in Access → Settings → General → Advanced, and put the console URL in `unifi.base_url` (usually `https://<console>:12445`).

Create an access policy named `Laser` (or whatever `unifi.policy` says) and assign it only to people who are trained. Deactivated users never enable the tube.

A generic USB reader reports the card UID. Official UniFi cards store a reader-generated token, not that UID. Put the number your reader prints into the card's **alias** or **note** in Access (or enroll a third-party card whose token is that id). MemberLock matches the scan against the token, display id, alias, note, and employee number, including hex/decimal and reversed byte order.

## RFID

Most USB readers show up as a COM port and send the id plus a newline at 9600 baud. `rfid.port = "auto"` uses the only USB serial port. If several exist, set `COM3` (or whichever port the reader uses).

A valid badge enables the tube for `session.minutes`. Scanning the same card again ends the session and sends stop. A different valid card takes over the session.

## Firewall

Set `firewall.enforce = true` and start MemberLock as Administrator once. It adds persistent outbound rules so `LightBurn.exe` cannot reach the controller's job and panel ports. Those rules stay in place after MemberLock quits, which keeps a direct connection from bypassing the badge. Remove them in Windows Defender Firewall if you take the gate out of service.

## Run at sign-in

The first launch writes `%APPDATA%\MemberLock\config.toml` if you do not already have `config.toml` in the working directory. The log is `memberlock.log` beside that file. The tray icon is red while the tube is locked and green while a session is open. Right-click it for Lock now, the config, the log, and Quit.

Build with `cargo build --release`. The executable is `target\release\memberlock.exe`. A shortcut in the Startup folder, or a scheduled task at logon, keeps the gate running. While the app is not running, LightBurn pointed at `127.0.0.1` cannot reach the controller.

## Release

Pushing a tag to GitHub runs the release workflow. It tests, builds `memberlock.exe`, and attaches `MemberLock-<version>-windows-x64.zip` (the exe and this README) to a GitHub release. A manual run from the Actions tab builds the same zip and uploads it as an artifact, without publishing a release.

The tag has to be `v` plus the `version` in `Cargo.toml`. For the current crate version:

```
git tag v0.1.0
git push origin v0.1.0
```

Bump `version` in `Cargo.toml` and commit that change before tagging the next release. `Cargo.lock` stays in the repo so the workflow builds with `cargo --locked`.
