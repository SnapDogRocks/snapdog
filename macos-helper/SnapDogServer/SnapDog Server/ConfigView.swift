import SwiftUI

struct ConfigView: View {
    @Bindable var serverManager: ServerManager
    @State private var configText = ""
    @State private var hasChanges = false

    var body: some View {
        VStack(spacing: 0) {
            TextEditor(text: $configText)
                .font(.system(.body, design: .monospaced))
                .onChange(of: configText) { _, _ in hasChanges = true }

            HStack {
                Text(serverManager.configPath.path)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                Spacer()
                Button("Reload") { loadConfig() }
                Button("Save") { saveConfig() }
                    .disabled(!hasChanges)
                    .keyboardShortcut("s", modifiers: .command)
            }
            .padding(8)
        }
        .frame(minWidth: 500, minHeight: 400)
        .onAppear { loadConfig() }
    }

    private func loadConfig() {
        serverManager.ensureConfigExists()
        configText = (try? String(contentsOf: serverManager.configPath, encoding: .utf8)) ?? ""
        hasChanges = false
    }

    private func saveConfig() {
        try? configText.write(to: serverManager.configPath, atomically: true, encoding: .utf8)
        hasChanges = false
    }
}
