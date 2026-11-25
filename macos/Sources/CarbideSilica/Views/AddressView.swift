import CarbideCore
import SwiftUI

/// A transparent address: balances, the identities that use it as a primary address, and
/// its transactions as tiles of inputs and outputs.
struct AddressView: View {
    let address: String

    @Environment(AppModel.self) private var model
    @State private var refreshes = 0

    var body: some View {
        LoadView(key: "address:\(address)|\(refreshes)") { explorer in
            try await explorer.addressOverview(address: address)
        } content: { overview in
            Page {
                PageHeader(kind: "Address", title: overview.address)
                ProvenanceNote(provenance: overview.provenance)

                Card("Balances") {
                    if let error = overview.balanceError {
                        Note(text: "The balances did not load: \(error)", warning: true)
                    } else if overview.balances.isEmpty {
                        Text("No balances.").foregroundStyle(.secondary)
                    } else {
                        BalanceList(balances: overview.balances)
                    }
                }

                Card("Activity") {
                    if let error = overview.historyError {
                        Note(text: "The transaction list did not load: \(error)", warning: true)
                    } else {
                        ValueRow(label: "Transactions", value: Format.integer(overview.transactionCount))
                        ValueRow(label: "First", value: overview.firstTime.map(Format.date) ?? "unknown")
                        ValueRow(label: "Most recent", value: overview.latestTime.map(Format.date) ?? "unknown")
                    }
                }

                PrimaryAddressCard(address: overview.address)

                Card("Transactions") {
                    AddressActivityList(address: overview.address, onStale: { refreshes += 1 })
                }

                JSONSection(json: overview.json)
            }
            .publishPlace(PlaceActions(refresh: { refreshes += 1 }))
            .task(id: overview.address) {
                await model.resolveNames(overview.balances.map(\.currencyId))
            }
        }
        .navigationTitle("Address")
    }
}

/// The identities that have this address as a primary address now. The request needs the
/// identity index.
private struct PrimaryAddressCard: View {
    let address: String
    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Primary address for") {
            SectionLoader(key: "primary:\(address)", loadingText: "Requesting identities…") { explorer in
                try await explorer.identitiesWithPrimaryAddress(address: address)
            } content: { found in
                Text("\(found.identities.count) identit\(found.identities.count == 1 ? "y" : "ies") via \(found.source == .publicApi ? "public API" : "local node")")
                    .foregroundStyle(.secondary)
                if found.identities.isEmpty {
                    Text("No associated identities found.").foregroundStyle(.secondary)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(found.identities.enumerated()), id: \.element.id) { index, entry in
                            if index > 0 { Divider() }
                            HStack {
                                Button(entry.name) { model.open(.identity(name: entry.id)) }
                                    .buttonStyle(.link)
                                Spacer()
                                Text(entry.id).font(.callout.monospaced()).foregroundStyle(.secondary)
                                    .textSelection(.enabled)
                            }
                            .padding(.vertical, 5)
                        }
                    }
                }
            }
        }
    }
}

/// An address's transactions, newest first, a page at a time. Each tile shows the
/// transaction's inputs and outputs, with the viewed address first.
struct AddressActivityList: View {
    let address: String
    /// Called when a reorganization removed the block of a loaded transaction. Without it,
    /// the list loads itself again.
    var onStale: (() -> Void)?

    @Environment(AppModel.self) private var model
    @State private var stale = false
    @State private var reloads = 0
    @State private var tiles: [AddressTile] = []
    @State private var missing: [String] = []
    @State private var total: UInt64?
    @State private var loading = false
    @State private var failure: String?

    private let pageSize: UInt64 = 10

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            if stale {
                Note(text: "Stale after reorg. A block of these transactions left the active chain. The list is refreshing.", warning: true)
            }
            if let total {
                Text(heading(total)).foregroundStyle(.secondary)
            }
            if let total, total == 0, !loading {
                Text("No indexed transactions.").foregroundStyle(.secondary)
            }
            ForEach(tiles, id: \.txid) { tile in
                AddressTileView(tile: tile, viewed: address)
            }
            ForEach(missing, id: \.self) { txid in
                HStack {
                    RouteLink(text: txid, route: .transaction(txid: txid))
                    Text("details did not load").foregroundStyle(.secondary)
                }
            }
            if loading {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading transactions…").foregroundStyle(.secondary)
                }
            } else if let failure {
                Note(text: failure, warning: true)
                Button("Try Again") { Task { await loadMore() } }
            } else if let total, loadedCount < total {
                Button("Show \(min(pageSize, total - loadedCount)) older") { Task { await loadMore() } }
            }
        }
        .task(id: "\(address)|\(model.generation)|\(reloads)") {
            stale = false
            tiles = []
            missing = []
            total = nil
            await loadMore()
        }
        .task(id: model.status?.blocks) {
            await recheckStale()
        }
    }

    private var loadedCount: UInt64 { UInt64(tiles.count + missing.count) }

    /// When the tip changes, checks that the blocks of the loaded transactions are still on
    /// the active chain, as the terminal marks an address stale after a reorg.
    private func recheckStale() async {
        let hashes = Array(Set(tiles.compactMap(\.blockHash)))
        guard !hashes.isEmpty, !stale, let explorer = model.explorer else { return }
        let checked = await explorer.blocksActive(hashes: hashes)
        guard !Task.isCancelled, checked.contains(where: { $0.active == false }) else { return }
        stale = true
        try? await Task.sleep(for: .seconds(2))
        guard !Task.isCancelled else { return }
        if let onStale { onStale() } else { reloads += 1 }
    }

    private func heading(_ total: UInt64) -> String {
        guard loadedCount > 0 else { return "\(Format.integer(total)) transactions" }
        return "Newest \(Format.integer(loadedCount)) of \(Format.integer(total))"
    }

    private func loadMore() async {
        guard !loading else { return }
        loading = true
        failure = nil
        defer { loading = false }
        do {
            let page = try await model.requireExplorer().addressActivity(
                address: address, offset: loadedCount, count: pageSize)
            total = page.total
            tiles += page.tiles
            missing += page.missing
            var ids: [String?] = []
            for tile in page.tiles {
                for flow in tile.inputs + tile.outputs {
                    ids += flow.addresses.map(Optional.some)
                    ids += flow.amounts.map { Optional.some($0.currency) }
                }
            }
            await model.resolveNames(ids)
        } catch is CancellationError {
        } catch {
            failure = describe(error)
        }
    }
}

/// One transaction: inputs on the left and outputs on the right, or one above the other
/// when the window is narrow.
private struct AddressTileView: View {
    let tile: AddressTile
    let viewed: String

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                RouteLink(text: tile.txid, route: .transaction(txid: tile.txid))
                if !tile.tags.isEmpty { TagRow(tags: tile.tags) }
                Spacer()
                if let height = tile.height {
                    RouteLink(
                        text: model.heightText(height, time: tile.time),
                        route: .block(id: tile.blockHash ?? "\(height)"), shorten: false)
                }
                Text(when).foregroundStyle(.secondary).monospacedDigit()
            }
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .top, spacing: 24) {
                    column("Inputs", tile.inputs, more: tile.moreInputs, outputs: false)
                        .frame(minWidth: 320, maxWidth: .infinity, alignment: .leading)
                    column("Outputs", tile.outputs, more: tile.moreOutputs, outputs: true)
                        .frame(minWidth: 320, maxWidth: .infinity, alignment: .leading)
                }
                VStack(alignment: .leading, spacing: 10) {
                    column("Inputs", tile.inputs, more: tile.moreInputs, outputs: false)
                    column("Outputs", tile.outputs, more: tile.moreOutputs, outputs: true)
                }
            }
        }
        .padding(12)
        .overlay(RoundedRectangle(cornerRadius: 8).stroke(Color.secondary.opacity(0.25)))
    }

    private var when: String {
        if let time = tile.time { return Format.date(time) }
        if tile.height == nil { return "Unconfirmed" }
        return "Time not available"
    }

    private func column(_ title: String, _ flows: [AddressFlow], more: UInt32, outputs: Bool) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title.uppercased())
                .font(.caption.weight(.semibold))
                .foregroundStyle(outputs ? .green : .orange)
            if flows.isEmpty {
                Text("none").font(.callout).foregroundStyle(.secondary)
            }
            ForEach(flows, id: \.index) { flow in
                FlowRow(flow: flow, output: outputs, viewed: viewed)
            }
            if more > 0 {
                Text("… \(more) more not shown").font(.callout).foregroundStyle(.orange)
            }
        }
    }
}

private struct FlowRow: View {
    let flow: AddressFlow
    let output: Bool
    let viewed: String

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            if output { statusMark }
            Text("#\(flow.index)").font(.callout.monospaced()).foregroundStyle(.secondary)
            who
            Spacer(minLength: 8)
            amount
        }
        .font(.callout)
    }

    @ViewBuilder
    private var statusMark: some View {
        if flow.private {
            Image(systemName: "diamond.fill").foregroundStyle(.purple).help("Private")
        } else {
            switch flow.status {
            case .unspent:
                Image(systemName: "circle.fill").foregroundStyle(.green).help("Unspent")
                    .accessibilityLabel("Unspent")
            case .spent:
                Image(systemName: "xmark").foregroundStyle(.red).help("Spent")
                    .accessibilityLabel("Spent")
            case .unknown:
                Image(systemName: "questionmark.circle").foregroundStyle(.secondary)
                    .help("Not known if spent").accessibilityLabel("Not known if spent")
            }
        }
    }

    @ViewBuilder
    private var who: some View {
        if flow.coinbase {
            Text("Coinbase").foregroundStyle(.orange)
        } else if flow.private {
            Text(output ? "Private Output" : "Private Input").foregroundStyle(.purple)
        } else if flow.addresses.isEmpty {
            Text("address unavailable").foregroundStyle(.secondary)
        } else {
            HStack(spacing: 4) {
                ForEach(flow.addresses, id: \.self) { address in
                    if address == viewed {
                        Text(model.displayID(address))
                            .fontWeight(.semibold)
                            .help("\(address) (this page)")
                    } else {
                        IDLink(id: address, route: .holder(address))
                    }
                }
            }
        }
    }

    @ViewBuilder
    private var amount: some View {
        if flow.private {
            Text("amount hidden").foregroundStyle(.secondary)
        } else if flow.amounts.isEmpty {
            Text("value unavailable").foregroundStyle(.secondary)
        } else {
            Text(flow.amounts.map { model.amountText($0.sats, currency: $0.currency) }
                .joined(separator: ", "))
                .monospacedDigit()
        }
    }
}
