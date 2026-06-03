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

    // Step 1: Generate ETS XML
    let xml = generate_xml();
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

fn write_application_program(x: &mut String) {
    w(x, "      <ApplicationPrograms>");
    w(
        x,
        &format!(
            r#"        <ApplicationProgram Id="{AID}" ProgramType="ApplicationProgram" MaskVersion="MV-07B0" Name="SnapDog" LoadProcedureStyle="MergedProcedure" PeiType="0" DefaultLanguage="de-DE" DynamicTableManagement="false" Linkable="true" MinEtsVersion="5.0" IPConfig="Custom" ApplicationNumber="65281" ApplicationVersion="1" ReplacesVersions="0">"#
        ),
    );
    w(x, "          <Static>");

    write_code_segment(x);
    write_parameter_types(x);
    write_parameters(x);
    write_com_objects(x);
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
        &[("44100 Hz", 0), ("48000 Hz", 1), ("96000 Hz", 2)],
    );
    pt_enum(
        x,
        "BitDepth",
        8,
        &[("16 Bit", 0), ("24 Bit", 1), ("32 Bit", 2)],
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
    let mut off = 0usize;

    // ── Zone parameters ───────────────────────────────────────
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        // Active flag (offset 0-9)
        param_mem(
            x,
            &p,
            "001",
            "Active",
            "YesNo",
            "Zone aktiv",
            "1",
            &mut off,
            8,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "002",
            "DefVol",
            "Percent",
            "Standard-Lautstärke",
            "50",
            &mut off,
            8,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "003",
            "MaxVol",
            "Percent",
            "Max. Lautstärke",
            "100",
            &mut off,
            8,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "004",
            "AirPlay",
            "YesNo",
            "AirPlay aktiviert",
            "1",
            &mut off,
            8,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "005",
            "Spotify",
            "YesNo",
            "Spotify aktiviert",
            "1",
            &mut off,
            8,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "006",
            "PresEn",
            "YesNo",
            "Präsenz aktiviert",
            "0",
            &mut off,
            8,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "007",
            "PresTO",
            "UInt16",
            "Präsenz Auto-Off (s)",
            "900",
            &mut off,
            16,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "008",
            "SRate",
            "SampleRate",
            "Sample Rate",
            "1",
            &mut off,
            8,
        );
    }
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "009",
            "BitD",
            "BitDepth",
            "Bit Depth",
            "0",
            &mut off,
            8,
        );
    }

    // ── Client parameters ─────────────────────────────────────
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        param_mem(
            x,
            &p,
            "001",
            "Active",
            "YesNo",
            "Client aktiv",
            "1",
            &mut off,
            8,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        param_mem(
            x,
            &p,
            "002",
            "DefZone",
            "ZoneSelect",
            "Standard-Zone",
            "1",
            &mut off,
            8,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        param_mem(
            x,
            &p,
            "003",
            "DefVol",
            "Percent",
            "Standard-Lautstärke",
            "100",
            &mut off,
            8,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        param_mem(
            x,
            &p,
            "004",
            "MaxVol",
            "Percent",
            "Max. Lautstärke",
            "100",
            &mut off,
            8,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        param_mem(
            x,
            &p,
            "005",
            "DefLat",
            "UInt8",
            "Standard-Latenz (ms)",
            "0",
            &mut off,
            8,
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
        &mut off,
        16,
    );
    param_mem(
        x,
        "G",
        "002",
        "LogLvl",
        "LogLevel",
        "Log Level",
        "2",
        &mut off,
        8,
    );

    // ── Radio active flags (numeric, 20 × 1 byte) ────────────
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        param_mem(x, &p, "002", "Active", "YesNo", "Aktiv", "0", &mut off, 8);
    }

    // ── String parameters (order matches mem:: layout) ────────
    for z in 1..=MAX_ZONES {
        let p = format!("Z{z:02}");
        param_mem(
            x,
            &p,
            "000",
            "Name",
            "Name",
            "Zonenname",
            &format!("Zone {z}"),
            &mut off,
            mem::ZONE_NAME_SIZE as u16 * 8,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        param_mem(
            x,
            &p,
            "000",
            "Name",
            "Name",
            "Clientname",
            &format!("Client {c}"),
            &mut off,
            mem::CLIENT_NAME_SIZE as u16 * 8,
        );
    }
    for c in 1..=MAX_CLIENTS {
        let p = format!("C{c:02}");
        param_mem(
            x,
            &p,
            "010",
            "MAC",
            "MAC",
            "MAC-Adresse",
            "",
            &mut off,
            mem::CLIENT_MAC_SIZE as u16 * 8,
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
        &mut off,
        mem::GLOBAL_SUB_URL_SIZE as u16 * 8,
    );
    param_mem(
        x,
        "G",
        "011",
        "SubUser",
        "Text20",
        "Subsonic Benutzer",
        "",
        &mut off,
        mem::GLOBAL_SUB_USER_SIZE as u16 * 8,
    );
    param_mem(
        x,
        "G",
        "012",
        "SubPass",
        "Text20",
        "Subsonic Passwort",
        "",
        &mut off,
        mem::GLOBAL_SUB_PASS_SIZE as u16 * 8,
    );
    param_mem(
        x,
        "G",
        "013",
        "MqttBrk",
        "Text40",
        "MQTT Broker",
        "",
        &mut off,
        mem::GLOBAL_MQTT_BROKER_SIZE as u16 * 8,
    );
    param_mem(
        x,
        "G",
        "014",
        "MqttTop",
        "Text20",
        "MQTT Base Topic",
        "snapdog",
        &mut off,
        mem::GLOBAL_MQTT_TOPIC_SIZE as u16 * 8,
    );
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        param_mem(
            x,
            &p,
            "000",
            "Name",
            "Text20",
            "Stationsname",
            "",
            &mut off,
            mem::RADIO_NAME_SIZE as u16 * 8,
        );
    }
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        param_mem(
            x,
            &p,
            "001",
            "URL",
            "Text80",
            "Stream URL",
            "",
            &mut off,
            mem::RADIO_URL_SIZE as u16 * 8,
        );
    }

    w(x, "            </Parameters>");
    eprintln!("  Memory layout: {off} bytes used");
    assert_eq!(
        off,
        mem::TOTAL,
        "Memory layout mismatch: xtask generated {off} bytes but mem::TOTAL is {}",
        mem::TOTAL
    );
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
    offset: &mut usize,
    bits: u16,
) {
    let bytes = (bits / 8) as usize;
    w(x, &format!(r#"              <Union SizeInBit="{bits}">"#));
    w(
        x,
        &format!(
            r#"                <Memory CodeSegment="{AID}_RS-04-00000" Offset="{}" BitOffset="0" />"#,
            *offset
        ),
    );
    w(
        x,
        &format!(
            r#"                <Parameter Id="{AID}_UP-{prefix}{num}" Name="{prefix}_{name}" Offset="0" BitOffset="0" ParameterType="{AID}_PT-{pt}" Text="{text}" Value="{default}" />"#
        ),
    );
    w(x, "              </Union>");
    *offset += bytes;
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
            r#"              <ComObject Id="{AID}_O-{id_suffix}" Name="{name}" Number="{number}" Text="{}" FunctionText="{}" ObjectSize="{}" DatapointType="{}" Priority="Low" ReadFlag="{r}" WriteFlag="{wr}" CommunicationFlag="Enabled" TransmitFlag="{t}" UpdateFlag="{u}" />"#,
            go.name_de, go.name, go.size_str, go.dpt
        ),
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
    w(
        x,
        r#"                <LdCtrlCompareProp InlineData="0000FF010100" ObjIdx="0" PropId="78">"#,
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
    // Zones
    for z in 1..=MAX_ZONES {
        write_channel_block(
            x,
            "Zone",
            z,
            &format!("{AID}_UP-Z{z:02}001"),
            &format!("{AID}_P-Z{z:02}000"),
            ZONE_GROUPS,
            &format!("Z{z:02}"),
        );
    }
    // Clients
    for c in 1..=MAX_CLIENTS {
        write_channel_block(
            x,
            "Client",
            c,
            &format!("{AID}_UP-C{c:02}001"),
            &format!("{AID}_P-C{c:02}000"),
            CLIENT_GROUPS,
            &format!("C{c:02}"),
        );
    }
    w(x, "          </Dynamic>");
}

fn write_channel_block(
    x: &mut String,
    prefix: &str,
    idx: usize,
    active_param_id: &str,
    name_param_id: &str,
    groups: &[CoGroup],
    id_prefix: &str,
) {
    let active_ref = format!("{active_param_id}_R-{active_param_id}");
    let name_ref = format!("{name_param_id}_R-{name_param_id}");
    w(
        x,
        &format!(r#"            <choose ParamRefId="{active_ref}">"#),
    );
    w(x, r#"              <when test="1">"#);
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
    // CO groups
    for group in groups {
        w(
            x,
            &format!(
                r#"                  <ParameterSeparator Id="{AID}_PS-{id_prefix}-{}" Text="{}" UIHint="Headline" />"#,
                group.title_en.replace(' ', ""),
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
    w(x, r"              <RegistrationInfo />");
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
    w(x, "            </Hardware2Program>");
    w(x, "          </Hardware2Programs>");
    w(x, "        </Hardware>");
    w(x, "      </Hardware>");
}

fn w(s: &mut String, line: &str) {
    s.push_str(line);
    s.push('\n');
}
