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
                Entry(keys: "⌘K", action: "The command list: each command and each card of the open page. Type some letters, then Return.", terminal: "none"),
                Entry(keys: "⌘F", action: "Search. In the field, Up and Down go through the earlier searches, and Esc stops a search that runs.", terminal: "/"),
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
                Entry(keys: "⇧⌘D", action: "Show dates for heights, confirmations and timestamps", terminal: "Shift-D"),
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
            "The cursor of a page",
            [
                Entry(keys: "j k, ↑ ↓", action: "Move the cursor through the rows and the links of the page. No click is necessary first.", terminal: "j, k"),
                Entry(keys: "Return, l, →", action: "Open the row or the link under the cursor", terminal: "Enter, l, Right"),
                Entry(keys: "Esc", action: "Go back one page. At a place in the sidebar, go to the dashboard.", terminal: "Esc"),
                Entry(keys: "h, ←, Delete", action: "Go back one page", terminal: "h, Left, Backspace"),
                Entry(keys: "Tab", action: "Show the raw JSON of the page and go to it. Tab again hides it. In the JSON, the cursor moves through its lines, Return follows an ID or opens a node, and s saves the file.", terminal: "Tab, s"),
                Entry(keys: "Tab, ⇧Tab", action: "On a page with no JSON: go to the next card, or to the card before", terminal: "Tab on the dashboard"),
                Entry(keys: "Page Up, Page Down", action: "Move the cursor 10 places", terminal: "PgUp, PgDn"),
                Entry(keys: "Home, End", action: "Go to the first or the last row or link of the page", terminal: "Home, End"),
                Entry(keys: "y, ⌘C", action: "Copy the full value under the cursor. ⌘C copies selected text when text is selected.", terminal: "y in JSON"),
            ]
        ),
        (
            "The keys of the terminal",
            [
                Entry(keys: "d m o c `", action: "Dashboard, mempool, marketplace, currencies, node and peers", terminal: "d m o c `"),
                Entry(keys: "F2 F6 F9", action: "Sources, bookmarks, charts", terminal: "F2 F6 F9"),
                Entry(keys: "/", action: "Search", terminal: "/"),
                Entry(keys: "n or ], p or [", action: "Next or previous block, identity version, or transaction of the same block, in place", terminal: "n ] p ["),
                Entry(keys: "r, F5", action: "Read the page again", terminal: "r, F5"),
                Entry(keys: "b", action: "Transaction: open its block", terminal: "b"),
                Entry(keys: "a", action: "Block on a side branch: open the active block at that height", terminal: "a"),
                Entry(keys: "g", action: "Currency: go to the state at a chosen block and type there", terminal: "g"),
                Entry(keys: "n", action: "Currency: go to the notarizations", terminal: "n"),
                Entry(keys: "v", action: "Identity: shared keys and authorities", terminal: "v"),
                Entry(keys: "F8, F4", action: "Transaction: follow a transfer, or find the signers, for the output under the cursor", terminal: "F8, F4"),
                Entry(keys: "F3, F7", action: "Transaction: protocol links, block history", terminal: "F3, F7"),
                Entry(keys: "F10", action: "Estimate a future block or date", terminal: "F10"),
                Entry(keys: "⇧B, ⇧I, ⇧D", action: "Bookmark this page, names or IDs, dates or heights", terminal: "Shift-B, Shift-I, Shift-D"),
                Entry(keys: "?, F1", action: "This window", terminal: "?, F1"),
            ]
        ),
        (
            "Where the keys do not apply",
            [
                Entry(keys: "In a text field", action: "The keys type text. Esc leaves the field.", terminal: "the same"),
                Entry(keys: "In a dialog", action: "Return and Esc answer the dialog", terminal: "the same"),
                Entry(keys: "Charts", action: "The charts have their own keys. The line below the charts lists them.", terminal: "the same keys"),
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
                            VStack(alignment: .leading, spacing: 8) {
                                PanelRule()
                                Self.heading(section.0)
                            }
                            .gridCellColumns(3)
                            .padding(.top, section.0 == Self.sections.first?.0 ? 0 : 14)
                        }
                        GridRow {
                            NameText("Keys")
                            NameText("Action")
                            NameText("Terminal key")
                        }
                        ForEach(section.1) { entry in
                            GridRow {
                                Self.key(entry.keys)
                                    .gridColumnAlignment(.leading)
                                Text(entry.action).foregroundStyle(Tone.value)
                                Text(entry.terminal).font(.callout.monospaced()).foregroundStyle(Tone.label)
                            }
                        }
                    }
                }
                PanelRule()
                VStack(alignment: .leading, spacing: 6) {
                    Self.heading("Search")
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
                .foregroundStyle(Tone.label)
                PanelRule()
                VStack(alignment: .leading, spacing: 6) {
                    Self.heading("Cache")
                    if let cache {
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            NameText("Location").frame(width: 90, alignment: .leading)
                            Text(cache.path).textSelection(.enabled).font(.callout.monospaced())
                                .foregroundStyle(Tone.value)
                        }
                        HStack(alignment: .firstTextBaseline, spacing: 12) {
                            NameText("Size").frame(width: 90, alignment: .leading)
                            Text(Format.bytes(cache.bytes)).monospacedDigit().foregroundStyle(Tone.value)
                        }
                    } else {
                        Text("Not connected. The cache location is not known.").foregroundStyle(.secondary)
                    }
                }
            }
            .padding(24)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .background(Tone.screen)
        .environment(\.colorScheme, .dark)
        .frame(minWidth: 620, idealWidth: 720, minHeight: 480, idealHeight: 720)
        .task(id: model.generation) { cache = model.explorer?.cacheInfo() }
    }

    /// The heading of a section, as the heading of a card.
    static func heading(_ title: String) -> some View {
        Text(title.uppercased()).font(Tone.heading).tracking(2.2).foregroundStyle(Tone.value)
    }

    /// A key, in an outline. The key keeps its case: "j" and "J" are not the same key.
    static func key(_ keys: String) -> some View {
        Text(keys)
            .font(.callout.monospaced())
            .foregroundStyle(Tone.value)
            .padding(.horizontal, 5)
            .padding(.vertical, 1)
            .overlay(Rectangle().strokeBorder(Tone.label.opacity(0.7), lineWidth: 1))
            .fixedSize()
    }
}
