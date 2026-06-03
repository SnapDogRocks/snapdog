// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `OpenAPI` documentation registry.

use utoipa::OpenApi;

use crate::api::routes::system::{self, SystemStatus, VersionInfo};

/// Registry of all endpoints and schemas exposed in the REST API.
#[derive(OpenApi)]
#[openapi(
    paths(
        system::get_status,
        system::get_version,
    ),
    components(
        schemas(SystemStatus, VersionInfo)
    ),
    tags(
        (name = "system", description = "System and platform administration endpoints")
    ),
    info(
        title = "SnapDog REST API",
        version = "1.0.0",
        description = "SnapDog Multi-zone synchronized audio controller API"
    )
)]
pub struct ApiDoc;
