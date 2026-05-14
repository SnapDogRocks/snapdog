import SwiftUI

@main
struct SnapDogHelperApp: App {
    @State private var serverManager = ServerManager()

    var body: some Scene {
        MenuBarExtra("SnapDog", systemImage: serverManager.isRunning ? "hifispeaker.fill" : "hifispeaker") {
            MenuView(serverManager: serverManager)
        }
        .menuBarExtraStyle(.window)

        Settings {
            ConfigView(serverManager: serverManager)
        }

        Window("Logs", id: "logs") {
            LogView(serverManager: serverManager)
        }
    }
}
