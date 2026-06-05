// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Typed API error responses.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// Standard REST API error response body.
#[derive(Serialize, utoipa::ToSchema)]
pub struct ErrorBody {
    /// High-level machine-readable error classification.
    pub error: &'static str,
    /// Detailed human-readable explanation of the error.
    pub message: String,
}

/// API error with JSON body.
pub enum ApiError {
    /// Resource not found (404).
    NotFound(&'static str),
    /// Invalid request parameters (400).
    BadRequest(String),
    /// Request conflicts with current server state (409).
    Conflict(&'static str),
    /// Request understood but cannot be processed (422).
    Unprocessable(String),
    /// Required service not configured (503).
    ServiceUnavailable(&'static str),
    /// Upstream service error (502).
    BadGateway(String),
    /// Unexpected internal error (500).
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error, message) = match self {
            Self::NotFound(what) => (
                StatusCode::NOT_FOUND,
                "not_found",
                format!("{what} not found"),
            ),
            Self::BadRequest(msg) => (StatusCode::BAD_REQUEST, "bad_request", msg),
            Self::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.into()),
            Self::Unprocessable(msg) => (StatusCode::UNPROCESSABLE_ENTITY, "unprocessable", msg),
            Self::ServiceUnavailable(what) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                format!("{what} not configured"),
            ),
            Self::BadGateway(msg) => (StatusCode::BAD_GATEWAY, "bad_gateway", msg),
            Self::Internal(msg) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error", msg),
        };
        (status, Json(ErrorBody { error, message })).into_response()
    }
}
