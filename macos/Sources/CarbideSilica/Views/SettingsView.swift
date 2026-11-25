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
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false
    @AppStorage(SettingsKey.cacheDir) private var cacheDir = ""
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
                    .footerStyle()
            }

            Section {
                TextField("API address", text: $apiURL, prompt: Text(SettingsKey.defaultAPI))
                Toggle("Allow this API without asking", isOn: $apiEnabled)
            } header: {
                Text("Public API")
            } footer: {
                Text(
                    "Sources lists this API. When the local node cannot answer a page that you open, the app asks before it sends the request to the API. "
                        + "Turn on the setting if this Mac has no node. The app then uses the API without asking. "
                        + "The operator of the API can see the addresses and identities that you look up."
                )
                .footerStyle()
            }

            Section {
                Toggle("Show names in place of identity and currency IDs", isOn: $showsNames)
                Toggle("Show dates for block heights and times", isOn: $showsDates)
                Toggle("Show times in UTC", isOn: $useUTC)
            } header: {
                Text("Display")
            } footer: {
                Text("The settings apply at the latest to pages that you open after the change.").footerStyle()
            }

            Section {
                if let cache {
                    LabeledContent("Size", value: Format.bytes(cache.bytes))
                    LabeledContent("File") {
                        Text(cache.path)
                            .textSelection(.enabled)
                            .lineLimit(2)
                            .truncationMode(.middle)
                    }
                    if let problem = cache.problem {
                        Note(text: problem, warning: true)
                    }
                }
                HStack {
                    Button("Choose Folder…", action: chooseCacheFolder)
                    Button("Use Default Folder") {
                        cacheDir = ""
                        model.connect()
                    }
                    .disabled(cacheDir.isEmpty)
                    Button("Show in Finder") {
                        if let cache {
                            NSWorkspace.shared.activateFileViewerSelecting(
                                [URL(fileURLWithPath: cache.path)])
                        }
                    }
                    .disabled(cache.map { !FileManager.default.fileExists(atPath: $0.path) } ?? true)
                    Spacer()
                    Button("Clear Cache…") { confirmingClear = true }
                        .disabled(cache == nil)
                }
                if let cacheMessage {
                    Text(cacheMessage).foregroundStyle(.secondary)
                }
            } header: {
                Text("Cache")
            } footer: {
                Text(
                    "The cache keeps every answer from your sources, with the source and the time. The app uses it to show stored data when no source answers, and for block history. "
                        + "It does not contain bookmarks or settings. The cache grows with use. Choose a folder on another disk to keep it off this Mac's internal disk. "
                        + "A new folder starts a new cache. The previous cache stays where it is."
                )
                .footerStyle()
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

    /// A folder for the cache. The app reconnects at once and starts a cache there.
    private func chooseCacheFolder() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.canCreateDirectories = true
        panel.allowsMultipleSelection = false
        panel.prompt = "Use Folder"
        panel.message = "Select a folder for the cache. The app stores one file for each network in it."
        if let cache {
            panel.directoryURL = URL(fileURLWithPath: cache.path).deletingLastPathComponent()
        }
        if panel.runModal() == .OK, let url = panel.url {
            cacheDir = url.path
            cacheMessage = nil
            model.connect()
        }
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

extension View {
    /// A section footer: secondary text from the leading edge. A grouped form otherwise
    /// aligns a footer of several lines to the trailing edge.
    fileprivate func footerStyle() -> some View {
        foregroundStyle(.secondary)
            .multilineTextAlignment(.leading)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}
