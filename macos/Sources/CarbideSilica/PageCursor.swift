import AppKit
import SwiftUI

// MARK: - The cursor of a page

/// One thing on a page that the keys can select and open: a row of a list or a link.
struct CursorStop {
    let id: String
    /// The card that holds the stop. Tab goes from card to card.
    let group: String
    /// What the stop is a part of, such as "output-3" on a transaction page.
    let context: String?
    /// The value that `y` and Command-C copy.
    let copy: String?
    let open: () -> Void
}

/// The keyboard cursor of one page. Each row and each link of the page is a stop, in the
/// order that the page shows them. The keys move the cursor and open the stop under it, as
/// j, k and Enter do in the terminal.
@MainActor
@Observable
final class PageCursor {
    private(set) var selection: String?
    @ObservationIgnored private(set) var stops: [CursorStop] = []
    @ObservationIgnored private var tops: [String: CGFloat] = [:]
    @ObservationIgnored var proxy: ScrollViewProxy?
    /// How far the page is scrolled. The first move selects the first stop in view.
    @ObservationIgnored var offset: CGFloat = 0
    /// Keys of this page alone, by the character that the key types.
    @ObservationIgnored var actions: [String: () -> Void] = [:]
    /// False for a page that has its own keys, such as the charts.
    @ObservationIgnored var enabled = true
    /// What the page gives to the menu commands. The keys use the same functions.
    @ObservationIgnored var adjacent: AdjacentObjects?
    @ObservationIgnored var investigations: InvestigationActions?
    @ObservationIgnored var place: PlaceActions?
    /// The titles of the cards of the page, from the top down. The command list goes to them.
    @ObservationIgnored var cards: [String] = []
    /// Shows or hides the raw JSON of the page. Nil for a page with no JSON.
    @ObservationIgnored var toggleJSON: (() -> Void)?
    /// Saves the raw JSON, while the page shows it.
    @ObservationIgnored var saveJSON: (() -> Void)?
    /// True while a part of the page has the arrow keys, such as the byte view.
    @ObservationIgnored var suspended = false

    func scroll(toCard title: String) {
        withAnimation { proxy?.scrollTo(Card<EmptyView>.anchor(title), anchor: .top) }
    }

    func scrollToTop() {
        withAnimation { proxy?.scrollTo(PageCursor.topID, anchor: .top) }
    }

    static let topID = "page-top"

    var selected: CursorStop? { stops.first { $0.id == selection } }
    var hasStops: Bool { enabled && !stops.isEmpty }

    func setStops(_ stops: [CursorStop], tops: [String: CGFloat]) {
        self.stops = stops
        self.tops = tops
    }

    func select(_ id: String?, scroll: Bool = false) {
        selection = id
        if scroll, let id { proxy?.scrollTo(id) }
    }

    private var index: Int? { selection.flatMap { id in stops.firstIndex { $0.id == id } } }

    func move(by step: Int) {
        guard !stops.isEmpty else { return }
        if let index {
            go(min(max(index + step, 0), stops.count - 1))
        } else {
            // The first stop in view, or the last stop of the page.
            go(stops.firstIndex { (tops[$0.id] ?? 0) >= offset } ?? stops.count - 1)
        }
    }

    func goToEnd(last: Bool) {
        guard !stops.isEmpty else { return }
        go(last ? stops.count - 1 : 0)
    }

    /// The first stop of the next card, or of the card before.
    func moveGroup(forward: Bool) {
        guard !stops.isEmpty else { return }
        guard let index else { return move(by: 1) }
        let group = stops[index].group
        if forward {
            go(stops[index...].firstIndex { $0.group != group } ?? index)
        } else {
            // The start of this card, or the start of the card before.
            let start = stops[...index].lastIndex { $0.group != group }.map { $0 + 1 } ?? 0
            if start < index { return go(start) }
            guard start > 0 else { return }
            let before = stops[start - 1].group
            go(stops[..<start].lastIndex { $0.group != before }.map { $0 + 1 } ?? 0)
        }
    }

    func openSelected() -> Bool {
        guard let selected else { return false }
        selected.open()
        return true
    }

    private func go(_ index: Int) {
        selection = stops[index].id
        proxy?.scrollTo(stops[index].id)
    }
}

// MARK: - Stops in the view tree

struct StopMark {
    let stop: CursorStop
    /// Changes when the stop opens a different thing with the same identity.
    let token: String
    let bounds: Anchor<CGRect>
}

extension CardCollector {
    fileprivate func ordered(_ geometry: GeometryProxy) -> [String] {
        var placed: [(title: String, rect: CGRect)] = []
        for mark in marks {
            placed.append((mark.title, geometry[mark.bounds]))
        }
        placed.sort { a, b in
            let (rowA, rowB) = (Int(a.rect.minY / 24), Int(b.rect.minY / 24))
            return rowA != rowB ? rowA < rowB : a.rect.minX < b.rect.minX
        }
        var seen = Set<String>()
        return placed.map(\.title).filter { seen.insert($0).inserted }
    }
}

/// The title of a card and where it is on the page.
struct CardMark {
    let title: String
    let bounds: Anchor<CGRect>
}

struct CardMarksKey: PreferenceKey {
    static var defaultValue: [CardMark] { [] }
    static func reduce(value: inout [CardMark], nextValue: () -> [CardMark]) {
        value.append(contentsOf: nextValue())
    }
}

/// Gives the titles of the cards of a page to its cursor, from the top down.
struct CardCollector: View {
    let marks: [CardMark]
    let cursor: PageCursor

    var body: some View {
        GeometryReader { geometry in
            let titles = ordered(geometry)
            Color.clear.onChange(of: titles, initial: true) { cursor.cards = titles }
        }
        .allowsHitTesting(false)
    }
}

struct StopMarksKey: PreferenceKey {
    static var defaultValue: [StopMark] { [] }
    static func reduce(value: inout [StopMark], nextValue: () -> [StopMark]) {
        value.append(contentsOf: nextValue())
    }
}

extension EnvironmentValues {
    @Entry var pageCursor: PageCursor?
    @Entry var stopGroup: String = ""
    @Entry var stopContext: String?
    /// True in a row that is one stop. Links in the row are not stops of their own.
    @Entry var insideStop = false
    @Entry var pageAdjacent: AdjacentObjects?
    @Entry var pageInvestigations: InvestigationActions?
    @Entry var pagePlace: PlaceActions?
    @Entry var pageKeys: [String: () -> Void] = [:]
}

extension View {
    /// Gives a page's functions to the menu commands and to the keys of the page.
    func publishAdjacent(_ value: AdjacentObjects?) -> some View {
        focusedSceneValue(\.adjacentObjects, value).environment(\.pageAdjacent, value)
    }

    func publishInvestigations(_ value: InvestigationActions?) -> some View {
        focusedSceneValue(\.investigations, value).environment(\.pageInvestigations, value)
    }

    /// Keys of this page alone, by the character that the key types.
    func publishKeys(_ keys: [String: () -> Void]) -> some View {
        environment(\.pageKeys, keys)
    }

    func publishPlace(_ value: PlaceActions?) -> some View {
        focusedSceneValue(\.placeActions, value).environment(\.pagePlace, value)
    }
}

/// Makes a view a stop of the page's cursor.
struct CursorStopModifier: ViewModifier {
    /// The identity of a row that has one. A link gets its own.
    var id: String?
    let token: String
    var copy: String?
    /// False when the view has `.id(id)` set by the caller.
    var tags = true
    var radius: CGFloat = 4
    let open: () -> Void

    @State private var own = UUID().uuidString
    @Environment(\.pageCursor) private var cursor
    @Environment(\.stopGroup) private var group
    @Environment(\.stopContext) private var context
    @Environment(\.insideStop) private var inside

    func body(content: Content) -> some View {
        if let cursor, !inside {
            let stopID = id ?? own
            let marked = content
                .environment(\.insideStop, true)
                .background(
                    cursor.selection == stopID ? Color.accentColor.opacity(0.22) : .clear,
                    in: RoundedRectangle(cornerRadius: radius).inset(by: id == nil ? -3 : 0)
                )
                .anchorPreference(key: StopMarksKey.self, value: .bounds) { bounds in
                    [
                        StopMark(
                            stop: CursorStop(id: stopID, group: group, context: context, copy: copy, open: open),
                            token: token, bounds: bounds)
                    ]
                }
            if tags {
                marked.id(stopID)
            } else {
                marked
            }
        } else {
            content
        }
    }
}

extension View {
    /// A link or a button that the keys can select and open.
    func cursorStop(token: String, copy: String? = nil, open: @escaping () -> Void) -> some View {
        modifier(CursorStopModifier(token: token, copy: copy, open: open))
    }
}

/// Reads the stops of a page from its layout and gives them to the cursor in order: card
/// by card, and in a card from the top down and from the left to the right.
struct StopCollector: View {
    let marks: [StopMark]
    let cursor: PageCursor

    private struct Placed: Equatable {
        let id: String
        let token: String
        let row: Int
        let x: Int
    }

    var body: some View {
        GeometryReader { geometry in
            let placed = marks.map { mark -> (StopMark, CGRect) in (mark, geometry[mark.bounds]) }
            let ordered = order(placed)
            let signature = ordered.map { mark, rect in
                Placed(id: mark.stop.id, token: mark.token, row: Int(rect.midY / 8), x: Int(rect.minX))
            }
            Color.clear
                .onChange(of: signature, initial: true) {
                    cursor.setStops(
                        ordered.map(\.0.stop),
                        tops: Dictionary(ordered.map { ($0.0.stop.id, $0.1.minY) }, uniquingKeysWith: { first, _ in first }))
                }
        }
        .allowsHitTesting(false)
    }

    private func order(_ placed: [(StopMark, CGRect)]) -> [(StopMark, CGRect)] {
        // The place of each card is the place of its first stop.
        var origin: [String: (Int, CGFloat)] = [:]
        for (mark, rect) in placed {
            let at = (Int(rect.minY / 24), rect.minX)
            if let known = origin[mark.stop.group], known <= at { continue }
            origin[mark.stop.group] = at
        }
        return placed.sorted { a, b in
            let (ga, gb) = (origin[a.0.stop.group]!, origin[b.0.stop.group]!)
            if a.0.stop.group != b.0.stop.group {
                return ga != gb ? ga < gb : a.0.stop.group < b.0.stop.group
            }
            let (ra, rb) = (Int(a.1.midY / 8), Int(b.1.midY / 8))
            return ra != rb ? ra < rb : a.1.minX < b.1.minX
        }
    }
}

/// Gives the content of a page its cursor. `Page` and `PlacePage` use it.
struct CursorHost: ViewModifier {
    let cursor: PageCursor
    let proxy: ScrollViewProxy
    @Environment(ShellState.self) private var shell
    @Environment(\.pageAdjacent) private var adjacent
    @Environment(\.pageInvestigations) private var investigations
    @Environment(\.pagePlace) private var place
    @Environment(\.pageKeys) private var keys

    func body(content: Content) -> some View {
        // The cursor does not observe these, so the assignment starts no update.
        let _ = adopt()
        content
            .environment(\.pageCursor, cursor)
            .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y } action: { _, offset in
                cursor.offset = offset
            }
            .onAppear {
                cursor.proxy = proxy
                shell.appeared(cursor)
            }
            .onDisappear { shell.disappeared(cursor) }
    }
}

extension CursorHost {
    private func adopt() {
        cursor.adjacent = adjacent
        cursor.investigations = investigations
        cursor.place = place
        cursor.actions = keys
    }
}

// MARK: - The keys of the window

/// The keys that work in the main window when no text field is in use. They are the keys of
/// the terminal: j and k move the cursor, Enter opens, Esc goes back, one letter goes to a
/// place, and the function keys start the investigations.
@MainActor
final class KeyLayer {
    static let shared = KeyLayer()
    weak var window: NSWindow?
    private var monitor: Any?
    private var model: AppModel?
    /// Opens the window that lists the keys.
    var showKeys: (() -> Void)?
    private var shell: ShellState?

    func install(model: AppModel, shell: ShellState) {
        self.model = model
        self.shell = shell
        guard monitor == nil else { return }
        monitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            // AppKit calls the monitor on the main thread.
            nonisolated(unsafe) let event = event
            let consumed = MainActor.assumeIsolated { self?.consumes(event) ?? false }
            return consumed ? nil : event
        }
    }

    private func consumes(_ event: NSEvent) -> Bool { handle(event) == nil }

    /// Returns the event when the key is not one of the app's keys here.
    private func handle(_ event: NSEvent) -> NSEvent? {
        // Esc in a text field: the field does what it does, and then the keyboard leaves
        // it, so that the keys of the page work again.
        if event.keyCode == 53, let window = event.window, window === self.window,
            window.attachedSheet == nil, (window.firstResponder as? NSTextView)?.isEditable == true
        {
            DispatchQueue.main.async {
                if (window.firstResponder as? NSTextView)?.isEditable == true {
                    window.makeFirstResponder(nil)
                }
            }
            return event
        }
        // Command-C with no text selected copies the value under the cursor.
        if event.keyCode == 8, event.modifierFlags.intersection([.command, .control, .option, .shift]) == .command,
            let window = event.window, window === self.window, window.attachedSheet == nil,
            (window.firstResponder as? NSTextView).map({ $0.selectedRange().length == 0 && !$0.isEditable }) ?? true,
            let text = shell?.cursor.flatMap({ $0.enabled ? $0 : nil })?.selected?.copy
        {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            return nil
        }
        guard let model, let shell, let window = event.window, window === self.window,
            window.attachedSheet == nil, !shell.paletteShown,
            event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
            // A field that the user types in keeps its keys. Text that is only selectable does not.
            (window.firstResponder as? NSTextView)?.isEditable != true
        else { return event }
        let cursor = shell.cursor.flatMap { $0.enabled ? $0 : nil }
        let shift = event.modifierFlags.contains(.shift)
        let typed = event.charactersIgnoringModifiers ?? ""
        let done: NSEvent? = nil

        // Keys with no character.
        switch event.keyCode {
        case 53:  // Esc
            return shell.goBack(model) ? done : event
        case 51, 123:  // Backspace, Left
            guard !model.path.isEmpty else { return event }
            _ = shell.goBack(model)
            return done
        case 36, 76, 124:  // Return, Enter, Right
            guard cursor?.suspended != true else { return event }
            return cursor?.openSelected() == true ? done : event
        case 125 where cursor?.suspended == true, 126 where cursor?.suspended == true,
            121 where cursor?.suspended == true, 116 where cursor?.suspended == true,
            115 where cursor?.suspended == true, 119 where cursor?.suspended == true:
            return event
        case 125, 126:  // Down, Up
            guard let cursor, cursor.hasStops else { return event }
            cursor.move(by: event.keyCode == 125 ? 1 : -1)
            return done
        case 121, 116:  // Page Down, Page Up
            guard let cursor, cursor.hasStops else { return event }
            cursor.move(by: event.keyCode == 121 ? 10 : -10)
            return done
        case 115, 119:  // Home, End
            guard let cursor, cursor.hasStops else { return event }
            cursor.goToEnd(last: event.keyCode == 119)
            return done
        case 48:  // Tab: the raw JSON of the page. On a page with no JSON, the next card.
            if let toggle = cursor?.toggleJSON {
                toggle()
                return done
            }
            guard let cursor, cursor.hasStops else { return event }
            cursor.moveGroup(forward: !shift)
            return done
        case 122: showKeys?(); return done  // F1
        case 120: model.show(.sources); return done  // F2
        case 99: return run(cursor?.investigations?.protocolLinks, event)  // F3
        case 118: return run(cursor?.investigations?.witnesses, event)  // F4
        case 96: refresh(cursor, model); return done  // F5
        case 97: model.show(.bookmarks); return done  // F6
        case 98: return run(cursor?.investigations?.inclusions, event)  // F7
        case 100: return run(cursor?.investigations?.followTransfer, event)  // F8
        case 101: model.show(.charts); return done  // F9
        case 109: shell.estimateRequests += 1; return done  // F10
        default: break
        }

        // The keys of the open page come first.
        if let action = cursor?.actions[typed] {
            action()
            return done
        }

        switch typed {
        case "j", "k":
            guard let cursor, cursor.hasStops else { return event }
            cursor.move(by: typed == "j" ? 1 : -1)
            return done
        case "l":
            return cursor?.openSelected() == true ? done : event
        case "h":
            guard !model.path.isEmpty else { return event }
            _ = shell.goBack(model)
            return done
        case "y":
            guard let text = cursor?.selected?.copy else { return event }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            return done
        case "s":
            guard let save = cursor?.saveJSON else { return event }
            save()
            return done
        case "d": model.show(.dashboard); return done
        case "m": model.show(.mempool); return done
        case "o": model.show(.marketplace); return done
        case "c": model.show(.currencies); return done
        case "`": model.show(.node); return done
        case "/": model.searchFocusRequests += 1; return done
        case "?": showKeys?(); return done
        case "n", "]": return run(cursor?.adjacent?.next, event)
        case "p", "[": return run(cursor?.adjacent?.previous, event)
        case "r": refresh(cursor, model); return done
        case "v": return run(cursor?.investigations?.sharedAuthorities, event)
        case "B":
            guard model.path.last?.isBookmarkable == true else { return event }
            shell.bookmarkRequests += 1
            return done
        case "I": toggle(DisplayKey.showsNames, default: true); return done
        case "D": toggle(DisplayKey.showsDates, default: false); return done
        default: return event
        }
    }

    /// Does a function of the page, when the page has it.
    private func run(_ action: (() -> Void)?, _ event: NSEvent) -> NSEvent? {
        guard let action else { return event }
        action()
        return nil
    }

    private func refresh(_ cursor: PageCursor?, _ model: AppModel) {
        if let action = cursor?.place?.refresh { return action() }
        Task { await model.refresh() }
    }

    private func toggle(_ key: String, default initial: Bool) {
        let defaults = UserDefaults.standard
        defaults.set(!(defaults.object(forKey: key) as? Bool ?? initial), forKey: key)
    }
}

/// Tells the key layer which window is the main window.
struct MainWindowReader: NSViewRepresentable {
    func makeNSView(context: Context) -> NSView {
        let view = NSView()
        DispatchQueue.main.async { KeyLayer.shared.window = view.window }
        return view
    }

    func updateNSView(_ view: NSView, context: Context) {
        if let window = view.window { KeyLayer.shared.window = window }
    }
}
