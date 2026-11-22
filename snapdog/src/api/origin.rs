// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Refuse requests that a web page on another origin sends.
//!
//! A browser adds `Origin` to cross-site requests and to every WebSocket handshake. Without
//! this check a page opened anywhere on the network could send the body-less POSTs (play,
//! pause, `updates/apply`) to a server without an API key and read the WebSocket event
//! stream. Native clients (the apps, Home Assistant, `curl`) send no `Origin` and are not
//! affected. The web interface is served by this server, so its `Origin` matches `Host`.
//!
//! The check compares `Origin` with `Host`, or with `X-Forwarded-Host` behind a reverse
//! proxy that rewrites `Host`. It does not stop DNS rebinding, where a page reaches the
//! server under its own name: `Origin` and `Host` agree there. An API key does.

use axum::extract::Request;
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::error::ErrorBody;

/// Middleware: 403 for a request whose `Origin` names another host than the request.
pub async fn reject_cross_origin(req: Request, next: Next) -> Response {
    if let Some(origin) = req.headers().get(header::ORIGIN)
        && !origin_matches(origin.to_str().unwrap_or_default(), req.headers())
    {
        tracing::warn!(
            origin = %origin.to_str().unwrap_or("?"),
            path = %req.uri().path(),
            "Refused a cross-origin request"
        );
        return (
            StatusCode::FORBIDDEN,
            axum::Json(ErrorBody {
                error: "cross_origin",
                message: "requests from another origin are not accepted".into(),
            }),
        )
            .into_response();
    }
    next.run(req).await
}

/// Whether `origin` (scheme, host, port) is the host the request was sent to.
fn origin_matches(origin: &str, headers: &HeaderMap) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false; // includes the opaque origin "null"
    };
    let (Some(host), Some(port)) = (url.host_str(), url.port_or_known_default()) else {
        return false;
    };
    let default_port = url::Url::parse(&format!("{}://x", url.scheme()))
        .ok()
        .and_then(|u| u.port_or_known_default());
    [header::HOST.as_str(), "x-forwarded-host"]
        .iter()
        .filter_map(|name| headers.get(*name)?.to_str().ok())
        // A proxy may list several hosts; the first is the one the client asked for.
        .filter_map(|value| value.split(',').next().map(str::trim))
        .any(|authority| same_authority(authority, host, port, default_port))
}

/// Compare a `Host` value (`name`, `name:port`, `[v6]:port`) with an origin's host and port.
fn same_authority(authority: &str, host: &str, port: u16, default_port: Option<u16>) -> bool {
    let Ok(parsed) = url::Url::parse(&format!("http://{authority}")) else {
        return false;
    };
    let Some(name) = parsed.host_str() else {
        return false;
    };
    // `Url` drops a port that is the default of its scheme, so read it from the text.
    let explicit_port = authority
        .rsplit_once(':')
        .filter(|(_, p)| !p.contains(']'))
        .and_then(|(_, p)| p.parse::<u16>().ok());
    let authority_port = explicit_port.or(default_port);
    name.eq_ignore_ascii_case(host) && authority_port == Some(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (k, v) in pairs {
            map.insert(
                axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                v.parse().unwrap(),
            );
        }
        map
    }

    #[test]
    fn the_same_host_and_port_match() {
        let h = headers(&[("host", "snapdog.local:5555")]);
        assert!(origin_matches("http://snapdog.local:5555", &h));
        assert!(origin_matches("http://SnapDog.local:5555", &h));
        let h = headers(&[("host", "192.168.1.10")]);
        assert!(origin_matches("http://192.168.1.10", &h));
        let h = headers(&[("host", "[fd00::1]:5555")]);
        assert!(origin_matches("http://[fd00::1]:5555", &h));
        let h = headers(&[("host", "music.example")]);
        assert!(
            origin_matches("https://music.example", &h),
            "443 is implied on both sides"
        );
    }

    #[test]
    fn another_host_port_or_null_does_not_match() {
        let h = headers(&[("host", "snapdog.local:5555")]);
        for origin in [
            "http://evil.example",
            "http://snapdog.local:8080",
            "http://snapdog.local",
            "null",
            "",
        ] {
            assert!(!origin_matches(origin, &h), "{origin}");
        }
    }

    #[test]
    fn a_reverse_proxy_is_matched_through_x_forwarded_host() {
        let h = headers(&[
            ("host", "127.0.0.1:5555"),
            ("x-forwarded-host", "music.example"),
        ]);
        assert!(origin_matches("https://music.example", &h));
        assert!(!origin_matches("https://evil.example", &h));
    }
}
