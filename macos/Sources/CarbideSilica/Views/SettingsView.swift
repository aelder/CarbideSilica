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

            HStack {
                Spacer()
                Button("Apply") { model.connect() }
                    .keyboardShortcut(.defaultAction)
            }
        }
        .formStyle(.grouped)
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
