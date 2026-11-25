import AppKit
import CarbideCore
import SwiftUI

@main
struct CarbideSilicaApp: App {
    @State private var model = AppModel()
    @State private var shell = ShellState()

    init() {
        // Lets the app take focus when run outside a bundle (`swift run`).
        NSApplication.shared.setActivationPolicy(.regular)
    }

    var body: some Scene {
        Window("Carbide Silica", id: "main") {
            RootView()
                .environment(model)
                .environment(shell)
                .frame(minWidth: 980, minHeight: 560)
        }
        .defaultSize(width: 1280, height: 820)
        .commands {
            // Menus and shortcuts for every function: see ShellCommands.swift.
            ShellCommands(model: model, shell: shell)
        }

        Window("Keyboard Shortcuts", id: KeyboardReference.windowID) {
            KeyboardReference()
                .environment(model)
        }
        .defaultSize(width: 720, height: 720)

        Settings {
            SettingsView()
                .environment(model)
        }
    }
}

/// The window's content: a navigation stack rooted at the dashboard, with search in the toolbar.
struct RootView: View {
    @Environment(AppModel.self) private var model
    @Environment(ShellState.self) private var shell
    @State private var bookmarking: Route?
    @State private var bookmarkNote = ""
    @State private var estimating = false
    @State private var estimateText = ""
    @State private var showsArgumentProblem = Launch.arguments.problem != nil
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        @Bindable var model = model
        NavigationSplitView {
            Sidebar()
                .navigationSplitViewColumnWidth(min: 180, ideal: 210, max: 280)
        } detail: {
            NavigationStack(path: $model.path) {
                Group {
                    switch model.sidebar {
                    case .dashboard: DashboardView()
                    case .mempool: MempoolView()
                    case .charts:
                        ChartsView(anchorHeight: model.chartAnchor, anchorHash: model.chartAnchorHash)
                            .id("\(model.chartAnchor ?? 0) \(model.chartAnchorHash ?? "")")
                    case .currencies: CurrenciesView()
                    case .marketplace: MarketplaceView()
                    case .bookmarks: BookmarksView()
                    case .node: NodeView()
                    case .sources: SourcesView()
                    }
                }
                // Each page carries the path bar itself: an inset on the stack shows only on
                // the root page.
                .safeAreaInset(edge: .top, spacing: 0) { Breadcrumb() }
                .navigationDestination(for: Route.self) { route in
                    destination(route)
                        .safeAreaInset(edge: .top, spacing: 0) { Breadcrumb() }
                }
            }
        }
        .background(MainWindowReader())
        .onAppear {
            KeyLayer.shared.install(model: model, shell: shell)
            KeyLayer.shared.showKeys = { openWindow(id: KeyboardReference.windowID) }
        }
        .alert("Start-up arguments", isPresented: $showsArgumentProblem) {
            Button("OK", role: .cancel) {}
        } message: {
            Text("The app did not use the start-up arguments. \(Launch.arguments.problem ?? "")")
        }
        .onChange(of: model.path) { old, new in shell.pathChanged(from: old, to: new) }
        .onChange(of: model.sidebar) { shell.clearForward() }
        .onChange(of: shell.bookmarkRequests) {
            guard let current = model.path.last, current.isBookmarkable else { return }
            bookmarkNote = ""
            bookmarking = current
        }
        .onChange(of: shell.estimateRequests) {
            estimateText = ""
            estimating = true
        }
        .alert("Estimate a future block", isPresented: $estimating) {
            TextField("Height or date", text: $estimateText)
            Button("Estimate") {
                let target = estimateText.trimmingCharacters(in: .whitespacesAndNewlines)
                guard !target.isEmpty else { return }
                model.searchText = "estimate:" + target
                Task { await model.search() }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Enter a future block height such as 5000000, or a date such as 2028-11-08 or in 2 hours.")
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                SearchField()
            }
            ToolbarItem(placement: .primaryAction) {
                if let current = model.path.last, current.isBookmarkable {
                    Button {
                        bookmarkNote = ""
                        bookmarking = current
                    } label: {
                        Label("Bookmark", systemImage: "bookmark")
                    }
                    .help("Bookmark this page")
                }
            }
            ToolbarItem(placement: .primaryAction) {
                ConnectionBadge()
            }
        }
        .alert(
            "Bookmark this page",
            isPresented: Binding(get: { bookmarking != nil }, set: { if !$0 { bookmarking = nil } })
        ) {
            TextField("Description (optional)", text: $bookmarkNote)
            Button("Save") {
                if let bookmarking { model.addBookmark(bookmarking, description: bookmarkNote) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("The app saves the bookmark in the Carbide bookmark file. The terminal app uses the same file.")
        }
        .alert(
            "Bookmarks",
            isPresented: Binding(
                get: { model.bookmarkError != nil && model.sidebar != .bookmarks },
                set: { _ in })
        ) {
            Button("OK", role: .cancel) { model.reloadBookmarks() }
        } message: {
            Text(model.bookmarkError ?? "")
        }
        .alert(
            "Search",
            isPresented: Binding(
                get: { model.searchError != nil },
                set: { if !$0 { model.searchError = nil } }
            )
        ) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(model.searchError ?? "")
        }
        .modifier(PermissionPrompt())
    }

    @ViewBuilder
    private func destination(_ route: Route) -> some View {
        switch route {
        case .block(let id): BlockView(id: id)
        case .transaction(let txid): TransactionView(txid: txid)
        case .identity(let name): IdentityView(name: name)
        case .currency(let name): CurrencyView(name: name)
        case .address(let address): AddressView(address: address)
        case .currencies: CurrenciesView()
        case .estimate(let query): EstimateView(query: query)
        }
    }
}

/// Places to go, and the trail of what was opened this session.
struct Sidebar: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        List(selection: selection) {
            Section("Watch") {
                Label("Dashboard", systemImage: "square.grid.2x2").tag(SidebarItem.dashboard)
                Label("Mempool", systemImage: "tray.full")
                    .badge(Int(model.node?.mempoolCount ?? 0))
                    .tag(SidebarItem.mempool)
                Label("Charts", systemImage: "chart.xyaxis.line").tag(SidebarItem.charts)
            }
            Section("Explore") {
                Label("Currencies", systemImage: "circle.hexagongrid").tag(SidebarItem.currencies)
                Label("Marketplace", systemImage: "tag").tag(SidebarItem.marketplace)
            }
            Section("Library") {
                Label("Bookmarks", systemImage: "bookmark")
                    .badge(model.bookmarks.count)
                    .tag(SidebarItem.bookmarks)
            }
            Section("Node") {
                Label("Node", systemImage: "server.rack")
                    .badge(model.syncFraction.map { Text(Format.percent($0)) })
                    .tag(SidebarItem.node)
                Label("Sources", systemImage: "antenna.radiowaves.left.and.right").tag(SidebarItem.sources)
            }
            if !model.recent.isEmpty {
                Section("Recent") {
                    ForEach(model.recent, id: \.self) { route in
                        Button {
                            model.path = [route]
                        } label: {
                            Label(
                                route.crumbTitle(names: showsNames ? model.names : [:]),
                                systemImage: route.symbol
                            )
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .help(route.title)
                    }
                }
            }
        }
        .listStyle(.sidebar)
        // Names for identity and currency IDs in the Recent list.
        .task(id: model.recent) {
            await model.resolveNames(model.recent.map { route -> String? in
                switch route {
                case .identity(let name), .currency(let name): name
                default: nil
                }
            })
        }
    }

    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    /// Nothing is selected while an object page is open on top of a sidebar place.
    private var selection: Binding<SidebarItem?> {
        Binding(
            get: { model.path.isEmpty ? model.sidebar : nil },
            set: { if let item = $0 { model.show(item) } }
        )
    }
}

/// The toolbar's search box. Submitting resolves the text and navigates to the result.
struct SearchField: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var model = model
        HStack(spacing: 6) {
            if model.isSearching {
                ProgressView().controlSize(.small)
            } else {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            }
            TextField(
                "Search", text: $model.searchText,
                prompt: Text("Height, hash, date, address, name@ or currency@@")
            )
            .textFieldStyle(.plain)
            .focused($focused)
            .onSubmit { Task { await model.search() } }
            // Esc cancels the input, as in the terminal.
            .onExitCommand {
                model.searchText = ""
                focused = false
            }
            .disabled(model.isSearching)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .frame(minWidth: 280, idealWidth: 420, maxWidth: 480)
        .background(.quaternary, in: RoundedRectangle(cornerRadius: 7))
        .onChange(of: model.searchFocusRequests) { focused = true }
    }
}

/// Chain, height and where the data is coming from.
struct ConnectionBadge: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        // The badge is also the chain menu: every daemon configured on this Mac.
        Menu {
            Section("Chains on this Mac") {
                ForEach(model.knownChainList, id: \.configFile) { chain in
                    Button {
                        model.switchChain(to: chain)
                    } label: {
                        if chain.chain == model.currentChain {
                            Label(chainLabel(chain), systemImage: "checkmark")
                        } else {
                            Text(chainLabel(chain))
                        }
                    }
                }
            }
            SettingsLink { Text("Connection Settings…") }
        } label: {
            HStack(spacing: 6) {
                Circle()
                    .fill(color)
                    .frame(width: 8, height: 8)
                Text(text)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
        }
        .menuStyle(.borderlessButton)
        .fixedSize()
        .padding(.horizontal, 6)
        .help(model.statusError ?? "Connection status. Click to select a chain.")
    }

    /// PBaaS chains are stored under a hex folder name; show a short form of it.
    private func chainLabel(_ chain: KnownChain) -> String {
        chain.chain.count == 40 ? "PBaaS chain \(chain.chain.prefix(8))…" : chain.chain
    }

    private var color: Color {
        if model.status != nil, model.statusError == nil { return .green }
        return model.statusError == nil ? .gray : .red
    }

    private var text: String {
        guard let status = model.status else {
            return model.statusError == nil ? "Connecting…" : "Not connected"
        }
        let source = status.source == .publicApi ? "Public API" : "Local node"
        let network = status.testnet ? " testnet" : ""
        return "\(status.name)\(network) · \(Format.integer(status.blocks)) · \(source)"
    }
}
