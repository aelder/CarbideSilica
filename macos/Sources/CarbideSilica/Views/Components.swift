import AppKit
import CarbideCore
import SwiftUI

/// Loads a value from the bridge and shows progress, an error with Retry, or the content.
/// Reloads when `key` changes or the app reconnects.
struct LoadView<Value: Sendable, Content: View>: View {
    let key: String
    let load: (Explorer) async throws -> Value
    @ViewBuilder let content: (Value) -> Content

    @Environment(AppModel.self) private var model
    @State private var phase: Phase = .loading
    @State private var attempt = 0

    private enum Phase {
        case loading
        case failed(String)
        case loaded(Value)
    }

    var body: some View {
        Group {
            switch phase {
            case .loading:
                ProgressView()
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .failed(let message):
                ContentUnavailableView {
                    Label("The data did not load", systemImage: "exclamationmark.triangle")
                } description: {
                    Text(message)
                } actions: {
                    Button("Try Again") { attempt += 1 }
                }
            case .loaded(let value):
                content(value)
            }
        }
        .task(id: "\(key)|\(model.generation)|\(attempt)") {
            phase = .loading
            do {
                phase = .loaded(try await load(try model.requireExplorer()))
            } catch is CancellationError {
            } catch {
                phase = .failed(describe(error))
            }
        }
    }
}

/// A scrolling page with a consistent content width and spacing.
struct Page<Content: View>: View {
    /// False for a page that has its own keys, such as the charts.
    var keys = true
    @ViewBuilder let content: Content

    @State private var cursor = PageCursor()

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    content
                }
                .padding(24)
                .frame(maxWidth: 1100, alignment: .leading)
                .overlayPreferenceValue(StopMarksKey.self) { StopCollector(marks: $0, cursor: cursor) }
                .frame(maxWidth: .infinity)
            }
            .modifier(CursorHost(cursor: cursor, proxy: proxy))
            .onAppear { cursor.enabled = keys }
        }
    }
}

/// A titled section of a page, set off by a rule above its heading.
///
/// Sections are not boxed. A fill and a rounded border mark something as a separate
/// object, so they are kept for content that is one, such as a block of raw bytes.
struct Card<Content: View>: View {
    let title: String
    var trailing: String?
    @ViewBuilder let content: Content

    init(_ title: String, trailing: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.trailing = trailing
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Divider()
            HStack(alignment: .firstTextBaseline) {
                Text(title).font(.title3.weight(.semibold))
                Spacer()
                if let trailing {
                    Text(trailing).font(.subheadline).foregroundStyle(.secondary)
                }
            }
            content
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .environment(\.stopGroup, title)
    }
}

/// The heading of a detail page: the title, an optional ID and tags. The window title
/// names the kind of page, so the heading does not repeat it.
struct PageHeader: View {
    let kind: String
    let title: String
    var subtitle: String?
    var tags: [String] = []

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(.title.weight(.semibold))
                .textSelection(.enabled)
                .lineLimit(2)
            if let subtitle {
                Text(subtitle)
                    .font(.callout.monospaced())
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if !tags.isEmpty {
                TagRow(tags: tags)
            }
        }
    }
}

struct TagRow: View {
    let tags: [String]

    var body: some View {
        HStack(spacing: 6) {
            ForEach(tags, id: \.self) { TagChip(text: $0) }
        }
    }
}

struct TagChip: View {
    let text: String
    var color: Color = .accentColor

    var body: some View {
        Text(text)
            .font(.caption.weight(.medium))
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .foregroundStyle(color)
            .background(color.opacity(0.14), in: Capsule())
            .lineLimit(1)
            .fixedSize()
    }
}

/// A label and value line. The value can be selected, copied, and optionally followed.
struct FieldRow: View {
    let label: String
    let value: String
    var route: Route?
    var monospaced = false

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    private func isCurrency(_ route: Route) -> Bool {
        if case .currency = route { return true }
        return false
    }

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text(label)
                .foregroundStyle(.secondary)
                .frame(width: 170, alignment: .leading)
            if let route {
                // A known name is shown before the raw ID; the ID stays one hover or copy away.
                if showsNames, let name = model.names[value] {
                    // A currency's name is its identity's name without the trailing "@".
                    Button(isCurrency(route) && name.hasSuffix("@") ? String(name.dropLast()) : name) {
                        model.open(route)
                    }
                        .buttonStyle(.link)
                        .lineLimit(1)
                        .help(value)
                        .cursorStop(token: "\(route)", copy: value) { model.open(route) }
                    Text(Format.short(value, head: 6, tail: 4))
                        .font(.caption.monospaced())
                        .foregroundStyle(.tertiary)
                } else {
                    Button(value) { model.open(route) }
                        .buttonStyle(.link)
                        .font(monospaced ? .body.monospaced() : .body)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .cursorStop(token: "\(route)", copy: value) { model.open(route) }
                }
            } else {
                Text(value)
                    .font(monospaced ? .body.monospaced() : .body)
                    .textSelection(.enabled)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            Spacer(minLength: 0)
            CopyButton(text: value)
        }
    }
}

/// Shows a row only when the optional value is present.
struct OptionalFieldRow: View {
    let label: String
    let value: String?
    var route: ((String) -> Route)?
    var monospaced = false

    var body: some View {
        if let value {
            FieldRow(label: label, value: value, route: route?(value), monospaced: monospaced)
        }
    }
}

struct CopyButton: View {
    let text: String
    @State private var copied = false

    var body: some View {
        Button {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(text, forType: .string)
            copied = true
            Task {
                try? await Task.sleep(for: .seconds(1.2))
                copied = false
            }
        } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
                .frame(width: 16)
        }
        .buttonStyle(.borderless)
        .foregroundStyle(copied ? AnyShapeStyle(.green) : AnyShapeStyle(.tertiary))
        .help("Copy")
        .accessibilityLabel("Copy \(text)")
    }
}

/// A link-styled button that navigates to a route, showing a shortened hash or address.
struct RouteLink: View {
    let text: String
    let route: Route
    var shorten = true

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    /// A currency's name is its identity's name without the trailing "@".
    private var isCurrency: Bool {
        if case .currency = route { return true }
        return false
    }

    var body: some View {
        Group {
            if showsNames, let name = model.names[text] {
                Button(isCurrency && name.hasSuffix("@") ? String(name.dropLast()) : name) {
                    model.open(route)
                }
                .buttonStyle(.link)
                .lineLimit(1)
                .help(text)
            } else {
                Button(shorten ? Format.short(text) : text) { model.open(route) }
                    .buttonStyle(.link)
                    .font(.body.monospaced())
                    .lineLimit(1)
                    .help(text)
            }
        }
        .cursorStop(token: "\(route)", copy: text) { model.open(route) }
    }
}

/// The daemon's JSON for the current object, collapsed by default. Expanded, it is a tree
/// with links, names and dates; see `JSONTreeView`.
///
/// Names and dates follow the app's display settings. `showNames` and `showDates` override
/// them when set.
struct JSONSection: View {
    let json: String
    var showNames: Bool?
    var showDates: Bool?
    @State private var expanded = false

    var body: some View {
        Card("Raw JSON") {
            DisclosureGroup(isExpanded: $expanded) {
                if expanded {
                    JSONTreeView(json: json, showNames: showNames, showDates: showDates)
                        .padding(.top, 8)
                }
            } label: {
                HStack {
                    Text(expanded ? "Hide" : "Show the data from the node")
                        .foregroundStyle(.secondary)
                    Spacer()
                    CopyButton(text: json)
                }
            }
        }
    }
}

/// A native-coin amount with its ticker.
struct CoinAmount: View {
    let sats: Int64
    @Environment(AppModel.self) private var model

    var body: some View {
        (Text(Format.coins(sats)).monospacedDigit()
            + Text(" \(model.coin)").foregroundStyle(.secondary))
            .lineLimit(1)
    }
}

/// A line of explanation. It carries a warning triangle only when it is a warning, so an
/// icon always means something.
struct Note: View {
    let text: String
    var warning = false

    var body: some View {
        if warning {
            Label(text, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
        } else {
            Text(text)
        }
    }
}
