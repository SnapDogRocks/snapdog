import SwiftUI
import ServiceManagement

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
        var name = "SnapDog"
        var bitrate = 320
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
    @State private var pendingSave = false
    @State private var isLoading = false
    @Environment(\.controlActiveState) private var controlActiveState

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
        .onChange(of: controlActiveState) { _, state in
            // Pick up edits made to the TOML outside the app when the window regains focus —
            // but never clobber edits the user hasn't saved yet.
            if state == .key && !pendingSave && saveState != .saving {
                load()
            }
        }
        .modifier(AutoSaveObserver(config: config, save: debounceSave))
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
                Picker("Stream format", selection: $config.subsonic.format) {
                    Text("Original file").tag("raw")
                    Text("FLAC").tag("flac")
                    Text("MP3").tag("mp3")
                    Text("Opus").tag("opus")
                }
                Toggle("Skip TLS verification", isOn: $config.subsonic.tlsSkipVerify)
            }
        }

        SwiftUI.Section("AirPlay") {
            SecureField("Password", text: $config.airplayPassword, prompt: Text("No password"))
                .help("Optional password for AirPlay connections")
            Picker("Protocol", selection: $config.airplayMode) {
                Text("AirPlay 2").tag("airplay2")
                Text("AirPlay 1").tag("airplay1")
            }
        }

        SwiftUI.Section("Spotify") {
            Toggle("Enable Spotify Connect", isOn: $config.spotify.enabled)
            if config.spotify.enabled {
                TextField("Device name", text: $config.spotify.name, prompt: Text("SnapDog"))
                Picker("Bitrate", selection: $config.spotify.bitrate) {
                    Text("96 kbps").tag(96)
                    Text("160 kbps").tag(160)
                    Text("320 kbps").tag(320)
                }
            }
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
        SwiftUI.Section("Mixing") {
            Picker("Source conflict", selection: $config.sourceConflict) {
                Text("Last wins").tag("last_wins")
                Text("Receiver wins").tag("receiver_wins")
            }
            Stepper("Zone switch fade: \(config.zoneSwitchFadeMs) ms",
                    value: $config.zoneSwitchFadeMs, in: 0...1000, step: 50)
            Stepper("Source switch fade: \(config.sourceSwitchFadeMs) ms",
                    value: $config.sourceSwitchFadeMs, in: 0...1000, step: 50)
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
            TextField("Streaming port", value: $config.streamingPort, format: .number.grouping(.never))
            Picker("Group volume mode", selection: $config.groupVolumeMode) {
                Text("Absolute").tag("absolute")
                Text("Relative").tag("relative")
                Text("Compressed").tag("compressed")
            }
        }
        SwiftUI.Section("Server") {
            TextField("HTTP port", value: $config.httpPort, format: .number.grouping(.never))
            Picker("Unknown clients", selection: $config.unknownClients) {
                Text("Accept").tag("accept")
                Text("Ignore").tag("ignore")
                Text("Reject").tag("reject")
            }
            Picker("Default zone", selection: $config.defaultZone) {
                Text("First zone").tag("")
                ForEach(config.zones) { zone in
                    if !zone.name.isEmpty { Text(zone.name).tag(zone.name) }
                }
                if !config.defaultZone.isEmpty
                    && !config.zones.contains(where: { $0.name == config.defaultZone }) {
                    Text("\(config.defaultZone) (unknown)").tag(config.defaultZone)
                }
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

        SwiftUI.Section {
            Toggle("Enable KNX", isOn: $config.knx.enabled)
            Group {
                Picker("Role", selection: $config.knx.role) {
                    Text("Client (connect to gateway)").tag("client")
                    Text("Device (ETS-programmable)").tag("device")
                }
                TextField("Gateway URL", text: $config.knx.url,
                          prompt: Text("udp://192.168.1.50:3671"))
                if config.knx.role == "device" {
                    TextField("Individual address", text: $config.knx.individualAddress,
                              prompt: Text("1.1.100"))
                    Toggle("Persist ETS config", isOn: $config.knx.persistEts)
                    Toggle("Restart after ETS programming", isOn: $config.knx.restartAfterEts)
                }
            }
            .disabled(!config.knx.enabled)
        } header: {
            Text("KNX")
        } footer: {
            Text("Per-zone/client group addresses are set in ETS, not managed here.")
        }

        SwiftUI.Section {
            ForEach($config.apiKeys) { $key in
                SecureField("API key", text: $key.value, prompt: Text("Bearer token"))
            }
            .onDelete { config.apiKeys.remove(atOffsets: $0) }
        } header: {
            Text("API Keys")
        } footer: {
            HStack {
                Button("Add API key", systemImage: "plus") { config.apiKeys.append(.init()) }
                Button("Remove last API key", systemImage: "minus") {
                    if !config.apiKeys.isEmpty { config.apiKeys.removeLast() }
                }
                .disabled(config.apiKeys.isEmpty)
                Spacer()
                Text(config.apiKeys.isEmpty ? "No keys → API is open" : "Bearer token required")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)
            .labelStyle(.iconOnly)
        }
    }

    // MARK: - Auto-save

    private func debounceSave() {
        // Ignore the model changes that a `load()` itself produces — only real user edits
        // should schedule a write (otherwise loading would spuriously re-save and flag
        // "Restart to apply").
        guard !isLoading else { return }
        pendingSave = true
        saveTask?.cancel()
        saveTask = Task {
            try? await Task.sleep(for: .milliseconds(500))
            guard !Task.isCancelled else { return }
            save()
        }
    }

    private func load() {
        isLoading = true
        saveTask?.cancel()
        pendingSave = false
        serverManager.ensureConfigExists()
        do {
            config = try TOMLConfigParser.load(from: serverManager.configPath)
        } catch {
            config = ConfigModel()
        }
        // Re-enable saving on the next runloop turn, after the config-replacement onChange
        // handlers have fired and been ignored.
        DispatchQueue.main.async { isLoading = false }
    }

    private func save() {
        pendingSave = false
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

/// Installs the config auto-save observers in a separate type-check context (and split into
/// two chains) so the ConfigView body expression stays small enough for the Swift compiler.
private struct AutoSaveObserver: ViewModifier {
    let config: ConfigModel
    let save: () -> Void

    func body(content: Content) -> some View {
        let a = content
            .onChange(of: config.subsonic) { _, _ in save() }
            .onChange(of: config.spotify) { _, _ in save() }
            .onChange(of: config.mqtt) { _, _ in save() }
            .onChange(of: config.knx) { _, _ in save() }
            .onChange(of: config.apiKeys) { _, _ in save() }
            .onChange(of: config.zones) { _, _ in save() }
            .onChange(of: config.clients) { _, _ in save() }
            .onChange(of: config.radios) { _, _ in save() }
            .onChange(of: config.airplayPassword) { _, _ in save() }
            .onChange(of: config.airplayMode) { _, _ in save() }
            .onChange(of: config.codec) { _, _ in save() }
            .onChange(of: config.encryptionPsk) { _, _ in save() }
        return a
            .onChange(of: config.sampleRate) { _, _ in save() }
            .onChange(of: config.bitDepth) { _, _ in save() }
            .onChange(of: config.sourceConflict) { _, _ in save() }
            .onChange(of: config.zoneSwitchFadeMs) { _, _ in save() }
            .onChange(of: config.sourceSwitchFadeMs) { _, _ in save() }
            .onChange(of: config.streamingPort) { _, _ in save() }
            .onChange(of: config.unknownClients) { _, _ in save() }
            .onChange(of: config.groupVolumeMode) { _, _ in save() }
            .onChange(of: config.defaultZone) { _, _ in save() }
            .onChange(of: config.httpPort) { _, _ in save() }
    }
}
