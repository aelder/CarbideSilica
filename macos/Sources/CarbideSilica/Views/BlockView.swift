import CarbideCore
import SwiftUI

/// A block's header and its transactions.
struct BlockView: View {
    /// A height or a block hash.
    let id: String

    @Environment(AppModel.self) private var model

    var body: some View {
        LoadView(key: "block:\(id)") { explorer in
            try await explorer.block(id: id)
        } content: { block in
            Page {
                PageHeader(
                    kind: "Block",
                    title: "#\(Format.integer(block.row.height))",
                    subtitle: block.row.hash,
                    tags: [block.row.staked ? "Staked" : "Mined"]
                        + (block.confirmations == 1 ? ["Chain tip"] : [])
                        + ((block.confirmations ?? 1) < 0 ? ["Not on the active chain"] : [])
                )
                HStack {
                    Button {
                        if let hash = block.previousHash { model.open(.block(id: hash)) }
                    } label: {
                        Label("Previous", systemImage: "chevron.left")
                    }
                    .disabled(block.previousHash == nil)
                    Button {
                        if let hash = block.nextHash { model.open(.block(id: hash)) }
                    } label: {
                        Label("Next", systemImage: "chevron.right")
                    }
                    .disabled(block.nextHash == nil)
                }

                Card("Header") {
                    FieldRow(label: "Hash", value: block.row.hash, monospaced: true)
                    FieldRow(
                        label: "Time",
                        value: "\(Format.date(block.row.time)) (\(Format.age(block.row.time)))")
                    if let confirmations = block.confirmations {
                        FieldRow(label: "Confirmations", value: "\(confirmations)")
                    }
                    if let size = block.row.size {
                        FieldRow(label: "Size", value: Format.bytes(size))
                    }
                    FieldRow(
                        label: "Produced", value: block.row.staked ? "Staked" : "Mined")
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
                    VStack(spacing: 0) {
                        ForEach(Array(block.transactions.enumerated()), id: \.element.txid) {
                            index, transaction in
                            if index > 0 { Divider() }
                            TransactionRowView(transaction: transaction)
                        }
                    }
                }

                JSONSection(json: block.json)
            }
        }
        .navigationTitle("Block")
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
