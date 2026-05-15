// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Build script: embeds Windows icon and version metadata on Windows targets.

fn main() {
    embed_windows_resource();
}

#[cfg(target_os = "windows")]
fn embed_windows_resource() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../assets/snapdog.ico");
    res.set("ProductName", "SnapDog Client");
    res.set("FileDescription", "SnapDog multiroom audio client");
    res.set("LegalCopyright", "Copyright © 2026 Fabian Schmieder");
    res.compile().expect("Failed to compile Windows resources");
}

#[cfg(not(target_os = "windows"))]
const fn embed_windows_resource() {}
