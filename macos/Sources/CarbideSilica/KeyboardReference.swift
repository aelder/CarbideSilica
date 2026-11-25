import CarbideCore
import SwiftUI

/// Every keyboard shortcut in the app, with the terminal key it replaces, and the cache
/// location and size, as the terminal's Help screen shows them.
///
/// The shortcuts are set in `ShellCommands`. Keep this list in step with it.
struct KeyboardReference: View {
    static let windowID = "keyboard-reference"

    @Environment(AppModel.self) private var model
    @State private var cache: CacheInfo?

    struct Entry: Identifiable {
        let keys: String
        let action: String
        let terminal: String
        var id: String { keys + action }
    }

    static let sections: [(String, [Entry])] = [
        (
            "Places",
            [
                Entry(keys: "⌘1", action: "Dashboard", terminal: "d"),
                Entry(keys: "⌘2", action: "Mempool", terminal: "m"),
                Entry(keys: "⌘3", action: "Charts", terminal: "F9"),
                Entry(keys: "⌘4", action: "Currencies", terminal: "c"),
                Entry(keys: "⌘5", action: "Marketplace", terminal: "o"),
                Entry(keys: "⌘6", action: "Bookmarks", terminal: "F6"),
                Entry(keys: "⌘7", action: "Node and peers", terminal: "`"),
                Entry(keys: "⌘8", action: "Sources", terminal: "F2"),
            ]
        ),
        (
            "Search and history",
            [
                Entry(keys: "⌘F", action: "Search", terminal: "/"),
                Entry(keys: "⌘[", action: "Back. At a place in the sidebar, go to the dashboard.", terminal: "h, Left, Backspace, Esc"),
                Entry(keys: "⌘]", action: "Forward", terminal: "none"),
                Entry(keys: "⌥⌘[", action: "Previous block or identity version", terminal: "[ or p"),
                Entry(keys: "⌥⌘]", action: "Next block or identity version", terminal: "] or n"),
                Entry(keys: "⇧⌘E", action: "Estimate a future block or date", terminal: "F10"),
                Entry(keys: "⌘R", action: "Refresh the visible page", terminal: "F5, r"),
                Entry(keys: "⌘D", action: "Bookmark this page", terminal: "Shift-B"),
            ]
        ),
        (
            "Investigations",
            [
                Entry(keys: "⌃⌘T", action: "Follow a transfer", terminal: "F8"),
                Entry(keys: "⌃⌘W", action: "Witnesses", terminal: "F4"),
                Entry(keys: "⌃⌘L", action: "Protocol links", terminal: "F3"),
                Entry(keys: "⌃⌘B", action: "Block history across side branches", terminal: "F7"),
                Entry(keys: "⌃⌘A", action: "Shared keys and authorities", terminal: "v"),
            ]
        ),
        (
            "Display",
            [
                Entry(keys: "⇧⌘I", action: "Show names or raw IDs for identities and currencies", terminal: "Shift-I"),
                Entry(keys: "⌥⌘D", action: "Show dates for heights, confirmations and timestamps", terminal: "Shift-D"),
                Entry(keys: "⌥⌘U", action: "Show times in UTC or local time", terminal: "t in Help"),
                Entry(keys: "⌥⌘S", action: "Dashboard: stream blocks", terminal: "Home in the block list"),
                Entry(keys: "⌥⌘L", action: "Mempool: tiles or list", terminal: "none"),
                Entry(keys: "⌥⌘G", action: "Peers: world map", terminal: "w"),
            ]
        ),
        (
            "Peers",
            [
                Entry(keys: "⌥⌘N", action: "Add a node", terminal: "a"),
                Entry(keys: "⌥⌘P", action: "Ping all peers", terminal: "p"),
                Entry(keys: "Peers menu", action: "Clear the ban list", terminal: "c"),
                Entry(keys: "Return", action: "Open the selected peer. Disconnect and Ban are in its page.", terminal: "Enter, x, b"),
            ]
        ),
        (
            "Lists and pages",
            [
                Entry(keys: "↑ ↓", action: "Select in a list", terminal: "j, k"),
                Entry(keys: "Return", action: "Open the selection", terminal: "Enter, l, Right"),
                Entry(keys: "Tab, ⇧Tab", action: "Dashboard: switch between recent blocks and recent transactions. Elsewhere: move between controls.", terminal: "Tab, Shift-Tab"),
                Entry(keys: "Home, End", action: "Select the first or last row of a list", terminal: "Home, End"),
                Entry(keys: "⌘C", action: "Copy the selection", terminal: "y"),
                Entry(keys: "⌘/", action: "This window", terminal: "F1, ?"),
                Entry(keys: "⌘Q", action: "Quit", terminal: "q"),
            ]
        ),
    ]

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                // One grid for all sections, so the columns line up down the window.
                Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 16, verticalSpacing: 4) {
                    ForEach(Self.sections, id: \.0) { section in
                        GridRow {
                            Text(section.0).font(.headline)
                                .gridCellColumns(3)
                                .padding(.top, section.0 == Self.sections.first?.0 ? 0 : 14)
                        }
                        GridRow {
                            Text("Keys")
                            Text("Action")
                            Text("Terminal key")
                        }
                        .font(.caption.weight(.medium))
                        .foregroundStyle(.secondary)
                        ForEach(section.1) { entry in
                            GridRow {
                                Text(entry.keys).font(.body.monospaced())
                                    .gridColumnAlignment(.leading)
                                Text(entry.action)
                                Text(entry.terminal).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                Divider()
                VStack(alignment: .leading, spacing: 6) {
                    Text("Search").font(.headline)
                    Text(
                        "Enter a block height, a block hash, a transaction ID, an address, an identity (name@ or an i-address) or a currency (name@@). "
                            + "Enter currencies to open the currency browser."
                    )
                    Text(
                        "Enter a date such as 2026-11-07, time:Nov 7 2026 2:30 pm UTC, or Unix seconds such as 1700000000. "
                            + "A date without a time means midnight UTC. The app opens the nearest block that it finds."
                    )
                    Text(
                        "A number up to the tip is a block height. A larger number at or after the first block's time is Unix seconds. "
                            + "A number between the two is a future height."
                    )
                    Text(
                        "A future height or date opens an estimate. Enter estimate:5000000, estimate:in 2 hours or estimate:unix:1857254400 to open the estimator directly."
                    )
                }
                .foregroundStyle(.secondary)
                Divider()
                VStack(alignment: .leading, spacing: 6) {
                    Text("Cache").font(.headline)
                    if let cache {
                        LabeledContent("Location") {
                            Text(cache.path).textSelection(.enabled).font(.callout.monospaced())
                        }
                        LabeledContent("Size", value: Format.bytes(cache.bytes))
                    } else {
                        Text("Not connected. The cache location is not known.").foregroundStyle(.secondary)
                    }
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .frame(minWidth: 620, idealWidth: 720, minHeight: 480, idealHeight: 720)
        .task(id: model.generation) { cache = model.explorer?.cacheInfo() }
    }
}
