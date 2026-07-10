import Foundation
import TOMLKit

// NOTE: read values through TOMLKit's typed accessors (`?.table`, `?.string`, `?.int`,
// `?.bool`, `?.array`). The `as?` casts that looked idiomatic (`table["x"] as? TOMLTable`,
// `x["k"] as? String`) silently return nil in TOMLKit 0.6 — the whole parser used to load
// nothing and round-trip only defaults. Verified against TOMLKit 0.6.0.
enum TOMLConfigParser {
    static func load(from url: URL) throws -> ConfigModel {
        let content = try String(contentsOf: url, encoding: .utf8)
        let table = try TOMLTable(string: content)
        let model = ConfigModel()

        // Subsonic
        if let sub = table["subsonic"]?.table {
            model.subsonic.enabled = true
            model.subsonic.url = sub["url"]?.string ?? ""
            model.subsonic.username = sub["username"]?.string ?? ""
            model.subsonic.password = sub["password"]?.string ?? ""
            model.subsonic.format = sub["format"]?.string ?? "raw"
            model.subsonic.tlsSkipVerify = sub["tls_skip_verify"]?.bool ?? false
        }

        // Spotify
        if let sp = table["spotify"]?.table {
            model.spotify.enabled = true
            model.spotify.name = sp["name"]?.string ?? "SnapDog"
            model.spotify.bitrate = sp["bitrate"]?.int ?? 320
        }

        // Snapcast / streaming
        if let snap = table["snapcast"]?.table {
            model.codec = snap["codec"]?.string ?? "flac"
            model.encryptionPsk = snap["encryption_psk"]?.string ?? ""
            model.streamingPort = snap["streaming_port"]?.int ?? 1704
            model.unknownClients = snap["unknown_clients"]?.string ?? "accept"
            model.groupVolumeMode = snap["group_volume_mode"]?.string ?? "compressed"
            model.defaultZone = snap["default_zone"]?.string ?? ""
        }

        // Audio output + mixing
        if let audio = table["audio"]?.table {
            model.sampleRate = audio["sample_rate"]?.int ?? 48000
            model.bitDepth = audio["bit_depth"]?.int ?? 16
            model.sourceConflict = audio["source_conflict"]?.string ?? "last_wins"
            model.zoneSwitchFadeMs = audio["zone_switch_fade_ms"]?.int ?? 300
            model.sourceSwitchFadeMs = audio["source_switch_fade_ms"]?.int ?? 300
        }

        // HTTP
        if let http = table["http"]?.table {
            model.httpPort = http["port"]?.int ?? 5555
            if let keys = http["api_keys"]?.array {
                model.apiKeys = keys.compactMap { $0.string }
                    .map { ConfigModel.ApiKeyEntry(value: $0) }
            }
        }

        // AirPlay
        if let ap = table["airplay"]?.table {
            model.airplayPassword = ap["password"]?.string ?? ""
            model.airplayMode = ap["mode"]?.string ?? "airplay2"
        }

        // MQTT
        if let mqtt = table["mqtt"]?.table {
            model.mqtt.enabled = true
            model.mqtt.broker = mqtt["broker"]?.string ?? ""
            model.mqtt.clientId = mqtt["client_id"]?.string ?? "snapdog"
            model.mqtt.username = mqtt["username"]?.string ?? ""
            model.mqtt.password = mqtt["password"]?.string ?? ""
            model.mqtt.baseTopic = mqtt["base_topic"]?.string ?? "snapdog"
        }

        // KNX — global settings only (per-zone/client GA matrices are left untouched)
        if let knx = table["knx"]?.table {
            model.knx.enabled = true
            model.knx.role = knx["role"]?.string ?? knx["mode"]?.string ?? "client"
            model.knx.url = knx["url"]?.string ?? ""
            model.knx.individualAddress = knx["individual_address"]?.string ?? ""
            model.knx.persistEts = knx["persist_ets_config"]?.bool ?? true
            model.knx.restartAfterEts = knx["restart_after_ets"]?.bool ?? true
        }

        // Zones
        if let zones = table["zone"]?.array {
            model.zones = zones.compactMap { $0.table }.map { z in
                ConfigModel.ZoneEntry(
                    name: z["name"]?.string ?? "",
                    icon: z["icon"]?.string ?? "🏠"
                )
            }
        }

        // Clients
        if let clients = table["client"]?.array {
            model.clients = clients.compactMap { $0.table }.map { c in
                ConfigModel.ClientEntry(
                    name: c["name"]?.string ?? "",
                    mac: c["mac"]?.string ?? "",
                    zone: c["zone"]?.string ?? "",
                    icon: c["icon"]?.string ?? "🔊"
                )
            }
        }

        // Radios
        if let radios = table["radio"]?.array {
            model.radios = radios.compactMap { $0.table }.map { r in
                ConfigModel.RadioEntry(
                    name: r["name"]?.string ?? "",
                    url: r["url"]?.string ?? "",
                    cover: r["cover"]?.string ?? ""
                )
            }
        }

        return model
    }

    static func save(_ model: ConfigModel, to url: URL) throws {
        // Load the existing file so keys the UI doesn't manage are preserved.
        let existing: TOMLTable
        if let content = try? String(contentsOf: url, encoding: .utf8),
           let table = try? TOMLTable(string: content) {
            existing = table
        } else {
            existing = TOMLTable()
        }

        // HTTP — port only; api_keys are Keychain-backed (injected via env), never on disk.
        let http = existing["http"]?.table ?? TOMLTable()
        http["port"] = model.httpPort
        if http["base_url"]?.string == nil {
            http["base_url"] = "http://localhost:\(model.httpPort)"
        }
        http["api_keys"] = nil
        existing["http"] = http

        // Audio — output format + mixing from the model
        let audio = existing["audio"]?.table ?? TOMLTable()
        audio["sample_rate"] = model.sampleRate
        audio["bit_depth"] = model.bitDepth
        audio["source_conflict"] = model.sourceConflict
        audio["zone_switch_fade_ms"] = model.zoneSwitchFadeMs
        audio["source_switch_fade_ms"] = model.sourceSwitchFadeMs
        existing["audio"] = audio

        // Snapcast — codec, streaming, grouping
        let snap = existing["snapcast"]?.table ?? TOMLTable()
        snap["codec"] = model.codec
        // Encryption PSK is Keychain-backed (injected via SNAPDOG_SNAPCAST_ENCRYPTION_PSK).
        snap["encryption_psk"] = nil
        snap["streaming_port"] = model.streamingPort
        snap["group_volume_mode"] = model.groupVolumeMode
        snap["unknown_clients"] = model.unknownClients
        snap["default_zone"] = model.defaultZone.isEmpty ? nil : model.defaultZone
        existing["snapcast"] = snap

        // Subsonic
        if model.subsonic.enabled && !model.subsonic.url.isEmpty {
            let sub = existing["subsonic"]?.table ?? TOMLTable()
            sub["url"] = model.subsonic.url
            sub["username"] = model.subsonic.username
            // Placeholder only — the server requires the key to deserialize, but the real
            // value is Keychain-backed and injected via SNAPDOG_SUBSONIC_PASSWORD.
            sub["password"] = ""
            sub["format"] = model.subsonic.format
            sub["tls_skip_verify"] = model.subsonic.tlsSkipVerify
            existing["subsonic"] = sub
        } else {
            existing["subsonic"] = nil
        }

        // Spotify
        if model.spotify.enabled && !model.spotify.name.isEmpty {
            let sp = existing["spotify"]?.table ?? TOMLTable()
            sp["name"] = model.spotify.name
            sp["bitrate"] = model.spotify.bitrate
            existing["spotify"] = sp
        } else {
            existing["spotify"] = nil
        }

        // AirPlay — write the section if a password or a non-default mode is set.
        if !model.airplayPassword.isEmpty || model.airplayMode != "airplay2" {
            let ap = existing["airplay"]?.table ?? TOMLTable()
            ap["password"] = model.airplayPassword.isEmpty ? nil : model.airplayPassword
            ap["mode"] = model.airplayMode
            existing["airplay"] = ap
        } else {
            existing["airplay"] = nil
        }

        // MQTT
        if model.mqtt.enabled {
            let mqtt = existing["mqtt"]?.table ?? TOMLTable()
            mqtt["broker"] = model.mqtt.broker
            mqtt["client_id"] = model.mqtt.clientId
            mqtt["username"] = model.mqtt.username.isEmpty ? nil : model.mqtt.username
            // Password is Keychain-backed (injected via SNAPDOG_MQTT_PASSWORD).
            mqtt["password"] = nil
            mqtt["base_topic"] = model.mqtt.baseTopic
            existing["mqtt"] = mqtt
        } else {
            existing["mqtt"] = nil
        }

        // KNX — global settings only; per-zone/client GA tables are preserved in place.
        if model.knx.enabled {
            let knx = existing["knx"]?.table ?? TOMLTable()
            knx["role"] = model.knx.role
            knx["url"] = model.knx.url.isEmpty ? nil : model.knx.url
            knx["individual_address"] =
                model.knx.individualAddress.isEmpty ? nil : model.knx.individualAddress
            knx["persist_ets_config"] = model.knx.persistEts
            knx["restart_after_ets"] = model.knx.restartAfterEts
            existing["knx"] = knx
        } else {
            existing["knx"] = nil
        }

        // Zones — preserve keys the app doesn't model (`.knx` GAs, `sink`, `airplay_name`,
        // `presence`, per-zone `group_volume_mode`) by reusing the existing table with the
        // same name; only name/icon are overwritten. Renamed/removed zones lose their extras.
        var existingZones: [String: TOMLTable] = [:]
        if let arr = existing["zone"]?.array {
            for item in arr {
                if let z = item.table, let name = z["name"]?.string, !name.isEmpty {
                    existingZones[name] = z
                }
            }
        }
        existing["zone"] = nil
        let zonesArr = TOMLArray()
        for zone in model.zones where !zone.name.isEmpty {
            let z = existingZones[zone.name] ?? TOMLTable()
            z["name"] = zone.name
            z["icon"] = zone.icon
            zonesArr.append(z)
        }
        if !model.zones.isEmpty { existing["zone"] = zonesArr }

        // Clients — same preserve-merge (keeps `.knx` GAs etc. the app doesn't model).
        var existingClients: [String: TOMLTable] = [:]
        if let arr = existing["client"]?.array {
            for item in arr {
                if let c = item.table, let name = c["name"]?.string, !name.isEmpty {
                    existingClients[name] = c
                }
            }
        }
        existing["client"] = nil
        let clientsArr = TOMLArray()
        for client in model.clients where !client.name.isEmpty {
            let c = existingClients[client.name] ?? TOMLTable()
            c["name"] = client.name
            c["mac"] = client.mac
            c["zone"] = client.zone
            c["icon"] = client.icon
            clientsArr.append(c)
        }
        if !model.clients.isEmpty { existing["client"] = clientsArr }

        // Radios (fully modelled by the app — name/url/cover)
        existing["radio"] = nil
        let radiosArr = TOMLArray()
        for radio in model.radios where !radio.name.isEmpty {
            let r = TOMLTable()
            r["name"] = radio.name
            r["url"] = radio.url
            if !radio.cover.isEmpty { r["cover"] = radio.cover }
            radiosArr.append(r)
        }
        if !model.radios.isEmpty { existing["radio"] = radiosArr }

        try existing.convert().write(to: url, atomically: true, encoding: .utf8)
    }
}
