import SwiftUI

struct LogView: View {
    @Bindable var serverManager: ServerManager

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 2) {
                    ForEach(Array(serverManager.logs.enumerated()), id: \.offset) { index, line in
                        Text(line)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(lineColor(line))
                            .textSelection(.enabled)
                            .id(index)
                    }
                }
                .padding(8)
            }
            .onChange(of: serverManager.logs.count) { _, _ in
                if let last = serverManager.logs.indices.last {
                    proxy.scrollTo(last, anchor: .bottom)
                }
            }
        }
        .frame(minWidth: 600, minHeight: 400)
        .navigationTitle("SnapDog Logs")
    }

    private func lineColor(_ line: String) -> Color {
        if line.contains("ERROR") { return .red }
        if line.contains("WARN") { return .orange }
        if line.contains("[SERVER]") { return .blue }
        return .primary
    }
}
