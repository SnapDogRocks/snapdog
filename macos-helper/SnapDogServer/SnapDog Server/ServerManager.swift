import Foundation
import AppKit
import os
import Security
import TOMLKit

/// Keychain-backed store for server secrets so they stay out of the plaintext `snapdog.toml`.
/// The values are injected as environment variables at spawn — the snapdog server reads
/// `SNAPDOG_SUBSONIC_PASSWORD` / `SNAPDOG_MQTT_PASSWORD` / `SNAPDOG_SNAPCAST_ENCRYPTION_PSK`
/// / `SNAPDOG_HTTP_API_KEYS`. Accounts: "subsonic.password", "mqtt.password",
/// "snapcast.encryption_psk", "http.api_keys". (AirPlay has no env override, so its password
/// still lives in the TOML.)
enum Secrets {
    static let subsonicPassword = "subsonic.password"
    static let mqttPassword = "mqtt.password"
    static let encryptionPsk = "snapcast.encryption_psk"
    static let apiKeys = "http.api_keys"

    private static let service = "com.metaneutrons.snapdog.helper"

    static func get(_ account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data,
              let value = String(data: data, encoding: .utf8) else { return nil }
        return value
    }

    static func set(_ value: String, _ account: String) {
        guard !value.isEmpty else { delete(account); return }
        let base: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        let data = Data(value.utf8)
        if SecItemCopyMatching(base as CFDictionary, nil) == errSecSuccess {
            SecItemUpdate(base as CFDictionary, [kSecValueData as String: data] as CFDictionary)
        } else {
            var add = base
            add[kSecValueData as String] = data
            SecItemAdd(add as CFDictionary, nil)
        }
    }

    static func delete(_ account: String) {
        SecItemDelete([
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ] as CFDictionary)
    }
}

@Observable
@MainActor
final class ServerManager {
    /// App-wide instance shared between the SwiftUI scenes and the AppDelegate
    /// (which needs it for start-on-launch and clean-quit coordination).
    static let shared = ServerManager()

    private(set) var isRunning = false
    private(set) var logs: [String] = []
    var lastError: String?
    /// Set when the config is saved while the server is running: the on-disk change
    /// is not live until the next (re)start. Drives the "Restart to apply" banner.
    var configDirty = false

    private var process: Process?
    private var intentionalStop = false
    private var pendingRestart = false
    private var crashCount = 0
    private var lastCrash: Date?
    private let logger = Logger(subsystem: "com.metaneutrons.snapdog.helper", category: "server")

    /// Reset crash count after this interval of stable running.
    private static let crashCountResetInterval: TimeInterval = 60
    /// Maximum consecutive restarts before giving up.
    private static let maxCrashRestarts = 5

    var configPath: URL {
        let appSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first!
        let dir = appSupport.appendingPathComponent("SnapDog", isDirectory: true)
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("snapdog.toml")
    }

    private var binaryPath: URL? {
        Bundle.main.bundleURL
            .appendingPathComponent("Contents/Helpers/snapdog")
    }

    func start() {
        guard !isRunning else { return }
        lastError = nil
        intentionalStop = false
        ensureConfigExists()
        guard let binary = binaryPath,
              FileManager.default.isExecutableFile(atPath: binary.path) else {
            appendLog("[ERROR] snapdog binary not found in app bundle")
            lastError = "snapdog binary not found in app bundle."
            return
        }

        let proc = Process()
        proc.executableURL = binary
        proc.arguments = ["--config", configPath.path, "--log-level", "info"]

        // Inject Keychain-stored secrets as env vars (the server applies them over the TOML).
        var env = ProcessInfo.processInfo.environment
        if let v = Secrets.get(Secrets.subsonicPassword), !v.isEmpty {
            env["SNAPDOG_SUBSONIC_PASSWORD"] = v
        }
        if let v = Secrets.get(Secrets.mqttPassword), !v.isEmpty {
            env["SNAPDOG_MQTT_PASSWORD"] = v
        }
        if let v = Secrets.get(Secrets.encryptionPsk), !v.isEmpty {
            env["SNAPDOG_SNAPCAST_ENCRYPTION_PSK"] = v
        }
        if let v = Secrets.get(Secrets.apiKeys), !v.isEmpty {
            env["SNAPDOG_HTTP_API_KEYS"] = v
        }
        proc.environment = env

        let pipe = Pipe()
        proc.standardOutput = pipe
        proc.standardError = pipe

        pipe.fileHandleForReading.readabilityHandler = { [weak self] handle in
            let data = handle.availableData
            guard !data.isEmpty, let line = String(data: data, encoding: .utf8) else { return }
            Task { @MainActor [weak self] in
                self?.appendLog(line.trimmingCharacters(in: .newlines))
            }
        }

        proc.terminationHandler = { [weak self] proc in
            Task { @MainActor [weak self] in
                guard let self else { return }
                self.isRunning = false
                self.process = nil
                let code = proc.terminationStatus
                if code == 0 || self.intentionalStop {
                    self.appendLog("[SERVER] Stopped")
                    self.crashCount = 0
                } else {
                    self.appendLog("[SERVER] Crashed (exit code \(code))")
                    self.lastError = "Server exited with code \(code). Check logs for details."
                    self.scheduleRestart()
                }
                // A restart() requested a clean stop-then-start: relaunch now that the
                // previous process has actually terminated.
                if self.pendingRestart {
                    self.pendingRestart = false
                    self.start()
                }
            }
        }

        do {
            try proc.run()
            process = proc
            isRunning = true
            configDirty = false
            appendLog("[SERVER] Started (PID \(proc.processIdentifier))")
            logger.info("Server started, PID \(proc.processIdentifier)")
        } catch {
            appendLog("[ERROR] Failed to start: \(error.localizedDescription)")
            lastError = "Failed to start server: \(error.localizedDescription)"
            logger.error("Failed to start server: \(error.localizedDescription)")
            scheduleRestart()
        }
    }

    /// Restart the server so a config change takes effect: stop, wait for the process to
    /// terminate, then start again. If it is not running, just start.
    func restart() {
        guard isRunning, let proc = process, proc.isRunning else {
            start()
            return
        }
        pendingRestart = true
        stop()
    }

    func stop() {
        guard let proc = process, proc.isRunning else { return }
        intentionalStop = true
        proc.interrupt() // SIGINT — graceful shutdown
        appendLog("[SERVER] Stopping...")
        logger.info("Sending SIGINT to server")
    }

    /// Graceful stop for app quit: SIGINT, then wait up to `timeout` for the process to
    /// actually exit, escalating to SIGTERM if it doesn't. Calls `completion` on the main
    /// actor once the process is gone, so the app can terminate without orphaning the server.
    func shutdownForQuit(timeout: TimeInterval = 5, completion: @escaping @MainActor () -> Void) {
        guard let proc = process, proc.isRunning else { completion(); return }
        intentionalStop = true
        proc.interrupt()
        appendLog("[SERVER] Stopping for quit…")
        Task { @MainActor in
            let deadline = Date().addingTimeInterval(timeout)
            while proc.isRunning && Date() < deadline {
                try? await Task.sleep(for: .milliseconds(100))
            }
            if proc.isRunning {
                appendLog("[SERVER] Force-terminating (did not exit in \(Int(timeout))s)")
                proc.terminate()
            }
            completion()
        }
    }

    func openWebUI() {
        NSWorkspace.shared.open(configuredWebUIURL())
    }

    func openConfigInEditor() {
        ensureConfigExists()
        NSWorkspace.shared.open(configPath)
    }

    func ensureConfigExists() {
        guard !FileManager.default.fileExists(atPath: configPath.path) else { return }
        let defaultConfig = """
        [http]
        port = 5555
        bind = "127.0.0.1"
        base_url = "http://localhost:5555"

        [audio]
        sample_rate = 48000
        bit_depth = 16
        source_conflict = "last_wins"
        zone_switch_fade_ms = 300
        source_switch_fade_ms = 300

        [snapcast]
        streaming_port = 1704
        codec = "flac"
        group_volume_mode = "relative"
        unknown_clients = "accept"
        """
        try? defaultConfig.write(to: configPath, atomically: true, encoding: .utf8)
    }

    private func configuredWebUIURL() -> URL {
        ensureConfigExists()
        guard let content = try? String(contentsOf: configPath, encoding: .utf8),
              let table = try? TOMLTable(string: content),
              let http = table["http"] as? TOMLTable else {
            return URL(string: "http://localhost:5555")!
        }

        if let baseURL = http["base_url"] as? String,
           let url = URL(string: baseURL),
           let scheme = url.scheme,
           ["http", "https"].contains(scheme) {
            return url
        }

        let port = tomlInt(http["port"]) ?? 5555
        return URL(string: "http://localhost:\(port)")!
    }

    private func tomlInt(_ value: Any?) -> Int? {
        switch value {
        case let int as Int:
            return int
        case let int64 as Int64:
            return Int(int64)
        case let number as NSNumber:
            return number.intValue
        default:
            return nil
        }
    }

    private func scheduleRestart() {
        // Reset crash count if last crash was long ago (server was stable)
        if let last = lastCrash, Date().timeIntervalSince(last) > Self.crashCountResetInterval {
            crashCount = 0
        }
        crashCount += 1
        lastCrash = Date()

        guard crashCount <= Self.maxCrashRestarts else {
            appendLog("[SERVER] Too many crashes (\(crashCount)), not restarting")
            logger.error("Giving up after \(self.crashCount) consecutive crashes")
            return
        }

        let delay = min(pow(2.0, Double(crashCount - 1)), 30.0) // 1s, 2s, 4s, 8s, 16s, 30s
        appendLog("[SERVER] Restarting in \(Int(delay))s (attempt \(crashCount)/\(Self.maxCrashRestarts))...")
        logger.info("Auto-restart in \(delay)s (attempt \(self.crashCount))")

        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(delay))
            guard let self, !self.isRunning, !self.intentionalStop else { return }
            self.start()
        }
    }

    private func appendLog(_ line: String) {
        logs.append(line)
        if logs.count > 1000 {
            logs.removeFirst(logs.count - 1000)
        }
    }
}
