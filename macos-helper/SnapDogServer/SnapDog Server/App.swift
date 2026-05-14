import SwiftUI

@main
struct SnapDogServerApp: App {
    @State private var serverManager = ServerManager()
    @Environment(\.openWindow) private var openWindow

    var body: some Scene {
        MenuBarExtra("SnapDog", systemImage: serverManager.isRunning ? "hifispeaker.fill" : "hifispeaker") {
            Section {
                Text(serverManager.isRunning ? "● Running" : "○ Stopped")
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
                Button("Edit Configuration...") {
                    serverManager.openConfigInEditor()
                }
                Button("View Logs...") {
                    openWindow(id: "logs")
                }
            }

            Section {
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
    }
}
