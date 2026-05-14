import Foundation
import TOMLKit

enum TOMLConfigParser {
    static func load(from url: URL) throws -> ConfigModel {
        let content = try String(contentsOf: url, encoding: .utf8)
        let table = try TOMLTable(string: content)
        let model = ConfigModel()

        // Subsonic
        if let sub = table["subsonic"] as? TOMLTable {
            model.subsonic.url = (sub["url"] as? String) ?? ""
            model.subsonic.username = (sub["username"] as? String) ?? ""
            model.subsonic.password = (sub["password"] as? String) ?? ""
        }

        // Codec
        if let snap = table["snapcast"] as? TOMLTable {
            model.codec = (snap["codec"] as? String) ?? "flac"
            model.encryptionPsk = (snap["encryption_psk"] as? String) ?? ""
        }

        // AirPlay
        if let ap = table["airplay"] as? TOMLTable {
            model.airplayPassword = (ap["password"] as? String) ?? ""
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
                    url: (t["url"] as? String) ?? ""
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

        // HTTP (preserve or set defaults)
        if existing["http"] == nil {
            let http = TOMLTable()
            http["port"] = 5555
            http["base_url"] = "http://localhost:5555"
            existing["http"] = http
        }

        // Audio (preserve or set defaults)
        if existing["audio"] == nil {
            let audio = TOMLTable()
            audio["sample_rate"] = 48000
            audio["bit_depth"] = 16
            audio["source_conflict"] = "last_wins"
            audio["zone_switch_fade_ms"] = 300
            audio["source_switch_fade_ms"] = 300
            existing["audio"] = audio
        }

        // Snapcast — update codec, preserve rest
        let snap = (existing["snapcast"] as? TOMLTable) ?? TOMLTable()
        snap["codec"] = model.codec
        if !model.encryptionPsk.isEmpty { snap["encryption_psk"] = model.encryptionPsk }
        if snap["streaming_port"] == nil { snap["streaming_port"] = 1704 }
        if snap["group_volume_mode"] == nil { snap["group_volume_mode"] = "relative" }
        if snap["unknown_clients"] == nil { snap["unknown_clients"] = "accept" }
        existing["snapcast"] = snap

        // Subsonic
        if !model.subsonic.url.isEmpty {
            let sub = (existing["subsonic"] as? TOMLTable) ?? TOMLTable()
            sub["url"] = model.subsonic.url
            sub["username"] = model.subsonic.username
            sub["password"] = model.subsonic.password
            if sub["format"] == nil { sub["format"] = "raw" }
            existing["subsonic"] = sub
        } else {
            existing["subsonic"] = nil
        }

        // AirPlay
        if !model.airplayPassword.isEmpty {
            let ap = TOMLTable()
            ap["password"] = model.airplayPassword
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
            radiosArr.append(t)
        }
        if !model.radios.isEmpty { existing["radio"] = radiosArr }

        try existing.convert().write(to: url, atomically: true, encoding: .utf8)
    }
}
