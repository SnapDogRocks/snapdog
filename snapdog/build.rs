// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Build script:
//! - Compiles the `WebUI` (Next.js static export) so rust-embed can bundle the assets.
//! - Embeds Windows icon and version metadata on Windows targets.

use std::path::Path;
use std::process::Command;

fn main() {
    build_webui();
    embed_windows_resource();
}

fn build_webui() {
    let webui_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../webui");
    let out_dir = webui_dir.join("out");

    // Re-run if webui sources change
    println!("cargo::rerun-if-changed=../webui/src");
    println!("cargo::rerun-if-changed=../webui/package.json");
    println!("cargo::rerun-if-changed=../webui/package-lock.json");
    println!("cargo::rerun-if-changed=../webui/next.config.ts");
    println!("cargo::rerun-if-changed=../webui/tsconfig.json");
    println!("cargo::rerun-if-changed=../webui/messages");

    // Skip if already built (CI) or explicitly disabled
    if out_dir.join("index.html").exists() {
        return;
    }
    if std::env::var("SKIP_WEBUI_BUILD").is_ok() {
        std::fs::create_dir_all(&out_dir).ok();
        std::fs::write(out_dir.join("index.html"), "<!-- placeholder -->").ok();
        return;
    }

    let status = Command::new("npm")
        .arg("ci")
        .current_dir(&webui_dir)
        .status()
        .expect("failed to run `npm ci` — is npm installed?");
    assert!(status.success(), "`npm ci` failed with {status}");

    let status = Command::new("npm")
        .args(["run", "build"])
        .current_dir(&webui_dir)
        .status()
        .expect("failed to run `npm run build` — is npm installed?");
    assert!(status.success(), "`npm run build` failed with {status}");
}

#[cfg(target_os = "windows")]
fn embed_windows_resource() {
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../assets/snapdog.ico");
    res.set("ProductName", "SnapDog");
    res.set("FileDescription", "Multi-zone audio controller");
    res.set("LegalCopyright", "Copyright © 2026 Fabian Schmieder");
    res.compile().expect("Failed to compile Windows resources");
}

#[cfg(not(target_os = "windows"))]
const fn embed_windows_resource() {}
