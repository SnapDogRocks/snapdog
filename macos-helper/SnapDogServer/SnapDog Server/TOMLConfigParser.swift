import Foundation
import TOMLKit

enum TOMLConfigParser {
    static func load(from url: URL) throws -> ConfigModel {
        let content = try String(contentsOf: url, encoding: .utf8)
        let table = try TOMLTable(string: content)
        let model = ConfigModel()

        // System
        if let sys = table["system"] as? TOMLTable {
            model.system.logLevel = (sys["log_level"] as? String) ?? "info"
            model.system.logFile = (sys["log_file"] as? String) ?? ""
            model.system.stateDir = (sys["state_dir"] as? String) ?? ""
        }

        // HTTP
        if let http = table["http"] as? TOMLTable {
            model.http.port = (http["port"] as? Int) ?? 5555
            model.http.baseUrl = (http["base_url"] as? String) ?? "http://localhost:5555"
        }

        // Audio
        if let audio = table["audio"] as? TOMLTable {
            model.audio.sampleRate = (audio["sample_rate"] as? Int) ?? 48000
            model.audio.bitDepth = (audio["bit_depth"] as? Int) ?? 16
            model.audio.channels = (audio["channels"] as? Int) ?? 2
            model.audio.sourceConflict = (audio["source_conflict"] as? String) ?? "last_wins"
            model.audio.zoneSwitch = (audio["zone_switch_fade_ms"] as? Int) ?? 300
            model.audio.sourceSwitch = (audio["source_switch_fade_ms"] as? Int) ?? 300
        }

        // Snapcast
        if let snap = table["snapcast"] as? TOMLTable {
            model.snapcast.streamingPort = (snap["streaming_port"] as? Int) ?? 1704
            model.snapcast.codec = (snap["codec"] as? String) ?? "flac"
            model.snapcast.encryptionPsk = (snap["encryption_psk"] as? String) ?? ""
            model.snapcast.groupVolumeMode = (snap["group_volume_mode"] as? String) ?? "relative"
            model.snapcast.unknownClients = (snap["unknown_clients"] as? String) ?? "accept"
            model.snapcast.defaultZone = (snap["default_zone"] as? String) ?? ""
        }

        // AirPlay
        if let ap = table["airplay"] as? TOMLTable {
            model.airplay.password = (ap["password"] as? String) ?? ""
        }

        // Subsonic
        if let sub = table["subsonic"] as? TOMLTable {
            model.subsonic.enabled = true
            model.subsonic.url = (sub["url"] as? String) ?? ""
            model.subsonic.username = (sub["username"] as? String) ?? ""
            model.subsonic.password = (sub["password"] as? String) ?? ""
            model.subsonic.format = (sub["format"] as? String) ?? "raw"
            if let cache = sub["cache"] as? TOMLTable {
                model.subsonic.cacheEnabled = (cache["enabled"] as? Bool) ?? true
                model.subsonic.cacheMaxSizeMb = (cache["max_size_mb"] as? Int) ?? 2048
                model.subsonic.cacheLookahead = (cache["lookahead"] as? Int) ?? 2
            }
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

        // Zones (array of tables)
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
        let table = TOMLTable()

        // System
        let sys = TOMLTable()
        sys["log_level"] = model.system.logLevel
        if !model.system.logFile.isEmpty { sys["log_file"] = model.system.logFile }
        if !model.system.stateDir.isEmpty { sys["state_dir"] = model.system.stateDir }
        table["system"] = sys

        // HTTP
        let http = TOMLTable()
        http["port"] = model.http.port
        http["base_url"] = model.http.baseUrl
        table["http"] = http

        // Audio
        let audio = TOMLTable()
        audio["sample_rate"] = model.audio.sampleRate
        audio["bit_depth"] = model.audio.bitDepth
        audio["channels"] = model.audio.channels
        audio["source_conflict"] = model.audio.sourceConflict
        audio["zone_switch_fade_ms"] = model.audio.zoneSwitch
        audio["source_switch_fade_ms"] = model.audio.sourceSwitch
        table["audio"] = audio

        // Snapcast
        let snap = TOMLTable()
        snap["streaming_port"] = model.snapcast.streamingPort
        snap["codec"] = model.snapcast.codec
        if !model.snapcast.encryptionPsk.isEmpty { snap["encryption_psk"] = model.snapcast.encryptionPsk }
        snap["group_volume_mode"] = model.snapcast.groupVolumeMode
        snap["unknown_clients"] = model.snapcast.unknownClients
        if !model.snapcast.defaultZone.isEmpty { snap["default_zone"] = model.snapcast.defaultZone }
        table["snapcast"] = snap

        // AirPlay
        if !model.airplay.password.isEmpty {
            let ap = TOMLTable()
            ap["password"] = model.airplay.password
            table["airplay"] = ap
        }

        // Subsonic
        if model.subsonic.enabled {
            let sub = TOMLTable()
            sub["url"] = model.subsonic.url
            sub["username"] = model.subsonic.username
            sub["password"] = model.subsonic.password
            sub["format"] = model.subsonic.format
            let cache = TOMLTable()
            cache["enabled"] = model.subsonic.cacheEnabled
            cache["max_size_mb"] = model.subsonic.cacheMaxSizeMb
            cache["lookahead"] = model.subsonic.cacheLookahead
            sub["cache"] = cache
            table["subsonic"] = sub
        }

        // MQTT
        if model.mqtt.enabled {
            let mqtt = TOMLTable()
            mqtt["broker"] = model.mqtt.broker
            mqtt["client_id"] = model.mqtt.clientId
            if !model.mqtt.username.isEmpty { mqtt["username"] = model.mqtt.username }
            if !model.mqtt.password.isEmpty { mqtt["password"] = model.mqtt.password }
            mqtt["base_topic"] = model.mqtt.baseTopic
            table["mqtt"] = mqtt
        }

        // Zones — array of tables ([[zone]])
        let zonesArr = TOMLArray()
        for zone in model.zones {
            let t = TOMLTable()
            t["name"] = zone.name
            t["icon"] = zone.icon
            zonesArr.append(t)
        }
        if !model.zones.isEmpty { table["zone"] = zonesArr }

        // Clients
        let clientsArr = TOMLArray()
        for client in model.clients {
            let t = TOMLTable()
            t["name"] = client.name
            t["mac"] = client.mac
            t["zone"] = client.zone
            t["icon"] = client.icon
            clientsArr.append(t)
        }
        if !model.clients.isEmpty { table["client"] = clientsArr }

        // Radios
        let radiosArr = TOMLArray()
        for radio in model.radios {
            let t = TOMLTable()
            t["name"] = radio.name
            t["url"] = radio.url
            if !radio.cover.isEmpty { t["cover"] = radio.cover }
            radiosArr.append(t)
        }
        if !model.radios.isEmpty { table["radio"] = radiosArr }

        try table.convert().write(to: url, atomically: true, encoding: .utf8)
    }
}
