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
                        "Daemon config file", text: $configFile,
                        prompt: Text("Standard location"))
                    Button("Choose…", action: chooseConfigFile)
                }
            } header: {
                Text("Local daemon")
            } footer: {
                Text(daemonSummary)
                    .foregroundStyle(.secondary)
            }

            Section {
                Toggle("Use a public API when the local daemon can't answer", isOn: $apiEnabled)
                TextField("API address", text: $apiURL, prompt: Text(SettingsKey.defaultAPI))
                    .disabled(!apiEnabled)
            } header: {
                Text("Public API")
            } footer: {
                Text(
                    "When this is on, lookups the local daemon can't answer, including the "
                        + "addresses and identities you search for, are sent to this server."
                )
                .foregroundStyle(.secondary)
            }

            Section {
                Toggle("Show times in UTC", isOn: $useUTC)
            } header: {
                Text("Display")
            } footer: {
                Text("Applies to dates on pages opened from now on.").foregroundStyle(.secondary)
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
                Text("The cache holds data that transfer tracing and witness lookups have fetched. Bookmarks and settings are kept separately.")
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
            Text("Cached lookups are removed and fetched again when needed. Bookmarks and settings are not affected.")
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
            let credentials = connection.hasCredentials ? "" : " It has no RPC credentials."
            return "Using \(file) and the daemon at \(connection.daemonUrl).\(credentials)"
        }
        return "No config file found (looked for \(file)). Trying \(connection.daemonUrl)."
    }

    private func chooseConfigFile() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = false
        panel.showsHiddenFiles = true
        panel.message = "Choose the Verus daemon's .conf file"
        panel.directoryURL = FileManager.default
            .urls(for: .applicationSupportDirectory, in: .userDomainMask).first?
            .appendingPathComponent("Komodo/VRSC")
        if panel.runModal() == .OK, let url = panel.url {
            configFile = url.path
        }
    }
}
