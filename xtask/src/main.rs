// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Generate monolithic ETS XML for `SnapDog` KNX product database.
//!
//! Uses GO definitions from `snapdog::knx::group_objects` (SSOT) and outputs
//! a complete ETS-compatible XML that `knx-prod` can convert to .knxprod.

// Pedantic lints allowed crate-wide: XML generation uses intentional casts for
// memory sizes, long functions are inherent to ETS XML structure, and must_use
// on internal helpers is noise.
#![allow(clippy::cast_possible_truncation)]

use std::fmt::Write as _;

use snapdog::knx::group_objects::{
    CGO_CONNECTED, CGO_LATENCY, CGO_LATENCY_STATUS, CGO_MUTE, CGO_MUTE_STATUS, CGO_MUTE_TOGGLE,
    CGO_VOLUME, CGO_VOLUME_DIM, CGO_VOLUME_STATUS, CGO_ZONE, CGO_ZONE_STATUS, CLIENT_GO_COUNT,
    CLIENT_GOS, GoDefinition, MAX_CLIENTS, MAX_ZONES, ZGO_CONTROL_STATUS, ZGO_MUTE,
    ZGO_MUTE_STATUS, ZGO_MUTE_TOGGLE, ZGO_PAUSE, ZGO_PLAY, ZGO_PLAYLIST, ZGO_PLAYLIST_NEXT,
    ZGO_PLAYLIST_PREVIOUS, ZGO_PLAYLIST_STATUS, ZGO_PRESENCE, ZGO_PRESENCE_ENABLE,
    ZGO_PRESENCE_SOURCE_OVERRIDE, ZGO_PRESENCE_TIMEOUT, ZGO_PRESENCE_TIMER_ACTIVE, ZGO_REPEAT,
    ZGO_REPEAT_STATUS, ZGO_REPEAT_TOGGLE, ZGO_SHUFFLE, ZGO_SHUFFLE_STATUS, ZGO_SHUFFLE_TOGGLE,
    ZGO_STOP, ZGO_TRACK_ALBUM, ZGO_TRACK_ARTIST, ZGO_TRACK_NEXT, ZGO_TRACK_PLAYING,
    ZGO_TRACK_PREVIOUS, ZGO_TRACK_PROGRESS, ZGO_TRACK_REPEAT, ZGO_TRACK_REPEAT_STATUS,
    ZGO_TRACK_REPEAT_TOGGLE, ZGO_TRACK_TITLE, ZGO_VOLUME, ZGO_VOLUME_DIM, ZGO_VOLUME_STATUS,
    ZONE_GO_COUNT, ZONE_GOS, mem,
};

const AID: &str = "M-00FA_A-FF01-01-0000";
const MFR: &str = "M-00FA";

fn main() {
    let cmd = std::env::args().nth(1).unwrap_or_default();
    match cmd.as_str() {
        "ci" => ci(),
        "gen-api-spec" => gen_api_spec(),
        "knxprod" | "" => knxprod(),
        arg if std::path::Path::new(arg)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("xml")) =>
        {
            knxprod();
        } // backward compat: `cargo xtask path.xml`
        _ => {
            eprintln!("Usage: cargo xtask <command>");
            eprintln!("Commands:");
            eprintln!("  knxprod [path]  Generate ETS XML and .knxprod (default)");
            eprintln!("  gen-api-spec    Generate OpenAPI JSON specification");
            eprintln!("  ci              Run all CI checks locally");
            std::process::exit(1);
        }
    }
}

/// Generate `OpenAPI` specification file using `utoipa` from `snapdog` crate.
fn gen_api_spec() {
    use snapdog::api::openapi::ApiDoc;
    use utoipa::OpenApi;

    let json = ApiDoc::openapi()
        .to_pretty_json()
        .expect("failed to serialize OpenAPI JSON");
    let out_path = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "openapi.json".to_string());

    if let Some(parent) = std::path::Path::new(&out_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).expect("failed to create output directory");
    }

    std::fs::write(&out_path, json).expect("failed to write OpenAPI JSON file");
    eprintln!("✅ Generated OpenAPI specification at: {out_path}");
}

/// Run all CI checks locally (mirrors .github/workflows/ci.yml).
fn ci() {
    let checks: &[(&str, &[&str])] = &[
        ("Formatting", &["cargo", "fmt", "--all", "--check"]),
        (
            "Clippy",
            &["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
        ),
        ("Unit tests", &["cargo", "test", "--workspace"]),
        (
            "Integration tests",
            &[
                "cargo",
                "test",
                "-p",
                "snapdog",
                "--no-default-features",
                "--features",
                "snapcast-process",
                "--test",
                "integration",
                "--",
                "--test-threads=1",
            ],
        ),
        ("WebUI build", &["npm", "run", "build"]),
    ];

    let mut failed = Vec::new();
    for (name, args) in checks {
        eprintln!("\n\x1b[1;34m▶ {name}\x1b[0m");
        let status = std::process::Command::new(args[0])
            .args(&args[1..])
            .current_dir(if *name == "WebUI build" { "webui" } else { "." })
            .status();
        match status {
            Ok(s) if s.success() => eprintln!("\x1b[32m  ✓ {name}\x1b[0m"),
            _ => {
                eprintln!("\x1b[31m  ✗ {name}\x1b[0m");
                failed.push(*name);
            }
        }
    }

    eprintln!();
    if failed.is_empty() {
        eprintln!("\x1b[1;32m✓ All CI checks passed\x1b[0m");
    } else {
        eprintln!("\x1b[1;31m✗ Failed: {}\x1b[0m", failed.join(", "));
        std::process::exit(1);
    }
}

fn knxprod() {
    // Accept path as: `cargo xtask knxprod path.xml` (arg 2) or `cargo xtask path.xml` (arg 1)
    let xml_path = std::env::args()
        .nth(2)
        .or_else(|| {
            std::env::args().nth(1).filter(|a| {
                std::path::Path::new(a)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("xml"))
            })
        })
        .unwrap_or_else(|| "knx/snapdog.xml".into());
    let knxprod_path = xml_path.replace(".xml", ".knxprod");

    // Step 1: Generate ETS XML, then normalise readable string ids to the pure
    // integers ETS parses at import (renumber + structural sanity) via knx-rs-prod —
    // so `cargo xtask knxprod` emits ETS-importable XML directly.
    let xml = generate_xml();
    assert_refs_resolve(&xml);
    let xml = knx_rs_prod::normalize_ids(&xml)
        .unwrap_or_else(|e| panic!("failed to normalise ids for ETS import: {e}"));
    std::fs::write(&xml_path, xml).expect("failed to write XML");
    eprintln!(
        "  Generated {xml_path} ({} zones × {} COs + {} clients × {} COs = {} COs)",
        MAX_ZONES,
        ZONE_GOS.len(),
        MAX_CLIENTS,
        CLIENT_GOS.len(),
        MAX_ZONES * ZONE_GOS.len() + MAX_CLIENTS * CLIENT_GOS.len()
    );

    // Step 2: Generate .knxprod (signed ZIP archive for ETS import)
    let xml_file = std::path::Path::new(&xml_path);
    let knxprod_file = std::path::Path::new(&knxprod_path);
    match knx_rs_prod::generate_knxprod(xml_file, knxprod_file) {
        Ok(metadata) => {
            let size = std::fs::metadata(knxprod_file).map_or(0, |m| m.len());
            eprintln!(
                "✅ Generated {knxprod_path} ({size} bytes, app: {})",
                metadata.application_id
            );
        }
        Err(e) => {
            eprintln!("❌ Failed to generate {knxprod_path}: {e}");
            std::process::exit(1);
        }
    }
}

struct CoGroup {
    title_de: &'static str,
    title_en: &'static str,
    indices: &'static [usize],
}

const ZONE_GROUPS: &[CoGroup] = &[
    CoGroup {
        title_de: "Wiedergabe",
        title_en: "Playback",
        indices: &[
            ZGO_PLAY,
            ZGO_PAUSE,
            ZGO_STOP,
            ZGO_TRACK_NEXT,
            ZGO_TRACK_PREVIOUS,
            ZGO_CONTROL_STATUS,
            ZGO_TRACK_PLAYING,
        ],
    },
    CoGroup {
        title_de: "Lautstärke",
        title_en: "Volume",
        indices: &[
            ZGO_VOLUME,
            ZGO_VOLUME_STATUS,
            ZGO_VOLUME_DIM,
            ZGO_MUTE,
            ZGO_MUTE_STATUS,
            ZGO_MUTE_TOGGLE,
        ],
    },
    CoGroup {
        title_de: "Zufallswiedergabe / Wiederholung",
        title_en: "Shuffle / Repeat",
        indices: &[
            ZGO_SHUFFLE,
            ZGO_SHUFFLE_STATUS,
            ZGO_SHUFFLE_TOGGLE,
            ZGO_REPEAT,
            ZGO_REPEAT_STATUS,
            ZGO_REPEAT_TOGGLE,
            ZGO_TRACK_REPEAT,
            ZGO_TRACK_REPEAT_STATUS,
            ZGO_TRACK_REPEAT_TOGGLE,
        ],
    },
    CoGroup {
        title_de: "Playlist",
        title_en: "Playlist",
        indices: &[
            ZGO_PLAYLIST,
            ZGO_PLAYLIST_STATUS,
            ZGO_PLAYLIST_NEXT,
            ZGO_PLAYLIST_PREVIOUS,
        ],
    },
    CoGroup {
        title_de: "Titelinformationen",
        title_en: "Track Info",
        indices: &[
            ZGO_TRACK_TITLE,
            ZGO_TRACK_ARTIST,
            ZGO_TRACK_ALBUM,
            ZGO_TRACK_PROGRESS,
        ],
    },
    CoGroup {
        title_de: "Präsenz",
        title_en: "Presence",
        indices: &[
            ZGO_PRESENCE,
            ZGO_PRESENCE_ENABLE,
            ZGO_PRESENCE_TIMEOUT,
            ZGO_PRESENCE_TIMER_ACTIVE,
            ZGO_PRESENCE_SOURCE_OVERRIDE,
        ],
    },
];

const CLIENT_GROUPS: &[CoGroup] = &[
    CoGroup {
        title_de: "Lautstärke",
        title_en: "Volume",
        indices: &[
            CGO_VOLUME,
            CGO_VOLUME_STATUS,
            CGO_VOLUME_DIM,
            CGO_MUTE,
            CGO_MUTE_STATUS,
            CGO_MUTE_TOGGLE,
        ],
    },
    CoGroup {
        title_de: "Latenz und Zone",
        title_en: "Latency and Zone",
        indices: &[CGO_LATENCY, CGO_LATENCY_STATUS, CGO_ZONE, CGO_ZONE_STATUS],
    },
    CoGroup {
        title_de: "Status",
        title_en: "Status",
        indices: &[CGO_CONNECTED],
    },
];

// ── XML generation ────────────────────────────────────────────

fn generate_xml() -> String {
    let mut x = String::with_capacity(128 * 1024);
    w(&mut x, r#"<?xml version="1.0" encoding="utf-8"?>"#);
    w(
        &mut x,
        r#"<KNX xmlns="http://knx.org/xml/project/20" CreatedBy="SnapDog xtask" ToolVersion="1.0">"#,
    );
    w(&mut x, "  <ManufacturerData>");
    w(&mut x, &format!(r#"    <Manufacturer RefId="{MFR}">"#));

    write_catalog(&mut x);
    write_application_program(&mut x);
    write_hardware(&mut x);

    w(&mut x, "    </Manufacturer>");
    w(&mut x, "  </ManufacturerData>");
    w(&mut x, "</KNX>");
    x
}

fn write_catalog(x: &mut String) {
    w(x, "      <Catalog>");
    w(
        x,
        &format!(
            r#"        <CatalogSection Id="{MFR}_CS-SnapDog" Name="SnapDog" Number="SnapDog" DefaultLanguage="de-DE">"#
        ),
    );
    w(
        x,
        &format!(
            r#"          <CatalogItem Id="{MFR}_H-0xFF01-1_HP-FF01-01-0000_CI-0xFF01-1" Name="SnapDog" Number="1" ProductRefId="{MFR}_H-0xFF01-1_P-0xFF01" Hardware2ProgramRefId="{MFR}_H-0xFF01-1_HP-FF01-01-0000" DefaultLanguage="de-DE" />"#
        ),
    );
    w(x, "        </CatalogSection>");
    w(x, "      </Catalog>");
}

/// Bump this when the ETS program changes (memory layout, parameters, com-objects).
/// ETS keys the program by `ApplicationNumber` + `ApplicationVersion`; re-importing an
/// unchanged version shows the *cached* content, so a fresh import needs a higher number.
/// Override for throwaway test builds with `SNAPDOG_APP_VERSION=<n>` (never go below the
/// last-imported version, and never back to 1).
const APP_VERSION: u32 = 8;

fn app_version() -> u32 {
    std::env::var("SNAPDOG_APP_VERSION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(APP_VERSION)
}

fn write_application_program(x: &mut String) {
    let version = app_version();
    // ReplacesVersions lists every prior version so ETS offers an in-place upgrade.
    let replaces = (0..version)
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(" ");
    w(x, "      <ApplicationPrograms>");
    w(
        x,
        &format!(
            r#"        <ApplicationProgram Id="{AID}" ProgramType="ApplicationProgram" MaskVersion="MV-07B0" Name="SnapDog" LoadProcedureStyle="MergedProcedure" PeiType="0" DefaultLanguage="de-DE" DynamicTableManagement="false" Linkable="true" MinEtsVersion="5.0" IPConfig="Custom" ApplicationNumber="65281" ApplicationVersion="{version}" ReplacesVersions="{replaces}">"#
        ),
    );
    w(x, "          <Static>");

    write_code_segment(x);
    write_parameter_types(x);
    write_parameters(x);
    write_parameter_refs(x);
    write_com_objects(x);
    write_com_object_refs(x);
    write_tables(x);
    write_load_procedures(x);
    write_options(x);

    w(x, "          </Static>");
    write_dynamic(x);
    w(x, "        </ApplicationProgram>");
    w(x, "      </ApplicationPrograms>");
}

fn write_code_segment(x: &mut String) {
    let memory_size = mem::TOTAL;
    w(x, "            <Code>");
    w(
        x,
        &format!(
            r#"              <RelativeSegment Id="{AID}_RS-04-00000" Name="Parameters" Offset="0" Size="{memory_size}" LoadStateMachine="4" />"#
        ),
    );
    w(x, "            </Code>");
}

#[allow(clippy::too_many_lines)]
fn write_parameter_types(x: &mut String) {
    w(x, "            <ParameterTypes>");
    // Bool
    pt_enum(x, "YesNo", 8, &[("Nein", 0), ("Ja", 1)]);
    // Text types
    pt_text(x, "Name", 160);
    pt_text(x, "Text20", 160);
    pt_text(x, "Text40", 320);
    pt_text(x, "Text60", 480);
    pt_text(x, "Text80", 640);
    pt_text(x, "Text64", 512); // PSK
    pt_text(x, "MAC", 136); // 17 chars
    // Numeric
    pt_num(x, "Percent", 8, "unsignedInt", 0, 100);
    pt_num(x, "UInt8", 8, "unsignedInt", 0, 255);
    pt_num(x, "UInt16", 16, "unsignedInt", 0, 65535);
    // Enums
    pt_enum(
        x,
        "LogLevel",
        8,
        &[
            ("Error", 0),
            ("Warn", 1),
            ("Info", 2),
            ("Debug", 3),
            ("Trace", 4),
        ],
    );
    pt_enum(
        x,
        "SampleRate",
        8,
        &[
            ("44100 Hz", 0),
            ("48000 Hz", 1),
            ("88200 Hz", 2),
            ("96000 Hz", 3),
            ("176400 Hz", 4),
            ("192000 Hz", 5),
        ],
    );
    pt_enum(
        x,
        "BitDepth",
        8,
        &[("16 Bit", 0), ("24 Bit", 1), ("32 Bit", 2)],
    );
    pt_enum(
        x,
        "Codec",
        8,
        &[
            ("PCM (unkomprimiert)", 0),
            ("FLAC (verlustfrei)", 1),
            ("f32lz4", 2),
            ("f32lz4e (verschlüsselt)", 3),
        ],
    );
    pt_enum(
        x,
        "SourceConflict",
        8,
        &[("Letzte Quelle gewinnt", 0), ("Empfänger hat Vorrang", 1)],
    );
    pt_enum(
        x,
        "ZoneSelect",
        8,
        &[
            ("Zone 1", 1),
            ("Zone 2", 2),
            ("Zone 3", 3),
            ("Zone 4", 4),
            ("Zone 5", 5),
            ("Zone 6", 6),
            ("Zone 7", 7),
            ("Zone 8", 8),
            ("Zone 9", 9),
            ("Zone 10", 10),
        ],
    );
    pt_enum(
        x,
        "NumZones",
        8,
        &[
            ("1 Zone", 1),
            ("2 Zonen", 2),
            ("3 Zonen", 3),
            ("4 Zonen", 4),
            ("5 Zonen", 5),
            ("6 Zonen", 6),
            ("7 Zonen", 7),
            ("8 Zonen", 8),
            ("9 Zonen", 9),
            ("10 Zonen", 10),
        ],
    );
    pt_enum(
        x,
        "NumClients",
        8,
        &[
            ("1 Client", 1),
            ("2 Clients", 2),
            ("3 Clients", 3),
            ("4 Clients", 4),
            ("5 Clients", 5),
            ("6 Clients", 6),
            ("7 Clients", 7),
            ("8 Clients", 8),
            ("9 Clients", 9),
            ("10 Clients", 10),
        ],
    );
    // Radio count 1..=MAX_RADIOS (built dynamically to avoid a 50-line literal).
    let radio_labels: Vec<String> = (1..=mem::MAX_RADIOS)
        .map(|n| {
            if n == 1 {
                "1 Sender".into()
            } else {
                format!("{n} Sender")
            }
        })
        .collect();
    let radio_pairs: Vec<(&str, u16)> = radio_labels
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), (i + 1) as u16))
        .collect();
    pt_enum(x, "NumRadios", 8, &radio_pairs);
    w(x, "            </ParameterTypes>");
}

fn pt_enum(x: &mut String, name: &str, bits: u16, values: &[(&str, u16)]) {
    w(
        x,
        &format!(r#"              <ParameterType Id="{AID}_PT-{name}" Name="{name}">"#),
    );
    w(
        x,
        &format!(r#"                <TypeRestriction Base="Value" SizeInBit="{bits}">"#),
    );
    for (i, (text, val)) in values.iter().enumerate() {
        w(
            x,
            &format!(
                r#"                  <Enumeration Text="{text}" Value="{val}" Id="{AID}_PT-{name}_EN-{i}" />"#
            ),
        );
    }
    w(x, "                </TypeRestriction>");
    w(x, "              </ParameterType>");
}

fn pt_text(x: &mut String, name: &str, bits: u16) {
    w(
        x,
        &format!(r#"              <ParameterType Id="{AID}_PT-{name}" Name="{name}">"#),
    );
    w(
        x,
        &format!(r#"                <TypeText SizeInBit="{bits}" />"#),
    );
    w(x, "              </ParameterType>");
}

fn pt_num(x: &mut String, name: &str, bits: u16, typ: &str, min: u32, max: u32) {
    w(
        x,
        &format!(r#"              <ParameterType Id="{AID}_PT-{name}" Name="{name}">"#),
    );
    w(
        x,
        &format!(
            r#"                <TypeNumber SizeInBit="{bits}" Type="{typ}" minInclusive="{min}" maxInclusive="{max}" />"#
        ),
    );
    w(x, "              </ParameterType>");
}

#[allow(clippy::too_many_lines)] // Repetitive XML parameter generation — not decomposable
fn write_parameters(x: &mut String) {
    w(x, "            <Parameters>");
    // Byte offsets come straight from `mem::` (the single source of truth the firmware
    // reads); `spans` collects them so we can assert the params tile the layout exactly.
    let mut spans: Vec<(usize, usize)> = Vec::new();

    // ── Zone count ────────────────────────────────────────────
    // A single "number of zones" dropdown (shown under Allgemein) drives how many zone
    // blocks ETS displays and how many zones the firmware activates. Replaces the former
    // per-zone active flags.
    param_mem(
        x,
        "G",
        "000",
        "NumZones",
        "NumZones",
        "Anzahl Zonen",
        "10",
        mem::NUM_ZONES,
        8,
        &mut spans,
    );

    // ── Zone parameters ───────────────────────────────────────
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "002",
            "DefVol",
            "Percent",
            "Standard-Lautstärke",
            "50",
            mem::ZONE_DEF_VOL + i,
            8,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "003",
            "MaxVol",
            "Percent",
            "Max. Lautstärke",
            "100",
            mem::ZONE_MAX_VOL + i,
            8,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "004",
            "AirPlay",
            "YesNo",
            "AirPlay aktiviert",
            "1",
            mem::ZONE_AIRPLAY + i,
            8,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "005",
            "Spotify",
            "YesNo",
            "Spotify aktiviert",
            "1",
            mem::ZONE_SPOTIFY + i,
            8,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "006",
            "PresEn",
            "YesNo",
            "Präsenz aktiviert",
            "0",
            mem::ZONE_PRESENCE_EN + i,
            8,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "007",
            "PresTO",
            "UInt16",
            "Präsenz Auto-Off (s)",
            "900",
            mem::ZONE_PRESENCE_TO + i * 2,
            16,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "008",
            "PresSrc",
            "UInt8",
            "Präsenz-Quelle (0=keine, 1..20=Radio)",
            "0",
            mem::ZONE_PRES_SRC + i,
            8,
            &mut spans,
        );
    }
    // Sample rate and bit depth are the global Snapcast output format (not per-zone) —
    // emitted as global parameters below.

    // ── Client parameters ─────────────────────────────────────
    // A single "number of clients" dropdown (shown under Allgemein, next to Anzahl Zonen)
    // drives how many client blocks ETS displays and how many clients the firmware
    // activates. Mirrors NumZones; replaces the former per-client active flags.
    param_mem(
        x,
        "G",
        "003",
        "NumClients",
        "NumClients",
        "Anzahl Clients",
        "10",
        mem::NUM_CLIENTS,
        8,
        &mut spans,
    );
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        let i = c - 1;
        param_mem(
            x,
            &p,
            "002",
            "DefZone",
            "ZoneSelect",
            "Standard-Zone",
            "1",
            mem::CLIENT_DEF_ZONE + i,
            8,
            &mut spans,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        let i = c - 1;
        param_mem(
            x,
            &p,
            "003",
            "DefVol",
            "Percent",
            "Standard-Lautstärke",
            "100",
            mem::CLIENT_DEF_VOL + i,
            8,
            &mut spans,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        let i = c - 1;
        param_mem(
            x,
            &p,
            "004",
            "MaxVol",
            "Percent",
            "Max. Lautstärke",
            "100",
            mem::CLIENT_MAX_VOL + i,
            8,
            &mut spans,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        let i = c - 1;
        param_mem(
            x,
            &p,
            "005",
            "DefLat",
            "UInt8",
            "Standard-Latenz (ms)",
            "0",
            mem::CLIENT_DEF_LAT + i,
            8,
            &mut spans,
        );
    }

    // ── Global numeric parameters ───────────────────────────────
    param_mem(
        x,
        "G",
        "001",
        "HttpPort",
        "UInt16",
        "HTTP Port",
        "5555",
        mem::GLOBAL_HTTP_PORT,
        16,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "002",
        "LogLvl",
        "LogLevel",
        "Log Level",
        "2",
        mem::GLOBAL_LOG_LVL,
        8,
        &mut spans,
    );
    // Global audio output format (server-wide Snapcast format, not per-zone).
    param_mem(
        x,
        "G",
        "004",
        "SRate",
        "SampleRate",
        "Sample Rate",
        "1",
        mem::GLOBAL_SRATE,
        8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "005",
        "BitD",
        "BitDepth",
        "Bit Depth",
        "0",
        mem::GLOBAL_BITD,
        8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "006",
        "Codec",
        "Codec",
        "Codec",
        "1",
        mem::GLOBAL_CODEC,
        8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "007",
        "SrcConflict",
        "SourceConflict",
        "Quellen-Konflikt",
        "0",
        mem::GLOBAL_SRC_CONFLICT,
        8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "008",
        "ZoneFade",
        "UInt16",
        "Zonenwechsel-Fade (ms)",
        "200",
        mem::GLOBAL_ZONE_FADE,
        16,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "009",
        "SrcFade",
        "UInt16",
        "Quellenwechsel-Fade (ms)",
        "200",
        mem::GLOBAL_SRC_FADE,
        16,
        &mut spans,
    );

    // ── Radio count ───────────────────────────────────────────
    // A single "number of radios" dropdown (shown first on the Radiosender tab) drives how
    // many station sections ETS displays and how many the firmware activates. Mirrors
    // NumZones/NumClients; replaces the former per-radio active flags.
    param_mem(
        x,
        "G",
        "020",
        "NumRadios",
        "NumRadios",
        "Anzahl Radiosender",
        "10",
        mem::NUM_RADIOS,
        8,
        &mut spans,
    );

    // ── String parameters (offsets from mem::) ────────────────
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "000",
            "Name",
            "Name",
            "Zonenname",
            &format!("Zone {z}"),
            mem::ZONE_NAME + i * mem::ZONE_NAME_SIZE,
            mem::ZONE_NAME_SIZE as u16 * 8,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "010",
            "AirName",
            "Name",
            "AirPlay-Name (leer = Zonenname)",
            "",
            mem::ZONE_AIRPLAY_NAME + i * mem::ZONE_AIRPLAY_NAME_SIZE,
            mem::ZONE_AIRPLAY_NAME_SIZE as u16 * 8,
            &mut spans,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        let i = z - 1;
        param_mem(
            x,
            &p,
            "011",
            "SpotName",
            "Name",
            "Spotify-Name (leer = Zonenname)",
            "",
            mem::ZONE_SPOTIFY_NAME + i * mem::ZONE_SPOTIFY_NAME_SIZE,
            mem::ZONE_SPOTIFY_NAME_SIZE as u16 * 8,
            &mut spans,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        let i = c - 1;
        param_mem(
            x,
            &p,
            "000",
            "Name",
            "Name",
            "Clientname",
            &format!("Client {c}"),
            mem::CLIENT_NAME + i * mem::CLIENT_NAME_SIZE,
            mem::CLIENT_NAME_SIZE as u16 * 8,
            &mut spans,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        let i = c - 1;
        param_mem(
            x,
            &p,
            "010",
            "MAC",
            "MAC",
            "MAC-Adresse",
            "",
            mem::CLIENT_MAC + i * mem::CLIENT_MAC_SIZE,
            mem::CLIENT_MAC_SIZE as u16 * 8,
            &mut spans,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        let i = c - 1;
        param_mem(
            x,
            &p,
            "011",
            "Icon",
            "Name",
            "Icon (Emoji)",
            "",
            mem::CLIENT_ICON + i * mem::CLIENT_ICON_SIZE,
            mem::CLIENT_ICON_SIZE as u16 * 8,
            &mut spans,
        );
    }
    param_mem(
        x,
        "G",
        "010",
        "SubURL",
        "Text60",
        "Subsonic URL",
        "",
        mem::GLOBAL_SUB_URL,
        mem::GLOBAL_SUB_URL_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "011",
        "SubUser",
        "Text20",
        "Subsonic Benutzer",
        "",
        mem::GLOBAL_SUB_USER,
        mem::GLOBAL_SUB_USER_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "012",
        "SubPass",
        "Text20",
        "Subsonic Passwort",
        "",
        mem::GLOBAL_SUB_PASS,
        mem::GLOBAL_SUB_PASS_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "013",
        "MqttBrk",
        "Text40",
        "MQTT Broker",
        "",
        mem::GLOBAL_MQTT_BROKER,
        mem::GLOBAL_MQTT_BROKER_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "014",
        "MqttTop",
        "Text20",
        "MQTT Base Topic",
        "snapdog",
        mem::GLOBAL_MQTT_TOPIC,
        mem::GLOBAL_MQTT_TOPIC_SIZE as u16 * 8,
        &mut spans,
    );
    // Secrets (plaintext in ETS memory by product decision).
    param_mem(
        x,
        "G",
        "015",
        "MqttPass",
        "Text20",
        "MQTT Passwort",
        "",
        mem::GLOBAL_MQTT_PASS,
        mem::GLOBAL_MQTT_PASS_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "016",
        "AirPass",
        "Text20",
        "AirPlay Passwort",
        "",
        mem::GLOBAL_AIRPLAY_PASS,
        mem::GLOBAL_AIRPLAY_PASS_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "017",
        "Psk",
        "Text64",
        "Snapcast PSK (Verschlüsselung)",
        "",
        mem::GLOBAL_PSK,
        mem::GLOBAL_PSK_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "018",
        "ApiKey1",
        "Text40",
        "API-Key 1",
        "",
        mem::GLOBAL_API_KEY1,
        mem::GLOBAL_API_KEY_SIZE as u16 * 8,
        &mut spans,
    );
    param_mem(
        x,
        "G",
        "019",
        "ApiKey2",
        "Text40",
        "API-Key 2",
        "",
        mem::GLOBAL_API_KEY2,
        mem::GLOBAL_API_KEY_SIZE as u16 * 8,
        &mut spans,
    );
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        let i = r - 1;
        param_mem(
            x,
            &p,
            "000",
            "Name",
            "Text20",
            "Stationsname",
            "",
            mem::RADIO_NAME + i * mem::RADIO_NAME_SIZE,
            mem::RADIO_NAME_SIZE as u16 * 8,
            &mut spans,
        );
    }
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        let i = r - 1;
        param_mem(
            x,
            &p,
            "001",
            "URL",
            "Text80",
            "Stream URL",
            "",
            mem::RADIO_URL + i * mem::RADIO_URL_SIZE,
            mem::RADIO_URL_SIZE as u16 * 8,
            &mut spans,
        );
    }
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        let i = r - 1;
        param_mem(
            x,
            &p,
            "003",
            "Cover",
            "Text80",
            "Cover URL (optional)",
            "",
            mem::RADIO_COVER + i * mem::RADIO_COVER_SIZE,
            mem::RADIO_COVER_SIZE as u16 * 8,
            &mut spans,
        );
    }

    w(x, "            </Parameters>");

    // Single-source guard: every param sits at its mem:: offset, so the emitted spans must
    // tile [0, mem::TOTAL) with no gap or overlap. A wrong mem:: constant or size trips this.
    spans.sort_unstable();
    let mut cursor = 0usize;
    for (offset, bytes) in &spans {
        assert_eq!(
            *offset, cursor,
            "parameter memory gap/overlap: a param sits at offset {offset} but {cursor} was \
             expected — a param_mem offset disagrees with the mem:: layout",
        );
        cursor += bytes;
    }
    assert_eq!(
        cursor,
        mem::TOTAL,
        "parameter memory covers {cursor} bytes but mem::TOTAL is {}",
        mem::TOTAL
    );
    eprintln!("  Memory layout: {cursor} bytes used (single-sourced from mem::)");
}

/// Emit a memory-backed parameter inside a Union.
#[allow(clippy::too_many_arguments)]
fn param_mem(
    x: &mut String,
    prefix: &str,
    num: &str,
    name: &str,
    pt: &str,
    text: &str,
    default: &str,
    offset: usize,
    bits: u16,
    spans: &mut Vec<(usize, usize)>,
) {
    // The byte offset is single-sourced from `mem::` (the layout the firmware reads).
    // Record the span so the caller can assert the params tile mem:: exactly.
    spans.push((offset, (bits / 8) as usize));
    w(x, &format!(r#"              <Union SizeInBit="{bits}">"#));
    w(
        x,
        &format!(
            r#"                <Memory CodeSegment="{AID}_RS-04-00000" Offset="{offset}" BitOffset="0" />"#
        ),
    );
    w(
        x,
        &format!(
            r#"                <Parameter Id="{AID}_UP-{prefix}{num}" Name="{prefix}_{name}" Offset="0" BitOffset="0" ParameterType="{AID}_PT-{pt}" Text="{text}" Value="{default}" />"#
        ),
    );
    w(x, "              </Union>");
}

fn write_com_objects(x: &mut String) {
    w(x, "            <ComObjectTable>");
    for z in 1..=MAX_ZONES {
        for (i, go) in ZONE_GOS.iter().enumerate() {
            let num = (z - 1) * ZONE_GOS.len() + i + 1;
            write_com_object(
                x,
                &format!("Z{z:02}{i:03}"),
                &format!("Zone {z} {}", go.name),
                go,
                num,
            );
        }
    }
    for c in 1..=MAX_CLIENTS {
        for (i, go) in CLIENT_GOS.iter().enumerate() {
            let num = MAX_ZONES * ZONE_GOS.len() + (c - 1) * CLIENT_GOS.len() + i + 1;
            write_com_object(
                x,
                &format!("C{c:02}{i:03}"),
                &format!("Client {c} {}", go.name),
                go,
                num,
            );
        }
    }
    w(x, "            </ComObjectTable>");
}

/// KNX `DatapointType` id format: the `Dpt` Display is `1.001`, but the ETS schema wants an
/// IDREF into the master (`DPST-1-1`). Sub-numbers drop leading zeros.
fn dpst(dpt: impl std::fmt::Display) -> String {
    let s = dpt.to_string();
    match s.split_once('.') {
        Some((main, sub)) => format!(
            "DPST-{}-{}",
            main.parse::<u32>().unwrap_or(0),
            sub.parse::<u32>().unwrap_or(0)
        ),
        None => format!("DPT-{}", s.parse::<u32>().unwrap_or(0)),
    }
}

fn write_com_object(x: &mut String, id_suffix: &str, name: &str, go: &GoDefinition, number: usize) {
    let r = if go.flags.read { "Enabled" } else { "Disabled" };
    let wr = if go.flags.write {
        "Enabled"
    } else {
        "Disabled"
    };
    let t = if go.flags.transmit {
        "Enabled"
    } else {
        "Disabled"
    };
    let u = if go.flags.update {
        "Enabled"
    } else {
        "Disabled"
    };
    w(
        x,
        &format!(
            r#"              <ComObject Id="{AID}_O-{id_suffix}" Name="{name}" Number="{number}" Text="{}" FunctionText="{}" ObjectSize="{}" DatapointType="{}" Priority="Low" ReadFlag="{r}" WriteFlag="{wr}" CommunicationFlag="Enabled" TransmitFlag="{t}" UpdateFlag="{u}" ReadOnInitFlag="Disabled" />"#,
            go.name_de,
            go.name,
            go.size_str,
            dpst(go.dpt)
        ),
    );
}

/// ETS requires a `<ParameterRef>` for every parameter referenced in the Dynamic section
/// (a missing one surfaces as an opaque `NullReferenceException` on import). We reference the
/// zone/client name fields, the per-client active flags, and the global zone-count param;
/// emit a self-referential ref (`{id}_R-{id}`) matching what `write_dynamic` points at.
fn write_parameter_refs(x: &mut String) {
    // One `<ParameterRef>` per declared `<Parameter>` (1:1). Derived by scanning the
    // already-emitted `<Parameters>` so it stays in lock-step with `write_parameters` —
    // every parameter becomes referenceable, so any of them can be shown in the Dynamic
    // view (the config knobs are all wired in `write_dynamic`).
    let ids: Vec<String> = x
        .lines()
        .filter_map(|l| {
            l.trim_start()
                .strip_prefix("<Parameter Id=\"")
                .and_then(|r| r.split('"').next())
                .map(str::to_string)
        })
        .collect();
    w(x, "            <ParameterRefs>");
    for p in ids {
        w(
            x,
            &format!(r#"              <ParameterRef Id="{p}_R-{p}" RefId="{p}" />"#),
        );
    }
    w(x, "            </ParameterRefs>");
}

/// Every `ComObject` referenced in the Dynamic needs a `<ComObjectRef>`. Mirror the
/// `write_com_objects` numbering exactly so the `_R-<number>` ids line up.
fn write_com_object_refs(x: &mut String) {
    w(x, "            <ComObjectRefs>");
    for z in 1..=MAX_ZONES {
        for i in 0..ZONE_GOS.len() {
            let num = (z - 1) * ZONE_GOS.len() + i + 1;
            let id = format!("{AID}_O-Z{z:02}{i:03}");
            w(
                x,
                &format!(r#"              <ComObjectRef Id="{id}_R-{num}" RefId="{id}" />"#),
            );
        }
    }
    for c in 1..=MAX_CLIENTS {
        for i in 0..CLIENT_GOS.len() {
            let num = MAX_ZONES * ZONE_GOS.len() + (c - 1) * CLIENT_GOS.len() + i + 1;
            let id = format!("{AID}_O-C{c:02}{i:03}");
            w(
                x,
                &format!(r#"              <ComObjectRef Id="{id}_R-{num}" RefId="{id}" />"#),
            );
        }
    }
    w(x, "            </ComObjectRefs>");
}

/// Guard: every ref in the Dynamic section must resolve to a defined `<ParameterRef>`/
/// `<ComObjectRef>`, and every ref must point at an existing `<Parameter>`/`<ComObject>`. ETS
/// reports a dangling ref only as an opaque `NullReferenceException` on import — fail here.
fn assert_refs_resolve(xml: &str) {
    use std::collections::HashSet;
    // Attribute values are space-separated; the leading space avoids `RefId` matching
    // inside `ParamRefId`/`TextParameterRefId`.
    fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
        let key = format!(" {name}=\"");
        let start = line.find(&key)? + key.len();
        let rest = &line[start..];
        Some(&rest[..rest.find('"')?])
    }
    let (mut params, mut comobjs) = (HashSet::new(), HashSet::new());
    let (mut param_refs, mut co_refs) = (HashSet::new(), HashSet::new());
    let mut ref_targets: Vec<(&str, String, String)> = Vec::new(); // (kind, ref-id, target)

    for line in xml.lines() {
        let t = line.trim_start();
        if t.starts_with("<Parameter ")
            && let Some(id) = attr(t, "Id")
        {
            params.insert(id.to_string());
        } else if t.starts_with("<ComObject ")
            && let Some(id) = attr(t, "Id")
        {
            comobjs.insert(id.to_string());
        } else if t.starts_with("<ParameterRef ")
            && let (Some(id), Some(rid)) = (attr(t, "Id"), attr(t, "RefId"))
        {
            param_refs.insert(id.to_string());
            ref_targets.push(("Parameter", id.to_string(), rid.to_string()));
        } else if t.starts_with("<ComObjectRef ")
            && let (Some(id), Some(rid)) = (attr(t, "Id"), attr(t, "RefId"))
        {
            co_refs.insert(id.to_string());
            ref_targets.push(("ComObject", id.to_string(), rid.to_string()));
        }
    }

    let mut bad = Vec::new();
    for (kind, id, target) in &ref_targets {
        let defined = if *kind == "Parameter" {
            &params
        } else {
            &comobjs
        };
        if !defined.contains(target) {
            bad.push(format!("{kind}Ref {id} -> missing {kind} {target}"));
        }
    }
    for line in xml.lines() {
        let t = line.trim_start();
        for key in ["ParamRefId", "TextParameterRefId"] {
            if let Some(r) = attr(t, key)
                && !param_refs.contains(r)
            {
                bad.push(format!("{key}={r} -> no <ParameterRef>"));
            }
        }
        if t.starts_with("<ParameterRefRef ")
            && let Some(r) = attr(t, "RefId")
            && !param_refs.contains(r)
        {
            bad.push(format!("ParameterRefRef {r} -> no <ParameterRef>"));
        }
        if t.starts_with("<ComObjectRefRef ")
            && let Some(r) = attr(t, "RefId")
            && !co_refs.contains(r)
        {
            bad.push(format!("ComObjectRefRef {r} -> no <ComObjectRef>"));
        }
    }
    assert!(
        bad.is_empty(),
        "ETS ref integrity: {} dangling reference(s):\n{}",
        bad.len(),
        bad.join("\n")
    );
}

fn write_tables(x: &mut String) {
    w(x, r#"            <AddressTable MaxEntries="2047" />"#);
    w(x, r#"            <AssociationTable MaxEntries="2047" />"#);
}

fn write_load_procedures(x: &mut String) {
    let memory_size = mem::TOTAL;
    w(x, "            <LoadProcedures>");
    w(x, r#"              <LoadProcedure MergeId="1">"#);
    // InlineData is the device HardwareType (PID 78) — SSOT with the firmware
    // (`group_objects::HARDWARE_TYPE`), so the download-gate compare can't drift.
    let hardware_type_hex =
        snapdog::knx::group_objects::HARDWARE_TYPE
            .iter()
            .fold(String::new(), |mut s, b| {
                let _ = write!(s, "{b:02X}");
                s
            });
    w(
        x,
        &format!(
            r#"                <LdCtrlCompareProp InlineData="{hardware_type_hex}" ObjIdx="0" PropId="78">"#
        ),
    );
    w(
        x,
        &format!(r#"                  <OnError Cause="CompareMismatch" MessageRef="{AID}_M-1" />"#),
    );
    w(x, "                </LdCtrlCompareProp>");
    w(x, "              </LoadProcedure>");
    w(x, r#"              <LoadProcedure MergeId="2">"#);
    w(
        x,
        &format!(
            r#"                <LdCtrlRelSegment LsmIdx="4" Size="{memory_size}" Mode="1" Fill="0" AppliesTo="full" />"#
        ),
    );
    w(
        x,
        &format!(
            r#"                <LdCtrlRelSegment LsmIdx="4" Size="{memory_size}" Mode="0" Fill="0" AppliesTo="par" />"#
        ),
    );
    w(x, "              </LoadProcedure>");
    w(x, r#"              <LoadProcedure MergeId="4">"#);
    w(
        x,
        &format!(
            r#"                <LdCtrlWriteRelMem ObjIdx="4" Offset="0" Size="{memory_size}" Verify="true" AppliesTo="full,par" />"#
        ),
    );
    w(x, "              </LoadProcedure>");
    w(x, r#"              <LoadProcedure MergeId="7">"#);
    w(
        x,
        r#"                <LdCtrlLoadImageProp ObjIdx="4" PropId="27" />"#,
    );
    w(x, "              </LoadProcedure>");
    w(x, "            </LoadProcedures>");
    w(x, "            <Messages>");
    w(
        x,
        &format!(
            r#"              <Message Id="{AID}_M-1" Name="VersionMismatch" Text="Application and firmware version mismatch." />"#
        ),
    );
    w(x, "            </Messages>");
}

fn write_options(x: &mut String) {
    w(
        x,
        r#"            <Options TextParameterEncoding="iso-8859-15" SupportsExtendedMemoryServices="true" SupportsExtendedPropertyServices="true" />"#,
    );
}

fn write_dynamic(x: &mut String) {
    w(x, "          <Dynamic>");
    // ETS only allows Channel/ChannelIndependentBlock/choose/… directly under <Dynamic>
    // (not bare ParameterBlocks), so wrap everything in one ChannelIndependentBlock.
    w(x, "            <ChannelIndependentBlock>");
    // General: the "number of zones" dropdown drives which zone blocks are shown.
    write_general_block(x);
    // Zones — a zone block is shown when the chosen count is >= this zone's index.
    // Config knobs: DefVol, MaxVol, AirPlay, Spotify, PresEn, PresTO (sample rate / bit
    // depth are global, shown in the General block).
    for z in 1..=MAX_ZONES {
        write_channel_block(
            x,
            "Zone",
            z,
            &format!("{AID}_UP-G000"),
            &format!("&gt;={z}"),
            &format!("{AID}_UP-Z{z:02}000"),
            ZONE_GROUPS,
            &format!("Z{z:02}"),
            &[
                "002", "003", "004", "005", "006", "007", "008", "010", "011",
            ],
        );
    }
    // Clients — a client block is shown when the chosen count is >= this client's index
    // (same logic as zones). Config knobs: DefZone, DefVol, MaxVol, DefLat, MAC.
    for c in 1..=MAX_CLIENTS {
        write_channel_block(
            x,
            "Client",
            c,
            &format!("{AID}_UP-G003"),
            &format!("&gt;={c}"),
            &format!("{AID}_UP-C{c:02}000"),
            CLIENT_GROUPS,
            &format!("C{c:02}"),
            &["002", "003", "004", "005", "010", "011"],
        );
    }
    // Radio stream presets: count dropdown + gated Name / URL / Cover per station.
    write_radio_block(x);
    w(x, "            </ChannelIndependentBlock>");
    w(x, "          </Dynamic>");
}

/// Radio presets block: a "number of radios" dropdown, then one section per station
/// (Name / URL / Cover), each shown when the chosen count is >= its index.
fn write_radio_block(x: &mut String) {
    let count_ref = format!("{AID}_UP-G020_R-{AID}_UP-G020");
    w(
        x,
        &format!(
            r#"            <ParameterBlock Id="{AID}_PB-Radios" Name="Radios" Text="Radiosender">"#
        ),
    );
    // The count dropdown comes first, then gates each station section.
    w(
        x,
        &format!(r#"              <ParameterRefRef RefId="{count_ref}" />"#),
    );
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        let ids = [
            format!("{AID}_UP-{p}000"), // Name
            format!("{AID}_UP-{p}001"), // URL
            format!("{AID}_UP-{p}003"), // Cover
        ];
        w(
            x,
            &format!(r#"              <choose ParamRefId="{count_ref}">"#),
        );
        w(x, &format!(r#"                <when test="&gt;={r}">"#));
        write_param_section(
            x,
            "                  ",
            &format!("{p}-Station"),
            &format!("Station {r}"),
            &ids,
        );
        w(x, "                </when>");
        w(x, "              </choose>");
    }
    w(x, "            </ParameterBlock>");
}

/// Top-level "Allgemein" block holding the number-of-zones dropdown. Displayed via the
/// `P-` parameter ref (like the channel name fields); the zone `<choose>` conditions gate
/// on the same parameter's `UP-` ref.
fn write_general_block(x: &mut String) {
    let zones_ref = format!("{AID}_UP-G000_R-{AID}_UP-G000");
    let clients_ref = format!("{AID}_UP-G003_R-{AID}_UP-G003");
    w(
        x,
        &format!(
            r#"            <ParameterBlock Id="{AID}_PB-General" Name="General" Text="Allgemein">"#
        ),
    );
    // Counts: number of zones then number of clients drive which channel blocks show.
    w(
        x,
        &format!(r#"              <ParameterRefRef RefId="{zones_ref}" />"#),
    );
    w(
        x,
        &format!(r#"              <ParameterRefRef RefId="{clients_ref}" />"#),
    );
    let indent = "              ";
    // Server settings.
    write_param_section(
        x,
        indent,
        "G-Server",
        "Server",
        &[format!("{AID}_UP-G001"), format!("{AID}_UP-G002")],
    );
    // Global audio output format (server-wide, not per-zone).
    write_param_section(
        x,
        indent,
        "G-Audio",
        "Audio",
        &[
            format!("{AID}_UP-G004"), // SampleRate
            format!("{AID}_UP-G005"), // BitDepth
            format!("{AID}_UP-G006"), // Codec
            format!("{AID}_UP-G007"), // SourceConflict
            format!("{AID}_UP-G008"), // ZoneFade
            format!("{AID}_UP-G009"), // SourceFade
        ],
    );
    // Subsonic.
    write_param_section(
        x,
        indent,
        "G-Subsonic",
        "Subsonic",
        &[
            format!("{AID}_UP-G010"),
            format!("{AID}_UP-G011"),
            format!("{AID}_UP-G012"),
        ],
    );
    // MQTT.
    write_param_section(
        x,
        indent,
        "G-MQTT",
        "MQTT",
        &[
            format!("{AID}_UP-G013"), // Broker
            format!("{AID}_UP-G014"), // Topic
            format!("{AID}_UP-G015"), // Password
        ],
    );
    // AirPlay.
    write_param_section(
        x,
        indent,
        "G-AirPlay",
        "AirPlay",
        &[format!("{AID}_UP-G016")], // Password
    );
    // Security / secrets (plaintext in ETS by product decision).
    write_param_section(
        x,
        indent,
        "G-Security",
        "Sicherheit",
        &[
            format!("{AID}_UP-G017"), // PSK
            format!("{AID}_UP-G018"), // API key 1
            format!("{AID}_UP-G019"), // API key 2
        ],
    );
    w(x, "            </ParameterBlock>");
}

/// Emit a `<ParameterSeparator>` headline followed by a `<ParameterRefRef>` for each
/// parameter id. `indent` is the leading whitespace for the block nesting level; `sep_id`
/// must be a document-unique `NCName`.
fn write_param_section(
    x: &mut String,
    indent: &str,
    sep_id: &str,
    title: &str,
    param_ids: &[String],
) {
    w(
        x,
        &format!(
            r#"{indent}<ParameterSeparator Id="{AID}_PS-{sep_id}" Text="{title}" UIHint="Headline" />"#
        ),
    );
    for pid in param_ids {
        w(
            x,
            &format!(r#"{indent}<ParameterRefRef RefId="{pid}_R-{pid}" />"#),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn write_channel_block(
    x: &mut String,
    prefix: &str,
    idx: usize,
    cond_param_id: &str,
    test: &str,
    name_param_id: &str,
    groups: &[CoGroup],
    id_prefix: &str,
    config_nums: &[&str],
) {
    let cond_ref = format!("{cond_param_id}_R-{cond_param_id}");
    let name_ref = format!("{name_param_id}_R-{name_param_id}");
    w(
        x,
        &format!(r#"            <choose ParamRefId="{cond_ref}">"#),
    );
    w(x, &format!(r#"              <when test="{test}">"#));
    w(
        x,
        &format!(
            r#"                <ParameterBlock Id="{AID}_PB-{id_prefix}" Name="{prefix}{idx}" Text="{prefix} {idx}: {{{{0: ...}}}}" TextParameterRefId="{name_ref}" ShowInComObjectTree="true">"#
        ),
    );
    // Name parameter
    w(
        x,
        &format!(r#"                  <ParameterRefRef RefId="{name_ref}" />"#),
    );
    // Editable configuration knobs for this channel (memory-backed, from mem::).
    if !config_nums.is_empty() {
        let ids: Vec<String> = config_nums
            .iter()
            .map(|num| format!("{AID}_UP-{id_prefix}{num}"))
            .collect();
        write_param_section(
            x,
            "                  ",
            &format!("{id_prefix}-Einstellungen"),
            "Einstellungen",
            &ids,
        );
    }
    // CO groups
    for group in groups {
        w(
            x,
            &format!(
                r#"                  <ParameterSeparator Id="{AID}_PS-{id_prefix}-{}" Text="{}" UIHint="Headline" />"#,
                group
                    .title_en
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .collect::<String>(),
                group.title_de
            ),
        );
        for &i in group.indices {
            let co_id = format!("{AID}_O-{id_prefix}{i:03}");
            let num = if prefix == "Zone" {
                (idx - 1) * ZONE_GO_COUNT + i + 1
            } else {
                MAX_ZONES * ZONE_GO_COUNT + (idx - 1) * CLIENT_GO_COUNT + i + 1
            };
            w(
                x,
                &format!(r#"                  <ComObjectRefRef RefId="{co_id}_R-{num}" />"#),
            );
        }
    }
    w(x, "                </ParameterBlock>");
    w(x, "              </when>");
    w(x, "            </choose>");
}

fn write_hardware(x: &mut String) {
    w(x, "      <Hardware>");
    w(
        x,
        &format!(
            r#"        <Hardware Id="{MFR}_H-0xFF01-1" Name="SnapDog" SerialNumber="0xFF01" VersionNumber="1" BusCurrent="0" HasIndividualAddress="true" HasApplicationProgram="true">"#
        ),
    );
    w(x, "          <Products>");
    w(
        x,
        &format!(
            r#"            <Product Id="{MFR}_H-0xFF01-1_P-0xFF01" Text="SnapDog" OrderNumber="0xFF01" IsRailMounted="false" DefaultLanguage="de-DE">"#
        ),
    );
    w(
        x,
        r#"              <RegistrationInfo RegistrationStatus="Registered" />"#,
    );
    w(x, "            </Product>");
    w(x, "          </Products>");
    w(x, "          <Hardware2Programs>");
    w(
        x,
        &format!(
            r#"            <Hardware2Program Id="{MFR}_H-0xFF01-1_HP-FF01-01-0000" MediumTypes="MT-0">"#
        ),
    );
    w(
        x,
        &format!(r#"              <ApplicationProgramRef RefId="{AID}" />"#),
    );
    // The RegistrationNumber (any `\d{4}/\d+`) is what makes ETS treat this as a registered
    // product from the M-00FA (OpenKNX) manufacturer space, so it imports without demanding
    // an unregistered-product test license — matching how OpenKNX's own products import.
    w(
        x,
        r#"              <RegistrationInfo RegistrationStatus="Registered" RegistrationNumber="0001/1" />"#,
    );
    w(x, "            </Hardware2Program>");
    w(x, "          </Hardware2Programs>");
    w(x, "        </Hardware>");
    w(x, "      </Hardware>");
}

fn w(s: &mut String, line: &str) {
    s.push_str(line);
    s.push('\n');
}
