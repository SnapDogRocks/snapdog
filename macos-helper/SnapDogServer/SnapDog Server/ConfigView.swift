import SwiftUI

// MARK: - Config Model

@Observable
final class ConfigModel {
    var system = SystemSection()
    var http = HttpSection()
    var audio = AudioSection()
    var snapcast = SnapcastSection()
    var airplay = AirplaySection()
    var subsonic = SubsonicSection()
    var mqtt = MqttSection()
    var zones: [ZoneEntry] = []
    var clients: [ClientEntry] = []
    var radios: [RadioEntry] = []

    struct SystemSection {
        var logLevel = "info"
        var logFile = ""
        var stateDir = ""
    }

    struct HttpSection {
        var port = 5555
        var baseUrl = "http://localhost:5555"
        var apiKeys: [String] = []
    }

    struct AudioSection {
        var sampleRate = 48000
        var bitDepth = 16
        var channels = 2
        var sourceConflict = "last_wins"
        var zoneSwitch = 300
        var sourceSwitch = 300
    }

    struct SnapcastSection {
        var streamingPort = 1704
        var codec = "flac"
        var encryptionPsk = ""
        var groupVolumeMode = "relative"
        var unknownClients = "accept"
        var defaultZone = ""
    }

    struct AirplaySection {
        var password = ""
    }

    struct SubsonicSection {
        var enabled = false
        var url = ""
        var username = ""
        var password = ""
        var format = "raw"
        var cacheEnabled = true
        var cacheMaxSizeMb = 2048
        var cacheLookahead = 2
    }

    struct MqttSection {
        var enabled = false
        var broker = ""
        var clientId = "snapdog"
        var username = ""
        var password = ""
        var baseTopic = "snapdog"
    }

    struct ZoneEntry: Identifiable {
        let id = UUID()
        var name = ""
        var icon = "🏠"
    }

    struct ClientEntry: Identifiable {
        let id = UUID()
        var name = ""
        var mac = ""
        var zone = ""
        var icon = "🔊"
    }

    struct RadioEntry: Identifiable {
        let id = UUID()
        var name = ""
        var url = ""
        var cover = ""
    }
}

// MARK: - Config View

struct ConfigView: View {
    @Bindable var serverManager: ServerManager
    @State private var config = ConfigModel()
    @State private var selectedSection: Section = .system
    @State private var hasChanges = false

    enum Section: String, CaseIterable, Identifiable {
        case system = "System"
        case http = "HTTP"
        case audio = "Audio"
        case snapcast = "Snapcast"
        case airplay = "AirPlay"
        case subsonic = "Subsonic"
        case mqtt = "MQTT"
        case zones = "Zones"
        case clients = "Clients"
        case radios = "Radio Stations"

        var id: String { rawValue }
        var icon: String {
            switch self {
            case .system: "gearshape"
            case .http: "network"
            case .audio: "waveform"
            case .snapcast: "hifispeaker.2"
            case .airplay: "airplayaudio"
            case .subsonic: "music.note.house"
            case .mqtt: "antenna.radiowaves.left.and.right"
            case .zones: "rectangle.split.3x1"
            case .clients: "speaker.wave.2"
            case .radios: "radio"
            }
        }
    }

    var body: some View {
        NavigationSplitView {
            List(Section.allCases, selection: $selectedSection) { section in
                Label(section.rawValue, systemImage: section.icon)
            }
            .listStyle(.sidebar)
            .frame(minWidth: 160)
        } detail: {
            Form {
                switch selectedSection {
                case .system: systemForm
                case .http: httpForm
                case .audio: audioForm
                case .snapcast: snapcastForm
                case .airplay: airplayForm
                case .subsonic: subsonicForm
                case .mqtt: mqttForm
                case .zones: zonesForm
                case .clients: clientsForm
                case .radios: radiosForm
                }
            }
            .formStyle(.grouped)
            .frame(minWidth: 400)
        }
        .frame(minWidth: 620, minHeight: 450)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Save") { save() }
                    .disabled(!hasChanges)
                    .keyboardShortcut("s", modifiers: .command)
            }
            ToolbarItem(placement: .cancellationAction) {
                Button("Revert") { load() }
                    .disabled(!hasChanges)
            }
        }
        .onAppear { load() }
        .onChange(of: config.system.logLevel) { _, _ in hasChanges = true }
        .navigationTitle("SnapDog Server Configuration")
    }

    // MARK: - Section Forms

    @ViewBuilder
    private var systemForm: some View {
        SwiftUI.Section("Logging") {
            Picker("Log Level", selection: $config.system.logLevel) {
                ForEach(["trace", "debug", "info", "warn", "error"], id: \.self) { Text($0) }
            }
            TextField("Log File", text: $config.system.logFile, prompt: Text("Optional path"))
        }
        SwiftUI.Section("Storage") {
            TextField("State Directory", text: $config.system.stateDir, prompt: Text("Platform default"))
        }
    }

    @ViewBuilder
    private var httpForm: some View {
        SwiftUI.Section("Server") {
            TextField("Port", value: $config.http.port, format: .number)
            TextField("Base URL", text: $config.http.baseUrl)
        }
    }

    @ViewBuilder
    private var audioForm: some View {
        SwiftUI.Section("Output Format") {
            TextField("Sample Rate", value: $config.audio.sampleRate, format: .number)
            Picker("Bit Depth", selection: $config.audio.bitDepth) {
                Text("16").tag(16)
                Text("24").tag(24)
                Text("32").tag(32)
            }
            TextField("Channels", value: $config.audio.channels, format: .number)
        }
        SwiftUI.Section("Behavior") {
            Picker("Source Conflict", selection: $config.audio.sourceConflict) {
                Text("Last Wins").tag("last_wins")
                Text("Receiver Wins").tag("receiver_wins")
            }
            TextField("Zone Switch Fade (ms)", value: $config.audio.zoneSwitch, format: .number)
            TextField("Source Switch Fade (ms)", value: $config.audio.sourceSwitch, format: .number)
        }
    }

    @ViewBuilder
    private var snapcastForm: some View {
        SwiftUI.Section("Network") {
            TextField("Streaming Port", value: $config.snapcast.streamingPort, format: .number)
        }
        SwiftUI.Section("Codec") {
            Picker("Codec", selection: $config.snapcast.codec) {
                ForEach(["pcm", "flac", "f32lz4", "f32lz4e"], id: \.self) { Text($0) }
            }
            TextField("Encryption PSK", text: $config.snapcast.encryptionPsk, prompt: Text("Optional"))
        }
        SwiftUI.Section("Clients") {
            Picker("Volume Mode", selection: $config.snapcast.groupVolumeMode) {
                Text("Relative").tag("relative")
                Text("Absolute").tag("absolute")
            }
            Picker("Unknown Clients", selection: $config.snapcast.unknownClients) {
                Text("Accept").tag("accept")
                Text("Ignore").tag("ignore")
                Text("Reject").tag("reject")
            }
            TextField("Default Zone", text: $config.snapcast.defaultZone, prompt: Text("First zone"))
        }
    }

    @ViewBuilder
    private var airplayForm: some View {
        SwiftUI.Section("AirPlay") {
            TextField("Password", text: $config.airplay.password, prompt: Text("Optional"))
        }
    }

    @ViewBuilder
    private var subsonicForm: some View {
        SwiftUI.Section("Connection") {
            Toggle("Enabled", isOn: $config.subsonic.enabled)
            TextField("URL", text: $config.subsonic.url, prompt: Text("http://navidrome:4533"))
            TextField("Username", text: $config.subsonic.username)
            SecureField("Password", text: $config.subsonic.password)
            Picker("Format", selection: $config.subsonic.format) {
                ForEach(["raw", "flac", "mp3", "opus"], id: \.self) { Text($0) }
            }
        }
        SwiftUI.Section("Cache") {
            Toggle("Enabled", isOn: $config.subsonic.cacheEnabled)
            TextField("Max Size (MB)", value: $config.subsonic.cacheMaxSizeMb, format: .number)
            TextField("Lookahead Tracks", value: $config.subsonic.cacheLookahead, format: .number)
        }
    }

    @ViewBuilder
    private var mqttForm: some View {
        SwiftUI.Section("Connection") {
            Toggle("Enabled", isOn: $config.mqtt.enabled)
            TextField("Broker", text: $config.mqtt.broker, prompt: Text("192.168.1.10:1883"))
            TextField("Client ID", text: $config.mqtt.clientId)
            TextField("Username", text: $config.mqtt.username)
            SecureField("Password", text: $config.mqtt.password)
            TextField("Base Topic", text: $config.mqtt.baseTopic)
        }
    }

    @ViewBuilder
    private var zonesForm: some View {
        SwiftUI.Section {
            ForEach($config.zones) { $zone in
                HStack {
                    TextField("Icon", text: $zone.icon)
                        .frame(width: 40)
                    TextField("Name", text: $zone.name)
                }
            }
            .onDelete { config.zones.remove(atOffsets: $0) }
            .onMove { config.zones.move(fromOffsets: $0, toOffset: $1) }
        } header: {
            HStack {
                Text("Zones")
                Spacer()
                Button("Add Zone", systemImage: "plus") {
                    config.zones.append(.init(name: "New Zone"))
                    hasChanges = true
                }
                .labelStyle(.iconOnly)
            }
        }
    }

    @ViewBuilder
    private var clientsForm: some View {
        SwiftUI.Section {
            ForEach($config.clients) { $client in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        TextField("Icon", text: $client.icon)
                            .frame(width: 40)
                        TextField("Name", text: $client.name)
                    }
                    HStack {
                        TextField("MAC", text: $client.mac, prompt: Text("aa:bb:cc:dd:ee:ff"))
                        TextField("Zone", text: $client.zone, prompt: Text("Zone name"))
                    }
                    .font(.caption)
                }
                .padding(.vertical, 2)
            }
            .onDelete { config.clients.remove(atOffsets: $0) }
            .onMove { config.clients.move(fromOffsets: $0, toOffset: $1) }
        } header: {
            HStack {
                Text("Clients")
                Spacer()
                Button("Add Client", systemImage: "plus") {
                    config.clients.append(.init())
                    hasChanges = true
                }
                .labelStyle(.iconOnly)
            }
        }
    }

    @ViewBuilder
    private var radiosForm: some View {
        SwiftUI.Section {
            ForEach($config.radios) { $radio in
                VStack(alignment: .leading, spacing: 4) {
                    TextField("Name", text: $radio.name)
                    TextField("Stream URL", text: $radio.url, prompt: Text("https://..."))
                    TextField("Cover URL", text: $radio.cover, prompt: Text("Optional"))
                }
                .padding(.vertical, 2)
            }
            .onDelete { config.radios.remove(atOffsets: $0) }
            .onMove { config.radios.move(fromOffsets: $0, toOffset: $1) }
        } header: {
            HStack {
                Text("Radio Stations")
                Spacer()
                Button("Add Station", systemImage: "plus") {
                    config.radios.append(.init())
                    hasChanges = true
                }
                .labelStyle(.iconOnly)
            }
        }
    }

    // MARK: - Load / Save

    private func load() {
        serverManager.ensureConfigExists()
        do {
            config = try TOMLConfigParser.load(from: serverManager.configPath)
        } catch {
            // If parsing fails, start with defaults
            config = ConfigModel()
        }
        hasChanges = false
    }

    private func save() {
        do {
            try TOMLConfigParser.save(config, to: serverManager.configPath)
            hasChanges = false
        } catch {
            // TODO: show alert
        }
    }
}
