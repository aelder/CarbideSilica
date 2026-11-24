import CarbideCore
import Charts
import SwiftUI

/// The fee pool's balance over time, read from the checkpoint each block's coinbase carries.
struct FeePoolChart: View {
    let endHeight: UInt64
    let blocks: UInt32
    let viaAPI: Bool

    @Environment(AppModel.self) private var model
    @State private var points: [FeePoolPoint] = []
    @State private var failure: String?
    @State private var loading = true

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            if loading {
                ProgressView("Reading fee pool checkpoints…").frame(maxWidth: .infinity, minHeight: 280)
            } else if let failure {
                Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary)
            } else if known.isEmpty {
                Text("The sampled blocks have no fee pool checkpoint. Blocks made before the fee pool have none.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, minHeight: 280)
            } else {
                Chart(known, id: \.height) { point in
                    AreaMark(
                        x: .value("Time", Date(timeIntervalSince1970: TimeInterval(point.time))),
                        y: .value("Fee pool", point.native ?? 0)
                    )
                    .foregroundStyle(Color.accentColor.opacity(0.15))
                    LineMark(
                        x: .value("Time", Date(timeIntervalSince1970: TimeInterval(point.time))),
                        y: .value("Fee pool", point.native ?? 0)
                    )
                    .foregroundStyle(Color.accentColor)
                }
                .frame(height: 280)
                Text(summary).font(.callout)
                Text("The chart shows \(known.count) sampled checkpoints. It can miss a peak between two samples. It shows the balance only.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .task(id: "\(endHeight)|\(blocks)") { await load() }
    }

    private var known: [FeePoolPoint] { points.filter { $0.native != nil } }

    private var summary: String {
        guard let first = known.first?.native, let last = known.last?.native else { return "" }
        let high = known.compactMap(\.native).max() ?? last
        return "The fee pool holds \(Format.amount(last)) \(model.coin). It held \(Format.amount(first)) at the start of the range. The highest sampled balance was \(Format.amount(high))."
    }

    private func load() async {
        loading = true
        failure = nil
        do {
            points = try await model.requireExplorer().feePoolSeries(
                endHeight: endHeight, blocks: blocks, samples: viaAPI ? 40 : 100)
        } catch is CancellationError {
            return
        } catch {
            points = []
            failure = describe(error)
        }
        loading = false
    }
}

/// Changes to the node's peer connections, each behind a confirmation, and the daemon's
/// log lines for a peer. These are the only things the app can change on the daemon.
struct PeerControlsCard: View {
    let node: NodeStatus

    @Environment(AppModel.self) private var model
    @State private var banned: [String] = []
    @State private var message: String?
    @State private var newNode = ""
    @State private var addingNode = false
    @State private var pending: PendingAction?
    @State private var logPeer: PeerRow?
    @State private var logLines: [String] = []

    private struct PendingAction: Identifiable {
        let id = UUID()
        let title: String
        let detail: String
        let action: PeerAction
    }

    var body: some View {
        if node.peersError == nil {
            Card("Manage peers", trailing: banned.isEmpty ? nil : "\(banned.count) banned") {
                HStack {
                    Button("Add Node…") { addingNode = true }
                    Button("Ping All") { run(.pingAll, done: "The node will ping all connected peers.") }
                    Button("Clear Ban List…") {
                        pending = PendingAction(
                            title: "Clear the ban list?",
                            detail: "All \(banned.count) banned addresses can connect again.",
                            action: .clearBans)
                    }
                    .disabled(banned.isEmpty)
                    Spacer()
                }
                if let message {
                    Text(message).font(.callout).foregroundStyle(.secondary)
                }
                VStack(spacing: 0) {
                    ForEach(node.peers, id: \.id) { peer in
                        Divider()
                        HStack(spacing: 10) {
                            Text(peer.address).font(.callout.monospaced()).lineLimit(1)
                            Spacer()
                            Button("Log") { showLog(peer) }
                            Button("Disconnect…") {
                                pending = PendingAction(
                                    title: "Disconnect \(peer.address)?",
                                    detail: "The node closes this connection. The peer can connect again.",
                                    action: .disconnect(address: peer.address))
                            }
                            Button("Ban…", role: .destructive) {
                                pending = PendingAction(
                                    title: "Ban \(peer.address)?",
                                    detail: "The node refuses connections from this address until the ban expires or you clear the ban list.",
                                    action: .ban(ip: peer.address))
                            }
                        }
                        .padding(.vertical, 6)
                    }
                }
                if let logPeer {
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text("Node log for \(logPeer.address)").font(.subheadline.weight(.semibold))
                            Spacer()
                            Button("Close") { self.logPeer = nil }
                        }
                        if logLines.isEmpty {
                            Text("The recent log has no lines for this peer.").foregroundStyle(.secondary)
                        } else {
                            ScrollView {
                                Text(logLines.joined(separator: "\n"))
                                    .font(.caption.monospaced())
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .frame(maxHeight: 220)
                        }
                    }
                    .padding(10)
                    .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 6))
                }
                if !banned.isEmpty {
                    Text("Banned: \(banned.joined(separator: ", "))").font(.caption).foregroundStyle(.secondary)
                }
                Text("These commands change only the node connections. They do not change funds, keys or the chain.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .task(id: model.generation) { await loadBanned() }
            .alert("Add a node", isPresented: $addingNode) {
                TextField("Address, such as 203.0.113.18:27485", text: $newNode)
                Button("Add") {
                    run(.addNode(address: newNode), done: "Added \(newNode) to the node list.")
                    newNode = ""
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The node tries to keep a connection to this address.")
            }
            .confirmationDialog(
                pending?.title ?? "", isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
                titleVisibility: .visible
            ) {
                if let pending {
                    Button("Confirm", role: .destructive) { run(pending.action, done: "Done.") }
                }
            } message: {
                Text(pending?.detail ?? "")
            }
        }
    }

    private func run(_ action: PeerAction, done: String) {
        Task {
            do {
                try await model.requireExplorer().peerAction(action: action)
                message = done
            } catch {
                message = describe(error)
            }
            await loadBanned()
            await model.refreshNode()
        }
    }

    private func loadBanned() async {
        banned = (try? await model.explorer?.bannedPeers()) ?? []
    }

    private func showLog(_ peer: PeerRow) {
        logPeer = peer
        logLines = (try? model.explorer?.peerLog(address: peer.address)) ?? []
    }
}

/// Which identities shared this one's addresses or authorities during a past window of blocks.
struct ReverseLookupCard: View {
    let identity: IdentityDetail

    @Environment(AppModel.self) private var model
    @State private var endHeight = ""
    @State private var found: [SharedIdentity]?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Shared keys and authorities") {
            HStack {
                Text("In the 1,000 blocks ending at").foregroundStyle(.secondary)
                TextField("Block", text: $endHeight, prompt: Text(model.status.map { "\($0.blocks)" } ?? "height"))
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 130)
                Button("Look up") { Task { await run() } }.disabled(running)
                Spacer()
            }
            if running {
                ProgressView().controlSize(.small)
            } else if let failure {
                Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary)
            } else if let found {
                if found.isEmpty {
                    Text("No other identity used these addresses or authorities in this range. Other ranges can give different results.")
                        .foregroundStyle(.secondary)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(found.enumerated()), id: \.offset) { index, shared in
                            if index > 0 { Divider() }
                            HStack(spacing: 12) {
                                Text(shared.relation).foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
                                RouteLink(
                                    text: shared.name,
                                    route: .identity(name: shared.identityAddress ?? shared.name), shorten: false)
                                Spacer()
                                Text("via \(model.names[shared.via] ?? Format.short(shared.via))")
                                    .font(.callout).foregroundStyle(.secondary)
                            }
                            .padding(.vertical, 7)
                        }
                    }
                }
            } else {
                Text("Find identities that used the same primary address, revocation authority or recovery authority. This search needs a node with the identity index.")
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private func run() async {
        guard let height = UInt64(endHeight.trimmingCharacters(in: .whitespaces)) ?? model.status?.blocks else { return }
        running = true
        failure = nil
        defer { running = false }
        do {
            found = try await model.requireExplorer().identitiesSharing(
                name: identity.identityAddress ?? identity.name, endHeight: height)
        } catch {
            found = nil
            failure = describe(error)
        }
    }
}

/// For an identity whose control is a token: who holds that token now.
struct TokenControlCard: View {
    let identity: IdentityDetail

    @Environment(AppModel.self) private var model
    @State private var holder: TokenHolder?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Tokenized control") {
            Text("The holder of the control token controls this identity.")
                .foregroundStyle(.secondary)
            if let holder {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    ConfidenceMark(level: holder.confidence, label: holder.holder == nil ? "Unresolved" : "Current holder")
                    if let address = holder.holder {
                        RouteLink(text: address, route: .holder(address), shorten: false)
                    }
                }
                Text(holder.summary).font(.callout)
                if let txid = holder.txid {
                    HStack(spacing: 6) {
                        Text("Last seen in").foregroundStyle(.secondary)
                        RouteLink(text: txid, route: .transaction(txid: txid))
                        if let output = holder.output { Text("output \(output)").foregroundStyle(.secondary) }
                    }
                    .font(.callout)
                }
            } else if running {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Tracing the token…").foregroundStyle(.secondary)
                }
            } else {
                if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                Button("Find the token holder") { Task { await run() } }
            }
        }
    }

    private func run() async {
        guard let address = identity.identityAddress else { return }
        running = true
        failure = nil
        defer { running = false }
        do {
            let result = try await model.requireExplorer().tokenHolder(identityAddress: address)
            holder = result
            await model.resolveNames([result.holder])
        } catch {
            failure = describe(error)
        }
    }
}

/// How an identity registration's cost was split: payments to other identities, which are
/// usually referrals or the issuer's charge, and the transaction fee.
struct RegistrationPaymentsCard: View {
    let transaction: TransactionDetail
    @Environment(AppModel.self) private var model

    private var payers: Set<String> { Set(transaction.inputs.flatMap(\.addresses)) }

    private var payments: [(String, Int64)] {
        var result: [(String, Int64)] = []
        for output in transaction.outputs {
            guard let address = output.addresses.first, address.hasPrefix("i"), address.count == 34,
                !payers.contains(address), output.tags.isEmpty, let sats = output.valueSats, sats > 0
            else { continue }
            if let index = result.firstIndex(where: { $0.0 == address }) {
                result[index].1 += sats
            } else {
                result.append((address, sats))
            }
        }
        return result
    }

    private var fee: Int64? {
        let inputs = transaction.inputs.compactMap(\.valueSats)
        guard inputs.count == transaction.inputs.count, !inputs.isEmpty else { return nil }
        let out = transaction.outputs.compactMap(\.valueSats).reduce(0, +)
        return inputs.reduce(0, +) - out
    }

    var body: some View {
        if !payments.isEmpty {
            Card("Registration payments") {
                ForEach(payments, id: \.0) { payment in
                    HStack {
                        RouteLink(text: payment.0, route: .identity(name: payment.0), shorten: true)
                        Spacer()
                        CoinAmount(sats: payment.1)
                    }
                }
                if let fee, fee >= 0 {
                    Divider()
                    HStack {
                        Text("Transaction fee").foregroundStyle(.secondary)
                        Spacer()
                        CoinAmount(sats: fee)
                    }
                    HStack {
                        Text("Total").fontWeight(.semibold)
                        Spacer()
                        CoinAmount(sats: payments.map(\.1).reduce(0, +) + fee).fontWeight(.semibold)
                    }
                }
                HStack(spacing: 10) {
                    ConfidenceMark(level: .candidate, label: "Not confirmed")
                    Text("A payment to an identity in a registration is usually a referral or a charge from the issuer. The chain does not record the type.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            }
            .task(id: transaction.txid) { await model.resolveNames(payments.map(\.0)) }
        }
    }
}

/// Every block a transaction is known to be in, including any on a side branch.
struct InclusionCard: View {
    let txid: String

    @Environment(AppModel.self) private var model
    @State private var found: [Inclusion]?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Block history") {
            if let found {
                let orphaned = found.filter { !$0.active }
                Note(
                    text: orphaned.isEmpty
                        ? "The transaction is on the active chain only. No side branch on the node contains it."
                        : "Also found in \(orphaned.count) block\(orphaned.count == 1 ? "" : "s") on a side branch.",
                    warning: !orphaned.isEmpty)
                ForEach(found, id: \.blockHash) { inclusion in
                    HStack(spacing: 12) {
                        ConfidenceMark(
                            level: inclusion.active ? .verified : .unknown,
                            label: inclusion.active ? "Active chain" : "Side branch"
                        )
                        .frame(width: 130, alignment: .leading)
                        if let height = inclusion.height {
                            Text("Block \(Format.integer(height))").monospacedDigit()
                        }
                        RouteLink(text: inclusion.blockHash, route: .block(id: inclusion.blockHash))
                        Spacer()
                    }
                }
                Text("The check uses the side branches on the node. An empty result does not prove that no other block contained the transaction.")
                    .font(.caption).foregroundStyle(.secondary)
            } else if running {
                ProgressView().controlSize(.small)
            } else {
                if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                HStack {
                    Text("Find other blocks that contained this transaction.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Check") { Task { await run() } }
                }
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
