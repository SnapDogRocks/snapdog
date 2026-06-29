// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Snapcast contract firewall (IT-T51 + IT-T54) — the headline regression guard
//! for the upgraded `snapcast-server`/`snapcast-proto` seam (RFC §9.1).
//!
//! Process backend only (the JSON-RPC client + builders live in
//! `#[cfg(feature="snapcast-process")] mod process_impl`). Run with:
//!   cargo test --test snapcast_rpc --no-default-features --features snapcast-process
//!
//! - IT-T51: a golden `ServerStatus` fixture deserializes (fails if snapcast-proto
//!   renames a status field on upgrade) and the `build_*` helpers return the
//!   golden shape.
//! - IT-T54: a line-delimited-JSON TCP fake records outgoing requests; golden
//!   vectors assert each method's wire `method` string + `params` shape (ids are
//!   per-request UUIDv4, so we assert structure, not the literal id). Encodes the
//!   two traps: `Group.SetMute` → key `mute` (not `muted`); `Stream.AddStream` →
//!   camelCase `streamUri`.

#![cfg(feature = "snapcast-process")]
#![allow(clippy::doc_markdown)] // module doc contains CLI flags / bare identifiers

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use snapcast_proto::status::ServerStatus;
use snapdog::snapcast::{
    SnapcastClient, build_client_mac_map, build_group_clients, build_group_ids,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

/// Verified-valid `ServerStatus` JSON (snapcast-proto 0.16.1): 2 groups, 3 clients
/// with mixed-case MACs to exercise `to_lowercase()`.
const FIXTURE: &str = r#"{
  "server": {
    "server": {
      "host": { "arch": "x86_64", "ip": "127.0.0.1", "mac": "", "name": "snaphost", "os": "linux" },
      "snapserver": { "name": "snapserver", "protocolVersion": 2, "controlProtocolVersion": 1, "version": "0.27.0" }
    },
    "groups": [
      {
        "id": "g1", "name": "Living Room", "stream_id": "default", "muted": false,
        "clients": [
          { "id": "client-aa", "connected": true,
            "config": { "latency": 0, "name": "Kitchen", "volume": { "muted": false, "percent": 50 } },
            "host": { "mac": "AA:BB:CC:DD:EE:01" } },
          { "id": "client-bb", "connected": true,
            "config": { "latency": 0, "name": "Bath", "volume": { "muted": false, "percent": 60 } },
            "host": { "mac": "AA:BB:CC:DD:EE:02" } }
        ]
      },
      {
        "id": "g2", "name": "Bedroom", "stream_id": "default", "muted": false,
        "clients": [
          { "id": "client-cc", "connected": true,
            "config": { "latency": 0, "name": "Bedroom", "volume": { "muted": false, "percent": 40 } },
            "host": { "mac": "AA:BB:CC:DD:EE:03" } }
        ]
      }
    ],
    "streams": []
  }
}"#;

// ── IT-T51: status deserialization + builder golden ───────────────

#[test]
fn fixture_deserializes_and_builders_match_golden() {
    let status: ServerStatus = serde_json::from_str(FIXTURE).expect(
        "ServerStatus deserializes — fails loudly if snapcast-proto renames a status field",
    );

    let ids = build_group_ids(&status);
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], "g1");
    assert_eq!(ids[1], "g2");

    let gc = build_group_clients(&status);
    assert_eq!(gc.len(), 2);
    assert_eq!(
        gc["g1"].iter().map(String::as_str).collect::<Vec<_>>(),
        ["client-aa", "client-bb"]
    );
    assert_eq!(
        gc["g2"].iter().map(String::as_str).collect::<Vec<_>>(),
        ["client-cc"]
    );

    let mm = build_client_mac_map(&status);
    assert_eq!(mm.len(), 3);
    assert_eq!(mm["aa:bb:cc:dd:ee:01"], "client-aa"); // MAC lowercased
    assert_eq!(mm["aa:bb:cc:dd:ee:02"], "client-bb");
    assert_eq!(mm["aa:bb:cc:dd:ee:03"], "client-cc");
}

// ── IT-T54: line-delimited-JSON TCP fake + golden request vectors ──

/// Spawn a fake snapcast control server on an ephemeral loopback port. It records
/// every incoming JSON-RPC request and replies with `result` echoing the request
/// id (responses are correlated by id). Returns the address + the recorded buffer.
async fn spawn_fake(result: Value) -> (SocketAddr, Arc<Mutex<Vec<Value>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let recorded = Arc::new(Mutex::new(Vec::<Value>::new()));
    let rec = recorded.clone();
    tokio::spawn(async move {
        let Ok((sock, _)) = listener.accept().await else {
            return;
        };
        let (rd, mut wr) = sock.into_split();
        let mut lines = BufReader::new(rd).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(req) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let id = req.get("id").cloned().unwrap_or(Value::Null);
            rec.lock().unwrap().push(req);
            let resp = json!({ "jsonrpc": "2.0", "id": id, "result": result.clone() });
            let mut s = serde_json::to_string(&resp).unwrap();
            s.push('\n');
            if wr.write_all(s.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    (addr, recorded)
}

fn find_request(recorded: &Arc<Mutex<Vec<Value>>>, method: &str) -> Value {
    let reqs = recorded.lock().unwrap();
    reqs.iter()
        .find(|r| r["method"] == method)
        .unwrap_or_else(|| panic!("no request with method {method}: {reqs:?}"))
        .clone()
}

#[tokio::test]
async fn rpc_server_get_status_framing_and_response() {
    let result: Value = serde_json::from_str(FIXTURE).unwrap();
    let (addr, rec) = spawn_fake(result).await;
    let client = SnapcastClient::connect(addr).await.unwrap();

    let status = client.server_get_status().await.unwrap();
    // Response deserialized through the real envelope into ServerStatus.
    assert_eq!(build_group_ids(&status).len(), 2);

    let req = find_request(&rec, "Server.GetStatus");
    assert_eq!(req["jsonrpc"], "2.0", "JSON-RPC 2.0 envelope");
    assert_eq!(req["params"], json!({}));
    assert!(req["id"].as_str().is_some(), "id is a UUID string");
}

#[tokio::test]
async fn rpc_client_set_volume_params() {
    let (addr, rec) = spawn_fake(Value::Null).await;
    let client = SnapcastClient::connect(addr).await.unwrap();
    client.client_set_volume("client-aa", 80).await.unwrap();
    assert_eq!(
        find_request(&rec, "Client.SetVolume")["params"],
        json!({ "id": "client-aa", "volume": { "percent": 80 } })
    );
}

#[tokio::test]
async fn rpc_client_set_mute_sends_disjoint_sub_object() {
    // SetMute reuses Client.SetVolume but sends only {"muted":...} (no percent).
    let (addr, rec) = spawn_fake(Value::Null).await;
    let client = SnapcastClient::connect(addr).await.unwrap();
    client.client_set_mute("client-aa", true).await.unwrap();
    assert_eq!(
        find_request(&rec, "Client.SetVolume")["params"],
        json!({ "id": "client-aa", "volume": { "muted": true } })
    );
}

#[tokio::test]
async fn rpc_group_set_mute_uses_mute_key_not_muted() {
    let (addr, rec) = spawn_fake(Value::Null).await;
    let client = SnapcastClient::connect(addr).await.unwrap();
    client.group_set_mute("g1", true).await.unwrap();
    assert_eq!(
        find_request(&rec, "Group.SetMute")["params"],
        json!({ "id": "g1", "mute": true }),
        "Group.SetMute wire key is `mute`, not `muted`"
    );
}

#[tokio::test]
async fn rpc_group_set_clients_params() {
    let (addr, rec) = spawn_fake(Value::Null).await;
    let client = SnapcastClient::connect(addr).await.unwrap();
    client
        .group_set_clients("g1", vec!["client-aa".into(), "client-bb".into()])
        .await
        .unwrap();
    assert_eq!(
        find_request(&rec, "Group.SetClients")["params"],
        json!({ "id": "g1", "clients": ["client-aa", "client-bb"] })
    );
}

#[tokio::test]
async fn rpc_stream_add_uses_camelcase_streamuri() {
    let (addr, rec) = spawn_fake(json!({ "id": "s1" })).await;
    let client = SnapcastClient::connect(addr).await.unwrap();
    let _ = client.stream_add("pipe:///tmp/x?name=y").await.unwrap();
    assert_eq!(
        find_request(&rec, "Stream.AddStream")["params"],
        json!({ "streamUri": "pipe:///tmp/x?name=y" }),
        "Stream.AddStream wire key is camelCase `streamUri`"
    );
}
