// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Optional API key authentication middleware.

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Middleware that checks `Authorization: Bearer <key>` on API routes.
pub async fn require_api_key(request: Request, next: Next) -> Response {
    let Some(keys) = request.extensions().get::<ApiKeys>().cloned() else {
        return next.run(request).await;
    };

    let header_token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    let query_token = if request.uri().path() == "/ws" {
        request.uri().query().and_then(|q| {
            url::form_urlencoded::parse(q.as_bytes())
                .find_map(|(key, value)| (key == "token").then(|| value.into_owned()))
        })
    } else {
        None
    };

    let authorized = header_token.is_some_and(|t| keys.0.iter().any(|k| k == t))
        || query_token
            .as_deref()
            .is_some_and(|t| keys.0.iter().any(|k| k == t));

    if authorized {
        next.run(request).await
    } else {
        (StatusCode::UNAUTHORIZED, "Invalid or missing API key").into_response()
    }
}

/// Extension type to pass API keys into the middleware.
#[derive(Clone)]
pub struct ApiKeys(pub Vec<String>);
