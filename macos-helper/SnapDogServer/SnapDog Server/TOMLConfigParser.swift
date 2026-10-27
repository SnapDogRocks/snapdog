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
            // A missing key stays empty: the server then names each device after its zone.
            // Defaulting to "SnapDog" here would rename every zone's Spotify device the
            // next time the settings are saved, now that the server honours the key.
            model.spotify.name = sp["name"]?.string ?? ""
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
                var entry = ConfigModel.ZoneEntry(
                    name: z["name"]?.string ?? "",
                    icon: z["icon"]?.string ?? "🏠"
                )
                entry.knx = readKnx(z["knx"]?.table)
                entry.spotifyName = z["spotify_name"]?.string ?? ""
                entry.fileName = entry.name
                return entry
            }
        }

        // Clients
        if let clients = table["client"]?.array {
            model.clients = clients.compactMap { $0.table }.map { c in
                var entry = ConfigModel.ClientEntry(
                    name: c["name"]?.string ?? "",
                    mac: c["mac"]?.string ?? "",
                    zone: c["zone"]?.string ?? "",
                    icon: c["icon"]?.string ?? "🔊"
                )
                entry.knx = readKnx(c["knx"]?.table)
                return entry
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

        // HTTP — port only; api_keys are Keychain-backed (injected via env) and never written by the
        // app. A value that is already in the file is left as it is.
        let http = existing["http"]?.table ?? TOMLTable()
        http["port"] = model.httpPort
        if http["base_url"]?.string == nil {
            http["base_url"] = "http://localhost:\(model.httpPort)"
        }
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
        // Encryption PSK is Keychain-backed (injected via SNAPDOG_SNAPCAST_ENCRYPTION_PSK); the app
        // never writes it and leaves a value that is already in the file as it is.
        snap["streaming_port"] = model.streamingPort
        snap["group_volume_mode"] = model.groupVolumeMode
        snap["unknown_clients"] = model.unknownClients
        snap.setOrRemove("default_zone", model.defaultZone.isEmpty ? nil : model.defaultZone)
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
            existing.removeIfPresent("subsonic")
        }

        // Spotify
        // The section is present when Spotify is enabled, whatever the name. The name is
        // written only when set: it names the Spotify device (the server adds the zone name
        // as a suffix when there are several zones), and an empty one means "zone name".
        if model.spotify.enabled {
            let sp = existing["spotify"]?.table ?? TOMLTable()
            let name = model.spotify.name.trimmingCharacters(in: .whitespacesAndNewlines)
            // TOMLKit 0.6 ignores `table[key] = nil`, so an emptied name has to be removed
            // explicitly, or the old one would stay in the file.
            if name.isEmpty {
                sp.removeIfPresent("name")
            } else {
                sp["name"] = name
            }
            sp["bitrate"] = model.spotify.bitrate
            existing["spotify"] = sp
        } else {
            existing.removeIfPresent("spotify")
        }

        // AirPlay — write the section if a password or a non-default mode is set.
        if !model.airplayPassword.isEmpty || model.airplayMode != "airplay2" {
            let ap = existing["airplay"]?.table ?? TOMLTable()
            ap.setOrRemove("password", model.airplayPassword.isEmpty ? nil : model.airplayPassword)
            ap["mode"] = model.airplayMode
            existing["airplay"] = ap
        } else {
            existing.removeIfPresent("airplay")
        }

        // MQTT
        if model.mqtt.enabled {
            let mqtt = existing["mqtt"]?.table ?? TOMLTable()
            mqtt["broker"] = model.mqtt.broker
            mqtt["client_id"] = model.mqtt.clientId
            mqtt.setOrRemove("username", model.mqtt.username.isEmpty ? nil : model.mqtt.username)
            // Password is Keychain-backed (injected via SNAPDOG_MQTT_PASSWORD); the app never writes
            // it and leaves a value that is already in the file as it is.
            mqtt["base_topic"] = model.mqtt.baseTopic
            existing["mqtt"] = mqtt
        } else {
            existing.removeIfPresent("mqtt")
        }

        // KNX — global settings only; per-zone/client GA tables are preserved in place.
        if model.knx.enabled {
            let knx = existing["knx"]?.table ?? TOMLTable()
            knx["role"] = model.knx.role
            knx.setOrRemove("url", model.knx.url.isEmpty ? nil : model.knx.url)
            knx.setOrRemove("individual_address",
                            model.knx.individualAddress.isEmpty ? nil : model.knx.individualAddress)
            knx["persist_ets_config"] = model.knx.persistEts
            knx["restart_after_ets"] = model.knx.restartAfterEts
            existing["knx"] = knx
        } else {
            existing.removeIfPresent("knx")
        }

        // Zones — preserve keys the app doesn't model (`.knx` GAs, `sink`, `airplay_name`,
        // `presence`, per-zone `group_volume_mode`) by reusing the existing table of the zone
        // (found under the name it had in the file, so a rename keeps them); name, icon and
        // `spotify_name` are overwritten. Removed zones lose their extras.
        var existingZones: [String: TOMLTable] = [:]
        if let arr = existing["zone"]?.array {
            for item in arr {
                if let z = item.table, let name = z["name"]?.string, !name.isEmpty {
                    existingZones[name] = z
                }
            }
        }
        let zonesArr = TOMLArray()
        for zone in model.zones where !zone.name.isEmpty {
            // A renamed zone keeps its table: look it up under the name it had in the file first.
            let z = zone.fileName.flatMap { existingZones[$0] } ?? existingZones[zone.name] ?? TOMLTable()
            z["name"] = zone.name
            z["icon"] = zone.icon
            // TOMLKit 0.6 ignores `table[key] = nil`, so a cleared name is removed explicitly.
            let ownSpotifyName = zone.spotifyName.trimmingCharacters(in: .whitespacesAndNewlines)
            if ownSpotifyName.isEmpty {
                z.removeIfPresent("spotify_name")
            } else {
                z["spotify_name"] = ownSpotifyName
            }
            writeKnx(zone.knx, into: z)
            zonesArr.append(z)
        }
        // Replace or drop the array only after it is built: the tables reused above live in the
        // old array, and removing it any earlier would free them.
        if model.zones.isEmpty { existing.removeIfPresent("zone") } else { existing["zone"] = zonesArr }

        // Clients — same preserve-merge (keeps `.knx` GAs etc. the app doesn't model).
        var existingClients: [String: TOMLTable] = [:]
        if let arr = existing["client"]?.array {
            for item in arr {
                if let c = item.table, let name = c["name"]?.string, !name.isEmpty {
                    existingClients[name] = c
                }
            }
        }
        let clientsArr = TOMLArray()
        for client in model.clients where !client.name.isEmpty {
            let c = existingClients[client.name] ?? TOMLTable()
            c["name"] = client.name
            c["mac"] = client.mac
            c["zone"] = client.zone
            c["icon"] = client.icon
            writeKnx(client.knx, into: c)
            clientsArr.append(c)
        }
        // Replace or drop the array only after it is built: the tables reused above live in the
        // old array, and removing it any earlier would free them.
        if model.clients.isEmpty { existing.removeIfPresent("client") } else { existing["client"] = clientsArr }

        // Radios (fully modelled by the app — name/url/cover)
        let radiosArr = TOMLArray()
        for radio in model.radios where !radio.name.isEmpty {
            let r = TOMLTable()
            r["name"] = radio.name
            r["url"] = radio.url
            if !radio.cover.isEmpty { r["cover"] = radio.cover }
            radiosArr.append(r)
        }
        // Replace or drop the array only after it is built: the tables reused above live in the
        // old array, and removing it any earlier would free them.
        if model.radios.isEmpty { existing.removeIfPresent("radio") } else { existing["radio"] = radiosArr }

        try existing.convert().write(to: url, atomically: true, encoding: .utf8)
    }

    /// Read a `[…].knx` sub-table into a `[field: group-address]` dictionary (all string keys,
    /// so unknown/future fields are preserved through an edit).
    private static func readKnx(_ table: TOMLTable?) -> [String: String] {
        guard let table else { return [:] }
        var out: [String: String] = [:]
        for key in table.keys {
            if let value = table[key]?.string { out[key] = value }
        }
        return out
    }

    /// Write the `knx` dictionary back as a sub-table (dropping empty values); removes the
    /// sub-table entirely when nothing is set.
    private static func writeKnx(_ ga: [String: String], into table: TOMLTable) {
        let filtered = ga.filter { !$0.value.isEmpty }
        if filtered.isEmpty {
            table.removeIfPresent("knx")
        } else {
            let knx = TOMLTable()
            for (key, value) in filtered { knx[key] = value }
            table["knx"] = knx
        }
    }
}

private extension TOMLTable {
    /// Removes a key if there is one, so a key that is not there is never passed to
    /// `remove(at:)`.
    func removeIfPresent(_ key: String) {
        if self[key] != nil { remove(at: key) }
    }

    /// Sets a string value, or removes the key when there is none. TOMLKit 0.6 ignores
    /// `table[key] = nil`, so an unset value has to be removed explicitly or it stays in the file.
    func setOrRemove(_ key: String, _ value: String?) {
        if let value { self[key] = value } else { removeIfPresent(key) }
    }
}
