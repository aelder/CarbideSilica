import CarbideCore
import Foundation
import Observation

/// User-chosen connection settings, stored in `UserDefaults`.
enum SettingsKey {
    static let chain = "chain"
    static let testnet = "testnet"
    static let configFile = "configFile"
    static let apiEnabled = "apiEnabled"
    static let chainConfigs = "chainConfigs"
    static let cacheDir = "cacheDir"
    static let useUTC = "useUTC"
    static let apiURL = "apiURL"
    /// Sources: show the activity of every source, not only the selected one.
    static let allSourceActivity = "allSourceActivity"

    static let defaultAPI = "https://api.verus.services"
}

/// The places the sidebar can show.
enum SidebarItem: Hashable {
    case dashboard
    case mempool
    case charts
    case currencies
    case marketplace
    case bookmarks
    case node
    case sources
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

    /// Names resolved for identity and currency IDs, so views can show a name before an address.
    private(set) var names: [String: String] = [:]
    private var nameRequests: Set<String> = []
    private var openedStartObject = false
    /// True until the person switches the chain: start-up arguments then stop applying.
    private var usesLaunchArguments = true
    /// Carbide's optional start-up bookmark population runs once for each launch.
    private static var populatedBookmarks = false

    private(set) var bookmarks: [BookmarkRow] = []
    private(set) var bookmarkError: String?

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
    /// When this app first saw each block that arrived while it was running, by block hash.
    /// A block that was loaded at the start has no entry: the app did not see it arrive.
    private(set) var arrivals: [String: Date] = [:]
    private(set) var lastRefresh: Date?

    var searchText = ""
    /// Incremented to ask the search field to take keyboard focus.
    var searchFocusRequests = 0
    private(set) var isSearching = false
    @ObservationIgnored private var searchRun = 0
    var searchError: String?

    /// Primary requests that wait for the person's decision about a public API, oldest first.
    private(set) var permissionRequests: [PermissionRequest] = []
    private var permissionListener: Task<Void, Never>?

    private let recentBlockCount: UInt32 = 40
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
        var options = ConnectionOptions(
            chain: chain,
            testnet: defaults.bool(forKey: SettingsKey.testnet),
            configFile: configFile.isEmpty ? nil : configFile,
            apiEndpoint: defaults.bool(forKey: SettingsKey.apiEnabled) ? apiURL : nil,
            cacheDir: defaults.string(forKey: SettingsKey.cacheDir)
        )
        // Start-up arguments override the saved connection for this run. They are not saved,
        // and they stop applying when the person switches the chain.
        let launch = Launch.arguments
        if usesLaunchArguments, launch.problem == nil {
            if let named = launch.chain {
                options.chain = named
                // A chain named on the command line does not use another chain's saved file.
                options.configFile = nil
            }
            if launch.testnet { options.testnet = true }
            // With a data directory, Carbide looks for the file there, as the node does.
            if launch.dataDir != nil { options.configFile = nil }
            if let file = launch.configFile { options.configFile = file }
            options.dataDir = launch.dataDir
            options.rpcHost = launch.rpcHost
            options.rpcPort = launch.rpcPort
            options.rpcUser = launch.rpcUser
            options.rpcPassword = launch.rpcPassword
        }
        // The API in Settings is always listed as a source, disabled unless it is allowed,
        // so that the app can offer it when the node cannot answer.
        let offered = apiURL.isEmpty ? [] : [apiURL]
        // Requests of the previous connection must not wait for an answer that never comes.
        if let previous = explorer {
            for request in permissionRequests {
                try? previous.answerPermission(id: request.id, answer: .cancel)
            }
        }
        permissionRequests = []
        permissionListener?.cancel()
        permissionListener = nil
        explorer = nil
        names = [:]
        nameRequests = []
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
            let result = Result { try Explorer.withSources(options: options, offeredApis: offered) }
            await self.finishConnecting(result, attempt: attempt)
        }
    }

    private func finishConnecting(_ result: Result<Explorer, Error>, attempt: Int) {
        guard attempt == generation else { return }
        switch result {
        case .success(let explorer):
            self.explorer = explorer
            connection = explorer.connection()
            listenForPermissionRequests(explorer)
            // Reconciliation of the current chain, and of monitored nodes, as the terminal does.
            Task { await explorer.startBackground() }
            populateBookmarksOnce(explorer)
            // The toolbar and each page need the chain status, on any page that is open.
            Task { await refresh() }
        case .failure(let error):
            connection = nil
            statusError = describe(error)
        }
        // Views reload when the generation changes, so they start once the bridge is ready.
        generation += 1
        reloadBookmarks()
        openStartObject()
    }

    /// Carbide's start-up population of uncached bookmarks, once per launch, in the
    /// background. It does nothing unless Carbide's config.toml turns it on, and it never
    /// asks for permission.
    private func populateBookmarksOnce(_ explorer: Explorer) {
        guard !Self.populatedBookmarks else { return }
        Self.populatedBookmarks = true
        Task { [weak self] in
            _ = try? await explorer.populateBookmarks()
            guard let self, self.explorer === explorer else { return }
            self.reloadBookmarks()
        }
    }

    /// Opens the object named on the command line, once: `CarbideSilica -open vETH@`.
    /// This mirrors Carbide's `--open`.
    private func openStartObject() {
        guard !openedStartObject, explorer != nil else { return }
        openedStartObject = true
        // Every form Carbide accepts: -open=, --open, --object and a bare object.
        guard let query = Launch.arguments.openObject, !query.isEmpty else { return }
        searchText = query
        Task { await search() }
    }

    /// Daemon configurations found on this Mac, for the chain menu.
    var knownChainList: [KnownChain] {
        knownChains(testnet: UserDefaults.standard.bool(forKey: SettingsKey.testnet))
    }

    var currentChain: String {
        connection?.chain ?? UserDefaults.standard.string(forKey: SettingsKey.chain) ?? "VRSC"
    }

    /// Switches to another configured chain. Each chain remembers the config file it was
    /// last used with, so a custom path for one chain survives a visit to another.
    func switchChain(to chain: KnownChain) {
        let defaults = UserDefaults.standard
        var remembered = defaults.dictionary(forKey: SettingsKey.chainConfigs) as? [String: String] ?? [:]
        let saved = defaults.string(forKey: SettingsKey.chain) ?? "VRSC"
        remembered[saved] = defaults.string(forKey: SettingsKey.configFile) ?? ""
        defaults.set(remembered, forKey: SettingsKey.chainConfigs)
        defaults.set(chain.chain, forKey: SettingsKey.chain)
        defaults.set(remembered[chain.chain] ?? chain.configFile, forKey: SettingsKey.configFile)
        path = []
        recent = []
        usesLaunchArguments = false
        connect()
    }

    /// Makes the chain of a node in Sources the current chain.
    func switchChain(to source: SourceRow) {
        guard let file = source.configFile else { return }
        switchChain(
            to: KnownChain(
                chain: source.chain, configFile: file,
                testnet: UserDefaults.standard.bool(forKey: SettingsKey.testnet)))
    }

    /// Waits for primary requests that need permission to use a public API, and queues them
    /// for the dialog. It stops when the connection changes.
    private func listenForPermissionRequests(_ explorer: Explorer) {
        permissionListener?.cancel()
        permissionListener = Task { [weak self] in
            while !Task.isCancelled {
                let request = await explorer.nextPermissionRequest()
                guard let self, self.explorer === explorer else { return }
                if let request {
                    self.permissionRequests.append(request)
                }
            }
        }
    }

    /// Answers the oldest permission request.
    func answerPermission(_ answer: PermissionAnswer) {
        guard let explorer, !permissionRequests.isEmpty else { return }
        let request = permissionRequests.removeFirst()
        try? explorer.answerPermission(id: request.id, answer: answer)
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
            // Only a small step forward is an arrival. A first load or a long catch-up is not.
            if let missing, missing <= 5 {
                for block in fetched where arrivals[block.hash] == nil { arrivals[block.hash] = .now }
            }
            blocks = Array((fetched + older).prefix(keptBlockCount))
            let kept = Set(blocks.map(\.hash))
            arrivals = arrivals.filter { kept.contains($0.key) }
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

    func reloadBookmarks() {
        guard let explorer else { return }
        do {
            bookmarks = try explorer.bookmarks()
            bookmarkError = nil
        } catch {
            bookmarkError = describe(error)
        }
    }

    /// Saves the object as a bookmark in Carbide's bookmark file.
    func addBookmark(_ route: Route, description: String) {
        guard let explorer, route.isBookmarkable else { return }
        let (kind, target): (String, String) =
            switch route {
            case .block(let id): ("block", id)
            case .transaction(let txid): ("transaction", txid)
            case .identity(let name): ("identity", name)
            case .currency(let name): ("currency", name)
            case .address(let address): ("address", address)
            case .currencies, .estimate: ("", "")
            }
        do {
            try explorer.addBookmark(kind: kind, target: target, description: description)
            bookmarkError = nil
        } catch {
            bookmarkError = describe(error)
        }
        reloadBookmarks()
    }

    func removeBookmark(_ index: UInt32) {
        guard let explorer else { return }
        do {
            try explorer.removeBookmark(index: index)
            bookmarkError = nil
        } catch {
            bookmarkError = describe(error)
        }
        reloadBookmarks()
    }

    func editBookmark(_ index: UInt32, description: String) {
        guard let explorer else { return }
        do {
            try explorer.editBookmark(index: index, description: description)
            bookmarkError = nil
        } catch {
            bookmarkError = describe(error)
        }
        reloadBookmarks()
    }

    /// Looks up names for any identity or currency IDs not yet known. Other strings are ignored.
    func resolveNames(_ ids: [String?]) async {
        guard let explorer else { return }
        let wanted = Set(ids.compactMap { $0 }).filter {
            $0.hasPrefix("i") && $0.count == 34 && names[$0] == nil && !nameRequests.contains($0)
        }
        guard !wanted.isEmpty else { return }
        nameRequests.formUnion(wanted)
        let generation = generation
        let found = await explorer.names(ids: Array(wanted))
        guard generation == self.generation else { return }
        for entry in found {
            names[entry.id] = entry.name
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

    // MARK: Requests from one page to another

    /// The block the Charts page opens at. Nil opens the latest range.
    private(set) var chartAnchor: UInt64?
    /// The hash of that block, when it is not on the active chain. The chart then follows
    /// the side branch from it.
    private(set) var chartAnchorHash: String?

    /// Opens the Charts page with `height` in view. Pass `hash` for a block on a side branch.
    func showCharts(at height: UInt64?, hash: String? = nil) {
        chartAnchor = height
        chartAnchorHash = hash
        show(.charts)
    }

    /// A transfer trace that the transaction page starts when it opens.
    struct TraceRequest: Equatable {
        let txid: String
        let output: UInt32
    }

    /// Set by `openTrace`. The transaction page reads it once, then sets it to nil.
    var pendingTrace: TraceRequest?

    /// Opens a transaction and starts the transfer trace at one of its outputs.
    func openTrace(txid: String, output: UInt32) {
        pendingTrace = TraceRequest(txid: txid, output: output)
        open(.transaction(txid: txid))
    }

    /// A currency state that the currency page shows when it opens.
    struct CurrencyStateRequest: Equatable {
        let currency: String
        let height: UInt64
    }

    /// Set by `openCurrency(_:at:)`. The currency page reads it once, then sets it to nil.
    var pendingCurrencyState: CurrencyStateRequest?

    /// Opens a currency with its state at `height` in the exact-state card.
    func openCurrency(_ name: String, at height: UInt64) {
        pendingCurrencyState = CurrencyStateRequest(currency: name, height: height)
        open(.currency(name: name))
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
        searchRun += 1
        let run = searchRun
        do {
            let route = try await explorer.resolve(query: query)
            // The user stopped this search, or started a different one.
            guard run == searchRun else { return }
            isSearching = false
            searchText = ""
            recordSearch(query)
            // A search starts a new trail, as in the terminal app.
            path = [route]
            remember(route)
        } catch {
            guard run == searchRun else { return }
            isSearching = false
            searchError = describe(error)
        }
    }

    /// Stops the search that runs. The app does not use its answer.
    func cancelSearch() {
        searchRun += 1
        isSearching = false
    }

    /// The searches that found something, newest first. The app keeps 30.
    private(set) var searchHistory: [String] = UserDefaults.standard.stringArray(forKey: "searchHistory") ?? []

    private func recordSearch(_ query: String) {
        searchHistory.removeAll { $0 == query }
        searchHistory.insert(query, at: 0)
        searchHistory = Array(searchHistory.prefix(30))
        UserDefaults.standard.set(searchHistory, forKey: "searchHistory")
    }

    func open(_ route: Route) {
        if path.last != route {
            path.append(route)
        }
        remember(route)
    }

    /// Replaces the open page with its neighbour, such as the next block. Back then goes
    /// to the page before the first of them, as in the terminal.
    func step(to route: Route) {
        if path.isEmpty {
            path.append(route)
        } else {
            path[path.count - 1] = route
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
    var title: String {
        switch self {
        case .block(let id):
            return id.count > 12 ? "Block \(Format.short(id, head: 6, tail: 6))" : "Block \(id)"
        case .transaction(let txid): return "Tx \(Format.short(txid, head: 6, tail: 6))"
        case .identity(let name): return Format.short(name, head: 12, tail: 6)
        case .currency(let name): return Format.short(name, head: 12, tail: 6)
        case .address(let address): return Format.short(address, head: 8, tail: 6)
        case .currencies: return "Currencies"
        case .estimate(let query):
            return query.hasPrefix("unix:") ? "Estimate for a date" : "Estimate for block \(query)"
        }
    }

    var symbol: String {
        switch self {
        case .block: return "cube"
        case .transaction: return "arrow.left.arrow.right"
        case .identity: return "person.crop.circle"
        case .currency: return "circle.hexagongrid"
        case .address: return "creditcard"
        case .currencies: return "circle.hexagongrid"
        case .estimate: return "calendar.badge.clock"
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
