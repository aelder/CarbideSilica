import AppKit
import CarbideCore
import SwiftUI

@main
struct CarbideSilicaApp: App {
    @State private var model = AppModel()
    @State private var shell = ShellState()

    init() {
        WindowPicture.arm()
        // Lets the app take focus when run outside a bundle (`swift run`).
        NSApplication.shared.setActivationPolicy(.regular)
    }

    var body: some Scene {
        Window("Carbide Silica", id: "main") {
            RootView()
                .environment(model)
                .environment(shell)
                .frame(minWidth: 980, minHeight: 560)
                // The app is a display: black, with its own colours, in each appearance.
                .preferredColorScheme(.dark)
                .tint(Tone.record)
                // Text that sets no colour has the white of the display, not the system's.
                .foregroundStyle(Tone.value)
        }
        .defaultSize(width: 1280, height: 820)
        .commands {
            // Menus and shortcuts for every function: see ShellCommands.swift.
            ShellCommands(model: model, shell: shell)
        }

        Window("Keyboard Shortcuts", id: KeyboardReference.windowID) {
            KeyboardReference()
                .environment(model)
                .preferredColorScheme(.dark)
                .tint(Tone.record)
                // Text that sets no colour has the white of the display, not the system's.
                .foregroundStyle(Tone.value)
        }
        .defaultSize(width: 720, height: 720)

        Settings {
            SettingsView()
                .environment(model)
                .preferredColorScheme(.dark)
                .tint(Tone.record)
                // Text that sets no colour has the white of the display, not the system's.
                .foregroundStyle(Tone.value)
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
                    // A pushed page does not get the styles of the stack, thus it sets them.
                    destination(route)
                        .displayControls()
                        .safeAreaInset(edge: .top, spacing: 0) { Breadcrumb() }
                }
            }
            // A button of a page that sets no style is a key of the display. The buttons of
            // the bar at the top keep the form of the system.
            .displayControls()
            // The page does not set the width of its column. Without this, a page with a
            // wide row made the sidebar narrower, and the sidebar moved between pages.
            .frame(minWidth: 560)
        }
        // Film grain over all of the display.
        .overlay { FilmGrain().ignoresSafeArea() }
        // While the command list is open, the page behind it takes no keys and no clicks.
        .disabled(shell.paletteShown)
        .overlay {
            if shell.paletteShown { CommandPalette() }
        }
        .background(MainWindowReader())
        .onAppear {
            KeyLayer.shared.install(model: model, shell: shell)
            ClickSound.shared.warm()
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
        // The bar at the top is a part of the display. The path line below it names the page.
        .toolbarBackground(Tone.screen, for: .windowToolbar)
        .toolbarBackground(.visible, for: .windowToolbar)
        .toolbar(removing: .title)
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
        // A list, so that the system places the rows below the bar at the top. A scroll view
        // here drew the rows in one place and took the clicks in a different place.
        List {
            Group {
                group("Watch") {
                    place(.dashboard, "Dashboard")
                    place(.mempool, "Mempool", figure: (model.node?.mempoolCount).flatMap { $0 > 0 ? "\($0)" : nil })
                    place(.charts, "Charts")
                }
                group("Explore") {
                    place(.currencies, "Currencies")
                    place(.marketplace, "Marketplace")
                }
                group("Library") {
                    place(.bookmarks, "Bookmarks", figure: model.bookmarks.isEmpty ? nil : "\(model.bookmarks.count)")
                }
                group("Node") {
                    place(.node, "Node", figure: model.syncFraction.map { Format.percent($0) })
                    place(.sources, "Sources")
                }
                if !model.recent.isEmpty {
                    group("Recent") {
                        ForEach(model.recent, id: \.self) { route in
                            recent(route)
                        }
                    }
                }
            }
            .listRowInsets(EdgeInsets())
            .listRowSeparator(.hidden)
            .listRowBackground(Color.clear)
        }
        .listStyle(.plain)
        .scrollContentBackground(.hidden)
        .environment(\.defaultMinListRowHeight, 1)
        // The sidebar is a part of the display: black, with a thin rule at its edge.
        .background(Tone.screen)
        .overlay(alignment: .trailing) { Rectangle().fill(Tone.rule).frame(width: 1) }
        // The record of the mempool continues on each page, so that the Mempool page has
        // the last minutes when a person opens it.
        .task(id: model.generation) {
            while !Task.isCancelled {
                await MempoolFeed.shared.refresh(model)
                try? await Task.sleep(for: MempoolFeed.interval)
            }
        }
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

    /// A group of places, with its name above a thin rule.
    private func group<Content: View>(_ name: String, @ViewBuilder content: () -> Content) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            NameText(name)
                .padding(.horizontal, 16)
                .padding(.top, 4)
                .padding(.bottom, 6)
            Rectangle().fill(Tone.rule).frame(height: 1).padding(.leading, 16).padding(.bottom, 5)
            content()
        }
        // A group is one row of the list, and the list cuts each row at its edge. The room
        // below the last key is in the row, so that the light of a lit key has its place.
        .padding(.top, 2)
        .padding(.bottom, 16)
    }

    /// One place, as a key with a lamp. Nothing is selected while an object page is open on
    /// top of a place.
    private func place(_ item: SidebarItem, _ title: String, figure: String? = nil) -> some View {
        let chosen = model.path.isEmpty && model.sidebar == item
        return LampKey(chosen: chosen) {
            model.show(item)
        } label: { lit in
            HStack(spacing: 6) {
                Text(title.uppercased())
                    .font(Tone.futura(11.5))
                    .tracking(1.6)
                    .foregroundStyle(lit ? LampTone.legend : Tone.value.opacity(0.85))
                Spacer(minLength: 6)
                if let figure {
                    Text(figure).font(Tone.figureSmall).monospacedDigit()
                        .foregroundStyle(lit ? LampTone.legend.opacity(0.8) : Tone.label)
                }
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 3)
        .accessibilityLabel(title)
        .accessibilityAddTraits(chosen ? .isSelected : [])
    }

    /// One object that was open in this session: a smaller key.
    private func recent(_ route: Route) -> some View {
        let chosen = model.path.last == route
        return LampKey(chosen: chosen, height: 24) {
            model.path = [route]
        } label: { lit in
            Text(route.crumbTitle(names: showsNames ? model.names : [:]))
                .font(.system(size: 12))
                .foregroundStyle(lit ? LampTone.legend : Tone.label)
                .lineLimit(1)
                .truncationMode(.middle)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 2)
        .help(route.title)
    }
}

/// The toolbar's search box. Submitting resolves the text and navigates to the result.
struct SearchField: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool
    /// The place in the earlier searches. Nil when the text is the user's own.
    @State private var historyPlace: Int?

    private func recall(_ step: Int) -> KeyPress.Result {
        let history = model.searchHistory
        guard !history.isEmpty else { return .ignored }
        let next = (historyPlace ?? -1) + step
        if next < 0 {
            historyPlace = nil
            model.searchText = ""
        } else {
            historyPlace = min(next, history.count - 1)
            model.searchText = history[min(next, history.count - 1)]
        }
        return .handled
    }

    var body: some View {
        @Bindable var model = model
        HStack(spacing: 6) {
            if model.isSearching {
                ProgressView().controlSize(.small)
            } else {
                Image(systemName: "magnifyingglass").foregroundStyle(Tone.label)
            }
            TextField(
                "Search", text: $model.searchText,
                prompt: Text("Height, hash, date, address, name@ or currency@@")
            )
            .textFieldStyle(.plain)
            .focused($focused)
            .onSubmit {
                historyPlace = nil
                Task { await model.search() }
            }
            // Esc stops a search that runs. If no search runs, it cancels the input, as
            // in the terminal.
            .onExitCommand {
                if model.isSearching {
                    model.cancelSearch()
                } else {
                    model.searchText = ""
                    focused = false
                }
                historyPlace = nil
            }
            // Up and Down go through the earlier searches.
            .onKeyPress(.upArrow) { recall(1) }
            .onKeyPress(.downArrow) { recall(-1) }
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 5)
        .frame(minWidth: 280, idealWidth: 420, maxWidth: 480)
        .background(Tone.screen)
        .overlay(Rectangle().strokeBorder(focused ? Tone.present.opacity(0.7) : Tone.axis, lineWidth: 1))
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
            HStack(spacing: 7) {
                Rectangle()
                    .fill(color)
                    .frame(width: 7, height: 7)
                Text(text.uppercased())
                    .font(Tone.small)
                    .tracking(1.1)
                    .foregroundStyle(Tone.value)
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
        if model.status != nil, model.statusError == nil { return Tone.good }
        return model.statusError == nil ? Tone.label : Tone.alert
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
