import Foundation
import Testing
import TOMLKit

/// Saving keeps the keys of a client the app does not model, also after a rename (#481).
@Suite struct TOMLConfigParserTests {
    private func roundTrip(_ toml: String, edit: (ConfigModel) -> Void) throws -> TOMLTable {
        let url = FileManager.default.temporaryDirectory
            .appendingPathComponent("snapdog-parser-\(UUID().uuidString).toml")
        defer { try? FileManager.default.removeItem(at: url) }
        try toml.write(to: url, atomically: true, encoding: .utf8)
        let model = try TOMLConfigParser.load(from: url)
        edit(model)
        try TOMLConfigParser.save(model, to: url)
        return try TOMLTable(string: String(contentsOf: url, encoding: .utf8))
    }

    private let file = """
        [[zone]]
        name = "Ground Floor"

        [[client]]
        name = "Kitchen"
        mac = "aa:bb:cc:dd:ee:01"
        zone = "Ground Floor"
        max_volume = 60
        default_volume = 30
        default_latency = 40

        [[client]]
        name = "Bath"
        mac = "aa:bb:cc:dd:ee:02"
        zone = "Ground Floor"
        max_volume = 70
        """

    private func client(_ table: TOMLTable, _ name: String) -> TOMLTable? {
        table["client"]?.array?.compactMap(\.table).first { $0["name"]?.string == name }
    }

    @Test func aRenamedClientKeepsItsOtherKeys() throws {
        let saved = try roundTrip(file) { model in
            model.clients[0].name = "Kitchen Speaker"
        }
        let kitchen = try #require(client(saved, "Kitchen Speaker"))
        #expect(kitchen["max_volume"]?.int == 60)
        #expect(kitchen["default_volume"]?.int == 30)
        #expect(kitchen["default_latency"]?.int == 40)
        #expect(kitchen["mac"]?.string == "aa:bb:cc:dd:ee:01")
        #expect(client(saved, "Kitchen") == nil)
        // The other client is untouched.
        #expect(client(saved, "Bath")?["max_volume"]?.int == 70)
    }

    @Test func anAddedClientMatchingAMacFindsItsTable() throws {
        // A client that was removed and added again with the same MAC finds the old table.
        let saved = try roundTrip(file) { model in
            var replacement = ConfigModel.ClientEntry()
            replacement.name = "Bath Speaker"
            replacement.mac = "AA-BB-CC-DD-EE-02"
            replacement.zone = "Ground Floor"
            model.clients[1] = replacement
        }
        #expect(client(saved, "Bath Speaker")?["max_volume"]?.int == 70)
    }

    @Test func anUnchangedSaveKeepsEveryKey() throws {
        let saved = try roundTrip(file) { _ in }
        let kitchen = try #require(client(saved, "Kitchen"))
        #expect(kitchen["default_latency"]?.int == 40)
    }
}
