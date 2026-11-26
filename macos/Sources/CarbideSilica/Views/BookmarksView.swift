import CarbideCore
import SwiftUI

/// Saved places, kept in Carbide's own bookmark file so the terminal app sees them too.
///
/// Each row shows tags derived from the object in Carbide's cache and the source that
/// observed it. Cached data does not prove that the object is still on the active chain.
struct BookmarksView: View {
    @Environment(AppModel.self) private var model
    @State private var file: BookmarkFile?
    @State private var failure: String?
    @State private var editing: BookmarkEntry?
    @State private var draft = ""
    @State private var removing: BookmarkEntry?
    @State private var adding = false
    @State private var message: String?
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        let _ = showsNames
        Page {
            Card("Bookmarks", trailing: file.map { "\($0.entries.count) saved" }) {
                HStack {
                    Button("Add Bookmark…") { adding = true }
                    Button("Reload") { Task { await reload() } }
                    Spacer()
                }
                if let failure = failure ?? model.bookmarkError {
                    Note(text: failure, warning: true)
                }
                if let message {
                    Text(message).foregroundStyle(.secondary)
                }
                if let file, !file.issues.isEmpty {
                    Note(
                        text: "\(file.issues.count) line\(file.issues.count == 1 ? " is" : "s are") not a valid bookmark. Carbide keeps them unchanged. \(file.issues[0])",
                        warning: true)
                }
                if let file {
                    if file.entries.isEmpty {
                        VStack(spacing: 6) {
                            Text("No bookmarks.").font(.title3)
                            Text("To add a bookmark, open a page and click the bookmark button in the toolbar, or click Add Bookmark.")
                                .foregroundStyle(.secondary)
                        }
                        .frame(maxWidth: .infinity, minHeight: 160)
                    } else {
                        VStack(spacing: 0) {
                            ForEach(file.entries, id: \.index) { entry in
                                if entry.index > 0 { Divider() }
                                row(entry)
                            }
                        }
                    }
                    Text("Bookmark file: \(file.path)").font(.caption).foregroundStyle(.secondary)
                        .textSelection(.enabled)
                } else if failure == nil {
                    ProgressView().frame(maxWidth: .infinity, minHeight: 120)
                }
            }
            if let file {
                startupCard(file)
            }
        }
        .navigationTitle("Bookmarks")
        .publishPlace(PlaceActions(refresh: { Task { await reload() } }))
        .task(id: model.generation) {
            await reload()
            await BookmarkStartup.runOnce(model: model)
            await reload()
        }
        .alert("Edit description", isPresented: Binding(get: { editing != nil }, set: { if !$0 { editing = nil } })) {
            TextField("Description", text: $draft)
            Button("Save") {
                if let editing {
                    model.editBookmark(editing.index, description: draft)
                    Task { await reload() }
                }
            }
            Button("Cancel", role: .cancel) {}
        }
        .confirmationDialog(
            "Remove this bookmark?",
            isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } }),
            presenting: removing
        ) { entry in
            Button("Remove", role: .destructive) {
                model.removeBookmark(entry.index)
                Task { await reload() }
            }
        } message: { entry in
            Text("The app removes \(entry.target) from the Carbide bookmark file. The terminal app uses the same file.")
        }
        .sheet(isPresented: $adding) {
            AddBookmarkSheet(defaultSystem: model.status?.chainId ?? "") { saved in
                adding = false
                if saved {
                    model.reloadBookmarks()
                    Task { await reload() }
                }
            }
            .environment(model)
        }
    }

    // MARK: Rows

    private func row(_ entry: BookmarkEntry) -> some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: Self.route(for: entry)?.symbol ?? (entry.kind == "notarizations" ? "link" : "bookmark"))
                .foregroundStyle(Tone.label).frame(width: 20)
            VStack(alignment: .leading, spacing: 3) {
                HStack {
                    Text(entry.description.isEmpty ? Self.kindWord(entry.kind) : entry.description)
                        .fontWeight(.semibold)
                    Spacer()
                    NameText(system(entry)).help(entry.systemId)
                }
                HStack(spacing: 8) {
                    Plate(entry.kind)
                    Button(targetText(entry)) { open(entry) }
                        .buttonStyle(.displayLink)
                        .cursorStop(token: "\(entry.index)", copy: targetText(entry)) { open(entry) }
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(entry.target)
                }
                cacheLine(entry)
                Text("Created \(Format.date(entry.createdAt))").font(.caption).foregroundStyle(.secondary)
            }
            VStack(spacing: 4) {
                Button("Edit") {
                    draft = entry.description
                    editing = entry
                }
                Button("Remove", role: .destructive) { removing = entry }
            }
        }
        .padding(.vertical, 8)
    }

    @ViewBuilder
    private func cacheLine(_ entry: BookmarkEntry) -> some View {
        switch entry.cache {
        case .cached(let source, let tags):
            HStack(spacing: 6) {
                Label("Cached observation", systemImage: "tray.full")
                    .font(.caption.weight(.semibold)).foregroundStyle(Tone.good)
                ForEach(tags, id: \.self) { TagChip(text: $0) }
                Text("from \(observationSource(source))").font(.caption).foregroundStyle(Tone.label).help(source)
            }
        case .uncached:
            Label("Uncached", systemImage: "tray")
                .font(.caption.weight(.semibold)).foregroundStyle(Tone.label)
        case .failed(let message):
            Label("Unavailable: \(message)", systemImage: "exclamationmark.triangle")
                .font(.caption).foregroundStyle(Tone.mined)
                .lineLimit(2)
        }
    }

    private func system(_ entry: BookmarkEntry) -> String {
        let name = knownName(entry.systemId, model: model, fallback: entry.systemName)
        return entry.thisChain ? name : "\(name), another chain"
    }

    private func targetText(_ entry: BookmarkEntry) -> String {
        if entry.kind == "notarizations" {
            return "Notarizations of \(knownName(entry.target, model: model, fallback: entry.target))"
        }
        let name = showsNames ? model.names[entry.target] : nil
        return name ?? Format.short(entry.target, head: 16, tail: 10)
    }

    // MARK: Start-up population

    private func startupCard(_ file: BookmarkFile) -> some View {
        Card("Start-up population") {
            if file.startupEnabled {
                Text("On. When the app starts, it requests up to \(file.startupLimit) uncached bookmarks, one at a time, with a 70-second limit for each. It uses only configured sources and does not ask for permission. Failures stay visible in the list.")
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                Text("Off. This is the default. To turn it on, add these lines to Carbide's config.toml. The terminal app reads the same setting.")
                    .fixedSize(horizontal: false, vertical: true)
                Text("[bookmarks]\nfetch_missing_on_startup = true\nstartup_limit = 20")
                    .font(.callout.monospaced())
                    .textSelection(.enabled)
                    .foregroundStyle(Tone.value)
                    .padding(8)
                    .background(Tone.screen, in: Rectangle())
                    .overlay(Rectangle().strokeBorder(Tone.rule, lineWidth: 1))
            }
            if !file.settingsPath.isEmpty {
                Text("Settings file: \(file.settingsPath)").font(.caption).foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            if let issue = file.settingsIssue {
                Note(text: issue, warning: true)
            }
            if let report = file.startupReport {
                Text(report).foregroundStyle(.secondary)
            }
        }
    }

    // MARK: Actions

    private func reload() async {
        guard let explorer = model.explorer else { return }
        do {
            let found = try await explorer.bookmarkFile()
            file = found
            failure = nil
            await model.resolveNames(found.entries.flatMap { [$0.target, $0.systemId] })
        } catch is CancellationError {
        } catch {
            failure = describe(error)
        }
    }

    static func kindWord(_ kind: String) -> String {
        switch kind {
        case "auto": "Search target"
        case "notarizations": "Notarizations"
        default: kind.capitalized
        }
    }

    static func route(for entry: BookmarkEntry) -> Route? {
        switch entry.kind {
        case "block": .block(id: entry.target)
        case "transaction": .transaction(txid: entry.target)
        case "address": .address(address: entry.target)
        case "identity": .identity(name: entry.target)
        case "currency": .currency(name: entry.target)
        default: nil
        }
    }

    /// A notarization bookmark opens the currency page with the saved perspective. A
    /// bookmark without a kind goes through search, which works out what it names.
    private func open(_ entry: BookmarkEntry) {
        message = nil
        guard let chain = model.status?.chainId else {
            message = "The app is not connected to a node."
            return
        }
        if entry.kind == "notarizations" {
            if entry.systemId == chain {
                NotarizationRequests.shared.request(currency: entry.target, observer: chain)
                model.open(.currency(name: entry.target))
            } else if entry.target == chain {
                // The other chain's view of this chain: open the other chain's currency.
                NotarizationRequests.shared.request(currency: entry.systemId, observer: entry.systemId)
                model.open(.currency(name: entry.systemId))
            } else {
                message = "This bookmark is the \(knownName(entry.systemId, model: model)) view of \(entry.target). Connect to that chain to open it."
            }
            return
        }
        if !entry.thisChain {
            message = "This bookmark belongs to \(knownName(entry.systemId, model: model, fallback: entry.systemName)). Connect to that chain to open it."
            return
        }
        if let route = Self.route(for: entry) {
            model.open(route)
        } else {
            model.searchText = entry.target
            Task { await model.search() }
        }
    }
}

/// Runs Carbide's optional start-up population if nothing has run it yet. It does nothing
/// unless Carbide's `config.toml` turns it on. The bridge runs population once per process,
/// so when the app has already run it after connecting, this call does nothing.
@MainActor
enum BookmarkStartup {
    private static var started = false

    static func runOnce(model: AppModel) async {
        guard !started, let explorer = model.explorer else { return }
        started = true
        _ = try? await explorer.populateBookmarks()
    }
}

/// Manual entry of a bookmark: a system ID, a kind and a target, as in Carbide's `n`.
private struct AddBookmarkSheet: View {
    let defaultSystem: String
    let done: (Bool) -> Void
    @Environment(AppModel.self) private var model
    @State private var system = ""
    @State private var kind = "auto"
    @State private var target = ""
    @State private var description = ""
    @State private var failure: String?

    private let kinds = ["auto", "block", "transaction", "address", "identity", "currency", "notarizations"]

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Add a bookmark").font(.title2.weight(.semibold))
            Form {
                TextField("System ID", text: $system, prompt: Text("The chain that observes the object"))
                Picker("Kind", selection: $kind) {
                    ForEach(kinds, id: \.self) { Text(BookmarksView.kindWord($0)).tag($0) }
                }
                TextField("Target", text: $target, prompt: Text(prompt))
                TextField("Description", text: $description, prompt: Text("Optional"))
            }
            Text(help).font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let failure {
                Note(text: failure, warning: true)
            }
            HStack {
                Spacer()
                Button("Cancel", role: .cancel) { done(false) }
                    .keyboardShortcut(.cancelAction)
                Button("Save") { save() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(target.trimmingCharacters(in: .whitespaces).isEmpty)
            }
        }
        .padding(20)
        .frame(width: 520)
        .onAppear { if system.isEmpty { system = defaultSystem } }
    }

    private var prompt: String {
        switch kind {
        case "notarizations": "A currency name or ID, such as vETH"
        case "block": "A block height or hash"
        case "transaction": "A transaction ID"
        case "address": "An address"
        case "identity": "An identity, such as alice@"
        case "currency": "A currency name or ID"
        default: "Anything that search accepts"
        }
    }

    private var help: String {
        if kind == "notarizations" {
            return "A notarization bookmark opens the notarization browser for the currency, as seen by the chain with this system ID. A target such as notarizations:vETH with the kind Search target also works."
        }
        return "The system ID is the chain that the object belongs to. The terminal app shares this bookmark file."
    }

    private func save() {
        do {
            try model.requireExplorer().saveBookmark(
                systemId: system, kind: kind, target: target, description: description)
            done(true)
        } catch {
            failure = describe(error)
        }
    }
}
