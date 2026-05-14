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

// MARK: - Config View (Apple Settings pattern: auto-save on change)

struct ConfigView: View {
    @Bindable var serverManager: ServerManager
    @State private var config = ConfigModel()
    @State private var selectedSection: Section = .http
    @State private var saveTask: Task<Void, Never>?

    enum Section: String, CaseIterable, Identifiable {
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
        TabView(selection: $selectedSection) {
            Tab("HTTP", systemImage: "network", value: Section.http) {
                Form { httpForm }.formStyle(.grouped)
            }
            Tab("Audio", systemImage: "waveform", value: Section.audio) {
                Form { audioForm }.formStyle(.grouped)
            }
            Tab("Snapcast", systemImage: "hifispeaker.2", value: Section.snapcast) {
                Form { snapcastForm }.formStyle(.grouped)
            }
            Tab("AirPlay", systemImage: "airplayaudio", value: Section.airplay) {
                Form { airplayForm }.formStyle(.grouped)
            }
            Tab("Subsonic", systemImage: "music.note.house", value: Section.subsonic) {
                Form { subsonicForm }.formStyle(.grouped)
            }
            Tab("MQTT", systemImage: "antenna.radiowaves.left.and.right", value: Section.mqtt) {
                Form { mqttForm }.formStyle(.grouped)
            }
            Tab("Zones", systemImage: "rectangle.split.3x1", value: Section.zones) {
                Form { zonesForm }.formStyle(.grouped)
            }
            Tab("Clients", systemImage: "speaker.wave.2", value: Section.clients) {
                Form { clientsForm }.formStyle(.grouped)
            }
            Tab("Radio", systemImage: "radio", value: Section.radios) {
                Form { radiosForm }.formStyle(.grouped)
            }
        }
        .tabViewStyle(.sidebarAdaptable)
        .frame(minWidth: 580, minHeight: 420)
        .onAppear { load() }
        .onChange(of: config.http) { _, _ in debounceSave() }
        .onChange(of: config.audio) { _, _ in debounceSave() }
        .onChange(of: config.snapcast) { _, _ in debounceSave() }
        .onChange(of: config.airplay) { _, _ in debounceSave() }
        .onChange(of: config.subsonic) { _, _ in debounceSave() }
        .onChange(of: config.mqtt) { _, _ in debounceSave() }
        .onChange(of: config.zones.count) { _, _ in debounceSave() }
        .onChange(of: config.clients.count) { _, _ in debounceSave() }
        .onChange(of: config.radios.count) { _, _ in debounceSave() }
    }

    // MARK: - Auto-save (debounced, Apple pattern)

    private func debounceSave() {
        saveTask?.cancel()
        saveTask = Task {
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            save()
        }
    }

    private func load() {
        serverManager.ensureConfigExists()
        do {
            config = try TOMLConfigParser.load(from: serverManager.configPath)
        } catch {
            config = ConfigModel()
        }
    }

    private func save() {
        try? TOMLConfigParser.save(config, to: serverManager.configPath)
    }

    // MARK: - Section Forms

    @ViewBuilder
    private var httpForm: some View {
        SwiftUI.Section("Server") {
            TextField("Port", value: $config.http.port, format: .number)
            TextField("Base URL", text: $config.http.baseUrl)
                .help("External URL for absolute links in API responses")
        }
    }

    @ViewBuilder
    private var audioForm: some View {
        SwiftUI.Section("Output Format") {
            Picker("Sample Rate", selection: $config.audio.sampleRate) {
                Text("44.1 kHz").tag(44100)
                Text("48 kHz").tag(48000)
                Text("88.2 kHz").tag(88200)
                Text("96 kHz").tag(96000)
            }
            .pickerStyle(.menu)
            Picker("Bit Depth", selection: $config.audio.bitDepth) {
                Text("16-bit").tag(16)
                Text("24-bit").tag(24)
                Text("32-bit").tag(32)
            }
            .pickerStyle(.menu)
        }
        SwiftUI.Section("Transitions") {
            Picker("Source Conflict", selection: $config.audio.sourceConflict) {
                Text("Last Wins").tag("last_wins")
                Text("Receiver Wins").tag("receiver_wins")
            }
            .pickerStyle(.menu)
            .help("How to resolve when AirPlay/Spotify is active and local playback starts")
            Stepper("Zone Switch Fade: \(config.audio.zoneSwitch) ms", value: $config.audio.zoneSwitch, in: 0...2000, step: 50)
            Stepper("Source Switch Fade: \(config.audio.sourceSwitch) ms", value: $config.audio.sourceSwitch, in: 0...2000, step: 50)
        }
    }

    @ViewBuilder
    private var snapcastForm: some View {
        SwiftUI.Section("Network") {
            TextField("Streaming Port", value: $config.snapcast.streamingPort, format: .number)
        }
        SwiftUI.Section("Codec") {
            Picker("Codec", selection: $config.snapcast.codec) {
                Text("PCM (uncompressed)").tag("pcm")
                Text("FLAC (lossless)").tag("flac")
                Text("F32+LZ4 (low latency)").tag("f32lz4")
                Text("F32+LZ4 encrypted").tag("f32lz4e")
            }
            .pickerStyle(.menu)
            if config.snapcast.codec == "f32lz4e" {
                SecureField("Encryption Key", text: $config.snapcast.encryptionPsk, prompt: Text("Pre-shared key"))
            }
        }
        SwiftUI.Section("Client Management") {
            Picker("Group Volume Mode", selection: $config.snapcast.groupVolumeMode) {
                Text("Relative (proportional)").tag("relative")
                Text("Absolute (override)").tag("absolute")
            }
            .pickerStyle(.menu)
            Picker("Unknown Clients", selection: $config.snapcast.unknownClients) {
                Text("Accept").tag("accept")
                Text("Ignore").tag("ignore")
                Text("Reject").tag("reject")
            }
            .pickerStyle(.menu)
            if config.snapcast.unknownClients == "accept" {
                TextField("Default Zone", text: $config.snapcast.defaultZone, prompt: Text("First zone"))
            }
        }
    }

    @ViewBuilder
    private var airplayForm: some View {
        SwiftUI.Section("AirPlay Receivers") {
            SecureField("Password", text: $config.airplay.password, prompt: Text("No password"))
                .help("Optional password required for AirPlay connections")
        }
    }

    @ViewBuilder
    private var subsonicForm: some View {
        SwiftUI.Section("Connection") {
            Toggle("Enable Subsonic", isOn: $config.subsonic.enabled)
            Group {
                TextField("Server URL", text: $config.subsonic.url, prompt: Text("http://navidrome:4533"))
                TextField("Username", text: $config.subsonic.username)
                SecureField("Password", text: $config.subsonic.password)
                Picker("Format", selection: $config.subsonic.format) {
                    Text("Raw (original)").tag("raw")
                    Text("FLAC").tag("flac")
                    Text("MP3").tag("mp3")
                    Text("Opus").tag("opus")
                }
                .pickerStyle(.menu)
            }
            .disabled(!config.subsonic.enabled)
        }
        SwiftUI.Section("Track Cache") {
            Toggle("Enable Cache", isOn: $config.subsonic.cacheEnabled)
                .disabled(!config.subsonic.enabled)
            Group {
                Stepper("Max Size: \(config.subsonic.cacheMaxSizeMb) MB", value: $config.subsonic.cacheMaxSizeMb, in: 256...16384, step: 256)
                Stepper("Lookahead: \(config.subsonic.cacheLookahead) tracks", value: $config.subsonic.cacheLookahead, in: 0...10)
            }
            .disabled(!config.subsonic.enabled || !config.subsonic.cacheEnabled)
        }
    }

    @ViewBuilder
    private var mqttForm: some View {
        SwiftUI.Section("Connection") {
            Toggle("Enable MQTT", isOn: $config.mqtt.enabled)
            Group {
                TextField("Broker", text: $config.mqtt.broker, prompt: Text("host:port"))
                TextField("Client ID", text: $config.mqtt.clientId)
                TextField("Username", text: $config.mqtt.username, prompt: Text("Optional"))
                SecureField("Password", text: $config.mqtt.password)
                TextField("Base Topic", text: $config.mqtt.baseTopic)
            }
            .disabled(!config.mqtt.enabled)
        }
    }

    @ViewBuilder
    private var zonesForm: some View {
        SwiftUI.Section {
            List {
                ForEach($config.zones) { $zone in
                    HStack {
                        TextField("", text: $zone.icon)
                            .frame(width: 36)
                            .multilineTextAlignment(.center)
                        TextField("Zone Name", text: $zone.name)
                    }
                }
                .onDelete { config.zones.remove(atOffsets: $0) }
                .onMove { config.zones.move(fromOffsets: $0, toOffset: $1) }
            }
            .frame(minHeight: 120)
        } footer: {
            HStack {
                Button("", systemImage: "plus") {
                    config.zones.append(.init(name: "New Zone"))
                }
                Button("", systemImage: "minus") {
                    // Remove last if no selection
                    if !config.zones.isEmpty { config.zones.removeLast() }
                }
                .disabled(config.zones.isEmpty)
                Spacer()
            }
            .buttonStyle(.borderless)
        }
    }

    @ViewBuilder
    private var clientsForm: some View {
        SwiftUI.Section {
            List {
                ForEach($config.clients) { $client in
                    VStack(alignment: .leading, spacing: 4) {
                        HStack {
                            TextField("", text: $client.icon)
                                .frame(width: 36)
                                .multilineTextAlignment(.center)
                            TextField("Name", text: $client.name)
                        }
                        HStack {
                            TextField("MAC Address", text: $client.mac, prompt: Text("aa:bb:cc:dd:ee:ff"))
                            TextField("Zone", text: $client.zone)
                        }
                        .font(.callout)
                        .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 2)
                }
                .onDelete { config.clients.remove(atOffsets: $0) }
                .onMove { config.clients.move(fromOffsets: $0, toOffset: $1) }
            }
            .frame(minHeight: 150)
        } footer: {
            HStack {
                Button("", systemImage: "plus") {
                    config.clients.append(.init())
                }
                Button("", systemImage: "minus") {
                    if !config.clients.isEmpty { config.clients.removeLast() }
                }
                .disabled(config.clients.isEmpty)
                Spacer()
            }
            .buttonStyle(.borderless)
        }
    }

    @ViewBuilder
    private var radiosForm: some View {
        SwiftUI.Section {
            List {
                ForEach($config.radios) { $radio in
                    VStack(alignment: .leading, spacing: 4) {
                        TextField("Station Name", text: $radio.name)
                        TextField("Stream URL", text: $radio.url, prompt: Text("https://..."))
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.vertical, 2)
                }
                .onDelete { config.radios.remove(atOffsets: $0) }
                .onMove { config.radios.move(fromOffsets: $0, toOffset: $1) }
            }
            .frame(minHeight: 120)
        } footer: {
            HStack {
                Button("", systemImage: "plus") {
                    config.radios.append(.init())
                }
                Button("", systemImage: "minus") {
                    if !config.radios.isEmpty { config.radios.removeLast() }
                }
                .disabled(config.radios.isEmpty)
                Spacer()
            }
            .buttonStyle(.borderless)
        }
    }
}

// MARK: - Equatable conformances for onChange detection

extension ConfigModel.SystemSection: Equatable {}
extension ConfigModel.HttpSection: Equatable {}
extension ConfigModel.AudioSection: Equatable {}
extension ConfigModel.SnapcastSection: Equatable {}
extension ConfigModel.AirplaySection: Equatable {}
extension ConfigModel.SubsonicSection: Equatable {}
extension ConfigModel.MqttSection: Equatable {}
