import CarbideCore
import SwiftUI

// MARK: - Shell state

/// State of the window shell that `AppModel` does not hold: the forward history, and
/// requests from menu commands to the root view.
@MainActor
@Observable
final class ShellState {
    /// Pages left by going back, the next one last.
    private(set) var forward: [Route] = []
    /// Incremented to ask the root view to bookmark the current page.
    var bookmarkRequests = 0
    /// Incremented to ask the root view to show the estimator entry.
    var estimateRequests = 0

    /// Keeps the forward history in step with the navigation path, however the path changed:
    /// a menu command, the toolbar's back button, a link or a search.
    func pathChanged(from old: [Route], to new: [Route]) {
        if new.count < old.count, Array(old.prefix(new.count)) == new {
            forward.append(contentsOf: old[new.count...].reversed())
        } else if new.count == old.count + 1, Array(new.prefix(old.count)) == old,
            new.last == forward.last
        {
            forward.removeLast()
        } else if new != old {
            forward.removeAll()
        }
    }

    func clearForward() { forward.removeAll() }

    /// The cursors of the pages that are open, the visible page last.
    @ObservationIgnored private var cursors: [PageCursor] = []
    var cursor: PageCursor? { cursors.last }

    func appeared(_ cursor: PageCursor) {
        cursors.removeAll { $0 === cursor }
        cursors.append(cursor)
    }

    func disappeared(_ cursor: PageCursor) {
        cursors.removeAll { $0 === cursor }
    }

    /// Goes back one page. At a place in the sidebar, goes to the dashboard, as Esc does
    /// in the terminal. Returns false when there is no page to go back to.
    @discardableResult
    func goBack(_ model: AppModel) -> Bool {
        if !model.path.isEmpty {
            _ = model.path.popLast()
        } else if model.sidebar != .dashboard {
            model.show(.dashboard)
        } else {
            return false
        }
        return true
    }

    func goForward(_ model: AppModel) {
        guard let next = forward.last else { return }
        model.path.append(next)
    }
}

// MARK: - Focused values that views publish for menu commands

/// The previous and next object of the same kind, such as the block before this block or the
/// identity version before this one. A view publishes it with
/// `.publishAdjacent(AdjacentObjects(previous: …, next: …))`.
struct AdjacentObjects {
    var noun: String
    var previous: (() -> Void)?
    var next: (() -> Void)?
}

/// The investigations a transaction page offers. Each is `nil` when it does not apply.
struct InvestigationActions {
    /// Follow a transfer (the terminal's F8).
    var followTransfer: (() -> Void)?
    /// Witnesses of notarization evidence (F4).
    var witnesses: (() -> Void)?
    /// References to and from other outputs (F3).
    var protocolLinks: (() -> Void)?
    /// The blocks that hold the transaction, across side branches (F7).
    var inclusions: (() -> Void)?
    /// Earlier identities that shared addresses or authorities (the terminal's `v`).
    var sharedAuthorities: (() -> Void)?
}

/// Commands for the visible place: refresh, and the place's own toggles.
struct PlaceActions {
    var refresh: (() -> Void)?
    /// Dashboard: keep new blocks in view.
    var streamBlocks: Binding<Bool>?
    /// Mempool: tiles or list.
    var mempoolAsList: Binding<Bool>?
    /// Node: peer commands. Each opens the same confirmation as its button.
    var addNode: (() -> Void)?
    var pingAll: (() -> Void)?
    var clearBans: (() -> Void)?
    var showMap: Binding<Bool>?
}

extension FocusedValues {
    @Entry var adjacentObjects: AdjacentObjects?
    @Entry var investigations: InvestigationActions?
    @Entry var placeActions: PlaceActions?
}

// MARK: - Menu commands

/// The app's menu commands. Each function of the terminal has a menu item, and the
/// shortcuts follow macOS conventions. `KeyboardReference` lists the same shortcuts.
struct ShellCommands: Commands {
    let model: AppModel
    let shell: ShellState

    @FocusedValue(\.adjacentObjects) private var adjacent
    @FocusedValue(\.investigations) private var investigations
    @FocusedValue(\.placeActions) private var place
    @Environment(\.openWindow) private var openWindow
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false
    @AppStorage(SettingsKey.useUTC) private var useUTC = false

    var body: some Commands {
        CommandGroup(after: .textEditing) {
            Button("Find…") { model.searchFocusRequests += 1 }
                .keyboardShortcut("f", modifiers: .command)
        }

        CommandGroup(before: .toolbar) {
            Toggle("Show Names for IDs", isOn: $showsNames)
                .keyboardShortcut("i", modifiers: [.command, .shift])
            Toggle("Show Dates for Heights", isOn: $showsDates)
                .keyboardShortcut("d", modifiers: [.command, .option])
            Toggle("Show Times in UTC", isOn: $useUTC)
                .keyboardShortcut("u", modifiers: [.command, .option])
            Divider()
            if let stream = place?.streamBlocks {
                Toggle("Stream Blocks", isOn: stream)
                    .keyboardShortcut("s", modifiers: [.command, .option])
            }
            if let list = place?.mempoolAsList {
                Toggle("Show Mempool as List", isOn: list)
                    .keyboardShortcut("l", modifiers: [.command, .option])
            }
            if let map = place?.showMap {
                Toggle("Show Peer Map", isOn: map)
                    .keyboardShortcut("g", modifiers: [.command, .option])
            }
            Divider()
        }

        CommandMenu("Go") {
            ForEach(SidebarPlace.all, id: \.title) { place in
                Button(place.title) { model.show(place.item) }
                    .keyboardShortcut(place.key, modifiers: .command)
            }
            Divider()
            Button("Back") { shell.goBack(model) }
                .keyboardShortcut("[", modifiers: .command)
                .disabled(model.path.isEmpty && model.sidebar == .dashboard)
            Button("Forward") { shell.goForward(model) }
                .keyboardShortcut("]", modifiers: .command)
                .disabled(shell.forward.isEmpty)
            Divider()
            Button("Previous \(adjacentNoun)") { previous() }
                .keyboardShortcut("[", modifiers: [.command, .option])
                .disabled(!canStep(next: false))
            Button("Next \(adjacentNoun)") { next() }
                .keyboardShortcut("]", modifiers: [.command, .option])
                .disabled(!canStep(next: true))
            Divider()
            Button("Estimate a Future Block…") { shell.estimateRequests += 1 }
                .keyboardShortcut("e", modifiers: [.command, .shift])
            Divider()
            Button("Refresh") { refresh() }
                .keyboardShortcut("r", modifiers: .command)
        }

        CommandMenu("Investigate") {
            Button("Follow Transfer") { investigations?.followTransfer?() }
                .keyboardShortcut("t", modifiers: [.command, .control])
                .disabled(investigations?.followTransfer == nil)
            Button("Witnesses") { investigations?.witnesses?() }
                .keyboardShortcut("w", modifiers: [.command, .control])
                .disabled(investigations?.witnesses == nil)
            Button("Protocol Links") { investigations?.protocolLinks?() }
                .keyboardShortcut("l", modifiers: [.command, .control])
                .disabled(investigations?.protocolLinks == nil)
            Button("Block History") { investigations?.inclusions?() }
                .keyboardShortcut("b", modifiers: [.command, .control])
                .disabled(investigations?.inclusions == nil)
            Button("Shared Keys and Authorities") { investigations?.sharedAuthorities?() }
                .keyboardShortcut("a", modifiers: [.command, .control])
                .disabled(investigations?.sharedAuthorities == nil)
        }

        CommandGroup(after: .newItem) {
            Button("Bookmark This Page…") { shell.bookmarkRequests += 1 }
                .keyboardShortcut("d", modifiers: .command)
                .disabled(!canBookmark)
        }

        CommandMenu("Peers") {
            Button("Add Node…") { place?.addNode?() }
                .keyboardShortcut("n", modifiers: [.command, .option])
                .disabled(place?.addNode == nil)
            Button("Ping All Peers") { place?.pingAll?() }
                .keyboardShortcut("p", modifiers: [.command, .option])
                .disabled(place?.pingAll == nil)
            Button("Clear Ban List…") { place?.clearBans?() }
                .disabled(place?.clearBans == nil)
        }

        CommandGroup(replacing: .help) {
            Button("Keyboard Shortcuts") { openWindow(id: KeyboardReference.windowID) }
                .keyboardShortcut("/", modifiers: .command)
        }
    }

    private var canBookmark: Bool {
        switch model.path.last {
        case .block?, .transaction?, .identity?, .currency?, .address?: true
        default: false
        }
    }

    private var adjacentNoun: String {
        if let adjacent { return adjacent.noun }
        if case .block? = model.path.last { return "Block" }
        return "Block or Version"
    }

    private func canStep(next: Bool) -> Bool {
        if let adjacent { return (next ? adjacent.next : adjacent.previous) != nil }
        if case .block? = model.path.last { return true }
        return false
    }

    private func previous() {
        if let action = adjacent?.previous { return action() }
        stepBlock(next: false)
    }

    private func next() {
        if let action = adjacent?.next { return action() }
        stepBlock(next: true)
    }

    /// Replaces the open block with its neighbour, so Back still returns to the page before.
    private func stepBlock(next: Bool) {
        guard case .block(let id)? = model.path.last, let explorer = model.explorer else { return }
        Task {
            do {
                guard let hash = try await explorer.adjacentBlock(id: id, next: next) else {
                    model.searchError = next ? "This block is the tip of the chain." : "This is the first block."
                    return
                }
                guard model.path.last == .block(id: id) else { return }
                model.path[model.path.count - 1] = .block(id: hash)
            } catch {
                model.searchError = describe(error)
            }
        }
    }

    private func refresh() {
        if let action = place?.refresh { return action() }
        Task { await model.refresh() }
    }
}

/// The places in the sidebar, in order, with their shortcuts.
struct SidebarPlace {
    let title: String
    let item: SidebarItem
    let key: KeyEquivalent

    static let all: [SidebarPlace] = [
        SidebarPlace(title: "Dashboard", item: .dashboard, key: "1"),
        SidebarPlace(title: "Mempool", item: .mempool, key: "2"),
        SidebarPlace(title: "Charts", item: .charts, key: "3"),
        SidebarPlace(title: "Currencies", item: .currencies, key: "4"),
        SidebarPlace(title: "Marketplace", item: .marketplace, key: "5"),
        SidebarPlace(title: "Bookmarks", item: .bookmarks, key: "6"),
        SidebarPlace(title: "Node and Peers", item: .node, key: "7"),
        SidebarPlace(title: "Sources", item: .sources, key: "8"),
    ]
}
