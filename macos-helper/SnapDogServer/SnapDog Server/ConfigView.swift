import SwiftUI

// MARK: - Config Model

@Observable
final class ConfigModel {
    var subsonic = SubsonicSection()
    var zones: [ZoneEntry] = []
    var clients: [ClientEntry] = []
    var radios: [RadioEntry] = []
    var mqtt = MqttSection()
    var knx = KnxSection()
    var airplayPassword = ""
    var codec = "flac"
    var encryptionPsk = ""
    var sampleRate = 48000
    var bitDepth = 16

    struct SubsonicSection: Equatable {
        var url = ""
        var username = ""
        var password = ""
    }

    struct MqttSection: Equatable {
        var enabled = false
        var broker = ""
        var clientId = "snapdog"
        var username = ""
        var password = ""
        var baseTopic = "snapdog"
    }

    struct KnxSection: Equatable {
        var enabled = false
        var role = "client"
        var url = ""
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
    @State private var saveTask: Task<Void, Never>?

    var body: some View {
        TabView {
            Tab("Sources", systemImage: "music.note.house") {
                Form { sourcesForm }.formStyle(.grouped)
            }
            Tab("Audio", systemImage: "waveform") {
                Form { audioForm }.formStyle(.grouped)
            }
            Tab("Zones", systemImage: "rectangle.split.3x1") {
                Form { zonesForm }.formStyle(.grouped)
            }
            Tab("Clients", systemImage: "speaker.wave.2") {
                Form { clientsForm }.formStyle(.grouped)
            }
            Tab("Integration", systemImage: "antenna.radiowaves.left.and.right") {
                Form { integrationForm }.formStyle(.grouped)
            }
        }
        .tabViewStyle(.automatic)
        .frame(width: 480, height: 360)
        .onAppear { load() }
        .onChange(of: config.subsonic) { _, _ in debounceSave() }
        .onChange(of: config.mqtt) { _, _ in debounceSave() }
        .onChange(of: config.knx) { _, _ in debounceSave() }
        .onChange(of: config.airplayPassword) { _, _ in debounceSave() }
        .onChange(of: config.codec) { _, _ in debounceSave() }
        .onChange(of: config.sampleRate) { _, _ in debounceSave() }
        .onChange(of: config.bitDepth) { _, _ in debounceSave() }
        .onChange(of: config.zones.count) { _, _ in debounceSave() }
        .onChange(of: config.clients.count) { _, _ in debounceSave() }
        .onChange(of: config.radios.count) { _, _ in debounceSave() }
    }

    // MARK: - Sources Tab

    @ViewBuilder
    private var sourcesForm: some View {
        SwiftUI.Section("Subsonic") {
            TextField("Server URL", text: $config.subsonic.url, prompt: Text("http://navidrome:4533"))
            TextField("Username", text: $config.subsonic.username)
            SecureField("Password", text: $config.subsonic.password)
        }

        SwiftUI.Section("AirPlay") {
            SecureField("Password", text: $config.airplayPassword, prompt: Text("No password"))
                .help("Optional password for AirPlay connections")
        }

        SwiftUI.Section {
            ForEach($config.radios) { $radio in
                VStack(alignment: .leading, spacing: 4) {
                    TextField("Station Name", text: $radio.name)
                    TextField("Stream URL", text: $radio.url, prompt: Text("https://..."))
                        .font(.callout)
                    TextField("Cover Image URL", text: $radio.cover, prompt: Text("Optional"))
                        .font(.callout)
                        .foregroundStyle(.secondary)
                }
                .padding(.vertical, 2)
            }
            .onDelete { config.radios.remove(atOffsets: $0) }
            .onMove { config.radios.move(fromOffsets: $0, toOffset: $1) }
        } header: {
            Text("Radio Stations")
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

    // MARK: - Audio Tab

    @ViewBuilder
    private var audioForm: some View {
        SwiftUI.Section("Output Format") {
            Picker("Sample Rate", selection: $config.sampleRate) {
                Text("44.1 kHz").tag(44100)
                Text("48 kHz").tag(48000)
                Text("88.2 kHz").tag(88200)
                Text("96 kHz").tag(96000)
            }
            .pickerStyle(.menu)
            Picker("Bit Depth", selection: $config.bitDepth) {
                Text("16-bit").tag(16)
                Text("24-bit").tag(24)
                Text("32-bit").tag(32)
            }
            .pickerStyle(.menu)
        }
        SwiftUI.Section("Streaming") {
            Picker("Codec", selection: $config.codec) {
                Text("FLAC (lossless)").tag("flac")
                Text("PCM (uncompressed)").tag("pcm")
                Text("F32+LZ4 (low latency)").tag("f32lz4")
                Text("F32+LZ4 encrypted").tag("f32lz4e")
            }
            .pickerStyle(.menu)
            if config.codec == "f32lz4e" {
                SecureField("Encryption Key", text: $config.encryptionPsk, prompt: Text("Pre-shared key"))
            }
        }
    }

    // MARK: - Zones & Clients Tab

    @ViewBuilder
    private var zonesForm: some View {
        SwiftUI.Section {
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
        } footer: {
            HStack {
                Button("", systemImage: "plus") {
                    config.zones.append(.init(name: "New Zone"))
                }
                Button("", systemImage: "minus") {
                    if !config.zones.isEmpty { config.zones.removeLast() }
                }
                .disabled(config.zones.isEmpty)
                Spacer()
            }
            .buttonStyle(.borderless)
        }
    }

    private var clientsForm: some View {
        SwiftUI.Section {
            ForEach($config.clients) { $client in
                VStack(alignment: .leading, spacing: 4) {
                    HStack {
                        TextField("", text: $client.icon)
                            .frame(width: 36)
                            .multilineTextAlignment(.center)
                        TextField("Name", text: $client.name)
                    }
                    HStack {
                        TextField("MAC", text: $client.mac, prompt: Text("aa:bb:cc:dd:ee:ff"))
                        TextField("Zone", text: $client.zone, prompt: Text("Zone name"))
                    }
                    .font(.callout)
                    .foregroundStyle(.secondary)
                }
                .padding(.vertical, 2)
            }
            .onDelete { config.clients.remove(atOffsets: $0) }
            .onMove { config.clients.move(fromOffsets: $0, toOffset: $1) }
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

    // MARK: - Integration Tab

    @ViewBuilder
    private var integrationForm: some View {
        SwiftUI.Section {
            Toggle("Enable MQTT", isOn: $config.mqtt.enabled)
            Group {
                TextField("Broker", text: $config.mqtt.broker, prompt: Text("host:port"))
                TextField("Client ID", text: $config.mqtt.clientId)
                TextField("Username", text: $config.mqtt.username, prompt: Text("Optional"))
                SecureField("Password", text: $config.mqtt.password)
                TextField("Base Topic", text: $config.mqtt.baseTopic)
            }
            .disabled(!config.mqtt.enabled)
        } header: {
            Text("MQTT")
        }

        SwiftUI.Section {
            Toggle("Enable KNX", isOn: $config.knx.enabled)
            Group {
                Picker("Role", selection: $config.knx.role) {
                    Text("Client (Gateway)").tag("client")
                    Text("Device (ETS)").tag("device")
                }
                .pickerStyle(.menu)
                TextField("Gateway URL", text: $config.knx.url, prompt: Text("udp://192.168.1.50:3671"))
            }
            .disabled(!config.knx.enabled)
        } header: {
            Text("KNX")
        }
    }

    // MARK: - Auto-save

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
}
