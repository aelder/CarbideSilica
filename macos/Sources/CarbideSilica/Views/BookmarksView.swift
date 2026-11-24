import CarbideCore
import SwiftUI

/// Saved places, kept in Carbide's own bookmark file so the terminal app sees them too.
struct BookmarksView: View {
    @Environment(AppModel.self) private var model
    @State private var editing: BookmarkRow?
    @State private var draft = ""

    var body: some View {
        Page {
            Card("Bookmarks", trailing: model.bookmarks.isEmpty ? nil : "\(model.bookmarks.count) saved") {
                if let failure = model.bookmarkError {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                if model.bookmarks.isEmpty {
                    VStack(spacing: 6) {
                        Text("No bookmarks.").font(.title3)
                        Text("To add a bookmark, open a page and click the bookmark button in the toolbar.")
                            .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 160)
                } else {
                    VStack(spacing: 0) {
                        ForEach(model.bookmarks, id: \.index) { bookmark in
                            if bookmark.index > 0 { Divider() }
                            HStack(spacing: 12) {
                                Image(systemName: Self.route(for: bookmark)?.symbol ?? "bookmark")
                                    .foregroundStyle(.secondary).frame(width: 20)
                                VStack(alignment: .leading, spacing: 2) {
                                    Button(model.names[bookmark.target] ?? Format.short(bookmark.target, head: 16, tail: 10)) {
                                        open(bookmark)
                                    }
                                    .buttonStyle(.link)
                                    if !bookmark.description.isEmpty {
                                        Text(bookmark.description).foregroundStyle(.secondary)
                                    }
                                }
                                Spacer()
                                Text(bookmark.kind.capitalized).font(.callout).foregroundStyle(.secondary)
                                Text(Format.date(bookmark.createdAt)).font(.callout).foregroundStyle(.secondary)
                                Button("Edit") {
                                    draft = bookmark.description
                                    editing = bookmark
                                }
                                Button("Remove", role: .destructive) { model.removeBookmark(bookmark.index) }
                            }
                            .padding(.vertical, 8)
                        }
                    }
                }
            }
        }
        .navigationTitle("Bookmarks")
        .task(id: model.generation) {
            model.reloadBookmarks()
            await model.resolveNames(model.bookmarks.map(\.target))
        }
        .alert("Edit description", isPresented: Binding(get: { editing != nil }, set: { if !$0 { editing = nil } })) {
            TextField("Description", text: $draft)
            Button("Save") {
                if let editing { model.editBookmark(editing.index, description: draft) }
            }
            Button("Cancel", role: .cancel) {}
        }
    }

    static func route(for bookmark: BookmarkRow) -> Route? {
        switch bookmark.kind {
        case "block": .block(id: bookmark.target)
        case "transaction": .transaction(txid: bookmark.target)
        case "address": .address(address: bookmark.target)
        case "identity": .identity(name: bookmark.target)
        case "currency": .currency(name: bookmark.target)
        default: nil
        }
    }

    /// Bookmarks Carbide saved without a kind go through search, which works out what they name.
    private func open(_ bookmark: BookmarkRow) {
        if let route = Self.route(for: bookmark) {
            model.path = [route]
        } else {
            model.searchText = bookmark.target
            Task { await model.search() }
        }
    }
}
