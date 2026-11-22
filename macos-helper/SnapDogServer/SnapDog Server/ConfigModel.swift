import Foundation
import Observation

// MARK: - Config Model

@Observable
final class ConfigModel {
    var subsonic = SubsonicSection()
    var spotify = SpotifySection()
    var zones: [ZoneEntry] = []
    var clients: [ClientEntry] = []
    var radios: [RadioEntry] = []
    var mqtt = MqttSection()
    var knx = KnxSection()
    var apiKeys: [ApiKeyEntry] = []
    var airplayPassword = ""
    var airplayMode = "airplay2"
    var codec = "flac"
    var encryptionPsk = ""
    var sampleRate = 48000
    var bitDepth = 16
    var sourceConflict = "last_wins"
    var zoneSwitchFadeMs = 300
    var sourceSwitchFadeMs = 300
    var streamingPort = 1704
    var unknownClients = "accept"
    var groupVolumeMode = "compressed"
    var defaultZone = ""
    var httpPort = 5555

    struct SubsonicSection: Equatable {
        var enabled = false
        var url = ""
        var username = ""
        var password = ""
        var format = "raw"
        var tlsSkipVerify = false
    }

    struct SpotifySection: Equatable {
        var enabled = false
        /// Empty: the server names each Spotify device after its zone.
        var name = ""
        var bitrate = 320
    }

    /// The Spotify Connect device names the server derives from `[spotify].name`, the zone
    /// names and the zones' own `spotify_name`: a zone's own name wins. Otherwise an empty
    /// `[spotify].name` keeps the zone name, one zone is named exactly `name`, and several
    /// zones get `"<name> <zone>"`.
    var spotifyDeviceNames: [String] {
        let named = zones.filter { !$0.name.isEmpty }
        let base = spotify.name.trimmingCharacters(in: .whitespacesAndNewlines)
        if named.isEmpty { return base.isEmpty ? [] : [base] }
        return named.map { zone in
            let own = zone.spotifyName.trimmingCharacters(in: .whitespacesAndNewlines)
            if !own.isEmpty { return own }
            if base.isEmpty { return zone.name }
            return named.count == 1 ? base : "\(base) \(zone.name)"
        }
    }

    /// Global `[knx]` settings only. Per-zone/client group-address matrices
    /// (`[[zone]].knx` / `[[client]].knx`) are not managed here — see RFC MAC-0006 MAC-T32.
    struct KnxSection: Equatable {
        var enabled = false
        var role = "client"
        var url = ""
        var individualAddress = ""
        var persistEts = true
        var restartAfterEts = true
    }

    struct ApiKeyEntry: Identifiable, Equatable {
        let id = UUID()
        var value = ""
    }

    struct MqttSection: Equatable {
        var enabled = false
        var broker = ""
        var clientId = "snapdog"
        var username = ""
        var password = ""
        var baseTopic = "snapdog"
    }

    struct ZoneEntry: Identifiable, Equatable {
        let id = UUID()
        var name = ""
        /// `[[zone]].spotify_name`: the zone's own Spotify Connect device name, empty for none.
        var spotifyName = ""
        /// The zone's name as it stands in the file. A renamed zone finds its table (and the
        /// keys the app does not model) under this name when the file is written.
        var fileName: String?
        var icon = "🏠"
        /// KNX group addresses (`[[zone]].knx`), keyed by field name. Unknown keys are
        /// preserved so future server fields survive an edit.
        var knx: [String: String] = [:]
    }

    struct ClientEntry: Identifiable, Equatable {
        let id = UUID()
        var name = ""
        /// The client's name as it stands in the file. A renamed client finds its table (and
        /// the keys the app does not model, such as `max_volume`) under this name when the
        /// file is written.
        var fileName: String?
        var mac = ""
        var zone = ""
        var icon = "🔊"
        var knx: [String: String] = [:]
    }

    /// KNX group-address field keys (match the server's `RawZoneKnxConfig` / `RawClientKnxConfig`).
    static let zoneKnxFields = [
        "play", "pause", "stop", "track_next", "track_previous", "control_status",
        "volume", "volume_status", "volume_dim", "mute", "mute_status", "mute_toggle",
        "track_title_status", "track_artist_status", "track_album_status", "track_progress_status",
        "track_playing_status", "track_repeat", "track_repeat_status", "track_repeat_toggle",
        "playlist", "playlist_status", "playlist_next", "playlist_previous", "playlist_step",
        "playlist_step_updown", "playlist_name_status", "play_pause", "track_step",
        "track_step_updown", "track_number", "track_number_status",
        "track_position", "track_seek", "track_seek_updown",
        "shuffle", "shuffle_status", "shuffle_toggle", "repeat", "repeat_status", "repeat_toggle",
        "presence", "presence_enable", "presence_enable_status", "presence_timeout",
        "presence_timeout_status", "presence_timer_status", "presence_source_override",
    ]
    static let clientKnxFields = [
        "volume", "volume_status", "volume_dim", "mute", "mute_status", "mute_toggle",
        "latency", "latency_status", "zone", "zone_status", "connected_status",
    ]

    struct RadioEntry: Identifiable, Equatable {
        let id = UUID()
        var name = ""
        var url = ""
        var cover = ""
    }
}

