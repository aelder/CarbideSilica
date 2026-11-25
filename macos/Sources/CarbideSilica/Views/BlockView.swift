import CarbideCore
import SwiftUI

/// A block's header, its place in the chain and its transactions.
///
/// Names and dates follow the app's display settings. `showNames` and `showDates` override
/// them when set.
struct BlockView: View {
    /// A height or a block hash.
    let id: String
    var showNames: Bool?
    var showDates: Bool?

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsDates) private var showsDatesSetting = false
    @State private var reloads = 0

    private var dates: Bool { showDates ?? showsDatesSetting }

    var body: some View {
        LoadView(key: "block:\(id)|\(reloads)") { explorer in
            try await explorer.block(id: id)
        } content: { block in
            page(block)
                .publishKeys(
                    // The terminal's key for the active block at this height.
                    block.onActiveChain || block.activeHash == nil
                        ? [:] : ["a": { if let active = block.activeHash { model.open(.block(id: active)) } }]
                )
                .publishAdjacent(AdjacentObjects(
                        noun: "Block",
                        previous: block.previousHash.map { hash in { model.step(to: .block(id: hash)) } },
                        next: block.nextHash.map { hash in { model.step(to: .block(id: hash)) } }))
        }
        .publishPlace(PlaceActions(refresh: { reloads += 1 }))
        .navigationTitle("Block")
    }

    private func page(_ block: BlockDetail) -> some View {
            Page {
                PageHeader(
                    kind: "Block",
                    title: "#\(Format.integer(block.row.height))",
                    subtitle: block.row.hash,
                    tags: [block.row.staked ? "Staked" : "Mined"] + block.combinedTags
                )
                ProvenanceNote(provenance: block.provenance)
                HStack(spacing: 12) {
                    chainStatus(block)
                    Spacer()
                    Button {
                        model.showCharts(
                            at: block.row.height, hash: block.onActiveChain ? nil : block.row.hash)
                    } label: {
                        Label("Open in Charts", systemImage: "chart.xyaxis.line")
                    }
                    .help("Open Charts at this block.")
                    Button {
                        if let hash = block.previousHash { model.step(to: .block(id: hash)) }
                    } label: {
                        Label("Previous", systemImage: "chevron.left")
                    }
                    .disabled(block.previousHash == nil)
                    Button {
                        if let hash = block.nextHash { model.step(to: .block(id: hash)) }
                    } label: {
                        Label("Next", systemImage: "chevron.right")
                    }
                    .disabled(block.nextHash == nil)
                }
                if !block.onActiveChain, block.confirmations.map({ $0 < 0 }) == true {
                    HStack(spacing: 10) {
                        Note(text: "This block is not on the active chain.", warning: true)
                        if let active = block.activeHash {
                            Button("Open the active block at height \(Format.integer(block.row.height))") {
                                model.open(.block(id: active))
                            }
                            .buttonStyle(.link)
                        }
                    }
                }

                Card("Header") {
                    FieldRow(label: "Hash", value: block.row.hash, monospaced: true)
                    if dates {
                        FieldRow(label: "Height", value: "\(Format.integer(block.row.height)), recorded \(Format.date(block.row.time))")
                    }
                    FieldRow(
                        label: "Time",
                        value: "\(Format.date(block.row.time)) (\(Format.age(block.row.time)))")
                    if let confirmations = block.confirmations {
                        FieldRow(
                            label: "Confirmations",
                            value: dates && confirmations > 0
                                ? HeightDates.shared.confirmations(confirmations, blockTime: block.row.time)
                                : (confirmations >= 0 ? Format.integer(UInt64(confirmations)) : "\(confirmations)"))
                    }
                    FieldRow(label: "Type", value: productionText(block))
                    FieldRow(label: "Size", value: block.row.size.map { "\(Format.integer($0)) bytes" } ?? "Unknown")
                    FieldRow(label: "Transactions", value: "\(block.row.transactionCount)")
                    FieldRow(label: "Detail", value: detail(block))
                    OptionalFieldRow(
                        label: "Previous block", value: block.previousHash,
                        route: { .block(id: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "Next block", value: block.nextHash,
                        route: { .block(id: $0) }, monospaced: true)
                    ForEach(block.fields, id: \.label) { field in
                        FieldRow(label: field.label, value: field.value, monospaced: true)
                    }
                }

                Card("Transactions", trailing: "\(block.transactions.count)") {
                    if !block.combinedTags.isEmpty {
                        TagRow(tags: block.combinedTags)
                        Text("The tags of all transactions in the block, without Coinbase and Stake.")
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    VStack(spacing: 0) {
                        ForEach(Array(block.transactions.enumerated()), id: \.element.txid) {
                            index, transaction in
                            if index > 0 { Divider() }
                            TransactionRowView(transaction: transaction)
                        }
                    }
                }

                BlockBranchesCard(hash: block.row.hash)

                JSONSection(json: block.json, showNames: showNames, showDates: showDates, filename: "block-\(block.row.height).json")
            }
    }

    /// The block's place in the chain, with shape, colour and a word.
    @ViewBuilder private func chainStatus(_ block: BlockDetail) -> some View {
        switch block.chainStatus {
        case "Chain tip": ConfidenceMark(level: .verified, label: "Chain tip")
        case "Active": ConfidenceMark(level: .verified, label: "Active chain")
        case "Orphaned": ConfidenceMark(level: .unknown, label: "Orphaned")
        case let other?: ConfidenceMark(level: .unknown, label: other)
        case nil: ConfidenceMark(level: .unknown, label: "Chain status unknown")
        }
    }

    private func detail(_ block: BlockDetail) -> String {
        let count = block.transactions.count
        if Int(block.bodiesLoaded) >= count {
            return "All \(count) transaction\(count == 1 ? "" : "s") loaded"
        }
        return "\(block.bodiesLoaded) of \(count) transactions loaded. Tags and totals cover the loaded ones."
    }
}

/// A transaction as listed inside a block or an address's activity.
struct TransactionRowView: View {
    let transaction: TransactionRow

    var body: some View {
        HStack(spacing: 12) {
            RouteLink(
                text: transaction.txid, route: .transaction(txid: transaction.txid)
            )
            TagRow(tags: transaction.tags)
            Spacer(minLength: 8)
            if transaction.inputCount + transaction.outputCount > 0 {
                Text("\(transaction.inputCount) in · \(transaction.outputCount) out")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            if let sats = transaction.outputSats {
                CoinAmount(sats: sats)
                    .frame(minWidth: 150, alignment: .trailing)
            }
        }
        .padding(.vertical, 7)
    }
}

/// "Staked" or "Mined", the app's words, with the node's own block type when it differs.
@MainActor
private func productionText(_ block: BlockDetail) -> String {
    let word = block.row.staked ? "Staked" : "Mined"
    let reported = block.production.trimmingCharacters(in: .whitespaces)
    return reported.isEmpty || reported.lowercased() == word.lowercased()
        ? word : "\(word) (the node reports \"\(reported)\")"
}
