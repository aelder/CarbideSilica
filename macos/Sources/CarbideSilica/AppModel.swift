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

/// The places the sidebar can show.
enum SidebarItem: Hashable {
    case dashboard
    case node
}

/// One measurement of how fast the node is validating blocks.
struct SpeedSample: Identifiable {
    let id: Int
    let time: Date
    let blocksPerSecond: Double
}

/// App-wide state: the connection, the navigation path and the dashboard's data.
@MainActor
@Observable
final class AppModel {
    var path: [Route] = []
    var sidebar: SidebarItem = .dashboard
    /// Objects opened this session, newest first.
    private(set) var recent: [Route] = []

    private(set) var node: NodeStatus?
    private(set) var nodeError: String?
    /// Validation speed over the last few minutes, oldest first.
    private(set) var speeds: [SpeedSample] = []
    private var lastHeightSample: (time: Date, height: UInt64)?

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
    ///
    /// The config file may live on an external drive, where the first read can wait on a
    /// macOS permission prompt, so this runs off the main thread and the window stays usable.
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
        explorer = nil
        status = nil
        statusError = nil
        blocks = []
        node = nil
        nodeError = nil
        speeds = []
        lastHeightSample = nil
        generation += 1
        let attempt = generation
        Task.detached {
            let result = Result { try Explorer(options: options) }
            await self.finishConnecting(result, attempt: attempt)
        }
    }

    private func finishConnecting(_ result: Result<Explorer, Error>, attempt: Int) {
        guard attempt == generation else { return }
        switch result {
        case .success(let explorer):
            self.explorer = explorer
            connection = explorer.connection()
        case .failure(let error):
            connection = nil
            statusError = describe(error)
        }
        // Views reload when the generation changes, so they start once the bridge is ready.
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
                await refreshNode()
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
        await refreshNode()
    }

    /// Refreshes sync progress, mempool size and peers, and records validation speed.
    func refreshNode() async {
        guard let explorer else { return }
        let generation = generation
        do {
            let status = try await explorer.nodeStatus()
            guard generation == self.generation else { return }
            let now = Date.now
            if let last = lastHeightSample {
                let elapsed = now.timeIntervalSince(last.time)
                if elapsed >= 1 {
                    let gained = status.blocks >= last.height ? status.blocks - last.height : 0
                    speeds.append(
                        SpeedSample(
                            id: (speeds.last?.id ?? 0) + 1, time: now,
                            blocksPerSecond: Double(gained) / elapsed))
                    if speeds.count > 150 { speeds.removeFirst(speeds.count - 150) }
                    lastHeightSample = (now, status.blocks)
                }
            } else {
                lastHeightSample = (now, status.blocks)
            }
            node = status
            nodeError = nil
        } catch {
            guard generation == self.generation else { return }
            nodeError = describe(error)
        }
    }

    /// How much of the chain the node has validated, when it is still catching up.
    var syncFraction: Double? {
        let blocks = node?.blocks ?? status?.blocks
        let longest = node?.longestChain ?? status?.longestChain
        guard let blocks, let longest, longest > blocks + 20 else { return nil }
        return Double(blocks) / Double(longest)
    }

    func show(_ item: SidebarItem) {
        sidebar = item
        path = []
    }

    private func remember(_ route: Route) {
        recent.removeAll { $0 == route }
        recent.insert(route, at: 0)
        if recent.count > 20 { recent.removeLast(recent.count - 20) }
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
            remember(route)
        } catch {
            searchError = describe(error)
        }
    }

    func open(_ route: Route) {
        if path.last != route {
            path.append(route)
        }
        remember(route)
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
    var title: String {
        switch self {
        case .block(let id):
            return id.count > 12 ? "Block \(Format.short(id, head: 6, tail: 6))" : "Block \(id)"
        case .transaction(let txid): return "Tx \(Format.short(txid, head: 6, tail: 6))"
        case .identity(let name): return Format.short(name, head: 12, tail: 6)
        case .currency(let name): return Format.short(name, head: 12, tail: 6)
        case .address(let address): return Format.short(address, head: 8, tail: 6)
        }
    }

    var symbol: String {
        switch self {
        case .block: return "cube"
        case .transaction: return "arrow.left.arrow.right"
        case .identity: return "person.crop.circle"
        case .currency: return "circle.hexagongrid"
        case .address: return "creditcard"
        }
    }

    /// The route for an address-like string: identities are i-addresses or names.
    static func holder(_ value: String) -> Route {
        if value.hasSuffix("@") || (value.hasPrefix("i") && value.count == 34) {
            return .identity(name: value)
        }
        return .address(address: value)
    }
}
