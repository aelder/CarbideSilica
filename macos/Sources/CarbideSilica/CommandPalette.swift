import CarbideCore
import SwiftUI

/// One line of the command list: a command or a place, with the key that does the same.
struct PaletteItem: Identifiable {
    let id: Int
    let title: String
    /// The group of the line, shown in small text: "This page", "Go", "Display".
    let group: String
    /// The key that does the same, or an empty text.
    let key: String
    let run: () -> Void
}

/// The command list (Command-K). It has each command of the app and each card of the open
/// page. Type some letters, select with the arrow keys, press Return. Each line shows its
/// key, so the list also teaches the keys.
struct CommandPalette: View {
    @Environment(AppModel.self) private var model
    @Environment(ShellState.self) private var shell
    @Environment(\.openWindow) private var openWindow
    @Environment(\.openSettings) private var openSettings
    @State private var query = ""
    @State private var selection = 0
    @State private var items: [PaletteItem] = []
    @FocusState private var focused: Bool

    private var shown: [PaletteItem] {
        let words = query.lowercased().split(separator: " ")
        guard !words.isEmpty else { return items }
        return items.filter { item in
            let text = (item.title + " " + item.group).lowercased()
            return words.allSatisfy { text.contains($0) }
        }
    }

    var body: some View {
        let rows = shown
        ZStack(alignment: .top) {
            Color.black.opacity(0.18)
                .ignoresSafeArea()
                .onTapGesture { close() }
            VStack(spacing: 0) {
                TextField("Go to or do", text: $query, prompt: Text("Type a command, a place or a card of this page"))
                    .textFieldStyle(.plain)
                    .font(.title3)
                    .padding(12)
                    .focused($focused)
                    .onSubmit { run(rows) }
                    .onExitCommand { close() }
                    .onKeyPress(.downArrow) { move(1, rows.count) }
                    .onKeyPress(.upArrow) { move(-1, rows.count) }
                    .onChange(of: query) { selection = 0 }
                Divider()
                ScrollViewReader { proxy in
                    ScrollView {
                        LazyVStack(spacing: 0) {
                            ForEach(Array(rows.enumerated()), id: \.element.id) { position, item in
                                line(item, selected: position == selection)
                                    .id(item.id)
                                    .onTapGesture {
                                        selection = position
                                        run(rows)
                                    }
                            }
                            if rows.isEmpty {
                                Text("No command has these letters.").foregroundStyle(.secondary).padding(14)
                            }
                        }
                        .padding(.vertical, 4)
                    }
                    .frame(maxHeight: 380)
                    .onChange(of: selection) { _, now in
                        if rows.indices.contains(now) { proxy.scrollTo(rows[now].id) }
                    }
                }
            }
            .frame(width: 560)
            .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(.separator))
            .shadow(radius: 18, y: 6)
            .padding(.top, 70)
        }
        .onAppear {
            items = build()
            focused = true
        }
    }

    private func line(_ item: PaletteItem, selected: Bool) -> some View {
        HStack(spacing: 10) {
            Text(item.title)
            Text(item.group).font(.caption).foregroundStyle(.secondary)
            Spacer()
            if !item.key.isEmpty {
                Text(item.key).font(.callout.monospaced()).foregroundStyle(.secondary)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 6)
        .background(selected ? Color.accentColor.opacity(0.22) : .clear, in: RoundedRectangle(cornerRadius: 5))
        .padding(.horizontal, 4)
        .contentShape(Rectangle())
    }

    private func move(_ step: Int, _ count: Int) -> KeyPress.Result {
        guard count > 0 else { return .handled }
        selection = min(max(selection + step, 0), count - 1)
        return .handled
    }

    private func run(_ rows: [PaletteItem]) {
        guard rows.indices.contains(selection) else { return }
        let action = rows[selection].run
        close()
        action()
    }

    private func close() {
        shell.paletteShown = false
    }

    // MARK: The lines

    private func build() -> [PaletteItem] {
        var lines: [(String, String, String, () -> Void)] = []
        let cursor = shell.cursor
        let page = "This page"

        // The cards of the open page.
        for card in cursor?.cards ?? [] {
            lines.append(("Go to \(card)", page, "", { cursor?.scroll(toCard: card) }))
        }
        if let toggle = cursor?.toggleJSON {
            lines.append(("Show or hide the raw JSON", page, "Tab", toggle))
        }
        if let save = cursor?.saveJSON {
            lines.append(("Save the raw JSON", page, "s", save))
        }
        // The functions of the open page.
        if let adjacent = cursor?.adjacent {
            if let next = adjacent.next { lines.append(("Next \(adjacent.noun)", page, "n", next)) }
            if let previous = adjacent.previous { lines.append(("Previous \(adjacent.noun)", page, "p", previous)) }
        }
        if let found = cursor?.investigations {
            if let run = found.followTransfer { lines.append(("Follow Transfer", page, "F8", run)) }
            if let run = found.witnesses { lines.append(("Witnesses", page, "F4", run)) }
            if let run = found.protocolLinks { lines.append(("Protocol Links", page, "F3", run)) }
            if let run = found.inclusions { lines.append(("Block History", page, "F7", run)) }
            if let run = found.sharedAuthorities { lines.append(("Shared Keys and Authorities", page, "v", run)) }
        }
        for (key, run) in (cursor?.actions ?? [:]).sorted(by: { $0.key < $1.key }) {
            lines.append((Self.pageKeyNames[key] ?? "Page key \(key)", page, key, run))
        }
        if let place = cursor?.place {
            if let run = place.addNode { lines.append(("Add Node", page, "⌥⌘N", run)) }
            if let run = place.pingAll { lines.append(("Ping All Peers", page, "⌥⌘P", run)) }
            if let run = place.clearBans { lines.append(("Clear Ban List", page, "", run)) }
        }
        lines.append((
            "Read the page again", page, "r",
            {
                if let run = cursor?.place?.refresh { run() } else { Task { await model.refresh() } }
            }
        ))
        if model.path.last?.isBookmarkable == true {
            lines.append(("Bookmark This Page", page, "⇧B", { shell.bookmarkRequests += 1 }))
        }

        // Places and history.
        let letters: [SidebarItem: String] = [
            .dashboard: "d", .mempool: "m", .charts: "F9", .currencies: "c", .marketplace: "o",
            .bookmarks: "F6", .node: "`", .sources: "F2",
        ]
        for place in SidebarPlace.all {
            lines.append((place.title, "Go", letters[place.item] ?? "", { model.show(place.item) }))
        }
        lines.append(("Back", "Go", "Esc", { shell.goBack(model) }))
        if !shell.forward.isEmpty {
            lines.append(("Forward", "Go", "⌘]", { shell.goForward(model) }))
        }
        lines.append(("Search", "Go", "/", { model.searchFocusRequests += 1 }))
        lines.append(("Estimate a Future Block", "Go", "F10", { shell.estimateRequests += 1 }))
        for route in model.recent.prefix(8) {
            lines.append(("Open \(route.title)", "Recent", "", { model.open(route) }))
        }

        // Display.
        lines.append(("Show Names or IDs", "Display", "⇧I", { Self.toggle(DisplayKey.showsNames, true) }))
        lines.append(("Show Dates or Heights", "Display", "⇧D", { Self.toggle(DisplayKey.showsDates, false) }))
        lines.append(("Show Times in UTC or Local Time", "Display", "⌥⌘U", { Self.toggle(SettingsKey.useUTC, false) }))

        // Chains and the app.
        for chain in model.knownChainList where chain.chain != model.currentChain {
            lines.append(("Change the Chain to \(chain.chain)", "Chain", "", { model.switchChain(to: chain) }))
        }
        lines.append(("Keyboard Shortcuts", "App", "?", { openWindow(id: KeyboardReference.windowID) }))
        lines.append(("Settings", "App", "⌘,", { openSettings() }))

        return lines.enumerated().map { PaletteItem(id: $0.offset, title: $0.element.0, group: $0.element.1, key: $0.element.2, run: $0.element.3) }
    }

    private static let pageKeyNames: [String: String] = [
        "b": "Open the Block of this Transaction",
        "a": "Open the Active Block at this Height",
        "g": "State at a Chosen Block",
        "n": "Go to Notarizations",
    ]

    private static func toggle(_ key: String, _ initial: Bool) {
        let defaults = UserDefaults.standard
        defaults.set(!(defaults.object(forKey: key) as? Bool ?? initial), forKey: key)
    }
}
