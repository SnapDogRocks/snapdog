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
    CGO_VOLUME, CGO_VOLUME_DIM, CGO_VOLUME_STATUS, CGO_ZONE, CGO_ZONE_STATUS, CLIENT_GOS,
    GLOBAL_GO_COUNT, GLOBAL_GOS, GoDefinition, KNXPROD_APP_NUMBER, KNXPROD_APP_VERSION,
    KNXPROD_HW_VERSION, MAX_API_KEYS, MAX_CLIENTS, MAX_ZONES, ZGO_CONTROL_STATUS, ZGO_MUTE,
    ZGO_MUTE_STATUS, ZGO_MUTE_TOGGLE, ZGO_PAUSE, ZGO_PLAY, ZGO_PLAYLIST, ZGO_PLAYLIST_NEXT,
    ZGO_PLAYLIST_PREVIOUS, ZGO_PLAYLIST_STATUS, ZGO_PRESENCE, ZGO_PRESENCE_ENABLE,
    ZGO_PRESENCE_TIMER_ACTIVE, ZGO_REPEAT, ZGO_REPEAT_STATUS, ZGO_REPEAT_TOGGLE, ZGO_SHUFFLE,
    ZGO_SHUFFLE_STATUS, ZGO_SHUFFLE_TOGGLE, ZGO_STOP, ZGO_TRACK_ALBUM, ZGO_TRACK_ARTIST,
    ZGO_TRACK_NEXT, ZGO_TRACK_PLAYING, ZGO_TRACK_PREVIOUS, ZGO_TRACK_PROGRESS, ZGO_TRACK_REPEAT,
    ZGO_TRACK_REPEAT_STATUS, ZGO_TRACK_REPEAT_TOGGLE, ZGO_TRACK_TITLE, ZGO_VOLUME, ZGO_VOLUME_DIM,
    ZGO_VOLUME_STATUS, ZONE_GOS, mem,
};

const AID: &str = "M-00FA_A-FF01-01-0000";
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
    // Schema version for the master download (only used when signing below).
    let ns_version =
        knx_rs_prod::parse::extract_metadata_from_str(&xml).map_or(20, |m| m.ns_version);
    std::fs::write(&xml_path, xml).expect("failed to write XML");
    eprintln!(
        "  Generated {xml_path} ({} zones × {} COs + {} clients × {} COs = {} COs)",
        MAX_ZONES,
        ZONE_GOS.len(),
        MAX_CLIENTS,
        CLIENT_GOS.len(),
        MAX_ZONES * ZONE_GOS.len() + MAX_CLIENTS * CLIENT_GOS.len()
    );

    // Step 2: package the .knxprod. When `SNAPDOG_ETS_KEY` points at an ETS signing key
    // (the release build with the secret set) the archive is RSA-signed and directly
    // ETS-importable; otherwise an unsigned archive is produced.
    let xml_file = std::path::Path::new(&xml_path);
    let knxprod_file = std::path::Path::new(&knxprod_path);
    let result = match std::env::var("SNAPDOG_ETS_KEY") {
        Ok(key_path) if !key_path.trim().is_empty() => {
            use knx_rs_prod::knx_master::KnxMaster;
            use knx_rs_prod::signature::SigningKey;
            let key = SigningKey::from_path(std::path::Path::new(&key_path))
                .unwrap_or_else(|e| panic!("failed to load ETS signing key {key_path}: {e}"));
            let master = KnxMaster::download(ns_version).unwrap_or_else(|e| {
                panic!("failed to fetch knx_master.xml (project/{ns_version}): {e}")
            });
            eprintln!("  Signing .knxprod with the ETS key from SNAPDOG_ETS_KEY");
            knx_rs_prod::generate_signed_knxprod(xml_file, knxprod_file, &key, &master)
        }
        _ => knx_rs_prod::generate_knxprod(xml_file, knxprod_file),
    };
    match result {
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
        indices: &[ZGO_PRESENCE, ZGO_PRESENCE_ENABLE, ZGO_PRESENCE_TIMER_ACTIVE],
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
    use knx_rs_prod::author::ProgramInfo;

    // ETS keys the program by ApplicationNumber + ApplicationVersion; System B (KNXnet/IP)
    // uses MaskVersion MV-57B0. Every other <ApplicationProgram> attribute is a ProgramInfo
    // default (ProgramType, LoadProcedureStyle, PeiType, DynamicTableManagement, Linkable,
    // MinEtsVersion, IPConfig) matching what SnapDog emitted before the migration.
    let info = ProgramInfo::new("SnapDog", "MV-57B0", "de-DE", 65281, app_version());
    let mut x = String::with_capacity(128 * 1024);
    build_app().write_knx_document(&info, "SnapDog xtask", "1.0", &mut x);
    x
}

/// The single typed product model behind the whole `.knxprod` document. Every `<Static>`
/// section (catalog, hardware, code segment, parameter types, parameters, com-objects,
/// tables, load procedures, messages, options) plus the `<Dynamic>` UI tree is registered
/// here, then rendered in ETS schema order by
/// [`write_knx_document`](knx_rs_prod::author::AppProgram::write_knx_document).
fn build_app() -> knx_rs_prod::author::AppProgram {
    use knx_rs_prod::author::{
        AppProgram, CatalogItem, CatalogSection, Hardware, Hardware2Program, Options, Product,
        Segment,
    };

    let mut app = AppProgram::new(AID);

    // Catalog — the product/hardware refs are derived by the author from the shared HW_*
    // identity, so they can't drift from <Hardware>.
    app.add_catalog_section(
        CatalogSection::new("SnapDog", "SnapDog", "SnapDog", "de-DE").with_item(CatalogItem::new(
            "SnapDog", "1", HW_SERIAL, HW_VERSION, HW_ORDER, "de-DE",
        )),
    );

    // Hardware — MT-5 is the KNXnet/IP (System B) medium; the \d{4}/\d+ RegistrationNumber
    // makes ETS import it as a registered M-00FA (OpenKNX) product without a test license.
    app.add_hardware(
        Hardware::new(HW_SERIAL, HW_VERSION, "SnapDog")
            .with_product(Product::new(HW_ORDER, "SnapDog", "de-DE"))
            .with_program(Hardware2Program::new("MT-5", "0001/1")),
    );

    // Code segment — the relative "Parameters" segment the memory-backed params live in,
    // pinned as the parameter segment so every <Memory CodeSegment> resolves to it.
    let seg = app.add_segment(Segment::Relative {
        name: Some("Parameters".into()),
        size: mem::TOTAL as u32,
        load_state_machine: 4,
        offset: 0,
    });
    app.set_parameter_segment(seg);

    // Parameter types + parameters + com-objects (the interdependent core). Order matters:
    // param_mem resolves its type handle by name, and the Dynamic tree (below) resolves its
    // param/com-object refs by suffix.
    register_types(&mut app);
    register_params(&mut app);
    register_com_objects(&mut app);

    // Address + association tables (System B max entries).
    app.set_address_table(2047);
    app.set_association_table(2047);

    // Download machine + the diagnostics message it references on a version mismatch.
    register_load_procedures(&mut app);
    register_messages(&mut app);

    // Text encoding + extended memory/property service support flags.
    app.set_options(Options::new("iso-8859-15", true, true));

    // Dynamic UI tree — resolves its refs against the params/com-objects registered above.
    let tree = build_dynamic(&app);
    app.add_dynamic(tree);

    app
}

/// Hardware identity (serial + version + order number). ETS threads these through the
/// `<Hardware>` and `<Catalog>` id grammars; kept here once so the two sections agree.
const HW_SERIAL: &str = "0xFF01";
const HW_VERSION: u32 = 1;
const HW_ORDER: &str = "0xFF01";

/// The ETS `ApplicationVersion`, sourced from the firmware SSOT
/// [`KNXPROD_APP_VERSION`](snapdog::knx::group_objects::KNXPROD_APP_VERSION) so the `WebUI`
/// product-info and the `.knxprod` always agree. ETS keys the program by
/// `ApplicationNumber` + `ApplicationVersion`; re-importing an unchanged version shows
/// the *cached* content, so a fresh import needs a higher number. **Bump
/// `KNXPROD_APP_VERSION` in `group_objects.rs`** on any layout/parameter change. Override
/// for throwaway test builds with `SNAPDOG_APP_VERSION=<n>`.
fn app_version() -> u32 {
    std::env::var("SNAPDOG_APP_VERSION")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(KNXPROD_APP_VERSION)
}

#[allow(clippy::too_many_lines)]
fn register_types(x: &mut knx_rs_prod::author::AppProgram) {
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
    // Client count 1..=MAX_CLIENTS (dynamic to avoid a long literal).
    let client_labels: Vec<String> = (1..=MAX_CLIENTS)
        .map(|n| {
            if n == 1 {
                "1 Client".into()
            } else {
                format!("{n} Clients")
            }
        })
        .collect();
    let client_pairs: Vec<(&str, u16)> = client_labels
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), (i + 1) as u16))
        .collect();
    pt_enum(x, "NumClients", 8, &client_pairs);
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
    // API-key count 1..=MAX_API_KEYS (default 1) — drives how many key fields ETS shows.
    let api_key_labels: Vec<String> = (1..=MAX_API_KEYS)
        .map(|n| {
            if n == 1 {
                "1 API-Key".into()
            } else {
                format!("{n} API-Keys")
            }
        })
        .collect();
    let api_key_pairs: Vec<(&str, u16)> = api_key_labels
        .iter()
        .enumerate()
        .map(|(i, s)| (s.as_str(), (i + 1) as u16))
        .collect();
    pt_enum(x, "NumApiKeys", 8, &api_key_pairs);
    // Heartbeat interval for the global "Server Online" cyclic send. The stored value is
    // the interval in minutes so the firmware reads it directly.
    pt_enum(
        x,
        "Heartbeat",
        8,
        &[
            ("1 Minute", 1),
            ("3 Minuten", 3),
            ("5 Minuten", 5),
            ("10 Minuten", 10),
            ("15 Minuten", 15),
            ("30 Minuten", 30),
            ("45 Minuten", 45),
            ("60 Minuten", 60),
        ],
    );
}

fn pt_enum(x: &mut knx_rs_prod::author::AppProgram, name: &str, bits: u16, values: &[(&str, u16)]) {
    x.add_parameter_type(knx_rs_prod::author::ParameterType::enumeration(
        name,
        bits,
        values
            .iter()
            .map(|&(text, val)| (text.to_string(), i64::from(val)))
            .collect(),
    ));
}

fn pt_text(x: &mut knx_rs_prod::author::AppProgram, name: &str, bits: u16) {
    x.add_parameter_type(knx_rs_prod::author::ParameterType::text(name, bits));
}

fn pt_num(
    x: &mut knx_rs_prod::author::AppProgram,
    name: &str,
    bits: u16,
    typ: &str,
    min: u32,
    max: u32,
) {
    x.add_parameter_type(knx_rs_prod::author::ParameterType::number(
        name,
        bits,
        typ,
        i64::from(min),
        i64::from(max),
    ));
}

/// Register every `<Parameter>` (the full #89 inline layout) into `app`, asserting the
/// `mem::` span tiling and the committed layout fingerprint. `register_types` must have run
/// first, since `param_mem` resolves each parameter's type handle by name.
#[allow(clippy::too_many_lines)]
fn register_params(x: &mut knx_rs_prod::author::AppProgram) {
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
    // API-key count dropdown (default 1), then MAX_API_KEYS memory-backed key slots. The
    // General block reveals key N only when the count is >= N (mirrors the radio presets).
    param_mem(
        x,
        "G",
        "021",
        "NumApiKeys",
        "NumApiKeys",
        "Anzahl API-Keys",
        "1",
        mem::GLOBAL_NUM_API_KEYS,
        8,
        &mut spans,
    );
    for k in 1..=MAX_API_KEYS {
        let num = format!("{:03}", 21 + k); // G022..
        param_mem(
            x,
            "G",
            &num,
            &format!("ApiKey{k}"),
            "Text40",
            &format!("API-Key {k}"),
            "",
            mem::GLOBAL_API_KEYS + (k - 1) * mem::GLOBAL_API_KEY_SIZE,
            mem::GLOBAL_API_KEY_SIZE as u16 * 8,
            &mut spans,
        );
    }
    // Heartbeat interval (minutes) for the global "Server Online" cyclic send.
    param_mem(
        x,
        "G",
        "040",
        "Heartbeat",
        "Heartbeat",
        "Heartbeat-Intervall",
        "5",
        mem::GLOBAL_HEARTBEAT,
        8,
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

    // Fail if the byte layout drifts without an APP_VERSION bump (see below).
    assert_layout_locked(&spans);
}

/// Committed fingerprint of the exact ETS memory layout. Update this **together with**
/// `KNXPROD_APP_VERSION` (`group_objects.rs`) whenever the layout changes — the assertion
/// below prints the new value.
const EXPECTED_LAYOUT_HASH: u64 = 0x3298_e5c0_ecd8_640d;

/// Guard: fail `.knxprod` generation when the ETS product definition drifts from the
/// committed fingerprint. ETS decides download scope by `ApplicationVersion`; a change
/// shipped under an unchanged version lets ETS reuse a programmed device's stale config
/// bytes → a mis-parameterized device. This ties every such change to a conscious
/// `APP_VERSION` bump.
///
/// The fingerprint covers both dimensions that can silently mis-parameterize a device:
/// the **parameter byte layout** (every `(offset, size)` span plus the `mem::` totals and
/// object counts) *and* each **communication object's semantics** (`DPT`, object size,
/// comm flags). The latter matters because a `DPT`/flag change at an *unchanged* byte
/// offset leaves the layout hash untouched yet makes ETS interpret the object differently.
/// Display names are deliberately excluded — a label edit should not force a version bump.
///
/// Enforced in three places: the CI `KNX Product Database` job (runs `cargo xtask
/// knxprod`), the `cargo test` suite (the `ets_memory_layout_is_locked` test drives the
/// same generation with no file writes), and the local pre-push hook.
fn assert_layout_locked(spans: &[(usize, usize)]) {
    const fn feed(mut h: u64, v: usize) -> u64 {
        let bytes = (v as u64).to_le_bytes();
        let mut i = 0;
        while i < 8 {
            h ^= bytes[i] as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV-1a prime
            i += 1;
        }
        h
    }
    fn feed_bytes(mut h: u64, bytes: &[u8]) -> u64 {
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a offset basis
    for v in [
        mem::TOTAL,
        MAX_ZONES,
        MAX_CLIENTS,
        mem::MAX_RADIOS,
        ZONE_GOS.len(),
        CLIENT_GOS.len(),
        GLOBAL_GO_COUNT,
    ] {
        h = feed(h, v);
    }
    for (off, size) in spans {
        h = feed(h, *off);
        h = feed(h, *size);
    }
    // Fold each communication object's ETS semantics so a DPT/flag change at an unchanged
    // byte offset also trips the guard (see the doc comment above).
    for go in ZONE_GOS.iter().chain(CLIENT_GOS).chain(GLOBAL_GOS) {
        h = feed_bytes(h, go.dpt_str.as_bytes());
        h = feed_bytes(h, go.size_str.as_bytes());
        h = feed(
            h,
            usize::from(go.flags.communicate)
                | usize::from(go.flags.read) << 1
                | usize::from(go.flags.write) << 2
                | usize::from(go.flags.transmit) << 3
                | usize::from(go.flags.update) << 4,
        );
    }
    assert_eq!(
        h, EXPECTED_LAYOUT_HASH,
        "\n\n  ETS PRODUCT DEFINITION CHANGED (fingerprint {h:#018x}).\n  \
         Bump KNXPROD_APP_VERSION (group_objects.rs) and set\n  \
         EXPECTED_LAYOUT_HASH = {h:#018x} in xtask/src/main.rs.\n  \
         (Shipping a layout/DPT/flag change under an unchanged version lets ETS reuse a\n  \
         device's stale config bytes — a mis-parameterized device.)\n"
    );
}

/// Emit a memory-backed parameter inside a Union.
#[allow(clippy::too_many_arguments)]
fn param_mem(
    x: &mut knx_rs_prod::author::AppProgram,
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
    // Record the span so the caller can assert the params tile mem:: exactly. The
    // `<Union>` width, the `_PT-` reference and the `_RS-04-00000` CodeSegment default
    // are all drawn from the type handle + the author, so they can't drift.
    spans.push((offset, (bits / 8) as usize));
    let ty = x.parameter_type_id(pt);
    x.add_param(knx_rs_prod::author::Parameter::new(
        format!("{prefix}{num}"),
        format!("{prefix}_{name}"),
        ty,
        text,
        default,
        offset,
    ));
}

/// Register every zone, client and device-level global group object into `app`, in the
/// exact `Number` order ETS expects, and return the suffix→handle map the Dynamic tree
/// resolves its `<ComObjectRefRef>`s through. Numbering is #89's verbatim arithmetic.
fn register_com_objects(
    app: &mut knx_rs_prod::author::AppProgram,
) -> std::collections::HashMap<String, knx_rs_prod::author::ComObjectRefId> {
    let mut oref: std::collections::HashMap<String, knx_rs_prod::author::ComObjectRefId> =
        std::collections::HashMap::new();
    for z in 1..=MAX_ZONES {
        for (i, go) in ZONE_GOS.iter().enumerate() {
            let num = (z - 1) * ZONE_GOS.len() + i + 1;
            let suffix = format!("Z{z:02}{i:03}");
            let (_, r) = app.add_com_object(go_com_object(
                &suffix,
                &format!("Zone {z} {}", go.name),
                num,
                go,
            ));
            oref.insert(suffix, r);
        }
    }
    for c in 1..=MAX_CLIENTS {
        for (i, go) in CLIENT_GOS.iter().enumerate() {
            let num = MAX_ZONES * ZONE_GOS.len() + (c - 1) * CLIENT_GOS.len() + i + 1;
            let suffix = format!("C{c:02}{i:03}");
            let (_, r) = app.add_com_object(go_com_object(
                &suffix,
                &format!("Client {c} {}", go.name),
                num,
                go,
            ));
            oref.insert(suffix, r);
        }
    }
    // Global (device-level) COs follow every zone and client CO.
    for (i, go) in GLOBAL_GOS.iter().enumerate() {
        let num = MAX_ZONES * ZONE_GOS.len() + MAX_CLIENTS * CLIENT_GOS.len() + i + 1;
        let suffix = format!("GG{i:03}");
        let (_, r) = app.add_com_object(go_com_object(&suffix, go.name, num, go));
        oref.insert(suffix, r);
    }
    oref
}

/// Map a firmware [`GoDefinition`] to the typed authoring `ComObject`. DPT main/sub are
/// parsed from the `Dpt` Display ("1.001") exactly as the old `dpst` helper did.
fn go_com_object(
    suffix: &str,
    name: &str,
    number: usize,
    go: &GoDefinition,
) -> knx_rs_prod::author::ComObject {
    use knx_rs_prod::author::{ComObject, Dpt, Flags};

    let s = go.dpt.to_string();
    let (main, sub) = s.split_once('.').unwrap_or((s.as_str(), "0"));
    ComObject::new(
        suffix,
        name,
        number as u32,
        go.name_de,
        go.name,
        go.size_str,
        Dpt::new(main.parse().unwrap_or(0), sub.parse().unwrap_or(0)),
        Flags {
            read: go.flags.read,
            write: go.flags.write,
            transmit: go.flags.transmit,
            update: go.flags.update,
        },
    )
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

/// Register the ETS download machine through the typed `author` model. `InlineData` on the
/// compare step is the firmware `HARDWARE_TYPE` SSOT; the segment size is `mem::TOTAL`, so
/// the download can't drift from the memory layout.
fn register_load_procedures(app: &mut knx_rs_prod::author::AppProgram) {
    use knx_rs_prod::author::{
        ErrorCause, LoadControl, LoadProcedure, ObjTarget, OnError, ProcType, StepBase,
    };
    use snapdog::knx::group_objects::HARDWARE_TYPE;

    let size = mem::TOTAL as u32;

    app.add_load_procedure(LoadProcedure::with_merge_id(1).step_with(
        LoadControl::CompareProp {
            target: ObjTarget::ObjIdx {
                obj_idx: 0,
                occurrence: 0,
            },
            prop_id: 78,
            inline_data: Some(HARDWARE_TYPE.to_vec()),
        },
        StepBase {
            applies_to: None,
            on_error: vec![OnError {
                cause: ErrorCause::CompareMismatch,
                ignore: false,
                message_ref: Some(format!("{AID}_M-1")),
            }],
        },
    ));

    app.add_load_procedure(
        LoadProcedure::with_merge_id(2)
            .step_with(
                LoadControl::RelSegment {
                    target: ObjTarget::Lsm(4),
                    size,
                    mode: 1,
                    fill: 0,
                },
                StepBase {
                    applies_to: Some(ProcType::Full),
                    on_error: Vec::new(),
                },
            )
            .step_with(
                LoadControl::RelSegment {
                    target: ObjTarget::Lsm(4),
                    size,
                    mode: 0,
                    fill: 0,
                },
                StepBase {
                    applies_to: Some(ProcType::Par),
                    on_error: Vec::new(),
                },
            ),
    );

    app.add_load_procedure(LoadProcedure::with_merge_id(4).step_with(
        LoadControl::WriteRelMem {
            target: ObjTarget::ObjIdx {
                obj_idx: 4,
                occurrence: 0,
            },
            offset: 0,
            size,
            verify: true,
            inline_data: None,
        },
        StepBase {
            applies_to: Some(ProcType::FullPar),
            on_error: Vec::new(),
        },
    ));

    app.add_load_procedure(
        LoadProcedure::with_merge_id(7).step(LoadControl::LoadImageProp {
            target: ObjTarget::ObjIdx {
                obj_idx: 4,
                occurrence: 0,
            },
            prop_id: 27,
        }),
    );
}

/// Register the `<Messages>` model — the diagnostics load procedures reference on error.
fn register_messages(app: &mut knx_rs_prod::author::AppProgram) {
    use knx_rs_prod::author::Message;

    app.add_message(Message::new(
        "1",
        "VersionMismatch",
        "Application and firmware version mismatch.",
    ));
}

/// Build the `<Dynamic>` UI tree: one `<ChannelIndependentBlock>` holding the General block,
/// a gated block per zone and per client, the radio presets and the device System block —
/// the exact structure #89 emitted, now resolved through the typed `Dyn` model.
fn build_dynamic(app: &knx_rs_prod::author::AppProgram) -> knx_rs_prod::author::Dyn {
    use knx_rs_prod::author::Dyn;

    let mut blocks: Vec<Dyn> = Vec::new();
    // General: the "number of zones"/"number of clients" dropdowns drive which blocks show.
    blocks.push(build_general_block(app));
    // Zones — a zone block is shown when the chosen count is >= this zone's index. Config
    // knobs: DefVol, MaxVol, AirPlay, Spotify, PresEn, PresTO (sample rate / bit depth are
    // global, shown in the General block).
    for z in 1..=MAX_ZONES {
        blocks.push(build_channel_block(
            app,
            "Zone",
            z,
            "G000",
            &format!(">={z}"),
            &format!("Z{z:02}000"),
            ZONE_GROUPS,
            &format!("Z{z:02}"),
            &[
                "002", "003", "004", "005", "006", "007", "008", "010", "011",
            ],
        ));
    }
    // Clients — same gating as zones. Config knobs: DefZone, DefVol, MaxVol, DefLat, MAC.
    for c in 1..=MAX_CLIENTS {
        blocks.push(build_channel_block(
            app,
            "Client",
            c,
            "G003",
            &format!(">={c}"),
            &format!("C{c:02}000"),
            CLIENT_GROUPS,
            &format!("C{c:02}"),
            &["002", "003", "004", "005", "010", "011"],
        ));
    }
    // Radio stream presets: count dropdown + gated Name / URL / Cover per station.
    blocks.push(build_radio_block(app));
    // Device-level "System" block: heartbeat interval param + the global group objects.
    blocks.push(build_system_block(app));
    Dyn::ChannelIndependentBlock(blocks)
}

/// Radio presets block: a "number of radios" dropdown, then one section per station
/// (Name / URL / Cover), each shown when the chosen count is >= its index.
fn build_radio_block(app: &knx_rs_prod::author::AppProgram) -> knx_rs_prod::author::Dyn {
    use knx_rs_prod::author::{Dyn, When};

    let count = app.param_ref("G020");
    let mut children: Vec<Dyn> = Vec::new();
    // The count dropdown comes first, then gates each station section.
    children.push(Dyn::ParamRefRef(count));
    for r in 1..=mem::MAX_RADIOS {
        let p = format!("R{r:02}");
        let name = format!("{p}000"); // Name
        let url = format!("{p}001"); // URL
        let cover = format!("{p}003"); // Cover
        let mut station: Vec<Dyn> = Vec::new();
        push_param_section(
            app,
            &mut station,
            &format!("{p}-Station"),
            &format!("Station {r}"),
            &[name.as_str(), url.as_str(), cover.as_str()],
        );
        children.push(Dyn::Choose {
            param_ref: count,
            whens: vec![When {
                test: format!(">={r}"),
                children: station,
            }],
        });
    }
    Dyn::ParameterBlock {
        suffix: "Radios".to_string(),
        name: "Radios".to_string(),
        text: "Radiosender".to_string(),
        text_param_ref: None,
        show_in_com_object_tree: false,
        children,
    }
}

/// Top-level "Allgemein" block holding the number-of-zones dropdown. Displayed via the
/// `P-` parameter ref (like the channel name fields); the zone `<choose>` conditions gate
/// on the same parameter's `UP-` ref.
fn build_general_block(app: &knx_rs_prod::author::AppProgram) -> knx_rs_prod::author::Dyn {
    use knx_rs_prod::author::{Dyn, When};

    let mut children: Vec<Dyn> = Vec::new();
    // Counts: number of zones then number of clients drive which channel blocks show.
    children.push(Dyn::ParamRefRef(app.param_ref("G000")));
    children.push(Dyn::ParamRefRef(app.param_ref("G003")));
    // Server settings + HTTP API keys — the key authenticates the HTTP API configured here,
    // so it lives next to the port. The count dropdown gates how many key fields show; key N
    // is revealed when NumApiKeys >= N (mirrors the radio presets).
    push_param_section(
        app,
        &mut children,
        "G-Server",
        "Server",
        &["G001", "G002", "G021"], // HTTP port, Log level, Number of API keys
    );
    let api_count = app.param_ref("G021");
    for k in 1..=MAX_API_KEYS {
        let kid = format!("G{:03}", 21 + k);
        children.push(Dyn::Choose {
            param_ref: api_count,
            whens: vec![When {
                test: format!(">={k}"),
                children: vec![Dyn::ParamRefRef(app.param_ref(&kid))],
            }],
        });
    }
    // Global audio output format (server-wide, not per-zone): SampleRate, BitDepth, Codec,
    // SourceConflict, ZoneFade, SourceFade, Snapcast PSK.
    push_param_section(
        app,
        &mut children,
        "G-Audio",
        "Audio",
        &["G004", "G005", "G006", "G007", "G008", "G009", "G017"],
    );
    // Subsonic.
    push_param_section(
        app,
        &mut children,
        "G-Subsonic",
        "Subsonic",
        &["G010", "G011", "G012"],
    );
    // MQTT: Broker, Topic, Password.
    push_param_section(
        app,
        &mut children,
        "G-MQTT",
        "MQTT",
        &["G013", "G014", "G015"],
    );
    // AirPlay: Password.
    push_param_section(app, &mut children, "G-AirPlay", "AirPlay", &["G016"]);
    // Secrets are grouped with their subsystem (API keys → Server, PSK → Audio, MQTT /
    // AirPlay / Subsonic passwords in their own sections) — no separate Security heading.
    push_info_section(&mut children);
    Dyn::ParameterBlock {
        suffix: "General".to_string(),
        name: "General".to_string(),
        text: "Allgemein".to_string(),
        text_param_ref: None,
        show_in_com_object_tree: false,
        children,
    }
}

/// Device-level "System" block: the heartbeat-interval parameter plus the global group
/// objects (Server Online, All Stop, All Mute, System Fault, KNX Time), shown under their
/// own drawer in the `ComObject` tree.
fn build_system_block(app: &knx_rs_prod::author::AppProgram) -> knx_rs_prod::author::Dyn {
    use knx_rs_prod::author::Dyn;

    let mut children: Vec<Dyn> = Vec::new();
    children.push(Dyn::ParamRefRef(app.param_ref("G040")));
    for i in 0..GLOBAL_GOS.len() {
        children.push(Dyn::ComObjRefRef(app.com_object_ref(&format!("GG{i:03}"))));
    }
    Dyn::ParameterBlock {
        suffix: "System".to_string(),
        name: "System".to_string(),
        text: "System".to_string(),
        text_param_ref: None,
        show_in_com_object_tree: true,
        children,
    }
}

/// Read-only "Info" block at the end of the General tab: product / copyright / license /
/// source and the DB identity. Pure `ParameterSeparator` display text — no memory, so it
/// neither touches the layout fingerprint nor needs an app-version bump. Identity fields
/// are single-sourced from the `KNXPROD_*` consts so they can never drift from the artifact.
fn push_info_section(out: &mut Vec<knx_rs_prod::author::Dyn>) {
    use knx_rs_prod::author::Dyn;

    out.push(Dyn::Separator {
        suffix: "Info".to_string(),
        text: "Info".to_string(),
        ui_hint: "Headline".to_string(),
    });
    // Note the raw `&` (not `&amp;`): the author escape_attr's separator text, so a
    // pre-escaped entity would double-escape. The other lines are plain UTF-8, unescaped.
    let info_lines = [
        "SnapDog — Multiroom Audio Server".to_string(),
        "© 2026 Fabian Schmieder".to_string(),
        "Lizenz: GPL-3.0-only · Open-Source-Firmware".to_string(),
        "Quellcode & Support: github.com/SnapDogRocks/snapdog".to_string(),
        format!(
            "Produkt-DB v{KNXPROD_APP_VERSION} · App 0x{KNXPROD_APP_NUMBER:04X} · HW {KNXPROD_HW_VERSION} · KNXnet/IP System B"
        ),
    ];
    for (i, line) in info_lines.iter().enumerate() {
        out.push(Dyn::Separator {
            suffix: format!("Info-{i}"),
            text: line.clone(),
            ui_hint: "Information".to_string(),
        });
    }
}

/// Push a `<ParameterSeparator>` headline followed by a `<ParameterRefRef>` for each
/// parameter suffix onto `out`. `sep_id` must be a document-unique `NCName`; each suffix is
/// resolved to its ref handle via `AppProgram::param_ref`.
fn push_param_section(
    app: &knx_rs_prod::author::AppProgram,
    out: &mut Vec<knx_rs_prod::author::Dyn>,
    sep_id: &str,
    title: &str,
    param_suffixes: &[&str],
) {
    use knx_rs_prod::author::Dyn;

    out.push(Dyn::Separator {
        suffix: sep_id.to_string(),
        text: title.to_string(),
        ui_hint: "Headline".to_string(),
    });
    for suffix in param_suffixes {
        out.push(Dyn::ParamRefRef(app.param_ref(suffix)));
    }
}

#[allow(clippy::too_many_arguments)]
fn build_channel_block(
    app: &knx_rs_prod::author::AppProgram,
    prefix: &str,
    idx: usize,
    cond_suffix: &str,
    test: &str,
    name_suffix: &str,
    groups: &[CoGroup],
    id_prefix: &str,
    config_nums: &[&str],
) -> knx_rs_prod::author::Dyn {
    use knx_rs_prod::author::{Dyn, When};

    let name_ref = app.param_ref(name_suffix);
    let mut block: Vec<Dyn> = Vec::new();
    // Name parameter
    block.push(Dyn::ParamRefRef(name_ref));
    // Editable configuration knobs for this channel (memory-backed, from mem::).
    if !config_nums.is_empty() {
        let ids: Vec<String> = config_nums
            .iter()
            .map(|num| format!("{id_prefix}{num}"))
            .collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        push_param_section(
            app,
            &mut block,
            &format!("{id_prefix}-Einstellungen"),
            "Einstellungen",
            &id_refs,
        );
    }
    // CO groups — the `_R-<number>` comes from the com-object handle, so it can't drift
    // from `register_com_objects`' numbering.
    for group in groups {
        let sanitized: String = group
            .title_en
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .collect();
        block.push(Dyn::Separator {
            suffix: format!("{id_prefix}-{sanitized}"),
            text: group.title_de.to_string(),
            ui_hint: "Headline".to_string(),
        });
        for &i in group.indices {
            block.push(Dyn::ComObjRefRef(
                app.com_object_ref(&format!("{id_prefix}{i:03}")),
            ));
        }
    }
    Dyn::Choose {
        param_ref: app.param_ref(cond_suffix),
        whens: vec![When {
            test: test.to_string(),
            children: vec![Dyn::ParameterBlock {
                suffix: id_prefix.to_string(),
                name: format!("{prefix}{idx}"),
                text: format!("{prefix} {idx}: {{{{0: ...}}}}"),
                text_param_ref: Some(name_ref),
                show_in_com_object_tree: true,
                children: block,
            }],
        }],
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    /// Runs the same generation the release job does (`generate_xml` →
    /// `assert_layout_locked`) but purely in memory — no file writes, no network. Any drift
    /// of the ETS byte layout or object semantics from the committed `EXPECTED_LAYOUT_HASH`
    /// panics here, so `cargo test` (CI + `cargo xtask ci` + pre-push) fails fast rather
    /// than only when the `.knxprod` is regenerated. Bump `KNXPROD_APP_VERSION` and the
    /// hash together to fix.
    #[test]
    fn ets_memory_layout_is_locked() {
        let xml = super::generate_xml();
        assert!(xml.contains("<KNX"), "generation produced no XML");
    }

    /// Freshness guard: the committed `knx/snapdog.knxprod` — embedded verbatim by the
    /// server via `include_bytes!` and served for ETS import — must carry the current
    /// `KNXPROD_APP_VERSION`. The layout guard forces a version bump on any layout/DPT
    /// change; this ensures that bump actually reached the shipped bytes, catching a
    /// "bumped the version but forgot to regenerate the artifact" mistake. Fix by running
    /// `cargo xtask knxprod` and committing the regenerated `.knxprod`.
    #[test]
    fn committed_knxprod_matches_app_version() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../knx/snapdog.knxprod");
        let file = std::fs::File::open(path).unwrap_or_else(|e| panic!("cannot open {path}: {e}"));
        let mut zip =
            zip::ZipArchive::new(file).expect("committed .knxprod is not a valid ZIP archive");

        let mut shipped: Option<u32> = None;
        for i in 0..zip.len() {
            let mut entry = zip.by_index(i).expect("read zip entry");
            let mut xml = String::new();
            // Skip non-text entries (e.g. a signature blob in a signed archive).
            if entry.read_to_string(&mut xml).is_err() {
                continue;
            }
            // Pin to the ApplicationProgram element's own attribute.
            if let Some(v) = xml
                .split("<ApplicationProgram ")
                .nth(1)
                .and_then(|tag| tag.split("ApplicationVersion=\"").nth(1))
                .and_then(|rest| rest.split('"').next())
                .and_then(|s| s.parse::<u32>().ok())
            {
                shipped = Some(v);
                break;
            }
        }
        let shipped =
            shipped.expect("no ApplicationProgram/ApplicationVersion in committed .knxprod");
        assert_eq!(
            shipped,
            super::KNXPROD_APP_VERSION,
            "\n\n  STALE ARTIFACT: committed knx/snapdog.knxprod ships \
             ApplicationVersion={shipped} but KNXPROD_APP_VERSION={}.\n  \
             Run `cargo xtask knxprod` and commit the regenerated knx/snapdog.knxprod.\n",
            super::KNXPROD_APP_VERSION,
        );
    }

    /// Byte-exact snapshot of `generate_xml()`. The `knx-rs-prod::author` migration
    /// strangles the hand-written generator section-by-section; this test must stay
    /// byte-identical against the pre-migration baseline at every step. Run with
    /// `BLESS=1` to (re)capture after a *conscious* product change.
    #[test]
    fn generate_xml_matches_golden() {
        let generated = super::generate_xml();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden/product.xml");
        let exists = std::path::Path::new(path).exists();
        if std::env::var_os("BLESS").is_some() || !exists {
            if let Some(dir) = std::path::Path::new(path).parent() {
                std::fs::create_dir_all(dir).expect("create golden dir");
            }
            std::fs::write(path, &generated).expect("write golden");
            return;
        }
        let golden = std::fs::read_to_string(path).expect("read golden");
        assert_eq!(
            generated, golden,
            "generate_xml() drifted from the golden baseline — set BLESS=1 after a conscious change"
        );
    }
}
