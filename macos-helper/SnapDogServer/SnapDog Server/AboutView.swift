import SwiftUI

struct AboutView: View {
    @Environment(\.dismiss) private var dismiss

    private var version: String {
        Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "–"
    }

    private var build: String {
        Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "–"
    }

    var body: some View {
        VStack(spacing: 16) {
            // Logo
            Image("AppIcon")
                .resizable()
                .frame(width: 80, height: 80)
                .cornerRadius(16)
                .shadow(color: .orange.opacity(0.2), radius: 12)

            // Title
            Text("SnapDog Server")
                .font(.title2)
                .fontWeight(.bold)

            // Description
            Text("Multi-zone audio controller with AirPlay, Snapcast, MQTT, and KNX integration.")
                .font(.caption)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
                .frame(maxWidth: 260)

            // Version grid
            Grid(alignment: .leading, horizontalSpacing: 24, verticalSpacing: 8) {
                GridRow {
                    Text("Version")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text("v\(version)")
                        .font(.caption)
                        .fontWeight(.medium)
                        .monospacedDigit()
                }
                GridRow {
                    Text("Build")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Text(build)
                        .font(.caption)
                        .fontWeight(.medium)
                        .monospacedDigit()
                }
                GridRow {
                    Text("License")
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                    Link("GPL-3.0", destination: URL(string: "https://www.gnu.org/licenses/gpl-3.0.html")!)
                        .font(.caption)
                        .fontWeight(.medium)
                }
            }

            Divider()

            // Links
            HStack(spacing: 16) {
                Link(destination: URL(string: "https://github.com/metaneutrons/snapdog")!) {
                    Label("GitHub", systemImage: "chevron.left.forwardslash.chevron.right")
                        .font(.caption)
                }
            }

            // Copyright
            Text("© \(Calendar.current.component(.year, from: Date())) Fabian Schmieder")
                .font(.caption2)
                .foregroundStyle(.tertiary)
        }
        .padding(24)
        .frame(width: 320)
    }
}
