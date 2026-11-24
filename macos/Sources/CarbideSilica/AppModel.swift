import CarbideCore
import Foundation
import Observation

/// User-chosen connection settings, stored in `UserDefaults`.
enum SettingsKey {
    static let chain = "chain"
    static let testnet = "testnet"
    static let configFile = "configFile"
    static let apiEnabled = "apiEnabled"
    static let apiURL = "apiURL"

    static let defaultAPI = "https://api.verus.services"
}

/// App-wide state: the connection, the navigation path and the dashboard's data.
@MainActor
@Observable
final class AppModel {
    var path: [Route] = []

    private(set) var explorer: Explorer?
    private(set) var connection: ConnectionInfo?
    /// Bumped on every reconnect so that open views reload.
    private(set) var generation = 0

    private(set) var status: ChainStatus?
    private(set) var statusError: String?
    private(set) var blocks: [BlockRow] = []
    private(set) var lastRefresh: Date?

    var searchText = ""
    /// Incremented to ask the search field to take keyboard focus.
    var searchFocusRequests = 0
    private(set) var isSearching = false
    var searchError: String?

    private let recentBlockCount: UInt32 = 30
    private let keptBlockCount = 60

    init() {
        connect()
    }

    /// The coin's ticker for native amounts.
    var coin: String { status?.name ?? connection?.chain ?? "VRSC" }

    /// Opens the bridge with the saved settings. Reads config files only; no network.
    func connect() {
        let defaults = UserDefaults.standard
        let chain = defaults.string(forKey: SettingsKey.chain) ?? "VRSC"
        let configFile = defaults.string(forKey: SettingsKey.configFile) ?? ""
        let apiURL = defaults.string(forKey: SettingsKey.apiURL) ?? SettingsKey.defaultAPI
        let options = ConnectionOptions(
            chain: chain,
            testnet: defaults.bool(forKey: SettingsKey.testnet),
            configFile: configFile.isEmpty ? nil : configFile,
            apiEndpoint: defaults.bool(forKey: SettingsKey.apiEnabled) ? apiURL : nil
        )
        status = nil
        statusError = nil
        blocks = []
        do {
            let explorer = try Explorer(options: options)
            self.explorer = explorer
            connection = explorer.connection()
        } catch {
            explorer = nil
            connection = nil
            statusError = describe(error)
        }
        generation += 1
    }

    /// Turns on the public API fallback. Only called from an explicit user action.
    func enablePublicAPI() {
        let defaults = UserDefaults.standard
        if (defaults.string(forKey: SettingsKey.apiURL) ?? "").isEmpty {
            defaults.set(SettingsKey.defaultAPI, forKey: SettingsKey.apiURL)
        }
        defaults.set(true, forKey: SettingsKey.apiEnabled)
        connect()
    }

    /// Refreshes the chain status and fetches only the blocks that are new.
    func refresh() async {
        guard let explorer else { return }
        let generation = generation
        do {
            let status = try await explorer.chainStatus()
            guard generation == self.generation else { return }
            self.status = status
            statusError = nil

            let newest = blocks.first?.height
            let missing = newest.map { status.blocks > $0 ? status.blocks - $0 : 0 }
            if missing == 0 {
                lastRefresh = .now
                return
            }
            let wanted = min(missing ?? UInt64(recentBlockCount), UInt64(recentBlockCount))
            let fetched = try await explorer.recentBlocks(
                before: status.blocks, count: UInt32(wanted))
            guard generation == self.generation else { return }
            let known = Set(fetched.map(\.hash))
            // A gap larger than one page means the old rows are no longer contiguous.
            let older = (missing ?? 0) > UInt64(recentBlockCount)
                ? [] : blocks.filter { !known.contains($0.hash) }
            blocks = Array((fetched + older).prefix(keptBlockCount))
            lastRefresh = .now
        } catch {
            guard generation == self.generation else { return }
            statusError = describe(error)
        }
    }

    /// Resolves the search field's text and navigates to what it names.
    func search() async {
        let query = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty, !isSearching else { return }
        guard let explorer else {
            searchError = statusError ?? "Not connected."
            return
        }
        isSearching = true
        defer { isSearching = false }
        do {
            let route = try await explorer.resolve(query: query)
            searchText = ""
            // A search starts a new trail, as in the terminal app.
            path = [route]
        } catch {
            searchError = describe(error)
        }
    }

    func open(_ route: Route) {
        if path.last != route {
            path.append(route)
        }
    }

    func requireExplorer() throws -> Explorer {
        guard let explorer else {
            throw CarbideError.Failed(message: statusError ?? "Not connected.")
        }
        return explorer
    }
}

func describe(_ error: Error) -> String {
    if case let CarbideError.Failed(message) = error {
        return message
    }
    return error.localizedDescription
}

extension Route {
    /// The route for an address-like string: identities are i-addresses or names.
    static func holder(_ value: String) -> Route {
        if value.hasSuffix("@") || (value.hasPrefix("i") && value.count == 34) {
            return .identity(name: value)
        }
        return .address(address: value)
    }
}
