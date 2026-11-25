import CarbideCore
import SwiftUI

/// Scroll positions of the sidebar places, kept for the session so that a visit to another
/// place does not lose them.
@MainActor
final class PlaceMemory {
    static let shared = PlaceMemory()
    var offsets: [String: CGFloat] = [:]
}

/// A sidebar place's page: one scroll view for the whole page, with the same width and
/// spacing as `Page`, that comes back to the same scroll position after a visit elsewhere.
/// `content` gets the scroll proxy, so lists in the page can keep a row in view.
struct PlacePage<Content: View>: View {
    let key: String
    @ViewBuilder let content: (ScrollViewProxy) -> Content

    @State private var position = ScrollPosition(edge: .top)
    @State private var restored = false
    @State private var cursor = PageCursor()

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    content(proxy)
                }
                .padding(24)
                .frame(maxWidth: 1100, alignment: .leading)
                .overlayPreferenceValue(StopMarksKey.self) { StopCollector(marks: $0, cursor: cursor) }
                .overlayPreferenceValue(CardMarksKey.self) { CardCollector(marks: $0, cursor: cursor) }
                .id(PageCursor.topID)
                .frame(maxWidth: .infinity)
            }
            .modifier(CursorHost(cursor: cursor, proxy: proxy))
            .scrollPosition($position)
            .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y } action: { _, offset in
                // Until the saved position is restored, the first offsets are the top.
                if restored { PlaceMemory.shared.offsets[key] = offset }
            }
            .task {
                // The page's data is kept in shared stores, so it is there at once. Wait for
                // one layout pass before the scroll.
                try? await Task.sleep(for: .milliseconds(60))
                if let offset = PlaceMemory.shared.offsets[key], offset > 0 {
                    position.scrollTo(y: offset)
                }
                restored = true
            }
        }
    }
}

/// Keyboard control for a list of rows in a page: Up and Down select, Home and End go to
/// the ends, Return opens. The page scrolls to keep the selected row in view.
struct KeyboardRows: ViewModifier {
    let ids: [String]
    @Binding var selection: String?
    let proxy: ScrollViewProxy?
    let open: (String) -> Void

    func body(content: Content) -> some View {
        content
            .focusable()
            .onKeyPress(.downArrow) { move(by: 1) }
            .onKeyPress(.upArrow) { move(by: -1) }
            .onKeyPress(.home) { select(ids.first) }
            .onKeyPress(.end) { select(ids.last) }
            .onKeyPress(.return) {
                guard let selection else { return .ignored }
                open(selection)
                return .handled
            }
    }

    private func move(by step: Int) -> KeyPress.Result {
        guard !ids.isEmpty else { return .ignored }
        let index = selection.flatMap { ids.firstIndex(of: $0) }
        let next = index.map { min(max($0 + step, 0), ids.count - 1) } ?? 0
        return select(ids[next])
    }

    private func select(_ id: String?) -> KeyPress.Result {
        guard let id else { return .ignored }
        selection = id
        proxy?.scrollTo(id)
        return .handled
    }
}

/// One row of a keyboard list. A click selects it, a double-click opens it. Links in the
/// row still open on one click.
struct SelectableRow<Content: View>: View {
    let id: String
    @Binding var selection: String?
    let open: () -> Void
    @ViewBuilder let content: Content

    @Environment(\.pageCursor) private var cursor

    var body: some View {
        content
            .padding(.horizontal, 8)
            .frame(maxWidth: .infinity, alignment: .leading)
            .modifier(CursorStopModifier(id: id, token: id, copy: id, tags: false, radius: 5, open: open))
            .contentShape(Rectangle())
            .onTapGesture(count: 2) {
                cursor?.select(id)
                open()
            }
            .onTapGesture { cursor?.select(id) }
            .onChange(of: cursor?.selection) { _, now in
                if now == id { selection = id }
            }
            .id(id)
    }
}

extension View {
    func keyboardRows(
        _ ids: [String], selection: Binding<String?>, proxy: ScrollViewProxy?,
        open: @escaping (String) -> Void
    ) -> some View {
        modifier(KeyboardRows(ids: ids, selection: selection, proxy: proxy, open: open))
    }
}

/// The source of a page's data in the status line: the source's name and whether the data
/// is live or a stored observation, as a shape, a colour and a word.
struct SourceLine: View {
    let provenance: Provenance?

    var body: some View {
        if let provenance, !provenance.source.isEmpty {
            HStack(spacing: 5) {
                Image(systemName: provenance.retained ? "clock.arrow.circlepath" : "circle.fill")
                    .font(.system(size: provenance.retained ? 10 : 8))
                    .foregroundStyle(provenance.retained ? .orange : .green)
                Text("Source").foregroundStyle(.secondary)
                Text(provenance.source).fontWeight(.semibold)
                Text(provenance.retained ? "stored, reported \(Format.age(provenance.observedAt))" : "live")
                    .foregroundStyle(provenance.retained ? .orange : .secondary)
            }
            .help(
                provenance.retained
                    ? "No source answered now. This is the latest stored observation."
                    : "The source answered this request now.")
        }
    }
}
