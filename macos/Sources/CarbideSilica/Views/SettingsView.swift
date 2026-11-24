import AppKit
import CarbideCore
import SwiftUI

/// Connection settings. Changes apply when the user presses Apply.
struct SettingsView: View {
    @Environment(AppModel.self) private var model

    @AppStorage(SettingsKey.chain) private var chain = "VRSC"
    @AppStorage(SettingsKey.testnet) private var testnet = false
    @AppStorage(SettingsKey.configFile) private var configFile = ""
    @AppStorage(SettingsKey.apiEnabled) private var apiEnabled = false
    @AppStorage(SettingsKey.apiURL) private var apiURL = SettingsKey.defaultAPI
    @AppStorage(SettingsKey.useUTC) private var useUTC = false
    @State private var cache: CacheInfo?
    @State private var cacheMessage: String?
    @State private var confirmingClear = false

    var body: some View {
        Form {
            Section {
                TextField("Chain", text: $chain, prompt: Text("VRSC"))
                Toggle("Testnet", isOn: $testnet)
                HStack {
                    TextField(
                        "Node configuration file", text: $configFile,
                        prompt: Text("Standard location"))
                    Button("Choose…", action: chooseConfigFile)
                }
            } header: {
                Text("Local node")
            } footer: {
                Text(daemonSummary)
                    .foregroundStyle(.secondary)
            }

            Section {
                Toggle("Use a public API when the local node cannot answer", isOn: $apiEnabled)
                TextField("API address", text: $apiURL, prompt: Text(SettingsKey.defaultAPI))
                    .disabled(!apiEnabled)
            } header: {
                Text("Public API")
            } footer: {
                Text(
                    "When this setting is on, the app sends requests to this server if the local node cannot answer. "
                        + "The requests include the addresses and identities that you search for."
                )
                .foregroundStyle(.secondary)
            }

            Section {
                Toggle("Show times in UTC", isOn: $useUTC)
            } header: {
                Text("Display")
            } footer: {
                Text("The setting applies to pages that you open after the change.").foregroundStyle(.secondary)
            }

            Section {
                if let cache {
                    LabeledContent("Size", value: Format.bytes(cache.bytes))
                    LabeledContent("Location", value: cache.path)
                }
                Button("Clear Cache…") { confirmingClear = true }
                    .disabled(cache == nil)
                if let cacheMessage {
                    Text(cacheMessage).foregroundStyle(.secondary)
                }
            } header: {
                Text("Storage")
            } footer: {
                Text("The cache contains data from transfer traces and witness searches. It does not contain bookmarks or settings.")
                    .foregroundStyle(.secondary)
            }

            HStack {
                Spacer()
                Button("Apply") { model.connect() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .formStyle(.grouped)
        .task(id: model.generation) { cache = model.explorer?.cacheInfo() }
        .confirmationDialog("Clear the cache?", isPresented: $confirmingClear) {
            Button("Clear Cache", role: .destructive) {
                do {
                    try model.explorer?.clearCache()
                    cacheMessage = "Cache cleared."
                } catch {
                    cacheMessage = describe(error)
                }
                cache = model.explorer?.cacheInfo()
            }
        } message: {
            Text("The app deletes the cached data and downloads it again when necessary. Bookmarks and settings do not change.")
        }
        .frame(width: 560)
        .fixedSize(horizontal: false, vertical: true)
    }

    private var daemonSummary: String {
        guard let connection = model.connection else {
            return "Not connected."
        }
        let file = connection.configFile ?? "none"
        if connection.configFound {
            let credentials = connection.hasCredentials ? "" : " The file has no RPC credentials."
            return "The app uses \(file) and the node at \(connection.daemonUrl).\(credentials)"
        }
        return "The app found no configuration file at \(file). It tries \(connection.daemonUrl)."
    }

    private func chooseConfigFile() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.showsHiddenFiles = true
        panel.message = "Select the .conf file of the Verus node"
        panel.directoryURL = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask).first?
            .appendingPathComponent("Komodo/VRSC")
        if panel.runModal() == .OK, let url = panel.url {
            configFile = url.path
        }
    }
}
