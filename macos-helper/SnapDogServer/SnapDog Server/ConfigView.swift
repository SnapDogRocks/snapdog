import SwiftUI
import ServiceManagement

// MARK: - Config Model

@Observable
final class ConfigModel {
    var subsonic = SubsonicSection()
    var zones: [ZoneEntry] = []
    var clients: [ClientEntry] = []
    var radios: [RadioEntry] = []
    var mqtt = MqttSection()
    var airplayPassword = ""
    var codec = "flac"
    var encryptionPsk = ""
    var sampleRate = 48000
    var bitDepth = 16

    struct SubsonicSection: Equatable {
        var enabled = false
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

    struct ZoneEntry: Identifiable, Equatable {
        let id = UUID()
        var name = ""
        var icon = "🏠"
    }

    struct ClientEntry: Identifiable, Equatable {
        let id = UUID()
        var name = ""
        var mac = ""
        var zone = ""
        var icon = "🔊"
    }

    struct RadioEntry: Identifiable, Equatable {
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
    @State private var saveState: SaveState = .idle

    enum SaveState: Equatable {
        case idle, saving, saved
        case failed(String)
    }

    @AppStorage("startServerOnLaunch") private var startServerOnLaunch = false
    @AppStorage("receiveBetaUpdates") private var receiveBetaUpdates = false
    @State private var launchAtLogin = false

    var body: some View {
        TabView {
            Tab("General", systemImage: "gearshape") {
                Form { generalForm }.formStyle(.grouped)
            }
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
        .safeAreaInset(edge: .bottom) { statusBar }
        .onAppear {
            load()
            launchAtLogin = (SMAppService.mainApp.status == .enabled)
        }
        .onChange(of: config.subsonic) { _, _ in debounceSave() }
        .onChange(of: config.mqtt) { _, _ in debounceSave() }
        .onChange(of: config.airplayPassword) { _, _ in debounceSave() }
        .onChange(of: config.codec) { _, _ in debounceSave() }
        .onChange(of: config.encryptionPsk) { _, _ in debounceSave() }
        .onChange(of: config.sampleRate) { _, _ in debounceSave() }
        .onChange(of: config.bitDepth) { _, _ in debounceSave() }
        .onChange(of: config.zones) { _, _ in debounceSave() }
        .onChange(of: config.clients) { _, _ in debounceSave() }
        .onChange(of: config.radios) { _, _ in debounceSave() }
    }

    // MARK: - General Tab

    @ViewBuilder
    private var generalForm: some View {
        SwiftUI.Section("Startup") {
            Toggle("Launch SnapDog at login", isOn: $launchAtLogin)
                .onChange(of: launchAtLogin) { _, enabled in setLaunchAtLogin(enabled) }
            Toggle("Start server automatically on launch", isOn: $startServerOnLaunch)
        }
        SwiftUI.Section {
            Toggle("Receive beta updates", isOn: $receiveBetaUpdates)
        } header: {
            Text("Updates")
        } footer: {
            Text("Beta builds may be unstable. Takes effect on the next update check.")
        }
    }

    private func setLaunchAtLogin(_ enabled: Bool) {
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            // Revert the toggle to the real service state on failure.
            launchAtLogin = (SMAppService.mainApp.status == .enabled)
        }
    }

    // MARK: - Sources Tab

    @ViewBuilder
    private var sourcesForm: some View {
        SwiftUI.Section("Subsonic") {
            Toggle("Enable", isOn: $config.subsonic.enabled)
            if config.subsonic.enabled {
                TextField("Server URL", text: $config.subsonic.url, prompt: Text("http://navidrome:4533"))
                if !config.subsonic.url.isEmpty && !isValidHTTPURL(config.subsonic.url) {
                    Text("Enter a valid http(s) URL").font(.caption).foregroundStyle(.red)
                }
                TextField("Username", text: $config.subsonic.username)
                SecureField("Password", text: $config.subsonic.password)
            }
        }

        SwiftUI.Section("AirPlay") {
            SecureField("Password", text: $config.airplayPassword, prompt: Text("No password"))
                .help("Optional password for AirPlay connections")
        }

        SwiftUI.Section {
            ForEach($config.radios) { $radio in
                VStack(alignment: .leading, spacing: 4) {
                    TextField("Station Name", text: $radio.name, prompt: Text("Station Name"))
                    TextField("Stream URL", text: $radio.url, prompt: Text("https://..."))
                    TextField("Cover Image URL", text: $radio.cover, prompt: Text("Optional"))
                }
                .padding(.vertical, 2)
            }
            .onDelete { config.radios.remove(atOffsets: $0) }
            .onMove { config.radios.move(fromOffsets: $0, toOffset: $1) }
        } header: {
            Text("Radio Stations")
        } footer: {
            HStack {
                Button("Add radio station", systemImage: "plus") {
                    config.radios.append(.init())
                }
                Button("Remove last radio station", systemImage: "minus") {
                    if !config.radios.isEmpty { config.radios.removeLast() }
                }
                .disabled(config.radios.isEmpty)
                Spacer()
            }
            .buttonStyle(.borderless)
            .labelStyle(.iconOnly)
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
                        .accessibilityLabel("Zone icon")
                        .onTapGesture {
                            NSApp.orderFrontCharacterPalette(nil)
                        }
                    TextField("Zone Name", text: $zone.name)
                }
            }
            .onDelete { config.zones.remove(atOffsets: $0) }
            .onMove { config.zones.move(fromOffsets: $0, toOffset: $1) }
        } footer: {
            HStack {
                Button("Add zone", systemImage: "plus") {
                    config.zones.append(.init(name: "New Zone"))
                }
                Button("Remove last zone", systemImage: "minus") {
                    if !config.zones.isEmpty { config.zones.removeLast() }
                }
                .disabled(config.zones.isEmpty)
                Spacer()
            }
            .buttonStyle(.borderless)
            .labelStyle(.iconOnly)
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
                            .accessibilityLabel("Client icon")
                            .onTapGesture {
                                NSApp.orderFrontCharacterPalette(nil)
                            }
                        TextField("Name", text: $client.name)
                    }
                    TextField("MAC", text: $client.mac, prompt: Text("aa:bb:cc:dd:ee:ff"))
                        .font(.callout)
                    if !client.mac.isEmpty && !isValidMAC(client.mac) {
                        Text("Invalid MAC address (expected aa:bb:cc:dd:ee:ff)")
                            .font(.caption).foregroundStyle(.red)
                    }
                    Picker("Zone", selection: $client.zone) {
                        Text("Unassigned").tag("")
                        ForEach(config.zones) { zone in
                            if !zone.name.isEmpty { Text(zone.name).tag(zone.name) }
                        }
                        // Keep a zone that isn't in the current list so editing doesn't drop it.
                        if !client.zone.isEmpty && !config.zones.contains(where: { $0.name == client.zone }) {
                            Text("\(client.zone) (unknown)").tag(client.zone)
                        }
                    }
                    .pickerStyle(.menu)
                }
                .padding(.vertical, 2)
            }
            .onDelete { config.clients.remove(atOffsets: $0) }
            .onMove { config.clients.move(fromOffsets: $0, toOffset: $1) }
        } footer: {
            HStack {
                Button("Add client", systemImage: "plus") {
                    config.clients.append(.init())
                }
                Button("Remove last client", systemImage: "minus") {
                    if !config.clients.isEmpty { config.clients.removeLast() }
                }
                .disabled(config.clients.isEmpty)
                Spacer()
            }
            .buttonStyle(.borderless)
            .labelStyle(.iconOnly)
        }
    }

    // MARK: - Integration Tab

    @ViewBuilder
    private var integrationForm: some View {
        SwiftUI.Section {
            Toggle("Enable MQTT", isOn: $config.mqtt.enabled)
            Group {
                TextField("Broker", text: $config.mqtt.broker, prompt: Text("host:port"))
                if config.mqtt.enabled && !config.mqtt.broker.isEmpty && !isValidHostPort(config.mqtt.broker) {
                    Text("Enter host:port (e.g. broker.local:1883)")
                        .font(.caption).foregroundStyle(.red)
                }
                TextField("Client ID", text: $config.mqtt.clientId)
                TextField("Username", text: $config.mqtt.username)
                SecureField("Password", text: $config.mqtt.password)
                TextField("Base Topic", text: $config.mqtt.baseTopic)
            }
            .disabled(!config.mqtt.enabled)
        } header: {
            Text("MQTT")
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
        saveState = .saving
        do {
            try TOMLConfigParser.save(config, to: serverManager.configPath)
            saveState = .saved
            // A running server read its config at launch; the on-disk change is not
            // live until a restart.
            if serverManager.isRunning { serverManager.configDirty = true }
            // Auto-clear the "Saved" indicator after a moment.
            Task {
                try? await Task.sleep(for: .seconds(2))
                if saveState == .saved { saveState = .idle }
            }
        } catch {
            saveState = .failed(error.localizedDescription)
        }
    }

    // MARK: - Status / restart bar

    @ViewBuilder
    private var statusBar: some View {
        HStack(spacing: 8) {
            switch saveState {
            case .idle:
                EmptyView()
            case .saving:
                ProgressView().controlSize(.small)
                Text("Saving…").font(.caption).foregroundStyle(.secondary)
            case .saved:
                Image(systemName: "checkmark.circle.fill").foregroundStyle(.green)
                Text("Saved").font(.caption).foregroundStyle(.secondary)
            case .failed(let message):
                Image(systemName: "exclamationmark.triangle.fill").foregroundStyle(.red)
                Text(message).font(.caption).foregroundStyle(.red).lineLimit(1)
            }
            Spacer()
            if serverManager.isRunning && serverManager.configDirty {
                Text("Config changed").font(.caption).foregroundStyle(.secondary)
                Button("Restart to apply") { serverManager.restart() }
                    .controlSize(.small)
            }
        }
        .frame(minHeight: 20)
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(.bar)
    }

    // MARK: - Validation

    private func isValidMAC(_ s: String) -> Bool {
        s.range(of: "^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$", options: .regularExpression) != nil
    }

    private func isValidHostPort(_ s: String) -> Bool {
        let parts = s.split(separator: ":", omittingEmptySubsequences: false)
        guard parts.count == 2, !parts[0].isEmpty,
              let port = Int(parts[1]), (1...65535).contains(port) else { return false }
        return true
    }

    private func isValidHTTPURL(_ s: String) -> Bool {
        guard let url = URL(string: s), let scheme = url.scheme?.lowercased(),
              scheme == "http" || scheme == "https", url.host?.isEmpty == false else { return false }
        return true
    }
}
