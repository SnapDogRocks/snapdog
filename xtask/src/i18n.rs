// SPDX-License-Identifier: GPL-3.0-only
// Copyright (C) 2026 Fabian Schmieder

//! Translations of every text ETS shows for the product: object names, parameter names,
//! the group and block headings and the drop-down entries.
//!
//! The product is written in German (`DefaultLanguage="de-DE"`). This module adds a
//! `<Languages>` section with English, Dutch, Spanish and French, keyed by the German text.
//! A text that is not in the table and matches no pattern stops the generation, so a new
//! parameter cannot ship untranslated. A translation that equals the German text is left out
//! (ETS shows the base text when there is no translation).
//!
//! Object names carry their English name in the table as well; a test keeps it equal to the
//! `FunctionText` the objects are generated with.

use std::fmt::Write as _;

/// Languages added to the product, in the order of the translations in [`TABLE`].
const LANGUAGES: [&str; 4] = ["en-US", "nl-NL", "es-ES", "fr-FR"];

/// The German text of the version-mismatch message, shown when the base language is German.
const MISMATCH_DE: &str = "Applikations- und Firmware-Version stimmen nicht überein.";

/// German text and its translations (English, Dutch, Spanish, French).
struct Entry {
    de: &'static str,
    tr: [&'static str; 4],
}

const fn e(
    de: &'static str,
    en: &'static str,
    nl: &'static str,
    es: &'static str,
    fr: &'static str,
) -> Entry {
    Entry {
        de,
        tr: [en, nl, es, fr],
    }
}

/// Fixed texts. Counted texts ("5 Zonen", "Station 7") are handled by [`pattern`].
const TABLE: &[Entry] = &[
    // ── Group objects (the English column is the object's `FunctionText`) ──
    e("Album", "Track Album", "Album", "Álbum", "Album"),
    e(
        "Alle Stopp",
        "All Stop",
        "Alles stoppen",
        "Detener todo",
        "Tout arrêter",
    ),
    e(
        "Alle Stumm",
        "All Mute",
        "Alles dempen",
        "Silenciar todo",
        "Tout couper",
    ),
    e(
        "Fortschritt",
        "Track Progress",
        "Voortgang",
        "Progreso",
        "Progression",
    ),
    e(
        "Titelnummer",
        "Track Number",
        "Tracknummer",
        "Número de pista",
        "Numéro de piste",
    ),
    e(
        "Titelnummer Status",
        "Track Number Status",
        "Tracknummer status",
        "Estado del número de pista",
        "État du numéro de piste",
    ),
    e("Interpret", "Track Artist", "Artiest", "Artista", "Artiste"),
    e(
        "KNX Uhrzeit",
        "KNX Time",
        "KNX-tijd",
        "Hora KNX",
        "Heure KNX",
    ),
    e("Latenz", "Latency", "Latentie", "Latencia", "Latence"),
    e(
        "Latenz Status",
        "Latency Status",
        "Latentie status",
        "Estado de latencia",
        "État de la latence",
    ),
    e("Lautstärke", "Volume", "Volume", "Volumen", "Volume"),
    e(
        "Lautstärke Dimmen",
        "Volume Dim",
        "Volume dimmen",
        "Volumen (regulación)",
        "Volume (variation)",
    ),
    e(
        "Lautstärke Status",
        "Volume Status",
        "Volume status",
        "Estado del volumen",
        "État du volume",
    ),
    e(
        "Nächste Playlist",
        "Playlist Next",
        "Volgende playlist",
        "Lista siguiente",
        "Playlist suivante",
    ),
    e(
        "Nächster Titel",
        "Track Next",
        "Volgend nummer",
        "Pista siguiente",
        "Titre suivant",
    ),
    e("Pause", "Pause", "Pauze", "Pausa", "Pause"),
    e("Play", "Play", "Afspelen", "Reproducir", "Lecture"),
    e(
        "Playlist",
        "Playlist",
        "Playlist",
        "Lista de reproducción",
        "Playlist",
    ),
    e(
        "Playlist Name",
        "Playlist Name",
        "Naam playlist",
        "Nombre de la lista",
        "Nom de la playlist",
    ),
    e(
        "Playlist Schritt",
        "Playlist Step",
        "Playlist stap",
        "Lista: paso",
        "Playlist: pas",
    ),
    e(
        "Playlist Schritt Auf/Ab",
        "Playlist Step Up/Down",
        "Playlist stap omhoog/omlaag",
        "Lista: paso arriba/abajo",
        "Playlist: pas haut/bas",
    ),
    e(
        "Playlist Status",
        "Playlist Status",
        "Playlist status",
        "Estado de la lista",
        "État de la playlist",
    ),
    e(
        "Präsenz",
        "Presence",
        "Aanwezigheid",
        "Presencia",
        "Présence",
    ),
    e(
        "Präsenz Aktiviert",
        "Presence Enable",
        "Aanwezigheid activeren",
        "Activar presencia",
        "Activer la présence",
    ),
    e(
        "Präsenz Timer",
        "Presence Timer Active",
        "Aanwezigheidstimer actief",
        "Temporizador de presencia activo",
        "Minuterie de présence active",
    ),
    e(
        "Server Online",
        "Server Online",
        "Server online",
        "Servidor en línea",
        "Serveur en ligne",
    ),
    e("Stop", "Stop", "Stop", "Detener", "Arrêt"),
    e("Stumm", "Mute", "Dempen", "Silenciar", "Sourdine"),
    e(
        "Stumm Status",
        "Mute Status",
        "Demping status",
        "Estado de silencio",
        "État de la sourdine",
    ),
    e(
        "Stumm Umschalten",
        "Mute Toggle",
        "Dempen omschakelen",
        "Alternar silencio",
        "Basculer la sourdine",
    ),
    e(
        "Systemstörung",
        "System Fault",
        "Systeemstoring",
        "Fallo del sistema",
        "Défaut système",
    ),
    e("Titel", "Track Title", "Titel", "Título", "Titre"),
    e(
        "Titel Schritt",
        "Track Step",
        "Nummer stap",
        "Pista: paso",
        "Titre: pas",
    ),
    e(
        "Titel Schritt Auf/Ab",
        "Track Step Up/Down",
        "Nummer stap omhoog/omlaag",
        "Pista: paso arriba/abajo",
        "Titre: pas haut/bas",
    ),
    e(
        "Titel Wdh. Status",
        "Track Repeat Status",
        "Nummer herhalen status",
        "Estado repetir pista",
        "État répétition titre",
    ),
    e(
        "Titel Wdh. Umsch.",
        "Track Repeat Toggle",
        "Nummer herhalen omschakelen",
        "Alternar repetir pista",
        "Basculer répétition titre",
    ),
    e(
        "Titel Wiederholung",
        "Track Repeat",
        "Nummer herhalen",
        "Repetir pista",
        "Répétition du titre",
    ),
    e(
        "Titel spielt",
        "Track Playing",
        "Nummer speelt",
        "Pista en reproducción",
        "Titre en lecture",
    ),
    e(
        "Verbunden",
        "Connected",
        "Verbonden",
        "Conectado",
        "Connecté",
    ),
    e(
        "Vorherige Playlist",
        "Playlist Previous",
        "Vorige playlist",
        "Lista anterior",
        "Playlist précédente",
    ),
    e(
        "Vorheriger Titel",
        "Track Previous",
        "Vorig nummer",
        "Pista anterior",
        "Titre précédent",
    ),
    e(
        "Wiedergabe Status",
        "Control Status",
        "Weergavestatus",
        "Estado de reproducción",
        "État de lecture",
    ),
    e(
        "Wiedergabe/Pause",
        "Play/Pause",
        "Afspelen/pauze",
        "Reproducir/pausa",
        "Lecture/pause",
    ),
    e("Wiederholung", "Repeat", "Herhalen", "Repetir", "Répéter"),
    e(
        "Wiederholung Status",
        "Repeat Status",
        "Herhalen status",
        "Estado de repetición",
        "État de répétition",
    ),
    e(
        "Wiederholung Umsch.",
        "Repeat Toggle",
        "Herhalen omschakelen",
        "Alternar repetición",
        "Basculer répétition",
    ),
    e(
        "Zone Status",
        "Zone Status",
        "Zone status",
        "Estado de zona",
        "État de la zone",
    ),
    e(
        "Zonenzuordnung",
        "Zone",
        "Zonetoewijzing",
        "Asignación de zona",
        "Affectation de zone",
    ),
    e(
        "Zufall Status",
        "Shuffle Status",
        "Willekeurig status",
        "Estado aleatorio",
        "État aléatoire",
    ),
    e(
        "Zufall Umschalten",
        "Shuffle Toggle",
        "Willekeurig omschakelen",
        "Alternar aleatorio",
        "Basculer aléatoire",
    ),
    e(
        "Zufallswiedergabe",
        "Shuffle",
        "Willekeurig afspelen",
        "Reproducción aleatoria",
        "Lecture aléatoire",
    ),
    // ── Parameters ──
    e(
        "AirPlay Passwort",
        "AirPlay password",
        "AirPlay-wachtwoord",
        "Contraseña de AirPlay",
        "Mot de passe AirPlay",
    ),
    e(
        "AirPlay aktiviert",
        "AirPlay enabled",
        "AirPlay ingeschakeld",
        "AirPlay activado",
        "AirPlay activé",
    ),
    e(
        "AirPlay-Name (leer = Zonenname)",
        "AirPlay name (empty = zone name)",
        "AirPlay-naam (leeg = zonenaam)",
        "Nombre de AirPlay (vacío = nombre de la zona)",
        "Nom AirPlay (vide = nom de la zone)",
    ),
    e(
        "Anzahl API-Keys",
        "Number of API keys",
        "Aantal API-sleutels",
        "Número de claves API",
        "Nombre de clés API",
    ),
    e(
        "Anzahl Clients",
        "Number of clients",
        "Aantal clients",
        "Número de clientes",
        "Nombre de clients",
    ),
    e(
        "Anzahl Radiosender",
        "Number of radio stations",
        "Aantal radiozenders",
        "Número de emisoras de radio",
        "Nombre de stations de radio",
    ),
    e(
        "Anzahl Zonen",
        "Number of zones",
        "Aantal zones",
        "Número de zonas",
        "Nombre de zones",
    ),
    e(
        "Bit Depth",
        "Bit depth",
        "Bitdiepte",
        "Profundidad de bits",
        "Profondeur de bits",
    ),
    e(
        "Clientname",
        "Client name",
        "Clientnaam",
        "Nombre del cliente",
        "Nom du client",
    ),
    e("Codec", "Codec", "Codec", "Códec", "Codec"),
    e(
        "Cover URL (optional)",
        "Cover URL (optional)",
        "Cover-URL (optioneel)",
        "URL de la carátula (opcional)",
        "URL de la pochette (facultatif)",
    ),
    e(
        "HTTP Port",
        "HTTP port",
        "HTTP-poort",
        "Puerto HTTP",
        "Port HTTP",
    ),
    e(
        "Heartbeat-Intervall",
        "Heartbeat interval",
        "Heartbeat-interval",
        "Intervalo de latido (heartbeat)",
        "Intervalle du heartbeat",
    ),
    e(
        "Icon (Emoji)",
        "Icon (emoji)",
        "Pictogram (emoji)",
        "Icono (emoji)",
        "Icône (emoji)",
    ),
    e(
        "Log Level",
        "Log level",
        "Logniveau",
        "Nivel de registro",
        "Niveau de journalisation",
    ),
    e(
        "MAC-Adresse",
        "MAC address",
        "MAC-adres",
        "Dirección MAC",
        "Adresse MAC",
    ),
    e(
        "MQTT Base Topic",
        "MQTT base topic",
        "MQTT-basistopic",
        "Tema base de MQTT",
        "Topic de base MQTT",
    ),
    e(
        "MQTT Broker",
        "MQTT broker",
        "MQTT-broker",
        "Broker MQTT",
        "Broker MQTT",
    ),
    e(
        "MQTT Passwort",
        "MQTT password",
        "MQTT-wachtwoord",
        "Contraseña de MQTT",
        "Mot de passe MQTT",
    ),
    e(
        "Max. Lautstärke",
        "Max. volume",
        "Max. volume",
        "Volumen máx.",
        "Volume max.",
    ),
    e(
        "Präsenz Auto-Off (s)",
        "Presence auto-off (s)",
        "Aanwezigheid auto-uit (s)",
        "Apagado automático por presencia (s)",
        "Arrêt automatique présence (s)",
    ),
    e(
        "Präsenz aktiviert",
        "Presence enabled",
        "Aanwezigheid ingeschakeld",
        "Presencia activada",
        "Présence activée",
    ),
    e(
        "Präsenz-Quelle (0=keine, 1..20=Radio)",
        "Presence source (0=none, 1..20=radio)",
        "Aanwezigheidsbron (0=geen, 1..20=radio)",
        "Fuente de presencia (0=ninguna, 1..20=radio)",
        "Source de présence (0=aucune, 1..20=radio)",
    ),
    e(
        "Quellen-Konflikt",
        "Source conflict",
        "Bronconflict",
        "Conflicto de fuentes",
        "Conflit de sources",
    ),
    e(
        "Quellenwechsel-Fade (ms)",
        "Source switch fade (ms)",
        "Fade bij bronwissel (ms)",
        "Fundido al cambiar de fuente (ms)",
        "Fondu au changement de source (ms)",
    ),
    e(
        "Sample Rate",
        "Sample rate",
        "Samplefrequentie",
        "Frecuencia de muestreo",
        "Fréquence d'échantillonnage",
    ),
    e(
        "Snapcast PSK (Verschlüsselung)",
        "Snapcast PSK (encryption)",
        "Snapcast-PSK (versleuteling)",
        "PSK de Snapcast (cifrado)",
        "PSK Snapcast (chiffrement)",
    ),
    e(
        "Spotify aktiviert",
        "Spotify enabled",
        "Spotify ingeschakeld",
        "Spotify activado",
        "Spotify activé",
    ),
    e(
        "Spotify-Name (leer = Zonenname)",
        "Spotify name (empty = zone name)",
        "Spotify-naam (leeg = zonenaam)",
        "Nombre de Spotify (vacío = nombre de la zona)",
        "Nom Spotify (vide = nom de la zone)",
    ),
    e(
        "Standard-Latenz (ms)",
        "Default latency (ms)",
        "Standaardlatentie (ms)",
        "Latencia predeterminada (ms)",
        "Latence par défaut (ms)",
    ),
    e(
        "Standard-Lautstärke",
        "Default volume",
        "Standaardvolume",
        "Volumen predeterminado",
        "Volume par défaut",
    ),
    e(
        "Standard-Zone",
        "Default zone",
        "Standaardzone",
        "Zona predeterminada",
        "Zone par défaut",
    ),
    e(
        "Stationsname",
        "Station name",
        "Zendernaam",
        "Nombre de la emisora",
        "Nom de la station",
    ),
    e(
        "Stream URL",
        "Stream URL",
        "Stream-URL",
        "URL del stream",
        "URL du flux",
    ),
    e(
        "Subsonic Benutzer",
        "Subsonic user",
        "Subsonic-gebruiker",
        "Usuario de Subsonic",
        "Utilisateur Subsonic",
    ),
    e(
        "Subsonic Passwort",
        "Subsonic password",
        "Subsonic-wachtwoord",
        "Contraseña de Subsonic",
        "Mot de passe Subsonic",
    ),
    e(
        "Subsonic URL",
        "Subsonic URL",
        "Subsonic-URL",
        "URL de Subsonic",
        "URL Subsonic",
    ),
    e(
        "Zonenname",
        "Zone name",
        "Zonenaam",
        "Nombre de la zona",
        "Nom de la zone",
    ),
    e(
        "Zonenwechsel-Fade (ms)",
        "Zone switch fade (ms)",
        "Fade bij zonewissel (ms)",
        "Fundido al cambiar de zona (ms)",
        "Fondu au changement de zone (ms)",
    ),
    // ── Drop-down entries ──
    e(
        "Empfänger hat Vorrang",
        "Receiver has priority",
        "Ontvanger heeft voorrang",
        "El receptor tiene prioridad",
        "Le récepteur est prioritaire",
    ),
    e(
        "Letzte Quelle gewinnt",
        "Last source wins",
        "Laatste bron wint",
        "Gana la última fuente",
        "La dernière source l'emporte",
    ),
    e("Ja", "Yes", "Ja", "Sí", "Oui"),
    e("Nein", "No", "Nee", "No", "Non"),
    e(
        "FLAC (verlustfrei)",
        "FLAC (lossless)",
        "FLAC (verliesvrij)",
        "FLAC (sin pérdidas)",
        "FLAC (sans perte)",
    ),
    e(
        "PCM (unkomprimiert)",
        "PCM (uncompressed)",
        "PCM (ongecomprimeerd)",
        "PCM (sin comprimir)",
        "PCM (non compressé)",
    ),
    e(
        "f32lz4e (verschlüsselt)",
        "f32lz4e (encrypted)",
        "f32lz4e (versleuteld)",
        "f32lz4e (cifrado)",
        "f32lz4e (chiffré)",
    ),
    e("Debug", "Debug", "Debug", "Debug", "Debug"),
    e("Error", "Error", "Error", "Error", "Error"),
    e("Info", "Info", "Info", "Info", "Info"),
    e("Trace", "Trace", "Trace", "Trace", "Trace"),
    e("Warn", "Warn", "Warn", "Warn", "Warn"),
    e("f32lz4", "f32lz4", "f32lz4", "f32lz4", "f32lz4"),
    // ── Blocks and headings ──
    e("Allgemein", "General", "Algemeen", "General", "Général"),
    e(
        "Radiosender",
        "Radio stations",
        "Radiozenders",
        "Emisoras de radio",
        "Stations de radio",
    ),
    e("System", "System", "Systeem", "Sistema", "Système"),
    e("AirPlay", "AirPlay", "AirPlay", "AirPlay", "AirPlay"),
    e("Audio", "Audio", "Audio", "Audio", "Audio"),
    e(
        "Einstellungen",
        "Settings",
        "Instellingen",
        "Ajustes",
        "Paramètres",
    ),
    e(
        "Latenz und Zone",
        "Latency and zone",
        "Latentie en zone",
        "Latencia y zona",
        "Latence et zone",
    ),
    e(
        "Lizenz: GPL-3.0-only · Open-Source-Firmware",
        "License: GPL-3.0-only · open-source firmware",
        "Licentie: GPL-3.0-only · open-sourcefirmware",
        "Licencia: GPL-3.0-only · firmware de código abierto",
        "Licence : GPL-3.0-only · firmware open source",
    ),
    e("MQTT", "MQTT", "MQTT", "MQTT", "MQTT"),
    e(
        "Quellcode & Support: github.com/SnapDogRocks/snapdog",
        "Source code & support: github.com/SnapDogRocks/snapdog",
        "Broncode & support: github.com/SnapDogRocks/snapdog",
        "Código fuente y soporte: github.com/SnapDogRocks/snapdog",
        "Code source et support : github.com/SnapDogRocks/snapdog",
    ),
    e("Server", "Server", "Server", "Servidor", "Serveur"),
    e(
        "SnapDog — Multiroom Audio Server",
        "SnapDog — Multiroom Audio Server",
        "SnapDog — Multiroom Audio Server",
        "SnapDog — Multiroom Audio Server",
        "SnapDog — Multiroom Audio Server",
    ),
    e("Status", "Status", "Status", "Estado", "État"),
    e("Subsonic", "Subsonic", "Subsonic", "Subsonic", "Subsonic"),
    e(
        "Titelinformationen",
        "Track information",
        "Nummerinformatie",
        "Información de la pista",
        "Informations sur le titre",
    ),
    e(
        "Wiedergabe",
        "Playback",
        "Weergave",
        "Reproducción",
        "Lecture",
    ),
    e(
        "Zufallswiedergabe / Wiederholung",
        "Shuffle / repeat",
        "Willekeurig afspelen / herhalen",
        "Reproducción aleatoria / repetir",
        "Lecture aléatoire / répétition",
    ),
    e(
        "© 2026 Fabian Schmieder",
        "© 2026 Fabian Schmieder",
        "© 2026 Fabian Schmieder",
        "© 2026 Fabian Schmieder",
        "© 2026 Fabian Schmieder",
    ),
    // ── Messages (the base text of this one is English) ──
    e(
        "Application and firmware version mismatch.",
        "Application and firmware version mismatch.",
        "Applicatie- en firmwareversie komen niet overeen.",
        "La versión de la aplicación y la del firmware no coinciden.",
        "La version de l'application et celle du firmware ne correspondent pas.",
    ),
];

/// Nouns that follow a count: singular and plural per language (English, Dutch, Spanish, French).
struct Noun {
    de_singular: &'static str,
    de_plural: &'static str,
    singular: [&'static str; 4],
    plural: [&'static str; 4],
}

const NOUNS: &[Noun] = &[
    Noun {
        de_singular: "Zone",
        de_plural: "Zonen",
        singular: ["zone", "zone", "zona", "zone"],
        plural: ["zones", "zones", "zonas", "zones"],
    },
    Noun {
        de_singular: "Client",
        de_plural: "Clients",
        singular: ["client", "client", "cliente", "client"],
        plural: ["clients", "clients", "clientes", "clients"],
    },
    Noun {
        de_singular: "Sender",
        de_plural: "Sender",
        singular: ["station", "zender", "emisora", "station"],
        plural: ["stations", "zenders", "emisoras", "stations"],
    },
    Noun {
        de_singular: "Minute",
        de_plural: "Minuten",
        singular: ["minute", "minuut", "minuto", "minute"],
        plural: ["minutes", "minuten", "minutos", "minutes"],
    },
    Noun {
        de_singular: "API-Key",
        de_plural: "API-Keys",
        singular: ["API key", "API-sleutel", "clave API", "clé API"],
        plural: ["API keys", "API-sleutels", "claves API", "clés API"],
    },
];

/// Word for "Station", "Client" and so on as a heading before a number ("Station 7").
const LABELS: &[(&str, [&str; 4])] = &[
    ("Station", ["Station", "Zender", "Emisora", "Station"]),
    ("Zone", ["Zone", "Zone", "Zona", "Zone"]),
    ("Client", ["Client", "Client", "Cliente", "Client"]),
    (
        "API-Key",
        ["API key", "API-sleutel", "Clave API", "Clé API"],
    ),
];

/// Capitalise the first character (nouns after a number are lower case, headings are not).
fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map_or_else(String::new, |c| {
        c.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Translate a counted or numbered text, `None` when `de` is not one.
fn pattern(de: &str) -> Option<[String; 4]> {
    // "5 Zonen", "1 Client", "30 Minuten": a count and a noun.
    if let Some((count, noun)) = de.split_once(' ')
        && let Ok(n) = count.parse::<u32>()
    {
        for entry in NOUNS {
            let plural = n != 1;
            if noun == entry.de_singular || noun == entry.de_plural {
                // "1 Sender" and "5 Sender" share the German word, the number decides.
                if noun == entry.de_singular && noun != entry.de_plural && plural {
                    continue;
                }
                let words = if plural {
                    &entry.plural
                } else {
                    &entry.singular
                };
                return Some(std::array::from_fn(|i| format!("{n} {}", words[i])));
            }
        }
        // Units and bit depths read the same in every language.
        if matches!(noun, "Hz" | "Bit") {
            return Some(std::array::from_fn(|_| de.to_string()));
        }
    }
    // "Station 7", "Zone 3", "Client 12: {{0: ...}}": a heading and a number, maybe a tail.
    for (label, words) in LABELS {
        if let Some(rest) = de.strip_prefix(label)
            && let Some(rest) = rest.strip_prefix(' ')
        {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let tail = &rest[digits.len()..];
            if !digits.is_empty() && (tail.is_empty() || tail.starts_with(':')) {
                return Some(std::array::from_fn(|i| {
                    format!("{} {digits}{tail}", capitalised(words[i]))
                }));
            }
        }
    }
    // "Produkt-DB v12 · App 0xFF01 · HW 1 · KNXnet/IP System B": the numbers stay.
    if let Some(rest) = de.strip_prefix("Produkt-DB ") {
        let heads = [
            "Product DB",
            "Productdatabase",
            "BD de producto",
            "BD produit",
        ];
        return Some(std::array::from_fn(|i| format!("{} {rest}", heads[i])));
    }
    None
}

/// The translations of one German text, or `None` when there are none.
fn translate(de: &str) -> Option<[String; 4]> {
    TABLE
        .iter()
        .find(|entry| entry.de == de)
        .map(|entry| entry.tr.map(str::to_string))
        .or_else(|| pattern(de))
}

// ── XML ────────────────────────────────────────────────────────────

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The value of attribute `name` in the start tag `tag`, unescaped.
fn attribute(tag: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    let end = tag[start..].find('"')? + start;
    Some(unescape(&tag[start..end]))
}

/// One element whose texts get translated.
struct Item {
    id: String,
    kind: &'static str,
    /// `(attribute, German text)`.
    texts: Vec<(&'static str, String)>,
}

/// Element names with texts ETS shows, and the attributes to translate.
const TRANSLATED: &[(&str, &[&str])] = &[
    ("ComObject", &["Text", "FunctionText"]),
    ("Parameter", &["Text"]),
    ("ParameterSeparator", &["Text"]),
    ("ParameterBlock", &["Text"]),
    ("Enumeration", &["Text"]),
    ("Message", &["Text"]),
];

fn collect(xml: &str) -> (String, Vec<Item>) {
    let mut app_id = String::new();
    let mut items = Vec::new();
    for line in xml.lines() {
        let tag = line.trim_start();
        if !tag.starts_with('<') {
            continue;
        }
        if app_id.is_empty()
            && tag.starts_with("<ApplicationProgram ")
            && let Some(id) = attribute(tag, "Id")
        {
            app_id = id;
        }
        for (name, attributes) in TRANSLATED {
            let head = format!("<{name} ");
            if !tag.starts_with(&head) {
                continue;
            }
            let Some(id) = attribute(tag, "Id") else {
                continue;
            };
            let texts: Vec<(&'static str, String)> = attributes
                .iter()
                .filter_map(|a| attribute(tag, a).map(|t| (*a, t)))
                .collect();
            if !texts.is_empty() {
                items.push(Item {
                    id,
                    kind: name,
                    texts,
                });
            }
        }
    }
    (app_id, items)
}

/// Add a `<Languages>` section with the translations of every text to the generated product XML.
///
/// # Panics
///
/// Panics with the list of texts that have no translation, so a new text cannot ship untranslated.
pub fn add_translations(xml: &str) -> String {
    let (app_id, items) = collect(xml);
    assert!(
        !app_id.is_empty(),
        "no ApplicationProgram in the generated XML"
    );

    let mut missing: Vec<String> = Vec::new();
    // Per language, per item: the translated attributes that differ from the base text.
    let mut blocks: Vec<String> = Vec::new();
    for (lang_index, lang) in LANGUAGES.iter().enumerate() {
        let mut body = String::new();
        for item in &items {
            let mut lines = String::new();
            for (attr, de) in &item.texts {
                let Some(tr) = translate_text(item, attr, de) else {
                    if !missing.contains(de) {
                        missing.push(de.clone());
                    }
                    continue;
                };
                let text = &tr[lang_index];
                // A function text is the English object name: the base text of English is it.
                if text != de {
                    let _ = writeln!(
                        lines,
                        "              <Translation AttributeName=\"{attr}\" Text=\"{}\" />",
                        escape(text)
                    );
                }
            }
            if !lines.is_empty() {
                let _ = write!(
                    body,
                    "            <TranslationElement RefId=\"{}\">\n{lines}            </TranslationElement>\n",
                    item.id
                );
            }
        }
        blocks.push(format!(
            "        <Language Identifier=\"{lang}\">\n          <TranslationUnit RefId=\"{app_id}\">\n{body}          </TranslationUnit>\n        </Language>\n"
        ));
    }
    assert!(
        missing.is_empty(),
        "texts without a translation (add them to xtask/src/i18n.rs): {missing:?}"
    );

    // The message's base text is English, so German needs its own entry.
    let message_id = items
        .iter()
        .find(|i| i.kind == "Message")
        .map(|i| i.id.clone());
    if let Some(id) = message_id {
        blocks.insert(
            0,
            format!(
                "        <Language Identifier=\"de-DE\">\n          <TranslationUnit RefId=\"{app_id}\">\n            <TranslationElement RefId=\"{id}\">\n              <Translation AttributeName=\"Text\" Text=\"{}\" />\n            </TranslationElement>\n          </TranslationUnit>\n        </Language>\n",
                escape(MISMATCH_DE)
            ),
        );
    }

    let languages = format!("      <Languages>\n{}      </Languages>\n", blocks.concat());
    // Languages follows Hardware inside <Manufacturer>.
    let close = "    </Manufacturer>";
    let at = xml
        .rfind(close)
        .expect("no </Manufacturer> in the generated XML");
    let mut out = String::with_capacity(xml.len() + languages.len());
    out.push_str(&xml[..at]);
    out.push_str(&languages);
    out.push_str(&xml[at..]);
    out
}

/// The translations of one text: an object's English name (the `FunctionText`, which is also
/// the English `Text`) and the German text through the table.
fn translate_text(item: &Item, attr: &str, de: &str) -> Option<[String; 4]> {
    if item.kind == "ComObject" && attr == "FunctionText" {
        // The base text is the English name. Other languages translate the object's German text.
        let german = item
            .texts
            .iter()
            .find(|(a, _)| *a == "Text")
            .map(|(_, t)| t.as_str())?;
        let mut tr = translate(german)?;
        tr[0] = de.to_string();
        return Some(tr);
    }
    translate(de)
}

#[cfg(test)]
mod tests {
    use super::*;
    use snapdog::knx::group_objects::{CLIENT_GOS, GLOBAL_GOS, ZONE_GOS};

    #[test]
    fn every_object_has_a_translation_with_its_english_name() {
        for go in ZONE_GOS.iter().chain(CLIENT_GOS).chain(GLOBAL_GOS) {
            let tr = translate(go.name_de)
                .unwrap_or_else(|| panic!("no translation for the object '{}'", go.name_de));
            assert_eq!(
                tr[0], go.name,
                "the English text of '{}' differs from the object's FunctionText",
                go.name_de
            );
        }
    }

    #[test]
    fn counted_texts_follow_the_number() {
        assert_eq!(
            translate("1 Zone").unwrap(),
            ["1 zone", "1 zone", "1 zona", "1 zone"]
        );
        assert_eq!(
            translate("10 Zonen").unwrap(),
            ["10 zones", "10 zones", "10 zonas", "10 zones"]
        );
        assert_eq!(
            translate("1 Sender").unwrap(),
            ["1 station", "1 zender", "1 emisora", "1 station"]
        );
        assert_eq!(
            translate("7 Sender").unwrap(),
            ["7 stations", "7 zenders", "7 emisoras", "7 stations"]
        );
        assert_eq!(translate("96000 Hz").unwrap()[2], "96000 Hz");
        assert_eq!(
            translate("Station 12").unwrap(),
            ["Station 12", "Zender 12", "Emisora 12", "Station 12"]
        );
        assert_eq!(
            translate("Client 3: {{0: ...}}").unwrap()[2],
            "Cliente 3: {{0: ...}}"
        );
        assert_eq!(translate("API-Key 4").unwrap()[3], "Clé API 4");
        assert_eq!(translate("45 Minuten").unwrap()[2], "45 minutos");
    }

    #[test]
    fn unknown_texts_have_no_translation() {
        assert!(translate("Ein Text, den es nicht gibt").is_none());
    }

    #[test]
    fn the_table_has_no_duplicate_german_text() {
        let mut seen = std::collections::HashSet::new();
        for entry in TABLE {
            assert!(seen.insert(entry.de), "duplicate entry for '{}'", entry.de);
        }
    }

    #[test]
    fn the_generated_product_is_translated_completely() {
        let xml = crate::generate_xml();
        assert!(xml.contains("<Languages>"));
        for lang in LANGUAGES {
            assert!(xml.contains(&format!("<Language Identifier=\"{lang}\">")));
        }
        assert!(xml.contains("<Language Identifier=\"de-DE\">"));
        assert!(xml.contains("Playlist: pas"), "French object text");
        assert!(xml.contains("Volumen predeterminado"), "Spanish parameter");
    }
}
