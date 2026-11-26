import CarbideCore
import SwiftUI

/// Every block a transaction is known to have been in: the terminal's F7, read from the
/// observations that the cache stored. Each source stays separate.
struct InclusionCard: View {
    let txid: String

    @Environment(AppModel.self) private var model
    @State private var page: UInt32 = 0
    @State private var found: ObservedBlockPage?
    @State private var failure: String?
    @State private var reloads = 0

    var body: some View {
        Card("Block history") {
            if let found {
                summary(found.rows)
                ObservedBlockList(rows: found.rows)
                Pager(page: $page, more: found.more) { reloads += 1 }
            } else if let failure {
                Note(text: failure, warning: true)
            } else {
                ProgressView().controlSize(.small)
            }
            Text(
                "The list shows only what sources reported to this app or to the terminal app. It does not search the chain. An empty list does not prove that no other block contained the transaction. \"Active\" and \"Orphaned\" are the last reports of each source. They are not a prediction of which branch stays."
            )
            .font(.caption).foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            NodeBranchCheck(txid: txid)
        }
        .modifier(ReloadOnRevision(reloads: $reloads))
        .task(id: "\(txid)|\(page)|\(reloads)|\(model.generation)") {
            do {
                found = try await model.requireExplorer().inclusionHistory(txid: txid, page: page)
                failure = nil
            } catch {
                failure = describe(error)
            }
        }
    }

    @ViewBuilder
    private func summary(_ rows: [ObservedBlock]) -> some View {
        let blocks = Set(rows.map(\.blockHash))
        let orphaned = Set(rows.filter { $0.membership == .orphaned }.map(\.blockHash))
        if rows.isEmpty {
            Text("No source has reported a block for this transaction yet.")
                .foregroundStyle(.secondary)
        } else if blocks.count == 1 {
            Text("Sources reported this transaction in one block.")
        } else {
            Note(
                text: "Sources reported this transaction in \(blocks.count) different blocks. \(orphaned.count) of them were last reported off the active chain.",
                warning: true)
        }
    }
}

/// The known children of a block, from stored observations. More than one child means
/// that sources reported more than one branch after this block. For the block page.
struct BlockBranchesCard: View {
    let hash: String

    @Environment(AppModel.self) private var model
    @State private var page: UInt32 = 0
    @State private var found: ObservedBlockPage?
    @State private var failure: String?
    @State private var reloads = 0

    var body: some View {
        Card("Next blocks") {
            if let found {
                let blocks = Set(found.rows.map(\.blockHash))
                if found.rows.isEmpty {
                    Text(
                        "The cache holds no stored block that follows this one. The page header gives the node's next block."
                    )
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                } else if blocks.count > 1 {
                    Note(
                        text: "Sources reported \(blocks.count) different blocks after this one. Each is a branch.",
                        warning: true)
                }
                ObservedBlockList(rows: found.rows)
                Pager(page: $page, more: found.more) { reloads += 1 }
            } else if let failure {
                Note(text: failure, warning: true)
            } else {
                ProgressView().controlSize(.small)
            }
            Text("The list shows only what sources reported. An empty list does not prove that no other branch exists.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .modifier(ReloadOnRevision(reloads: $reloads))
        .task(id: "\(hash)|\(page)|\(reloads)|\(model.generation)") {
            do {
                found = try await model.requireExplorer().blockChildren(parentHash: hash, page: page)
                failure = nil
            } catch {
                failure = describe(error)
            }
        }
    }
}

/// Rows of stored block observations: membership, height, block, source and whether the
/// cache holds the block's body.
private struct ObservedBlockList: View {
    let rows: [ObservedBlock]

    var body: some View {
        if !rows.isEmpty {
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 6) {
                GridRow {
                    NameText("Last report")
                    NameText("Height")
                    NameText("Block")
                    NameText("Source")
                    NameText("Reported")
                    NameText("Stored body")
                }
                ForEach(Array(rows.enumerated()), id: \.offset) { index, row in
                    // Rows of one block are adjacent. Name the block once, then each source.
                    let repeated = index > 0 && rows[index - 1].blockHash == row.blockHash
                    GridRow {
                        MembershipMark(membership: row.membership)
                        if repeated {
                            Text("")
                            Text("Same block, another source").foregroundStyle(.secondary)
                        } else {
                            Text(row.height.map { Format.integer($0) } ?? "Unknown").monospacedDigit()
                            RouteLink(text: row.blockHash, route: .block(id: row.blockHash))
                        }
                        Text(row.source)
                            .lineLimit(1)
                            .truncationMode(.middle)
                            .frame(maxWidth: 220, alignment: .leading)
                            .help(row.sourceId)
                        Text(row.observedAt.map { Format.age($0) } ?? "Unknown")
                            .foregroundStyle(.secondary)
                            .help(row.observedAt.map { Format.date($0) } ?? "")
                        Text(row.cachedBody ? "Yes" : "No")
                            .foregroundStyle(.secondary)
                            .help(
                                row.cachedBody
                                    ? "The block opens without a source."
                                    : "Opening the block needs an enabled source.")
                    }
                }
            }
            if Set(rows.map(\.blockHash)).count < rows.count {
                Text(
                    "Each source that reported a block has its own line. Lines under one block are one block reported by more than one source, not more than one inclusion."
                )
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// Reloads stored history when the background reconciliation commits a change, as the
/// terminal's F7 panel does. It waits on the bridge and sends no request to a source.
struct ReloadOnRevision: ViewModifier {
    @Binding var reloads: Int
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        content.task(id: model.generation) {
            guard let explorer = model.explorer else { return }
            var seen = explorer.observationRevision()
            while !Task.isCancelled {
                let revision = await explorer.waitForRevision(after: seen)
                guard !Task.isCancelled else { return }
                if revision != seen {
                    seen = revision
                    reloads += 1
                }
            }
        }
    }
}

/// Last observed membership as shape, colour and word.
struct MembershipMark: View {
    let membership: Membership

    var body: some View {
        switch membership {
        case .active:
            Label("Active", systemImage: "checkmark.circle.fill").foregroundStyle(Tone.good)
        case .orphaned:
            Label("Orphaned", systemImage: "arrow.triangle.branch").foregroundStyle(Tone.mined)
        case .unverified:
            Label("Unverified", systemImage: "questionmark.circle").foregroundStyle(Tone.label)
                .help("No stored report says whether this block is on the active chain.")
        }
    }
}

/// Pages of 100 observations, and a reload of the stored data.
private struct Pager: View {
    @Binding var page: UInt32
    let more: Bool
    let reload: () -> Void

    var body: some View {
        HStack {
            if page > 0 || more {
                Button("Previous") { page -= 1 }.disabled(page == 0)
                Text("Page \(page + 1)").monospacedDigit().foregroundStyle(.secondary)
                Button("Next") { page += 1 }.disabled(!more)
            }
            Spacer()
            Button("Reload", action: reload)
                .help("Read the stored observations again")
        }
    }
}

/// The previous check: side branches that the node still holds near the transaction's
/// height. It asks the node for its chain tips, which can take several seconds.
private struct NodeBranchCheck: View {
    let txid: String

    @Environment(AppModel.self) private var model
    @State private var found: [Inclusion]?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        if let found {
            let orphaned = found.filter { !$0.active }
            Note(
                text: orphaned.isEmpty
                    ? "No side branch on the node contains the transaction."
                    : "The node holds \(orphaned.count) block\(orphaned.count == 1 ? "" : "s") on a side branch that contain the transaction.",
                warning: !orphaned.isEmpty)
            ForEach(orphaned, id: \.blockHash) { inclusion in
                HStack(spacing: 12) {
                    if let height = inclusion.height {
                        Text("Block \(Format.integer(height))").monospacedDigit()
                    }
                    RouteLink(text: inclusion.blockHash, route: .block(id: inclusion.blockHash))
                    Spacer()
                }
            }
        } else if running {
            ProgressView().controlSize(.small)
        } else {
            if let failure {
                Note(text: failure, warning: true)
            }
            HStack {
                Text("Also ask the node for side branches near this height. The request can take several seconds.")
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Check Node") { Task { await run() } }
            }
        }
    }

    private func run() async {
        running = true
        failure = nil
        defer { running = false }
        do {
            found = try await model.requireExplorer().inclusions(txid: txid)
        } catch {
            failure = describe(error)
        }
    }
}
