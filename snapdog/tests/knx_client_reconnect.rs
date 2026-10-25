// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! KNX client mode over loopback: the bridge connects to a gateway that is not there yet,
//! survives the gateway going away and coming back, and publishes its state again after every
//! reconnect.
//!
//! The gateway is knx-rs's own tunnelling server (`DeviceServer`) on a loopback port, killed
//! and restarted by the test. Nothing here touches UDP 3671 or a real KNX installation.

#![allow(clippy::doc_markdown, clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::{Duration, Instant};

use knx_rs_core::address::{DestinationAddress, GroupAddress, IndividualAddress};
use knx_rs_core::cemi::CemiFrame;
use knx_rs_core::dpt::{self, DPT_SCALING};
use knx_rs_core::knxip::{Hpai, KnxIpFrame, ServiceType, tunnel_crd};
use knx_rs_core::message::MessageCode;
use knx_rs_core::types::Priority;
use knx_rs_ip::tunnel_server::{DeviceServer, ServerEvent};
use snapdog::api::ws::Notification;
use snapdog::config;
use snapdog::knx;
use snapdog::player::{SnapcastCmd, ZoneCommand};
use snapdog::state;
use tokio::net::UdpSocket;
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

/// Generous: a reconnect after a lost connection waits one to two seconds, then two to four.
const T: Duration = Duration::from_secs(15);

fn ga(s: &str) -> GroupAddress {
    GroupAddress::from_str(s).unwrap()
}

/// A free loopback UDP port (bound and released).
fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

struct Harness {
    bridge: knx::KnxBridge,
    store: state::SharedState,
    notify: broadcast::Sender<std::sync::Arc<str>>,
    zone_rx: mpsc::Receiver<ZoneCommand>,
    _snap_rx: mpsc::Receiver<SnapcastCmd>,
}

async fn start_bridge(url: &str) -> Harness {
    // `RUST_LOG=snapdog=debug,knx_rs_ip=debug cargo test ... -- --nocapture` shows the reconnects.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();
    let toml = format!(
        r#"
[knx]
role = "client"
url = "{url}"

[[zone]]
name = "Living"
[zone.knx]
play = "1/1/1"
volume_status = "1/1/3"
"#
    );
    let raw: config::FileConfig = toml::from_str(&toml).unwrap();
    let cfg = config::load_raw(raw).unwrap();
    let store = state::init(&cfg, None).unwrap();
    store.write().await.zones.get_mut(&1).unwrap().volume = 40;

    let (notify, notify_rx) = broadcast::channel(64);
    let (zone_tx, zone_rx) = mpsc::channel(16);
    let (snap_tx, snap_rx) = mpsc::channel(16);
    let bridge = knx::start(
        &cfg,
        store.clone(),
        notify_rx,
        HashMap::from([(1, zone_tx)]),
        snap_tx,
    )
    .await
    .expect("client mode starts even though the gateway is not there");
    Harness {
        bridge,
        store,
        notify,
        zone_rx,
        _snap_rx: snap_rx,
    }
}

async fn set_volume(b: &Harness, volume: i32) {
    b.store.write().await.zones.get_mut(&1).unwrap().volume = volume;
    let n = Notification::ZoneVolumeChanged {
        zone: 1,
        volume,
        muted: false,
    };
    b.notify
        .send(serde_json::to_string(&n).unwrap().into())
        .unwrap();
}

/// Next group write from the bridge to `dst` on the gateway side.
async fn expect_write(gw: &mut DeviceServer, dst: &str) -> Vec<u8> {
    let want = ga(dst);
    timeout(T, async {
        loop {
            let Some(ServerEvent::TunnelFrame(frame)) = gw.recv().await else {
                continue;
            };
            if let Some((g, data)) = frame.as_group_write()
                && g == want
            {
                return data;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no group write to {dst} within {T:?}"))
}

fn volume_of(data: &[u8]) -> f64 {
    dpt::decode(DPT_SCALING, data)
        .unwrap()
        .as_f64()
        .unwrap()
        .round()
}

/// A group telegram as the gateway delivers it to its tunnel client.
fn bus_telegram(code: MessageCode, dst: &str) -> CemiFrame {
    CemiFrame::new_l_data(
        code,
        IndividualAddress::from_raw(0x1101),
        DestinationAddress::Group(ga(dst)),
        Priority::Low,
        &[0x00, 0x81], // GroupValueWrite, 1
    )
}

async fn gateway(addr: SocketAddr) -> DeviceServer {
    DeviceServer::start_at(addr).await.expect("gateway binds")
}

#[tokio::test]
async fn bridge_survives_late_start_and_gateway_restart() {
    let addr: SocketAddr = format!("127.0.0.1:{}", free_port()).parse().unwrap();
    let mut b = start_bridge(&format!("tunnel://{addr}")).await;

    // 1. Start without a gateway, bring it up a moment later: the first attempts fail and the
    //    bridge connects on a later one.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let mut gw = gateway(addr).await;
    let started = Instant::now();
    assert_eq!(volume_of(&expect_write(&mut gw, "1/1/3").await), 40.0);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "first retry comes after about a second"
    );

    // Commands flow, but only bus telegrams count: the gateway's confirmation of a telegram
    // this client sent (`L_Data.con`) is not a command.
    gw.send_frame(bus_telegram(MessageCode::LDataCon, "1/1/1"))
        .await
        .unwrap();
    gw.send_frame(bus_telegram(MessageCode::LDataInd, "1/1/1"))
        .await
        .unwrap();
    let cmd = timeout(T, b.zone_rx.recv()).await;
    assert!(matches!(cmd, Ok(Some(ZoneCommand::Play))), "{cmd:?}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        b.zone_rx.try_recv().is_err(),
        "the confirmation did not trigger a second command"
    );

    // 2. Kill the gateway. While it is gone the state moves on.
    gw.stop().await;
    drop(gw);
    tokio::time::sleep(Duration::from_millis(300)).await;
    set_volume(&b, 55).await;

    // 3. Bring a gateway back on the same address: the bridge reconnects by itself and publishes
    //    the current state, including values it had already sent to the previous gateway.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut gw = gateway(addr).await;
    assert_eq!(
        volume_of(&expect_write(&mut gw, "1/1/3").await),
        55.0,
        "state is published again after the reconnect"
    );
    gw.send_frame(bus_telegram(MessageCode::LDataInd, "1/1/1"))
        .await
        .unwrap();
    let cmd = timeout(T, b.zone_rx.recv()).await;
    assert!(
        matches!(cmd, Ok(Some(ZoneCommand::Play))),
        "commands flow again: {cmd:?}"
    );

    // A second loss and recovery works the same way. The loss is noticed by the next write
    // (an idle connection would be noticed by the tunnel's heartbeat after a couple of
    // minutes), so the state moves on again.
    gw.stop().await;
    drop(gw);
    tokio::time::sleep(Duration::from_millis(300)).await;
    set_volume(&b, 60).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let mut gw = gateway(addr).await;
    assert_eq!(volume_of(&expect_write(&mut gw, "1/1/3").await), 60.0);

    // 4. Shutdown stops the loop promptly and for good.
    let t = Instant::now();
    b.bridge.shutdown().await;
    assert!(t.elapsed() < Duration::from_secs(2), "shutdown is prompt");
}

/// A gateway that accepts the tunnel connection and then never answers: no ICMP error, no
/// disconnect, nothing for the tunnel's own heartbeat to notice for minutes. Writes that
/// cannot be acknowledged must end the session instead.
#[tokio::test]
async fn silent_gateway_ends_the_session_after_failed_writes() {
    let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = socket.local_addr().unwrap();
    let b = start_bridge(&format!("tunnel://{addr}")).await;

    // The first status write (the initial state) times out after the transport's three
    // retries; a change a moment later is the second one in a row.
    let writer = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        set_volume(&b, 41).await;
    };

    let mut connects = 0;
    let mut buf = [0u8; 512];
    let deadline = Instant::now() + Duration::from_secs(40);
    let ((), ()) = tokio::join!(writer, async {
        while connects < 2 && Instant::now() < deadline {
            let Ok(Ok((n, from))) =
                timeout(Duration::from_millis(500), socket.recv_from(&mut buf)).await
            else {
                continue;
            };
            let Ok(frame) = KnxIpFrame::parse(&buf[..n]) else {
                continue;
            };
            if frame.service_type == ServiceType::ConnectRequest {
                connects += 1;
                // Accept the connection, then stay silent.
                let mut body = vec![1, 0];
                body.extend_from_slice(&Hpai::nat_udp(addr.port()).to_bytes());
                body.extend_from_slice(&tunnel_crd(0xFF01));
                let response = KnxIpFrame {
                    service_type: ServiceType::ConnectResponse,
                    body,
                };
                socket.send_to(&response.to_bytes(), from).await.unwrap();
            }
        }
    });
    assert!(
        connects >= 2,
        "the bridge gave up on the silent gateway and connected again ({connects} connects)"
    );
    b.bridge.shutdown().await;
}
