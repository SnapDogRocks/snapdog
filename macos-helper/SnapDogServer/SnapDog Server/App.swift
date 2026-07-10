import SwiftUI
import Sparkle

@main
struct SnapDogServerApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var appDelegate
    @State private var serverManager = ServerManager.shared
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    private let updaterController = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: UpdaterDelegate.shared, userDriverDelegate: nil)

    var body: some Scene {
        MenuBarExtra("SnapDog", image: "MenuBarIcon") {
            Section {
                Text(serverManager.isRunning ? "● Running" : "○ Stopped")
                if let error = serverManager.lastError {
                    Text(error)
                        .foregroundStyle(.red)
                        .font(.caption)
                }
            }

            Section {
                if serverManager.isRunning {
                    Button("Stop Server") {
                        serverManager.stop()
                    }
                    Button("Open WebUI") {
                        serverManager.openWebUI()
                    }
                } else {
                    Button("Start Server") {
                        serverManager.start()
                    }
                }
            }

            Section {
                Button("Settings…") {
                    NSApp.activate(ignoringOtherApps: true)
                    openSettings()
                }
                .keyboardShortcut(",")
                Button("Check for Updates…") {
                    NSApp.activate(ignoringOtherApps: true)
                    updaterController.checkForUpdates(nil)
                }
                Button("View Logs...") {
                    openWindow(id: "logs")
                    NSApp.activate(ignoringOtherApps: true)
                }
            }

            Section {
                Button("About SnapDog Server…") {
                    openWindow(id: "about")
                    NSApp.activate(ignoringOtherApps: true)
                }
                Button("Quit SnapDog Server") {
                    // Routes through AppDelegate.applicationShouldTerminate, which stops the
                    // server and waits for it to exit before the app terminates.
                    NSApplication.shared.terminate(nil)
                }
                .keyboardShortcut("q")
            }
        }
        .menuBarExtraStyle(.menu)

        Settings {
            ConfigView(serverManager: serverManager)
        }

        Window("Logs", id: "logs") {
            LogView(serverManager: serverManager)
        }

        Window("About SnapDog Server", id: "about") {
            AboutView()
        }
        .windowResizability(.contentSize)
    }
}

// MARK: - App lifecycle

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    func applicationDidFinishLaunching(_ notification: Notification) {
        // Single-instance guard: if another copy is already running, bow out so two
        // supervisors don't fight over the same server process.
        let current = NSRunningApplication.current
        let others = NSRunningApplication
            .runningApplications(withBundleIdentifier: Bundle.main.bundleIdentifier ?? "")
            .filter { $0.processIdentifier != current.processIdentifier }
        if !others.isEmpty {
            NSApp.terminate(nil)
            return
        }

        // Start the server on launch if the user opted in.
        if UserDefaults.standard.bool(forKey: "startServerOnLaunch") {
            ServerManager.shared.start()
        }
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        let manager = ServerManager.shared
        guard manager.isRunning else { return .terminateNow }
        // Stop the server and wait for it to actually exit before we quit.
        manager.shutdownForQuit {
            NSApp.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}

// MARK: - Sparkle update channel

/// Gates beta updates behind the `receiveBetaUpdates` preference. For this to take effect
/// the appcast must tag beta items with `<sparkle:channel>beta</sparkle:channel>`; stable
/// items carry no channel and are always offered (RFC MAC-0006 MAC-T23, appcast side TBD).
final class UpdaterDelegate: NSObject, SPUUpdaterDelegate {
    static let shared = UpdaterDelegate()

    func allowedChannels(for updater: SPUUpdater) -> Set<String> {
        UserDefaults.standard.bool(forKey: "receiveBetaUpdates") ? ["beta"] : []
    }
}
