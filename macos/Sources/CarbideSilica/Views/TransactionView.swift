import CarbideCore
import SwiftUI

/// A transaction's summary, inputs and outputs.
struct TransactionView: View {
    let txid: String

    var body: some View {
        LoadView(key: "tx:\(txid)") { explorer in
            try await explorer.transaction(txid: txid)
        } content: { transaction in
            Page {
                PageHeader(
                    kind: "Transaction",
                    title: Format.short(transaction.txid, head: 16, tail: 16),
                    subtitle: transaction.txid,
                    tags: transaction.tags
                )

                Card("Summary") {
                    FieldRow(label: "Transaction ID", value: transaction.txid, monospaced: true)
                    if let height = transaction.height {
                        FieldRow(
                            label: "Block", value: Format.integer(height),
                            route: transaction.blockHash.map { .block(id: $0) }
                                ?? .block(id: "\(height)"))
                    } else {
                        FieldRow(label: "Block", value: "Not yet in a block")
                    }
                    if let time = transaction.time {
                        FieldRow(
                            label: "Time", value: "\(Format.date(time)) (\(Format.age(time)))")
                    }
                    if let confirmations = transaction.confirmations {
                        FieldRow(label: "Confirmations", value: "\(confirmations)")
                    }
                    if let size = transaction.size {
                        FieldRow(label: "Size", value: Format.bytes(size))
                    }
                    if let version = transaction.version {
                        FieldRow(label: "Version", value: "\(version)")
                    }
                    if let expiry = transaction.expiryHeight, expiry > 0 {
                        FieldRow(label: "Expiry height", value: Format.integer(expiry))
                    }
                    if transaction.privateInputs + transaction.privateOutputs > 0 {
                        FieldRow(
                            label: "Shielded",
                            value: "\(transaction.privateInputs) private in · "
                                + "\(transaction.privateOutputs) private out")
                    }
                }

                Card("Inputs", trailing: "\(transaction.inputs.count)") {
                    VStack(spacing: 0) {
                        ForEach(Array(transaction.inputs.enumerated()), id: \.element.index) {
                            position, input in
                            if position > 0 { Divider() }
                            InputRow(input: input)
                        }
                    }
                }

                Card("Outputs", trailing: "\(transaction.outputs.count)") {
                    VStack(spacing: 0) {
                        ForEach(Array(transaction.outputs.enumerated()), id: \.element.index) {
                            position, output in
                            if position > 0 { Divider() }
                            OutputRow(output: output)
                        }
                    }
                }

                JSONSection(json: transaction.json)
            }
        }
        .navigationTitle("Transaction")
    }
}

private struct IndexLabel: View {
    let index: UInt32

    var body: some View {
        Text("\(index)")
            .font(.callout.monospacedDigit())
            .foregroundStyle(.secondary)
            .frame(width: 28, alignment: .trailing)
    }
}

private struct InputRow: View {
    let input: TransactionInput

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            IndexLabel(index: input.index)
            VStack(alignment: .leading, spacing: 4) {
                if input.coinbase {
                    TagChip(text: "Coinbase", color: .orange)
                } else {
                    ForEach(input.addresses, id: \.self) { address in
                        RouteLink(text: address, route: .holder(address), shorten: false)
                    }
                    if let previous = input.previousTxid {
                        HStack(spacing: 4) {
                            Text("spends").foregroundStyle(.secondary)
                            RouteLink(text: previous, route: .transaction(txid: previous))
                            if let output = input.previousOutput {
                                Text("output \(output)").foregroundStyle(.secondary)
                            }
                        }
                        .font(.callout)
                    }
                }
            }
            Spacer(minLength: 8)
            if let sats = input.valueSats {
                CoinAmount(sats: sats)
            }
        }
        .padding(.vertical, 8)
    }
}

private struct OutputRow: View {
    let output: TransactionOutput
    @State private var showingJSON = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                IndexLabel(index: output.index)
                VStack(alignment: .leading, spacing: 4) {
                    if output.addresses.isEmpty {
                        Text(output.scriptType ?? "No address")
                            .foregroundStyle(.secondary)
                    }
                    ForEach(output.addresses, id: \.self) { address in
                        RouteLink(text: address, route: .holder(address), shorten: false)
                    }
                    if !output.tags.isEmpty {
                        TagRow(tags: output.tags)
                    }
                    if let spender = output.spentTxid {
                        HStack(spacing: 4) {
                            Text("spent by").foregroundStyle(.secondary)
                            RouteLink(text: spender, route: .transaction(txid: spender))
                        }
                        .font(.callout)
                    }
                }
                Spacer(minLength: 8)
                VStack(alignment: .trailing, spacing: 2) {
                    if let sats = output.valueSats {
                        CoinAmount(sats: sats)
                    }
                    ForEach(output.currencyValues, id: \.currency) { value in
                        HStack(spacing: 4) {
                            Text(Format.amount(value.amount)).monospacedDigit()
                            RouteLink(
                                text: value.currency, route: .currency(name: value.currency))
                        }
                        .font(.callout)
                    }
                }
                Button {
                    showingJSON.toggle()
                } label: {
                    Image(systemName: "curlybraces")
                }
                .buttonStyle(.borderless)
                .help("Show this output's JSON")
                .accessibilityLabel("Show JSON for output \(output.index)")
            }
            if showingJSON {
                ScrollView(.horizontal) {
                    Text(output.json)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                        .padding(10)
                }
                .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 6))
                .padding(.leading, 40)
            }
        }
        .padding(.vertical, 8)
    }
}
