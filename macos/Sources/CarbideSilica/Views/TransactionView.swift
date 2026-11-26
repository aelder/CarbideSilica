import CarbideCore
import SwiftUI

/// A transaction's overview, as Carbide's terminal shows it: status, value accounting and
/// fee, then where the coins went, then each input and output with its annotations.
///
/// Names and dates follow the app's display settings. `showNames` and `showDates` override
/// them when set.
struct TransactionView: View {
    let txid: String
    var showNames: Bool?
    var showDates: Bool?

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNamesSetting = true
    @AppStorage(DisplayKey.showsDates) private var showsDatesSetting = false
    @Environment(ShellState.self) private var shell
    @State private var reloads = 0
    /// Requests to start an investigation card, by card ID. Each request increments.
    @State private var starts: [String: Int] = [:]
    @State private var scrollTarget: String?
    /// The transactions of the block that holds this one, in order.
    @State private var siblings: [String] = []
    /// Carbide's verified registration payment annotations, by output.
    @State private var paymentAnnotations: [UInt32: String] = [:]
    /// The row of the statement whose inputs and outputs are marked in the lists.
    @State private var marked: StatementSelection?
    /// True when a long list of inputs or outputs shows all of its rows.
    @State private var allInputs = false
    @State private var allOutputs = false
    /// A list longer than this shows only its first rows until the person asks for all.
    private static let longList = 40
    private static let firstRows = 20

    /// A row that the statement marks can be out of the first rows: the list then shows all.
    private var showsAllInputs: Bool { allInputs || !(marked?.inputs.isEmpty ?? true) }
    private var showsAllOutputs: Bool { allOutputs || !(marked?.outputs.isEmpty ?? true) }

    /// The key below a long list: it shows all the rows, or only the first rows again.
    private func listKey(count: Int, noun: String, all: Bool, action: @escaping () -> Void) -> some View {
        HStack(spacing: 12) {
            Button(all ? "Show the first \(Self.firstRows)" : "Show all \(count) \(noun)", action: action)
            if !all {
                Text("The first \(Self.firstRows) of \(count) \(noun) are shown.")
                    .font(Tone.caption).foregroundStyle(Tone.label)
            }
        }
        .padding(.top, 8)
    }

    private var names: Bool { showNames ?? showsNamesSetting }
    private var dates: Bool { showDates ?? showsDatesSetting }

    var body: some View {
        LoadView(key: "tx:\(txid)|\(reloads)") { explorer in
            try await explorer.transaction(txid: txid)
        } content: { transaction in
            ScrollViewReader { proxy in
                page(transaction)
                    .onChange(of: scrollTarget) { _, target in
                        guard let target else { return }
                        scrollTarget = nil
                        // A row of a long list can be out of the first rows: show all first.
                        if target.hasPrefix("input-") { allInputs = true }
                        if target.hasPrefix("output-") { allOutputs = true }
                        Task {
                            try? await Task.sleep(for: .milliseconds(60))
                            withAnimation { proxy.scrollTo(target, anchor: .top) }
                        }
                    }
            }
            .publishInvestigations(investigations(transaction))
            // The terminal's key for the block of a transaction.
            .publishKeys(transaction.blockHash.map { hash in ["b": { model.open(.block(id: hash)) }] } ?? [:])
            // n and p go to the transaction after and before this one in its block, in place.
            .publishAdjacent(
                AdjacentObjects(
                    noun: "Transaction in Block",
                    previous: sibling(of: transaction.txid, by: -1).map { txid in
                        { model.step(to: .transaction(txid: txid)) }
                    },
                    next: sibling(of: transaction.txid, by: 1).map { txid in
                        { model.step(to: .transaction(txid: txid)) }
                    }))
            .task(id: "siblings|\(transaction.blockHash ?? "")") {
                guard let hash = transaction.blockHash, let explorer = model.explorer else {
                    siblings = []
                    return
                }
                siblings = (try? await explorer.block(id: hash))?.transactions.map(\.txid) ?? []
            }
            .task(id: transaction.txid) {
                marked = nil
                allInputs = false
                allOutputs = false
                consumePendingTrace(transaction)
                await model.resolveNames(Self.ids(in: transaction))
            }
            .task(id: "payments|\(transaction.txid)|\(model.generation)") {
                await loadPayments(transaction)
            }
        }
        .publishPlace(PlaceActions(refresh: { reloads += 1 }))
        .navigationTitle("Transaction")
    }

    private func page(_ transaction: TransactionDetail) -> some View {
        Page {
            PageHeader(
                kind: "Transaction",
                title: Format.short(transaction.txid, head: 16, tail: 16),
                subtitle: transaction.txid,
                tags: transaction.tags
            )
            ProvenanceNote(provenance: transaction.provenance)
            if let height = transaction.height {
                HStack {
                    Spacer()
                    Button {
                        model.showCharts(at: height)
                    } label: {
                        Label("Open in Charts", systemImage: "chart.xyaxis.line")
                    }
                    .help("Open Charts at the block that holds this transaction.")
                }
            }

            TransactionHeader(transaction: transaction, showDates: dates)

            Card("Statement") {
                TransactionStatementView(
                    transaction: transaction, selection: $marked,
                    scroll: { scrollTarget = $0 },
                    follow: { start("trace-\($0)") })
            }

            ForEach(transaction.traceRoots, id: \.output) { root in
                TransferTraceCard(
                    txid: transaction.txid, root: root, startRequests: starts["trace-\(root.output)"] ?? 0)
                    .id("trace-\(root.output)")
            }

            ForEach(transaction.witnessRoots, id: \.output) { root in
                WitnessCard(
                    txid: transaction.txid, root: root, startRequests: starts["witness-\(root.output)"] ?? 0)
                    .id("witness-\(root.output)")
            }

            ProtocolLinksCard(txid: transaction.txid, forward: transaction.links)
                .id("links")

            if transaction.tags.contains("Identity Registration") {
                RegistrationPaymentsCard(transaction: transaction)
            }

            Card("Inputs", trailing: "\(transaction.inputs.count)") {
                let long = transaction.inputs.count > Self.longList
                let shown = long && !showsAllInputs ? Array(transaction.inputs.prefix(Self.firstRows)) : transaction.inputs
                LazyVStack(spacing: 0) {
                    ForEach(Array(shown.enumerated()), id: \.element.index) {
                        position, input in
                        if position > 0 { Divider() }
                        InputRow(input: input, spentInBlock: transaction.blockHash != nil, showDates: dates)
                            .background(mark(marked?.inputs.contains(input.index) == true))
                            .id("input-\(input.index)")
                    }
                }
                if long {
                    listKey(count: transaction.inputs.count, noun: "inputs", all: showsAllInputs) { allInputs.toggle() }
                }
            }

            Card("Outputs", trailing: "\(transaction.outputs.count)") {
                let long = transaction.outputs.count > Self.longList
                let shown = long && !showsAllOutputs ? Array(transaction.outputs.prefix(Self.firstRows)) : transaction.outputs
                LazyVStack(spacing: 0) {
                    ForEach(Array(shown.enumerated()), id: \.element.index) {
                        position, output in
                        if position > 0 { Divider() }
                        OutputRow(
                            output: output, txid: transaction.txid,
                            paymentAnnotation: paymentAnnotations[output.index])
                            .environment(\.stopContext, "output-\(output.index)")
                            .background(mark(marked?.outputs.contains(output.index) == true))
                            .id("output-\(output.index)")
                    }
                }
                if long {
                    listKey(count: transaction.outputs.count, noun: "outputs", all: showsAllOutputs) { allOutputs.toggle() }
                }
            }

            if transaction.height != nil {
                InclusionCard(txid: transaction.txid)
                    .id("inclusions")
            }

            JSONSection(json: transaction.json, showNames: names, showDates: dates, filename: "tx-\(transaction.txid).json")
        }
    }

    /// The fill of a row that the statement marks.
    private func mark(_ on: Bool) -> Color {
        on ? Tone.present.opacity(0.08) : .clear
    }

    /// The menu's investigation commands: each scrolls to its card and starts it.
    private func investigations(_ transaction: TransactionDetail) -> InvestigationActions {
        InvestigationActions(
            // The output under the cursor, when it has the function. If not, the first that has.
            followTransfer: transaction.traceRoots.first.map { first in
                {
                    let chosen = cursorOutput()
                    let root = transaction.traceRoots.first { $0.output == chosen } ?? first
                    start("trace-\(root.output)")
                }
            },
            witnesses: transaction.witnessRoots.first.map { first in
                {
                    let chosen = cursorOutput()
                    let root = transaction.witnessRoots.first { $0.output == chosen } ?? first
                    start("witness-\(root.output)")
                }
            },
            protocolLinks: { scrollTarget = "links" },
            inclusions: transaction.height == nil ? nil : { scrollTarget = "inclusions" }
        )
    }

    /// The transaction a number of places from this one in its block.
    private func sibling(of txid: String, by step: Int) -> String? {
        guard let index = siblings.firstIndex(of: txid), siblings.indices.contains(index + step) else { return nil }
        return siblings[index + step]
    }

    /// The output that the cursor of the page is in.
    private func cursorOutput() -> UInt32? {
        guard let context = shell.cursor?.selected?.context, context.hasPrefix("output-") else { return nil }
        return UInt32(context.dropFirst("output-".count))
    }

    private func start(_ card: String) {
        starts[card, default: 0] += 1
        scrollTarget = card
    }

    /// Starts the trace that another page asked for, once, when it names this transaction.
    private func consumePendingTrace(_ transaction: TransactionDetail) {
        guard let request = model.pendingTrace, request.txid == transaction.txid else { return }
        model.pendingTrace = nil
        guard transaction.traceRoots.contains(where: { $0.output == request.output }) else { return }
        start("trace-\(request.output)")
    }

    /// Carbide's registration payment annotations. Checking and unverified states add nothing.
    private func loadPayments(_ transaction: TransactionDetail) async {
        paymentAnnotations = [:]
        guard transaction.json.contains("\"identityreservation\"")
            || transaction.json.contains("\"identityadvancedreservation\""),
            let explorer = model.explorer,
            let payments = try? await explorer.registrationPayments(txid: transaction.txid),
            case .verified = payments.status
        else { return }
        var found: [UInt32: String] = [:]
        for row in payments.referrals + [payments.charge].compactMap({ $0 }) {
            found[row.output] = row.annotation
        }
        paymentAnnotations = found
    }

    /// Every identity and currency ID the overview can name.
    static func ids(in transaction: TransactionDetail) -> [String] {
        var ids: [String] = []
        for input in transaction.inputs {
            ids += input.addresses + input.reserveAmounts.map(\.currency)
        }
        for output in transaction.outputs {
            ids += output.addresses + output.reserveAmounts.map(\.currency)
            ids += output.tagDetails.compactMap(\.currencyId)
            for reference in output.references {
                switch reference.target {
                case .currency(let id): ids.append(id)
                case .destination(let address, let system): ids += [address, system].compactMap { $0 }
                case .output(let outpoint): ids.append(outpoint.systemId)
                default: break
                }
            }
        }
        ids += transaction.statement.columns.compactMap { $0 }
        ids += transaction.statement.parties.compactMap(\.currencyId)
        ids += transaction.statement.marks.compactMap(\.currencyId)
        for route in transaction.statement.routes {
            ids += [route.via, route.toCurrency, route.destination].compactMap { $0 }
        }
        ids += transaction.accounting.inputs.reserves.map(\.currency)
        ids += transaction.accounting.outputs.reserves.map(\.currency)
        return ids
    }
}

/// The terminal's header lines: status, block, expiry, size, values, fee and the context
/// lines for notarizations, shielded value and identity registrations.
private struct TransactionHeader: View {
    let transaction: TransactionDetail
    let showDates: Bool

    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Overview") {
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                statusMark
                if transaction.inMempool {
                    Plate("Mempool", fill: Tone.mined)
                }
                if transaction.fromPublicApi {
                    Plate("Public API")
                        .help("A public API supplied this transaction, not the local node.")
                }
                Spacer()
            }
            ForEach(transaction.notarizationContext, id: \.self) { line in
                Text(line).foregroundStyle(.secondary)
            }

            if let hash = transaction.blockHash {
                FieldRow(
                    label: "Block",
                    value: transaction.height.map { height(of: $0, known: transaction.blockTime) }
                        ?? "Height unknown",
                    route: .block(id: hash))
                FieldRow(label: "Block hash", value: hash, route: .block(id: hash), monospaced: true)
            } else {
                FieldRow(label: "Block", value: "Not in a block")
            }
            FieldRow(
                label: "Block time",
                value: transaction.blockTime.map { "\(Format.date($0)) (\(Format.age($0)))" }
                    ?? "Not yet confirmed")
            if let confirmations = transaction.confirmations, confirmations > 0 {
                FieldRow(
                    label: "Confirmations",
                    value: showDates
                        ? HeightDates.shared.confirmations(confirmations, blockTime: transaction.blockTime)
                        : confirmations.formatted())
            }
            FieldRow(label: "Expiry", value: expiry)
            FieldRow(label: "Size", value: transaction.size.map { "\(Format.integer($0)) bytes" } ?? "Unknown")
            FieldRow(label: "Version", value: transaction.version.map(String.init) ?? "Unknown")
            FieldRow(label: "Locktime", value: locktime)

            ValueLines(transaction: transaction)

            ForEach(transaction.shielded, id: \.self) { line in
                FieldRow(label: "Shielded", value: line)
            }
            ForEach(transaction.registration, id: \.self) { line in
                RegistrationLine(line: line)
            }
            if transaction.feePoolCheckpoint {
                Note(text: "The fee pool output is an unspendable protocol checkpoint. Its transfer trace compares adjacent coinbase states.")
                    .foregroundStyle(.secondary)
            }
        }
    }

    @ViewBuilder private var statusMark: some View {
        switch transaction.status {
        case .confirmed:
            if showDates, let time = transaction.blockTime {
                ConfidenceMark(level: .verified, label: "Confirmed since \(Format.date(time))")
            } else {
                ConfidenceMark(
                    level: .verified,
                    label: "Confirmed (\(transaction.statusText.filter(\.isNumber)))")
            }
        case .unconfirmed:
            ConfidenceMark(level: .observed, label: "Unconfirmed")
        case .confirmationsUnverified:
            ConfidenceMark(level: .unknown, label: "Confirmations unverified")
                .help("The transaction names a block, but the source did not establish its confirmations.")
        case .orphanedInclusion:
            ConfidenceMark(level: .unknown, label: "Orphaned inclusion")
                .help("The block that holds this transaction is not on the active chain. The app did not check the mempool.")
        }
    }

    /// A height, or its date when dates are on.
    private func height(of value: UInt64, known: UInt64? = nil) -> String {
        showDates ? HeightDates.shared.text(value, known: known, model: model) : Format.integer(value)
    }

    private var expiry: String {
        guard let value = transaction.expiryHeight, value > 0 else { return "None" }
        return height(of: value)
    }

    /// A locktime below 500,000,000 is a height. Above it is a Unix time.
    private var locktime: String {
        guard let value = transaction.locktime else { return "Unknown" }
        if value == 0 { return "0" }
        if value >= 500_000_000 { return showDates ? Format.date(value) : "\(value) (Unix time \(Format.date(value)))" }
        return showDates ? height(of: value) : "Height \(Format.integer(value))"
    }
}

/// Input and output totals, fee and, for protocol transactions, the public native delta.
private struct ValueLines: View {
    let transaction: TransactionDetail

    private var accounting: ValueAccounting { transaction.accounting }

    var body: some View {
        TotalRow(label: "Inputs (\(accounting.inputCount))", total: accounting.inputs)
        TotalRow(label: "Outputs (\(accounting.outputCount))", total: accounting.outputs)
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            NameText("Fee").frame(width: 170, alignment: .leading)
            switch accounting.feeKind {
            case .known:
                Text(accounting.feeLabel).monospacedDigit().textSelection(.enabled)
            case .unknown:
                Text("Unknown. A value is missing from the source.").foregroundStyle(.secondary)
            case .protocolAccounting:
                Text("Unknown (protocol accounting)").foregroundStyle(.secondary)
                    .help("Protocol outputs account for value elsewhere. Carbide does not reconstruct the consensus fee.")
            case .notApplicable:
                Text("Not applicable to a coinbase").foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
        }
        if accounting.showPublicDelta {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                NameText("Public native delta").frame(width: 170, alignment: .leading)
                if let delta = accounting.publicNativeDelta {
                    (Text(Format.coins(delta)).monospacedDigit() + Text(" \(transaction.coin)").foregroundStyle(.secondary))
                        .textSelection(.enabled)
                } else {
                    Text("Unknown").foregroundStyle(.secondary)
                }
                Spacer(minLength: 0)
            }
            Text("Transparent inputs minus transparent outputs, plus the net shielded flow. It is not the fee.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }
}

private struct TotalRow: View {
    let label: String
    let total: ValueTotal

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            NameText(label).frame(width: 170, alignment: .leading)
            VStack(alignment: .leading, spacing: 3) {
                Text(total.label).monospacedDigit().textSelection(.enabled)
                ForEach(total.reserves, id: \.currency) { reserve in
                    ReserveAmount(amount: reserve, prefix: "+ ")
                }
                if total.remainingUnknown {
                    Text("Remaining values unknown").font(.callout).foregroundStyle(.secondary)
                }
            }
            Spacer(minLength: 0)
        }
    }
}

/// A token amount: the number, then the currency's name as a link.
struct ReserveAmount: View {
    let amount: CurrencyAmount
    var prefix = ""

    var body: some View {
        HStack(spacing: 4) {
            Text(prefix + Format.amount(amount.amount)).monospacedDigit()
            RouteLink(text: amount.currency, route: .currency(name: amount.currency))
        }
        .font(.callout)
    }
}

/// An identity registration line. IDs in it are links that show names.
private struct RegistrationLine: View {
    let line: String

    var body: some View {
        if let pair = split, pair.1.hasPrefix("i"), pair.1.count == 34 {
            FieldRow(label: pair.0, value: pair.1, route: .identity(name: pair.1))
        } else {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                NameText("Identity").frame(width: 170, alignment: .leading)
                Text(line).textSelection(.enabled)
                Spacer(minLength: 0)
            }
        }
    }

    private var split: (String, String)? {
        guard let range = line.range(of: ": ") else { return nil }
        return (String(line[..<range.lowerBound]), String(line[range.upperBound...]))
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

/// An annotation or status badge, in Carbide's words.
private struct AnnotationRow: View {
    let annotations: [String]
    var badges: [String] = []

    var body: some View {
        if !annotations.isEmpty || !badges.isEmpty {
            HStack(spacing: 6) {
                ForEach(annotations, id: \.self) { Plate($0, fill: Tone.record) }
                ForEach(badges, id: \.self) { badge in
                    Label(badge, systemImage: "hourglass")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(Tone.mined)
                        .help(badge == "Immature"
                            ? "A coinbase output cannot be spent until it is 100 blocks deep."
                            : "An output can stake only after 150 confirmations.")
                }
            }
        }
    }
}

private struct InputRow: View {
    let input: TransactionInput
    let spentInBlock: Bool
    let showDates: Bool

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            IndexLabel(index: input.index)
            VStack(alignment: .leading, spacing: 4) {
                if input.coinbase {
                    Plate("Coinbase", fill: Tone.mined)
                } else if let pool = input.shieldedPool {
                    Text("Shielded pool · \(pool)").foregroundStyle(.secondary)
                    Text("The address is shielded and not known.").font(.callout).foregroundStyle(.secondary)
                }
                ForEach(input.addresses, id: \.self) { address in
                    RouteLink(text: address, route: .holder(address), shorten: false)
                }
                if !input.coinbase, input.shieldedPool == nil, input.addresses.isEmpty {
                    Text("Address unavailable").foregroundStyle(.secondary)
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
                details
                AnnotationRow(annotations: input.annotations.filter { !$0.hasPrefix("Derived from ") })
            }
            Spacer(minLength: 8)
            VStack(alignment: .trailing, spacing: 2) {
                ValueText(sats: input.valueSats, derived: input.derivedValue, shielded: input.shieldedPool != nil)
                ForEach(input.reserveAmounts, id: \.currency) { ReserveAmount(amount: $0) }
            }
        }
        .padding(.vertical, 8)
    }

    /// Script type, sequence and the confirmations the spent output had.
    private var details: some View {
        VStack(alignment: .leading, spacing: 2) {
            let parts = [
                input.scriptType.map { "script \($0)" },
                input.sequence.map { "sequence \($0)" },
            ].compactMap { $0 }
            if !parts.isEmpty {
                Text(parts.joined(separator: " · "))
            }
            if input.previousTxid != nil {
                Text(spentContext)
            }
        }
        .font(.callout)
        .foregroundStyle(.secondary)
    }

    private var spentContext: String {
        let time = input.sourceBlockTime.map { "Block time \(Format.date($0))" } ?? "Block time unknown"
        if !spentInBlock {
            return "\(time) · confirmations when spent: not yet confirmed"
        }
        if showDates, let time = input.sourceBlockTime {
            return "Confirmed since \(Format.date(time)) when spent"
                + (input.sourceHeight.map { " · height \(Format.integer($0))" } ?? "")
        }
        let count = input.confirmationsWhenSpent.map(String.init) ?? "unknown"
        return "\(time) · confirmations when spent \(count)"
            + (input.sourceHeight.map { " · height \(Format.integer($0))" } ?? "")
    }
}

/// A native value: exact, derived from the shielded value balance, or hidden.
private struct ValueText: View {
    let sats: Int64?
    let derived: Bool
    let shielded: Bool

    var body: some View {
        if let sats {
            VStack(alignment: .trailing, spacing: 0) {
                CoinAmount(sats: sats)
                if derived {
                    Text("Derived from Sapling valueBalance").font(.caption).foregroundStyle(.secondary)
                }
            }
        } else if shielded {
            Text("Hidden").foregroundStyle(.secondary)
                .help("A shielded value is private. Carbide shows it only when the value balance proves it.")
        } else {
            Text("Value unavailable").foregroundStyle(.secondary)
        }
    }
}

private struct OutputRow: View {
    let output: TransactionOutput
    let txid: String
    /// A verified registration payment annotation, such as "Direct Referral Payment".
    var paymentAnnotation: String?

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @State private var showingJSON = false
    @State private var showingBytes = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                IndexLabel(index: output.index)
                VStack(alignment: .leading, spacing: 4) {
                    if let pool = output.shieldedPool {
                        Text("Shielded pool · \(pool)").foregroundStyle(.secondary)
                        Text("The address is shielded and not known.").font(.callout).foregroundStyle(.secondary)
                    } else if output.addresses.isEmpty {
                        Text("Address unavailable").foregroundStyle(.secondary)
                    }
                    ForEach(output.addresses, id: \.self) { address in
                        if let routing = output.routing.first(where: { $0.label == address }) {
                            Text(routing.value).font(.callout).foregroundStyle(.secondary)
                        }
                        RouteLink(text: address, route: .holder(address), shorten: false)
                    }
                    details
                    tags
                    AnnotationRow(
                        annotations: output.annotations.filter { !$0.hasPrefix("Derived from ") }
                            + [paymentAnnotation].compactMap { $0 },
                        badges: output.badges)
                    ForEach(Array(output.references.enumerated()), id: \.offset) { _, reference in
                        ReferenceRow(reference: reference, showingBytes: $showingBytes)
                    }
                    ForEach(output.referenceNotes, id: \.self) { note in
                        Text(note).font(.callout).foregroundStyle(.secondary)
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
                    ValueText(sats: output.valueSats, derived: output.derivedValue, shielded: output.shieldedPool != nil)
                    ForEach(output.reserveAmounts, id: \.currency) { ReserveAmount(amount: $0) }
                }
                if !output.json.isEmpty {
                    Button {
                        showingJSON.toggle()
                    } label: {
                        Image(systemName: "curlybraces")
                    }
                    .buttonStyle(.borderless)
                    .help("Show the JSON of this output")
                    .accessibilityLabel("Show JSON for output \(output.index)")
                }
            }
            if showingBytes, let bytes = bytesReference {
                BytesView(hex: bytes.hex, scriptBody: bytes.scriptBody).padding(.leading, 40)
            }
            if showingJSON {
                JSONTreeView(json: output.json)
                    .padding(10)
                    .background(Tone.screen, in: Rectangle())
                    .overlay(Rectangle().strokeBorder(Tone.rule, lineWidth: 1))
                    .padding(.leading, 40)
            }
        }
        .padding(.vertical, 8)
    }

    private var bytesReference: (hex: String, scriptBody: Bool)? {
        for reference in output.references {
            if case .bytes(let hex, let scriptBody) = reference.target { return (hex, scriptBody) }
        }
        return nil
    }

    /// Value, script type and whether the output type can be spent.
    private var details: some View {
        let parts = [
            output.scriptType.map { "script \($0)" },
            output.spendable.map { $0 ? "spendable output type" : "protocol output type" },
        ].compactMap { $0 }
        return Group {
            if !parts.isEmpty {
                Text(parts.joined(separator: " · ")).font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    /// Carbide's output tags. A DeFi tag names its currency. A tag that opens something is a link.
    private var tags: some View {
        HStack(spacing: 6) {
            ForEach(Array(output.tagDetails.enumerated()), id: \.offset) { _, tag in
                if let target = tag.target {
                    Button {
                        open(target)
                    } label: {
                        Plate(tag.label + " ↗")
                    }
                    .buttonStyle(.plain)
                    .help("Open output \(target.output) of \(Format.short(target.txid))")
                } else if let currency = tag.currencyId {
                    Button {
                        model.open(.currency(name: currency))
                    } label: {
                        Plate("\(tag.label): \(currencyName(currency))")
                    }
                    .buttonStyle(.plain)
                    .help(currency)
                } else {
                    Plate(tag.label)
                        .help(tag.traceable ? "The transfer trace for this output is above." : "")
                }
            }
        }
    }

    private func currencyName(_ id: String) -> String {
        guard showsNames, let name = model.names[id] else { return Format.short(id, head: 6, tail: 4) }
        return name.hasSuffix("@") ? String(name.dropLast()) : name
    }

    private func open(_ target: Outpoint) {
        if target.systemId == model.status?.chainId || model.status?.chainId == nil {
            model.open(.transaction(txid: target.txid))
        }
    }
}

/// One explicit reference from an output: a currency, a destination, another output, a
/// notarized block, or the raw bytes of unrecognised data.
private struct ReferenceRow: View {
    let reference: OutputReference
    @Binding var showingBytes: Bool

    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            Text("\(reference.label):").foregroundStyle(.secondary)
            switch reference.target {
            case .currency(let id):
                RouteLink(text: id, route: .currency(name: id))
            case .destination(let address, let system):
                RouteLink(text: address, route: .holder(address))
                if let system, system != model.status?.chainId {
                    Text("on").foregroundStyle(.secondary)
                    RouteLink(text: system, route: .currency(name: system))
                }
            case .output(let outpoint):
                if outpoint.systemId == model.status?.chainId {
                    RouteLink(text: outpoint.txid, route: .transaction(txid: outpoint.txid))
                } else {
                    Text(Format.short(outpoint.txid)).font(.body.monospaced()).textSelection(.enabled)
                }
                Text("output \(outpoint.output) on").foregroundStyle(.secondary)
                RouteLink(text: outpoint.systemId, route: .currency(name: outpoint.systemId))
            case .block(let hash, let height):
                RouteLink(text: "#\(Format.integer(UInt64(height))) \(Format.short(hash))", route: .block(id: hash), shorten: false)
                    .help(hash)
            case .bytes(let hex, let scriptBody):
                Button(showingBytes ? "Hide bytes" : "View \(hex.count / 2) bytes (\(scriptBody ? "script bytes" : "payload"))") {
                    showingBytes.toggle()
                }
                .buttonStyle(.displayLink)
            }
        }
        .font(.callout)
    }
}
