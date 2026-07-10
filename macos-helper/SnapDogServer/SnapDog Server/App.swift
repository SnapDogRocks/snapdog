import SwiftUI
import Sparkle

@main
struct SnapDogServerApp: App {
    @State private var serverManager = ServerManager()
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    private let updaterController = SPUStandardUpdaterController(startingUpdater: true, updaterDelegate: nil, userDriverDelegate: nil)

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
                    if serverManager.isRunning {
                        serverManager.stop()
                    }
                    DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                        NSApplication.shared.terminate(nil)
                    }
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
