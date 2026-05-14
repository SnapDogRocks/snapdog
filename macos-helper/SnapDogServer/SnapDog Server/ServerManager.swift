import Foundation
import AppKit
import os

@Observable
@MainActor
final class ServerManager {
    private(set) var isRunning = false
    private(set) var logs: [String] = []

    private var process: Process?
    private let logger = Logger(subsystem: "eu.schmieder.snapdog.helper", category: "server")

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
        guard let binary = binaryPath else {
            appendLog("[ERROR] snapdog binary not found in app bundle")
            return
        }

        let proc = Process()
        proc.executableURL = binary
        proc.arguments = ["--config", configPath.path]

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

        proc.terminationHandler = { [weak self] _ in
            Task { @MainActor [weak self] in
                self?.isRunning = false
                self?.process = nil
                self?.appendLog("[SERVER] Process terminated")
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
        proc.interrupt() // SIGINT — graceful shutdown
        appendLog("[SERVER] Stopping...")
        logger.info("Sending SIGINT to server")
    }

    func openWebUI() {
        // Read port from config or default to 5555
        let url = URL(string: "http://localhost:5555")!
        NSWorkspace.shared.open(url)
    }

    func openConfigInEditor() {
        ensureConfigExists()
        NSWorkspace.shared.open(configPath)
    }

    func ensureConfigExists() {
        guard !FileManager.default.fileExists(atPath: configPath.path) else { return }
        // Create a minimal default config
        let defaultConfig = """
        [http]
        port = 5555
        base_url = "http://localhost:5555"

        [audio]
        sample_rate = 48000
        bit_depth = 16
        channels = 2

        [snapcast]
        codec = "flac"
        streaming_port = 1704
        """
        try? defaultConfig.write(to: configPath, atomically: true, encoding: .utf8)
    }

    private func appendLog(_ line: String) {
        logs.append(line)
        if logs.count > 1000 {
            logs.removeFirst(logs.count - 1000)
        }
    }
}
