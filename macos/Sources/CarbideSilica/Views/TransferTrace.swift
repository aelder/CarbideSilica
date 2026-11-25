import CarbideCore
import SwiftUI

/// One mark for each of the four levels of confidence, used wherever a fact's standing matters.
/// Shape, colour and word together, so it never depends on colour alone.
struct ConfidenceMark: View {
    let level: Confidence
    var label: String?

    var body: some View {
        Label(label ?? word, systemImage: symbol)
            .font(.callout.weight(.semibold))
            .foregroundStyle(color)
            .help(meaning)
    }

    private var word: String {
        switch level {
        case .verified: "Verified"
        case .observed: "In progress"
        case .candidate: "Candidate"
        case .unknown: "Unresolved"
        }
    }

    private var symbol: String {
        switch level {
        case .verified: "checkmark.seal.fill"
        case .observed: "circle.lefthalf.filled"
        case .candidate: "circle.dashed"
        case .unknown: "questionmark.circle"
        }
    }

    private var color: Color {
        switch level {
        case .verified: .green
        case .observed: .accentColor
        case .candidate: .secondary
        case .unknown: .orange
        }
    }

    private var meaning: String {
        switch level {
        case .verified: "Proven by exact match or replay."
        case .observed: "A source reported this at a known block. The trace is not complete."
        case .candidate: "An estimate. Not confirmed."
        case .unknown: "The data is not available."
        }
    }
}

/// Follows one transfer from its source output as far as the evidence reaches, using
/// Carbide's DeFi trail (the terminal's F8). It shows one verdict, then a receipt, the
/// stages on an axis of blocks, and the place of the transfer in its export batch.
///
/// While the search runs, the card shows the trail so far. For a cross-system import, the
/// search can jump to a destination height, step through adjacent 1,000-block windows, or
/// scan up to 32 older windows.
struct TransferTraceCard: View {
    let txid: String
    let root: TraceRoot
    /// Incremented by the page to start the trace, from a menu command or another page.
    var startRequests = 0

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @State private var trace: TransferTrace?
    @State private var progress: TransferTrace?
    @State private var failure: String?
    @State private var running = false
    @State private var showEvidence = false
    @State private var jumpHeight = ""

    var body: some View {
        Card("Transfer trace", trailing: "\(root.label) · output \(root.output)") {
            if let shown = progress ?? trace {
                content(shown, live: progress != nil)
            } else if running {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Tracing the transfer…").foregroundStyle(.secondary)
                }
            }
            if let failure {
                Note(text: failure, warning: true)
            }
            if trace == nil, !running {
                HStack {
                    Text(prompt).foregroundStyle(.secondary)
                    Spacer()
                    Button("Trace") { Task { await run(.initial, target: nil) } }
                        .keyboardShortcut(.defaultAction)
                }
            }
        }
        .onAppear {
            if startRequests > 0, trace == nil, !running { Task { await run(.initial, target: nil) } }
        }
        .onChange(of: startRequests) {
            if trace == nil, !running { Task { await run(.initial, target: nil) } }
        }
    }

    private var prompt: String {
        switch root.label {
        case "Fee pool checkpoint":
            "Compare this fee pool checkpoint with the adjacent coinbase checkpoints."
        case "Currency import":
            "Open this import's source export and the outputs it generated."
        default:
            "Trace this transfer through its export, its import and its result."
        }
    }

    @ViewBuilder private func content(_ trace: TransferTrace, live: Bool) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            if live {
                ProgressView().controlSize(.small)
                Text("Searching. This is the trail so far.").foregroundStyle(.secondary)
            } else {
                ConfidenceMark(level: trace.confidence)
                Text(trace.verdict).font(.title3)
            }
            Spacer()
            if !live {
                Button("Trace again") { Task { await run(.initial, target: nil) } }
                    .disabled(running)
            }
        }
        if !trace.legs.isEmpty {
            TraceRoute(trace: trace, live: live)
        }

        ForEach(Array(trace.legs.enumerated()), id: \.offset) { index, leg in
            LegDetail(
                leg: leg, number: trace.legs.count > 1 ? index + 1 : nil,
                txid: txid, output: root.output, legIndex: UInt32(index), live: live)
        }

        if let stop = trace.stop {
            Note(text: stop, warning: true)
        }
        if let search = trace.search {
            searchControls(search, trace: trace, live: live)
        }

        DisclosureGroup("Evidence (\(trace.steps.count) step\(trace.steps.count == 1 ? "" : "s"))", isExpanded: $showEvidence) {
            VStack(alignment: .leading, spacing: 10) {
                ForEach(Array(trace.steps.enumerated()), id: \.offset) { _, step in
                    VStack(alignment: .leading, spacing: 3) {
                        HStack(spacing: 8) {
                            Text(step.label).fontWeight(.semibold)
                            if step.systemId == model.status?.chainId {
                                RouteLink(text: step.txid, route: .transaction(txid: step.txid))
                            } else {
                                Text(Format.short(step.txid)).font(.body.monospaced()).textSelection(.enabled)
                                    .help(step.txid)
                            }
                            Text("output \(step.output) on").foregroundStyle(.secondary)
                            RouteLink(text: step.systemId, route: .currency(name: step.systemId))
                        }
                        Text(step.detail)
                            .font(.caption.monospaced())
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
                ForEach(trace.notes, id: \.self) { note in
                    Text(note).font(.caption).foregroundStyle(.secondary)
                }
            }
            .padding(.top, 8)
        }
        .onAppear { if trace.legs.isEmpty { showEvidence = true } }
    }

    /// The searched range, and the controls to search other windows of the destination chain.
    @ViewBuilder private func searchControls(_ search: TraceSearch, trace: TransferTrace, live: Bool) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(
                "Searched heights \(Format.integer(search.first)) to \(Format.integer(search.last)) of \(systemName(search.systemId)) "
                    + "in \(search.windows) window\(search.windows == 1 ? "" : "s"). The tip is \(Format.integer(search.tip))."
            )
            .font(.callout)
            if trace.confidence != .verified {
                Note(text: "A missing import in the searched windows is not proof that none exists.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            if !live, trace.confidence != .verified {
                HStack(spacing: 10) {
                    Button("Earlier window") {
                        Task { await run(.windowEndingAt(height: search.first > 0 ? search.first - 1 : 0), target: trace.searchRoot, trace: trace) }
                    }
                    .disabled(running || search.first == 0)
                    .help("Search the 1,000 blocks before this range.")
                    Button("Later window") {
                        let end = min(search.last + 1000, search.tip)
                        Task { await run(.windowEndingAt(height: end), target: trace.searchRoot, trace: trace) }
                    }
                    .disabled(running || search.last >= search.tip)
                    .help("Search the 1,000 blocks after this range.")
                    if search.moreOlder {
                        Button("Scan older windows") {
                            Task { await run(.backwardFrom(height: search.first), target: trace.searchRoot, trace: trace) }
                        }
                        .disabled(running)
                        .help("Search up to 32 older 1,000-block windows. Select it again to continue.")
                    }
                    Divider().frame(height: 18)
                    TextField("Destination height", text: $jumpHeight)
                        .textFieldStyle(.roundedBorder)
                        .frame(width: 150)
                        .onSubmit { jump(trace) }
                    Button("Go") { jump(trace) }
                        .disabled(running || UInt64(jumpHeight.filter(\.isNumber)) == nil)
                        .help("Search the 1,000-block window that ends at this height.")
                }
                .controlSize(.small)
            }
        }
    }

    private func jump(_ trace: TransferTrace) {
        guard let height = UInt64(jumpHeight.filter(\.isNumber)) else { return }
        Task { await run(.windowEndingAt(height: height), target: trace.searchRoot, trace: trace) }
    }

    private func systemName(_ id: String) -> String {
        guard showsNames, let name = model.names[id] else { return Format.short(id, head: 6, tail: 4) }
        return name.hasSuffix("@") ? String(name.dropLast()) : name
    }

    /// Runs the trace. A search of one leg keeps the other legs' import heights.
    private func run(_ request: TraceRequest, target: Outpoint?, trace previous: TransferTrace? = nil) async {
        guard !running else { return }
        running = true
        failure = nil
        progress = nil
        defer {
            running = false
            progress = nil
        }
        let explorer: Explorer
        do {
            explorer = try model.requireExplorer()
        } catch {
            failure = describe(error)
            return
        }
        let token = UInt64.random(in: 1...UInt64.max)
        let poll = Task { @MainActor in
            while !Task.isCancelled {
                if let partial = explorer.traceProgress(token: token) {
                    progress = partial
                }
                try? await Task.sleep(for: .milliseconds(400))
            }
        }
        defer { poll.cancel() }
        let plan = TracePlan(request: request, target: target, knownImports: previous?.importHeights ?? [])
        do {
            let result = try await explorer.traceTransfer(txid: txid, output: root.output, plan: plan, token: token)
            poll.cancel()
            trace = result
            var ids = result.legs.flatMap(\.currencies)
            ids += result.steps.map(\.systemId)
            ids += result.search.map { [$0.systemId] } ?? []
            await model.resolveNames(ids)
        } catch {
            failure = describe(error)
        }
    }
}

/// What one leg of the transfer asked for and what it produced, with its fee budgets kept
/// apart. Budgets are never added across legs: the next leg spends the same funds.
private struct LegDetail: View {
    let leg: TraceLeg
    let number: Int?
    let txid: String
    let output: UInt32
    let legIndex: UInt32
    let live: Bool

    @Environment(AppModel.self) private var model
    @State private var comparison: PoolComparison?
    @State private var comparing = false
    @State private var comparisonFailure: String?
    @State private var showCurrencies = false
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                if let number {
                    Text("Leg \(number)").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                }
                ConfidenceMark(level: leg.confidence, label: leg.stage)
                Spacer()
                if leg.sourceSystem == model.status?.chainId, leg.sourceTxid != txid {
                    RouteLink(text: leg.sourceTxid, route: .transaction(txid: leg.sourceTxid))
                }
            }
            HStack(alignment: .top, spacing: 28) {
                column("Sent", amounts: leg.principal, unknown: !leg.principalKnown)
                if let pool = leg.pool {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Through").font(.caption).foregroundStyle(.secondary)
                        RouteLink(text: pool, route: .currency(name: pool))
                    }
                }
                if let result = leg.result {
                    column(result, amounts: leg.resultAmounts)
                } else if let burned = leg.burned {
                    column("Burned", amounts: [burned])
                } else {
                    VStack(alignment: .leading, spacing: 2) {
                        Text(leg.stage.contains("burn") ? "Burned" : "Received").font(.caption).foregroundStyle(.secondary)
                        Text("Unresolved").foregroundStyle(.secondary)
                    }
                }
                if let height = leg.importHeight {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Imported at").font(.caption).foregroundStyle(.secondary)
                        if leg.executionSystem == model.status?.chainId {
                            RouteLink(text: "block \(Format.integer(height))", route: .block(id: "\(height)"), shorten: false)
                        } else {
                            Text("height \(Format.integer(height))")
                        }
                    }
                }
            }

            budgets

            if let batch = leg.batchBurned {
                amountLine("Batch supply reduction", batch)
            }
            if !leg.proceeds.isEmpty {
                HStack(spacing: 6) {
                    Text("Currency proceeds").foregroundStyle(.secondary)
                    ForEach(leg.proceeds, id: \.currency) { ReserveAmount(amount: $0) }
                    if let outpoint = leg.proceedsOutput {
                        Text("at output \(outpoint.output) (currency identity)").foregroundStyle(.secondary)
                    }
                }
                .font(.callout)
            }
            if let txid = leg.resultTxid, let output = leg.resultOutput {
                HStack(spacing: 6) {
                    Text("\(leg.result ?? "Result") output").foregroundStyle(.secondary)
                    if leg.resultSystem == model.status?.chainId {
                        RouteLink(text: txid, route: .transaction(txid: txid))
                    } else {
                        Text(Format.short(txid)).font(.body.monospaced()).textSelection(.enabled).help(txid)
                    }
                    Text("output \(output)").foregroundStyle(.secondary)
                    if let system = leg.resultSystem {
                        Text("on").foregroundStyle(.secondary)
                        RouteLink(text: system, route: .currency(name: system))
                    }
                }
                .font(.callout)
            }

            ForEach(Array(leg.rates.enumerated()), id: \.offset) { _, rate in
                RateLines(rate: rate)
            }

            HStack(spacing: 14) {
                Button(showCurrencies ? "Hide currencies" : "Currencies (\(leg.currencies.count))") {
                    showCurrencies.toggle()
                }
                .buttonStyle(.link)
                if let pool = leg.pool {
                    Button("Open pool") { openPool(pool) }
                        .buttonStyle(.link)
                        .help(poolHelp)
                }
                if leg.poolInspection?.previous != nil, !live {
                    Button(comparing ? "Comparing…" : "Compare with the previous pool notarization") {
                        Task { await compare() }
                    }
                    .buttonStyle(.link)
                    .disabled(comparing)
                }
            }
            .font(.callout)
            if showCurrencies {
                VStack(alignment: .leading, spacing: 3) {
                    ForEach(leg.currencies, id: \.self) { id in
                        HStack(spacing: 6) {
                            Text(id == leg.pool ? "Pool" : "Currency").foregroundStyle(.secondary).frame(width: 64, alignment: .leading)
                            RouteLink(text: id, route: .currency(name: id))
                            Text(id).font(.caption.monospaced()).foregroundStyle(.tertiary)
                        }
                    }
                }
                .font(.callout)
            }
            if let comparisonFailure {
                Note(text: comparisonFailure, warning: true).font(.callout)
            }
            if let comparison {
                PoolComparisonView(comparison: comparison)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 8))
    }

    /// The pool at the import height when it is on the connected chain, else its current state.
    private func openPool(_ pool: String) {
        if let height = leg.importHeight, leg.executionSystem == model.status?.chainId {
            model.openCurrency(pool, at: height)
        } else {
            model.open(.currency(name: pool))
        }
    }

    private var poolHelp: String {
        if let height = leg.importHeight, leg.executionSystem == model.status?.chainId {
            return "Opens the pool's state at height \(Format.integer(height)), where this leg executed."
        }
        var text = "Opens the pool's current state. The pool is on another chain."
        if let height = leg.importHeight {
            text += " This leg executed at height \(Format.integer(height))"
            if let system = leg.executionSystem { text += " on \(Display.name(for: system, in: model, currency: true))" }
            text += "."
        }
        return text
    }

    /// Instruction principal, transfer custody and fees, each on its own line.
    @ViewBuilder private var budgets: some View {
        VStack(alignment: .leading, spacing: 3) {
            if let budget = leg.transferFeeBudget {
                amountLine("Fee budget", budget)
            } else {
                HStack { Text("Fee budget").foregroundStyle(.secondary).frame(width: 170, alignment: .leading); Text("Unknown").foregroundStyle(.secondary) }
            }
            if let fee = leg.conversionFee { amountLine("Conversion fee", fee) }
            if let budget = leg.forwardingBudget { amountLine("Forward budget", budget) }
            if let fee = leg.forwardingConversionFee { amountLine("Forward conversion fee", fee) }
            if let budget = leg.nextFeeBudget {
                amountLine("Next fee budget", budget, note: "carried into the next instruction")
            }
        }
        .font(.callout)
    }

    private func amountLine(_ label: String, _ amount: CurrencyAmount, note: String? = nil) -> some View {
        HStack(spacing: 6) {
            Text(label).foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
            ReserveAmount(amount: amount)
            if let note { Text("(\(note))").foregroundStyle(.secondary) }
        }
    }

    private func column(_ title: String, amounts: [CurrencyAmount], unknown: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            if amounts.isEmpty {
                Text(unknown ? "Unknown" : "–").foregroundStyle(.secondary)
            }
            ForEach(amounts, id: \.currency) { amount in
                HStack(spacing: 5) {
                    Text(Format.amount(amount.amount)).monospacedDigit().fontWeight(.medium)
                    RouteLink(text: amount.currency, route: .currency(name: amount.currency))
                }
            }
        }
    }

    private func compare() async {
        comparing = true
        comparisonFailure = nil
        defer { comparing = false }
        do {
            let found = try await model.requireExplorer().poolComparison(txid: txid, output: output, leg: legIndex)
            comparison = found
            await model.resolveNames(found.changes.map(\.label))
        } catch {
            comparisonFailure = describe(error)
        }
    }
}

/// The net amounts of a conversion, its effective rate and the import batch's named prices.
private struct RateLines: View {
    let rate: TraceRate
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(spacing: 6) {
                Text("\(rate.purpose) net").foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
                ReserveAmount(amount: rate.netInput)
                Image(systemName: "arrow.right").foregroundStyle(.secondary)
                ReserveAmount(amount: rate.output)
            }
            if let sats = rate.rateSats {
                HStack(spacing: 6) {
                    Text("Execution rate").foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
                    Text("\(rate.exact ? "=" : "≈") \(Format.coins(sats)) \(name(rate.output.currency)) per \(name(rate.netInput.currency))")
                        .monospacedDigit()
                }
            }
            ForEach(Array(rate.prices.enumerated()), id: \.offset) { _, price in
                HStack(spacing: 6) {
                    Text("Import batch").foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
                    Text("\(name(price.currency)) \(price.field) = \(Format.amount(price.value))").monospacedDigit()
                }
            }
            if !rate.prices.isEmpty {
                Text("Import batch prices are the batch builder's fields. They are not market quotes.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .font(.callout)
    }

    private func name(_ id: String) -> String {
        guard showsNames, let name = model.names[id] else { return Format.short(id, head: 6, tail: 4) }
        return name.hasSuffix("@") ? String(name.dropLast()) : name
    }
}

/// The pool's supply and reserves at the linked previous notarization and at the import.
private struct PoolComparisonView: View {
    let comparison: PoolComparison
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Pool states").font(.headline)
            Text("Pool-wide changes between notarizations include other transfers and fees.")
                .font(.caption).foregroundStyle(.secondary)
            if let previous = comparison.previous {
                snapshot("Previous notarization", previous)
            }
            snapshot("Import state", comparison.imported)
            if let note = comparison.note {
                Note(text: note, warning: true).font(.callout)
            }
            Grid(alignment: .trailing, horizontalSpacing: 18, verticalSpacing: 4) {
                GridRow {
                    Text("").gridColumnAlignment(.leading)
                    Text("Before").foregroundStyle(.secondary)
                    Text("After").foregroundStyle(.secondary)
                    Text("Change").foregroundStyle(.secondary)
                }
                ForEach(Array(comparison.changes.enumerated()), id: \.offset) { _, change in
                    GridRow {
                        Group {
                            if change.isSupply {
                                Text("Supply")
                            } else {
                                RouteLink(text: change.label, route: .currency(name: change.label))
                            }
                        }
                        .gridColumnAlignment(.leading)
                        value(change.before)
                        value(change.after)
                        value(change.change, signed: true)
                    }
                }
            }
            .font(.callout.monospacedDigit())
            if comparison.moreReserves {
                Text("The first 12 reserves are shown. Open the pool for the complete state.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(.top, 4)
    }

    private func value(_ number: Double?, signed: Bool = false) -> some View {
        Text(number.map { (signed && $0 > 0 ? "+" : "") + Format.amount($0) } ?? "Unavailable")
            .foregroundStyle(number == nil ? .secondary : .primary)
    }

    private func snapshot(_ label: String, _ snapshot: PoolSnapshot) -> some View {
        HStack(spacing: 6) {
            Text(label).foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
            if snapshot.output.systemId == model.status?.chainId {
                RouteLink(text: snapshot.output.txid, route: .transaction(txid: snapshot.output.txid))
            } else {
                Text(Format.short(snapshot.output.txid)).font(.body.monospaced())
            }
            Text("output \(snapshot.output.output)").foregroundStyle(.secondary)
            if let height = snapshot.height {
                Text("at height \(Format.integer(height))").foregroundStyle(.secondary)
            }
        }
        .font(.callout)
    }
}
