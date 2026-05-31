// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! `org.mpris.MediaPlayer2` root interface.

/// MPRIS2 root interface — identity and capabilities.
pub struct RootInterface {
    identity: String,
}

impl RootInterface {
    pub fn new(zone_name: &str) -> Self {
        Self {
            identity: format!("SnapDog — {zone_name}"),
        }
    }
}

#[allow(clippy::unused_self, clippy::missing_const_for_fn)]
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl RootInterface {
    #[zbus(property)]
    fn identity(&self) -> &str {
        &self.identity
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }

    fn quit(&self) {}

    fn raise(&self) {}
}
