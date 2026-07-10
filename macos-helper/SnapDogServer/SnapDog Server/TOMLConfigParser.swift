import Foundation
import TOMLKit

enum TOMLConfigParser {
    static func load(from url: URL) throws -> ConfigModel {
        let content = try String(contentsOf: url, encoding: .utf8)
        let table = try TOMLTable(string: content)
        let model = ConfigModel()

        // Subsonic
        if let sub = table["subsonic"] as? TOMLTable {
            model.subsonic.enabled = true
            model.subsonic.url = (sub["url"] as? String) ?? ""
            model.subsonic.username = (sub["username"] as? String) ?? ""
            model.subsonic.password = (sub["password"] as? String) ?? ""
            model.subsonic.format = (sub["format"] as? String) ?? "raw"
            model.subsonic.tlsSkipVerify = (sub["tls_skip_verify"] as? Bool) ?? false
        }

        // Spotify
        if let sp = table["spotify"] as? TOMLTable {
            model.spotify.enabled = true
            model.spotify.name = (sp["name"] as? String) ?? "SnapDog"
            model.spotify.bitrate = intValue(sp["bitrate"]) ?? 320
        }

        // Snapcast / streaming
        if let snap = table["snapcast"] as? TOMLTable {
            model.codec = (snap["codec"] as? String) ?? "flac"
            model.encryptionPsk = (snap["encryption_psk"] as? String) ?? ""
            model.streamingPort = intValue(snap["streaming_port"]) ?? 1704
            model.unknownClients = (snap["unknown_clients"] as? String) ?? "accept"
            model.groupVolumeMode = (snap["group_volume_mode"] as? String) ?? "compressed"
            model.defaultZone = (snap["default_zone"] as? String) ?? ""
        }

        // Audio output + mixing
        if let audio = table["audio"] as? TOMLTable {
            model.sampleRate = intValue(audio["sample_rate"]) ?? 48000
            model.bitDepth = intValue(audio["bit_depth"]) ?? 16
            model.sourceConflict = (audio["source_conflict"] as? String) ?? "last_wins"
            model.zoneSwitchFadeMs = intValue(audio["zone_switch_fade_ms"]) ?? 300
            model.sourceSwitchFadeMs = intValue(audio["source_switch_fade_ms"]) ?? 300
        }

        // HTTP
        if let http = table["http"] as? TOMLTable {
            model.httpPort = intValue(http["port"]) ?? 5555
            if let keys = http["api_keys"] as? [String] {
                model.apiKeys = keys.map { ConfigModel.ApiKeyEntry(value: $0) }
            }
        }

        // AirPlay
        if let ap = table["airplay"] as? TOMLTable {
            model.airplayPassword = (ap["password"] as? String) ?? ""
            model.airplayMode = (ap["mode"] as? String) ?? "airplay2"
        }

        // MQTT
        if let mqtt = table["mqtt"] as? TOMLTable {
            model.mqtt.enabled = true
            model.mqtt.broker = (mqtt["broker"] as? String) ?? ""
            model.mqtt.clientId = (mqtt["client_id"] as? String) ?? "snapdog"
            model.mqtt.username = (mqtt["username"] as? String) ?? ""
            model.mqtt.password = (mqtt["password"] as? String) ?? ""
            model.mqtt.baseTopic = (mqtt["base_topic"] as? String) ?? "snapdog"
        }

        // Zones
        if let zones = table["zone"] as? [TOMLTable] {
            model.zones = zones.map { t in
                ConfigModel.ZoneEntry(
                    name: (t["name"] as? String) ?? "",
                    icon: (t["icon"] as? String) ?? "🏠"
                )
            }
        }

        // Clients
        if let clients = table["client"] as? [TOMLTable] {
            model.clients = clients.map { t in
                ConfigModel.ClientEntry(
                    name: (t["name"] as? String) ?? "",
                    mac: (t["mac"] as? String) ?? "",
                    zone: (t["zone"] as? String) ?? "",
                    icon: (t["icon"] as? String) ?? "🔊"
                )
            }
        }

        // Radios
        if let radios = table["radio"] as? [TOMLTable] {
            model.radios = radios.map { t in
                ConfigModel.RadioEntry(
                    name: (t["name"] as? String) ?? "",
                    url: (t["url"] as? String) ?? "",
                    cover: (t["cover"] as? String) ?? ""
                )
            }
        }

        return model
    }

    static func save(_ model: ConfigModel, to url: URL) throws {
        // Load existing file to preserve fields the UI doesn't manage
        let existing: TOMLTable
        if let content = try? String(contentsOf: url, encoding: .utf8),
           let table = try? TOMLTable(string: content) {
            existing = table
        } else {
            existing = TOMLTable()
        }

        // HTTP — port + API keys from the model, preserving other keys (tls, bind, …)
        let http = (existing["http"] as? TOMLTable) ?? TOMLTable()
        http["port"] = model.httpPort
        if http["base_url"] == nil { http["base_url"] = "http://localhost:\(model.httpPort)" }
        let apiKeys = model.apiKeys.map(\.value).filter { !$0.isEmpty }
        if apiKeys.isEmpty {
            http["api_keys"] = nil
        } else {
            let arr = TOMLArray()
            for key in apiKeys { arr.append(key) }
            http["api_keys"] = arr
        }
        existing["http"] = http

        // Audio — output format + mixing from the model, preserving any other keys
        let audio = (existing["audio"] as? TOMLTable) ?? TOMLTable()
        audio["sample_rate"] = model.sampleRate
        audio["bit_depth"] = model.bitDepth
        audio["source_conflict"] = model.sourceConflict
        audio["zone_switch_fade_ms"] = model.zoneSwitchFadeMs
        audio["source_switch_fade_ms"] = model.sourceSwitchFadeMs
        existing["audio"] = audio

        // Snapcast — codec, streaming and grouping from the model, preserving the rest
        let snap = (existing["snapcast"] as? TOMLTable) ?? TOMLTable()
        snap["codec"] = model.codec
        // Keep the pre-shared key only for the encrypted codec; drop any stale key otherwise.
        snap["encryption_psk"] = (model.codec == "f32lz4e" && !model.encryptionPsk.isEmpty)
            ? model.encryptionPsk : nil
        snap["streaming_port"] = model.streamingPort
        snap["group_volume_mode"] = model.groupVolumeMode
        snap["unknown_clients"] = model.unknownClients
        snap["default_zone"] = model.defaultZone.isEmpty ? nil : model.defaultZone
        existing["snapcast"] = snap

        // Subsonic
        if model.subsonic.enabled && !model.subsonic.url.isEmpty {
            let sub = (existing["subsonic"] as? TOMLTable) ?? TOMLTable()
            sub["url"] = model.subsonic.url
            sub["username"] = model.subsonic.username
            sub["password"] = model.subsonic.password
            sub["format"] = model.subsonic.format
            sub["tls_skip_verify"] = model.subsonic.tlsSkipVerify
            existing["subsonic"] = sub
        } else {
            existing["subsonic"] = nil
        }

        // Spotify
        if model.spotify.enabled && !model.spotify.name.isEmpty {
            let sp = (existing["spotify"] as? TOMLTable) ?? TOMLTable()
            sp["name"] = model.spotify.name
            sp["bitrate"] = model.spotify.bitrate
            existing["spotify"] = sp
        } else {
            existing["spotify"] = nil
        }

        // AirPlay — write the section if a password or a non-default mode is set.
        if !model.airplayPassword.isEmpty || model.airplayMode != "airplay2" {
            let ap = (existing["airplay"] as? TOMLTable) ?? TOMLTable()
            ap["password"] = model.airplayPassword.isEmpty ? nil : model.airplayPassword
            ap["mode"] = model.airplayMode
            existing["airplay"] = ap
        } else {
            existing["airplay"] = nil
        }

        // MQTT
        if model.mqtt.enabled {
            let mqtt = TOMLTable()
            mqtt["broker"] = model.mqtt.broker
            mqtt["client_id"] = model.mqtt.clientId
            if !model.mqtt.username.isEmpty { mqtt["username"] = model.mqtt.username }
            if !model.mqtt.password.isEmpty { mqtt["password"] = model.mqtt.password }
            mqtt["base_topic"] = model.mqtt.baseTopic
            existing["mqtt"] = mqtt
        } else {
            existing["mqtt"] = nil
        }

        // Zones
        existing["zone"] = nil
        let zonesArr = TOMLArray()
        for zone in model.zones where !zone.name.isEmpty {
            let t = TOMLTable()
            t["name"] = zone.name
            t["icon"] = zone.icon
            zonesArr.append(t)
        }
        if !model.zones.isEmpty { existing["zone"] = zonesArr }

        // Clients
        existing["client"] = nil
        let clientsArr = TOMLArray()
        for client in model.clients where !client.name.isEmpty {
            let t = TOMLTable()
            t["name"] = client.name
            t["mac"] = client.mac
            t["zone"] = client.zone
            t["icon"] = client.icon
            clientsArr.append(t)
        }
        if !model.clients.isEmpty { existing["client"] = clientsArr }

        // Radios
        existing["radio"] = nil
        let radiosArr = TOMLArray()
        for radio in model.radios where !radio.name.isEmpty {
            let t = TOMLTable()
            t["name"] = radio.name
            t["url"] = radio.url
            if !radio.cover.isEmpty { t["cover"] = radio.cover }
            radiosArr.append(t)
        }
        if !model.radios.isEmpty { existing["radio"] = radiosArr }

        try existing.convert().write(to: url, atomically: true, encoding: .utf8)
    }

    /// TOMLKit surfaces integers as `Int`, `Int64`, or `NSNumber` depending on the value;
    /// coerce any of them to `Int`.
    private static func intValue(_ value: Any?) -> Int? {
        switch value {
        case let int as Int: return int
        case let int64 as Int64: return Int(int64)
        case let number as NSNumber: return number.intValue
        default: return nil
        }
    }
}
