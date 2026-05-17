import Foundation
import AppKit
import os
import TOMLKit

@Observable
@MainActor
final class ServerManager {
    private(set) var isRunning = false
    private(set) var logs: [String] = []
    var lastError: String?

    private var process: Process?
    private var intentionalStop = false
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
            }
        }

        do {
            try proc.run()
            process = proc
            isRunning = true
            appendLog("[SERVER] Started (PID \(proc.processIdentifier))")
            logger.info("Server started, PID \(proc.processIdentifier)")
        } catch {
            appendLog("[ERROR] Failed to start: \(error.localizedDescription)")
            logger.error("Failed to start server: \(error.localizedDescription)")
        }
    }

    func stop() {
        guard let proc = process, proc.isRunning else { return }
        intentionalStop = true
        proc.interrupt() // SIGINT — graceful shutdown
        appendLog("[SERVER] Stopping...")
        logger.info("Sending SIGINT to server")
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
