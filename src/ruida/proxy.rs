use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::UdpSocket;
use tokio::sync::watch;

use crate::log;
use crate::state::App;

use super::codec::encode_job;
use super::filter::{job_stop_clear, judge, panel_stop, Channel, Verdict};

pub fn spawn(app: Arc<App>, stop_rx: watch::Receiver<u64>) {
    let app_job = Arc::clone(&app);
    let job_rx = stop_rx.clone();
    tokio::spawn(async move {
        run_channel(app_job, Channel::Job, job_rx).await;
    });
    tokio::spawn(async move {
        run_channel(app, Channel::Panel, stop_rx).await;
    });
}

async fn run_channel(app: Arc<App>, channel: Channel, mut stop_rx: watch::Receiver<u64>) {
    let label = match channel {
        Channel::Job => "job",
        Channel::Panel => "panel",
    };
    loop {
        match bind_pair(&app, channel).await {
            Ok((listen, outbound, source)) => {
                log::write(
                    "info",
                    &format!(
                        "{label} proxy up. LightBurn → {}:{}   controller {} via {}",
                        app.cfg.laser.listen,
                        listen_port(&app, channel),
                        app.cfg.laser.address,
                        source
                    ),
                );
                app.set_link(format!(
                    "LightBurn device: 127.0.0.1 or this PC's LAN address. Controller {} is behind MemberLock.",
                    app.cfg.laser.address
                ));
                if let Err(err) = pump(&app, channel, listen, outbound, &mut stop_rx).await {
                    log::write("error", &format!("{label} proxy stopped: {err}"));
                }
            }
            Err(err) => {
                let msg = format!("{label} proxy failed: {err}");
                log::write("error", &msg);
                app.set_link(msg);
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

fn listen_port(app: &App, channel: Channel) -> u16 {
    match channel {
        Channel::Job => app.cfg.laser.job_port,
        Channel::Panel => app.cfg.laser.panel_port,
    }
}

fn source_port(app: &App, channel: Channel) -> u16 {
    match channel {
        Channel::Job => app.cfg.laser.job_source_port,
        Channel::Panel => app.cfg.laser.panel_source_port,
    }
}

async fn bind_pair(
    app: &App,
    channel: Channel,
) -> std::io::Result<(UdpSocket, UdpSocket, SocketAddrV4)> {
    let listen_addr = SocketAddrV4::new(app.cfg.laser.listen, listen_port(app, channel));
    let listen = UdpSocket::bind(listen_addr).await?;

    let laser = SocketAddrV4::new(app.cfg.laser.address, listen_port(app, channel));
    let source_ip = match app.cfg.laser.source_address {
        Some(ip) => ip,
        None => local_ip_toward(app.cfg.laser.address)?,
    };
    let source = SocketAddrV4::new(source_ip, source_port(app, channel));
    let outbound = bind_connected(source, laser)?;
    Ok((listen, outbound, source))
}

fn local_ip_toward(laser: Ipv4Addr) -> std::io::Result<Ipv4Addr> {
    let probe = std::net::UdpSocket::bind("0.0.0.0:0")?;
    probe.connect(SocketAddrV4::new(laser, 9))?;
    match probe.local_addr()? {
        SocketAddr::V4(addr) => Ok(*addr.ip()),
        _ => Err(std::io::Error::other(
            "no IPv4 route to the laser controller",
        )),
    }
}

fn bind_connected(local: SocketAddrV4, remote: SocketAddrV4) -> std::io::Result<UdpSocket> {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&socket2::SockAddr::from(local))?;
    socket.connect(&socket2::SockAddr::from(remote))?;
    let std_socket: std::net::UdpSocket = socket.into();
    UdpSocket::from_std(std_socket)
}

async fn pump(
    app: &App,
    channel: Channel,
    listen: UdpSocket,
    outbound: UdpSocket,
    stop_rx: &mut watch::Receiver<u64>,
) -> std::io::Result<()> {
    let mut from_host = vec![0u8; 2048];
    let mut from_laser = vec![0u8; 2048];
    let mut client: Option<SocketAddr> = None;
    let mut last_notice = Instant::now() - Duration::from_secs(30);

    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() {
                    continue;
                }
                if *stop_rx.borrow() == 0 || app.is_unlocked() {
                    continue;
                }
                send_stop(channel, &outbound).await;
            }
            received = listen.recv_from(&mut from_host) => {
                let (n, addr) = received?;
                client = Some(addr);
                let datagram = &from_host[..n];
                match judge(channel, datagram, !app.is_unlocked()) {
                    Verdict::Forward => {
                        outbound.send(datagram).await?;
                    }
                    Verdict::Drop { reply } => {
                        let _ = listen.send_to(reply, addr).await;
                        if last_notice.elapsed() > Duration::from_secs(4) {
                            last_notice = Instant::now();
                            let what = match channel {
                                Channel::Job => "job / tube-on command",
                                Channel::Panel => "pulse, start, or laser-gate command",
                            };
                            log::write("info", &format!("blocked a {what} from {addr}"));
                            app.balloon(
                                "Laser locked",
                                "Badge in to cut. Jog and the red-dot frame still work.",
                            );
                        }
                    }
                }
            }
            received = outbound.recv(&mut from_laser) => {
                let n = received?;
                if let Some(addr) = client {
                    listen.send_to(&from_laser[..n], addr).await?;
                }
            }
        }
    }
}

async fn send_stop(channel: Channel, outbound: &UdpSocket) {
    let packets: Vec<Vec<u8>> = match channel {
        Channel::Job => vec![encode_job(job_stop_clear())],
        Channel::Panel => panel_stop().iter().map(|p| p.to_vec()).collect(),
    };
    for packet in packets {
        if let Err(err) = outbound.send(&packet).await {
            log::write("error", &format!("failed to send stop: {err}"));
        }
    }
    log::write(
        "info",
        &match channel {
            Channel::Job => "sent process-stop to the controller",
            Channel::Panel => "sent panel-stop to the controller",
        },
    );
}
