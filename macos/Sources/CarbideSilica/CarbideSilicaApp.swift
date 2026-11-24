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
                .frame(minWidth: 820, minHeight: 520)
        }
        .defaultSize(width: 1120, height: 780)
        .commands {
            CommandGroup(after: .textEditing) {
                Button("Find…") { model.searchFocusRequests += 1 }
                    .keyboardShortcut("f", modifiers: .command)
            }
            CommandMenu("Go") {
                Button("Dashboard") { model.path = [] }
                    .keyboardShortcut("d", modifiers: [.command, .shift])
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
        NavigationStack(path: $model.path) {
            DashboardView()
                .navigationDestination(for: Route.self) { route in
                    destination(route)
                }
        }
        .toolbar {
            ToolbarItem(placement: .principal) {
                SearchField()
            }
            ToolbarItem(placement: .navigation) {
                Button {
                    model.path = []
                } label: {
                    Label("Dashboard", systemImage: "square.grid.2x2")
                }
                .help("Dashboard")
                .disabled(model.path.isEmpty)
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
