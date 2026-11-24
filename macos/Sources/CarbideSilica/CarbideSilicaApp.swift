import AppKit
import CarbideCore
import SwiftUI

@main
struct CarbideSilicaApp: App {
    @State private var model = AppModel()

    init() {
        // Lets the app take focus when run outside a bundle (`swift run`).
        NSApplication.shared.setActivationPolicy(.regular)
    }

    var body: some Scene {
        Window("Carbide Silica", id: "main") {
            RootView()
                .environment(model)
                .frame(minWidth: 980, minHeight: 560)
        }
        .defaultSize(width: 1280, height: 820)
        .commands {
            CommandGroup(after: .textEditing) {
                Button("Find…") { model.searchFocusRequests += 1 }
                    .keyboardShortcut("f", modifiers: .command)
            }
            CommandMenu("Go") {
                Button("Dashboard") { model.show(.dashboard) }
                    .keyboardShortcut("1", modifiers: .command)
                Button("Mempool") { model.show(.mempool) }
                    .keyboardShortcut("2", modifiers: .command)
                Button("Marketplace") { model.show(.marketplace) }
                    .keyboardShortcut("3", modifiers: .command)
                Button("Node") { model.show(.node) }
                    .keyboardShortcut("4", modifiers: .command)
                Button("Back") { _ = model.path.popLast() }
                    .keyboardShortcut("[", modifiers: .command)
                    .disabled(model.path.isEmpty)
                Divider()
                Button("Refresh") { Task { await model.refresh() } }
                    .keyboardShortcut("r", modifiers: .command)
            }
        }

        Settings {
            SettingsView()
                .environment(model)
        }
    }
}

/// The window's content: a navigation stack rooted at the dashboard, with search in the toolbar.
struct RootView: View {
    @Environment(AppModel.self) private var model

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
                    case .marketplace: MarketplaceView()
                    case .node: NodeView()
                    }
                }
                .navigationDestination(for: Route.self) { route in
                    destination(route)
                }
            }
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                SearchField()
            }
            ToolbarItem(placement: .primaryAction) {
                ConnectionBadge()
            }
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
    }

    @ViewBuilder
    private func destination(_ route: Route) -> some View {
        switch route {
        case .block(let id): BlockView(id: id)
        case .transaction(let txid): TransactionView(txid: txid)
        case .identity(let name): IdentityView(name: name)
        case .currency(let name): CurrencyView(name: name)
        case .address(let address): AddressView(address: address)
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
            }
            Section("Explore") {
                Label("Marketplace", systemImage: "tag").tag(SidebarItem.marketplace)
            }
            Section("Node") {
                Label("Node", systemImage: "server.rack")
                    .badge(model.syncFraction.map { Text(Format.percent($0)) })
                    .tag(SidebarItem.node)
            }
            if !model.recent.isEmpty {
                Section("Recent") {
                    ForEach(model.recent, id: \.self) { route in
                        Button {
                            model.path = [route]
                        } label: {
                            Label(route.title, systemImage: route.symbol)
                                .lineLimit(1)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
        .listStyle(.sidebar)
    }

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
        HStack(spacing: 6) {
            Circle()
                .fill(color)
                .frame(width: 8, height: 8)
            Text(text)
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .padding(.horizontal, 6)
        .help(model.statusError ?? "Connection status")
        .accessibilityElement(children: .combine)
    }

    private var color: Color {
        if model.status != nil, model.statusError == nil { return .green }
        return model.statusError == nil ? .gray : .red
    }

    private var text: String {
        guard let status = model.status else {
            return model.statusError == nil ? "Connecting…" : "Not connected"
        }
        let source = status.source == .publicApi ? "Public API" : "Local daemon"
        let network = status.testnet ? " testnet" : ""
        return "\(status.name)\(network) · \(Format.integer(status.blocks)) · \(source)"
    }
}
