// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `AsyncAPI` description of the `/ws` notification stream.
//!
//! The message payloads come from the schema of [`Notification`], so a new event or a new
//! field shows up here without anyone remembering it. `docs/api/asyncapi.json` is this
//! document, written by `cargo run --package xtask -- gen-asyncapi`; the
//! `asyncapi_contract` test fails when the two differ.

use serde_json::{Value, json};
use utoipa::{PartialSchema, ToSchema};

use crate::api::ws::Notification;

/// Events the server declares but never sends. They stay in the document, marked reserved,
/// so a client does not have to guess whether a name it sees in the code is live.
const RESERVED: &[&str] = &["zone_progress"];

/// Build the document.
///
/// # Panics
///
/// Never in practice: the schema of `Notification` is a `oneOf` of objects that each carry a
/// `type` tag, which `the_document_describes_every_notification` checks.
#[must_use]
pub fn document() -> Value {
    let schema = serde_json::to_value(Notification::schema()).expect("the schema serialises");
    let variants = schema["oneOf"]
        .as_array()
        .expect("Notification is a oneOf of its variants");

    let mut messages = serde_json::Map::new();
    let mut refs = serde_json::Map::new();
    let mut op_refs = Vec::new();
    for variant in variants {
        let tag = tag_of(variant);
        let mut message = json!({
            "name": tag,
            "title": variant["description"],
            "contentType": "application/json",
            "payload": variant,
        });
        if RESERVED.contains(&tag.as_str()) {
            message["x-reserved"] = json!(true);
            message["summary"] = json!("Reserved: the server never sends this event yet.");
        }
        messages.insert(tag.clone(), message);
        refs.insert(
            tag.clone(),
            json!({ "$ref": format!("#/components/messages/{tag}") }),
        );
        op_refs.push(json!({ "$ref": format!("#/channels/notifications/messages/{tag}") }));
    }

    json!({
        "asyncapi": "3.0.0",
        "info": {
            "title": "SnapDog WebSocket notifications",
            "version": "1.0.0",
            "description": "State changes the server pushes on `/ws`. Every message is a JSON text frame \
    with a `type` tag and absolute values, never a delta. All messages travel on one ordered channel. The \
    server sends no initial state: read it over the REST API, open the socket first and apply what arrives \
    while you read. A client that falls behind is closed with code 1013 and has to reconnect and read the \
    state again. A server that shuts down closes with 1001. The server pings every 30 seconds."
        },
        "servers": {
            "snapdog": {
                "host": "{host}:{port}",
                "protocol": "ws",
                "pathname": "/ws",
                "description": "Use `wss` when the server has TLS enabled.",
                "variables": {
                    "host": { "description": "Host name or address of the SnapDog server" },
                    "port": { "default": "5555" }
                },
                "security": [{ "$ref": "#/components/securitySchemes/bearer" }]
            }
        },
        "channels": {
            "notifications": {
                "address": "/ws",
                "messages": refs
            }
        },
        "operations": {
            "receiveNotifications": {
                "action": "receive",
                "channel": { "$ref": "#/channels/notifications" },
                "messages": op_refs
            }
        },
        "components": {
            "messages": messages,
            "schemas": {
                "EqConfig": component::<snapdog_common::EqConfig>(),
                "EqBand": component::<snapdog_common::EqBand>(),
                "FilterType": component::<snapdog_common::FilterType>(),
                "RepeatMode": component::<snapdog_common::RepeatMode>(),
            },
            "securitySchemes": {
                "bearer": {
                    "type": "httpApiKey",
                    "name": "token",
                    "in": "query",
                    "description": "A configured API key. Send it as `Authorization: Bearer <key>` \
    or, for a client that cannot set headers on the upgrade request, as the `token` query parameter. \
    Not needed when the server has no `api_keys`."
                }
            }
        }
    })
}

/// The document as pretty-printed JSON with a trailing newline, as it is checked in.
///
/// # Panics
///
/// Never in practice: a `serde_json::Value` always serialises.
#[must_use]
pub fn document_json() -> String {
    let mut json = serde_json::to_string_pretty(&document()).expect("the document serialises");
    json.push('\n');
    json
}

fn component<T: ToSchema>() -> Value {
    serde_json::to_value(T::schema()).expect("the schema serialises")
}

/// The `type` tag of a variant. A variant with a flattened field is an `allOf` of its own
/// object and the flattened schema, so the tag sits in one of the parts.
fn tag_of(variant: &Value) -> String {
    let parts = variant["allOf"].as_array().cloned().unwrap_or_default();
    std::iter::once(variant)
        .chain(parts.iter())
        .find_map(|part| part["properties"]["type"]["enum"][0].as_str())
        .expect("every notification variant carries a type tag")
        .to_string()
}
