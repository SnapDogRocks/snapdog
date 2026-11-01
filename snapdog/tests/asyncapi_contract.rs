// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! The `AsyncAPI` document of `/ws` stays in step with `Notification`. Pure (no harness, no
//! socket).

#![allow(clippy::doc_markdown)] // module doc mentions AsyncAPI

use snapdog::api::asyncapi::document;
use snapdog::api::ws::Notification;

/// The checked-in document is what the code generates. After a change to `Notification`:
/// `cargo run --package xtask -- gen-asyncapi` and commit the result.
#[test]
fn the_checked_in_document_is_what_the_code_generates() {
    let checked_in: serde_json::Value =
        serde_json::from_str(include_str!("../../docs/api/asyncapi.json"))
            .expect("docs/api/asyncapi.json is valid JSON");
    assert_eq!(
        document(),
        checked_in,
        "docs/api/asyncapi.json differs from the generated document; run \
         `cargo run --package xtask -- gen-asyncapi` and commit the result"
    );
}

/// Every message the server really sends has a message in the document, under the `type`
/// it puts on the wire, and its payload names the fields that are on the wire.
#[test]
fn the_document_describes_every_notification() {
    let samples = [
        Notification::ZoneVolumeChanged {
            zone: 1,
            volume: 40,
            muted: false,
        },
        Notification::ClientStateChanged {
            client: 1,
            volume: 40,
            muted: false,
            connected: true,
            zone: 1,
            is_snapdog: true,
        },
        Notification::ZonePresenceChanged {
            zone: 1,
            presence: true,
            enabled: true,
            timer_active: false,
        },
        Notification::PlaybackError {
            zone: 1,
            message: "x".into(),
            details: None,
            recoverable: true,
        },
        Notification::ZoneProgress {
            zone: 1,
            position_ms: 1,
            duration_ms: 2,
            buffered_ms: None,
        },
    ];

    let doc = document();
    let messages = doc["components"]["messages"]
        .as_object()
        .expect("messages object");
    for sample in samples {
        let wire = serde_json::to_value(&sample).expect("notification serialises");
        let tag = wire["type"].as_str().expect("type tag");
        let message = messages
            .get(tag)
            .unwrap_or_else(|| panic!("no message for {tag}"));
        let properties = message["payload"]["properties"]
            .as_object()
            .expect("payload properties");
        for field in wire.as_object().expect("object").keys() {
            assert!(
                properties.contains_key(field),
                "{tag}: field {field} is on the wire but not in the document"
            );
        }
    }
    // All seven variants of Notification, each listed on the channel and the operation.
    assert_eq!(messages.len(), 7);
    assert_eq!(
        doc["channels"]["notifications"]["messages"]
            .as_object()
            .map(serde_json::Map::len),
        Some(7)
    );
    assert_eq!(
        doc["operations"]["receiveNotifications"]["messages"]
            .as_array()
            .map(Vec::len),
        Some(7)
    );
}

/// `zone_progress` is declared and never sent; the document says so.
#[test]
fn zone_progress_is_marked_reserved() {
    let doc = document();
    assert_eq!(
        doc["components"]["messages"]["zone_progress"]["x-reserved"],
        true
    );
    assert!(doc["components"]["messages"]["zone_changed"]["x-reserved"].is_null());
}
