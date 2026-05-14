import SwiftUI

struct MenuView: View {
    @Bindable var serverManager: ServerManager
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            // Status
            HStack {
                Circle()
                    .fill(serverManager.isRunning ? .green : .red)
                    .frame(width: 8, height: 8)
                Text(serverManager.isRunning ? "Running" : "Stopped")
                    .font(.headline)
            }
            .padding(.horizontal)
            .padding(.top, 8)

            Divider()

            // Controls
            if serverManager.isRunning {
                Button("Stop Server") {
                    serverManager.stop()
                }
                .padding(.horizontal)

                Button("Open WebUI") {
                    serverManager.openWebUI()
                }
                .padding(.horizontal)
            } else {
                Button("Start Server") {
                    serverManager.start()
                }
                .padding(.horizontal)
            }

            Button("Edit Config...") {
                serverManager.openConfigInEditor()
            }
            .padding(.horizontal)

            Button("View Logs...") {
                openWindow(id: "logs")
            }
            .padding(.horizontal)

            Divider()

            Button("Quit SnapDog Helper") {
                if serverManager.isRunning {
                    serverManager.stop()
                }
                // Give server time to shut down
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
                    NSApplication.shared.terminate(nil)
                }
            }
            .padding(.horizontal)
            .padding(.bottom, 8)
        }
        .frame(width: 200)
    }
}
