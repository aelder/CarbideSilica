import CarbideCore
import SwiftUI

/// The inputs and outputs of one party, for the marks in the lists below the statement.
struct StatementSelection: Equatable {
    let id: String
    let inputs: Set<UInt32>
    let outputs: Set<UInt32>
}

/// One line of the statement.
struct StatementRow: Identifiable {
    enum Style {
        case party, hidden, rest, closing, unchanged
    }

    let id: String
    let title: String
    /// The address or identity that the title opens.
    let address: String?
    let style: Style
    /// The net change in each shown column, in satoshis. Negative is given.
    let cells: [Int64]
    let inputs: [UInt32]
    let outputs: [UInt32]
}

/// A transaction as a statement: one sentence, then who ended with less and who ended
/// with more in each currency.
///
/// The bridge groups the inputs and outputs by party and calculates the net change. This
/// type selects what the page shows: five receivers at most, five currencies at most, and
/// the words of the sentence. The chain does not record which input paid which output,
/// thus the statement has no line from one to the other.
struct StatementLayout {
    static let maxReceivers = 8
    static let keptReceivers = 5
    static let maxColumns = 5

    let columns: [String?]
    let rows: [StatementRow]
    /// The largest amount in each shown column. The bars of a column share this scale.
    let scale: [Int64]
    /// The net amounts of all receivers in the first column, largest first, when the
    /// statement joins some of them into one row.
    let spread: [Int64]
    /// The receivers that the statement joins into one row, largest first, one row each.
    let joined: [StatementRow]

    init(statement: TransactionStatement, only: String??, title: (StatementParty) -> String) {
        let all = statement.columns
        let columns: [String?] = if let only { [only] } else { Array(all.prefix(Self.maxColumns)) }
        self.columns = columns

        func cells(_ amounts: [StatementAmount]) -> [Int64] {
            columns.map { column in amounts.first { $0.currencyId == column }?.sats ?? 0 }
        }
        func row(_ party: StatementParty, _ position: Int) -> StatementRow {
            StatementRow(
                id: "party-\(position)", title: title(party),
                address: party.kind == .address ? party.addresses.first : nil,
                style: party.net.isEmpty ? .unchanged : party.kind == .shieldedPool ? .hidden : .party,
                cells: cells(party.net), inputs: party.inputs, outputs: party.outputs)
        }

        let parties = Array(statement.parties.enumerated())
        let givers = parties.filter { $0.element.net.contains { $0.sats < 0 } }
        let unchanged = parties.filter { $0.element.net.isEmpty }
        let allReceivers = parties.filter { entry in
            !entry.element.net.isEmpty && !givers.contains { $0.offset == entry.offset }
        }

        // Only addresses are joined into one row. The reserve deposits and the shielded
        // pool keep their own rows.
        let others = allReceivers.filter { $0.element.kind != .address }
        let receivers = allReceivers.filter { $0.element.kind == .address }
        var rows = givers.map { row($0.element, $0.offset) }
        var spread: [Int64] = []
        var joined: [StatementRow] = []
        if receivers.count > Self.maxReceivers {
            let first = columns.first ?? nil
            let amount = { (party: StatementParty) in party.net.first { $0.currencyId == first }?.sats ?? 0 }
            let sorted = receivers.sorted { amount($0.element) > amount($1.element) }
            rows += sorted.prefix(Self.keptReceivers).map { row($0.element, $0.offset) }
            let rest = sorted.dropFirst(Self.keptReceivers)
            var sums: [String?: Int64] = [:]
            for entry in rest {
                for amount in entry.element.net { sums[amount.currencyId, default: 0] += amount.sats }
            }
            rows.append(
                StatementRow(
                    id: "rest", title: "\(rest.count) more addresses", address: nil, style: .rest,
                    cells: columns.map { sums[$0] ?? 0 },
                    inputs: rest.flatMap(\.element.inputs), outputs: rest.flatMap(\.element.outputs)))
            spread = sorted.map { amount($0.element) }.filter { $0 > 0 }
            joined = rest.map { row($0.element, $0.offset) }
        } else {
            rows += receivers.map { row($0.element, $0.offset) }
        }
        rows += others.map { row($0.element, $0.offset) }
        if statement.closingKind != .none {
            rows.append(
                StatementRow(
                    id: "closing",
                    title: statement.closingKind == .fee ? "Fee" : "Inputs less outputs",
                    address: nil, style: .closing, cells: cells(statement.closing), inputs: [], outputs: []))
        }
        rows += unchanged.map { row($0.element, $0.offset) }
        // A row with no amount in the shown columns says nothing.
        self.rows = rows.filter { $0.style == .unchanged || $0.cells.contains { $0 != 0 } }
        self.scale = columns.indices.map { column in
            max(rows.map { abs($0.cells[column]) }.max() ?? 1, 1)
        }
        self.spread = spread
        self.joined = joined
    }
}

enum StatementText {
    /// An amount with as many decimals as its size needs. The full amount is in the help text.
    static func brief(_ sats: Int64) -> String {
        let value = Decimal(abs(sats)) / Decimal(100_000_000)
        let digits = abs(sats) >= 100_000_000_000 ? 2 : abs(sats) >= 100_000_000 ? 4 : 8
        return value.formatted(.number.precision(.fractionLength(0...digits)))
    }

    static func signed(_ sats: Int64) -> String {
        (sats < 0 ? "−" : "+") + brief(sats)
    }

    static func list(_ parts: [String]) -> String {
        switch parts.count {
        case 0: ""
        case 1: parts[0]
        default: parts.dropLast().joined(separator: ", ") + " and " + parts[parts.count - 1]
        }
    }

    static func capital(_ text: String) -> String {
        text.prefix(1).uppercased() + text.dropFirst()
    }
}

struct TransactionStatementView: View {
    let transaction: TransactionDetail
    @Binding var selection: StatementSelection?
    /// Scrolls the page to the row or card with this ID.
    var scroll: (String) -> Void
    /// Starts the transfer trace of an output.
    var follow: (UInt32) -> Void

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    /// The one currency to show, when the statement has more columns than fit.
    @State private var only: String??
    /// True shows the statement as a flow picture, false as the table. A test.
    @AppStorage("statementShowsFlow") private var showsFlow = false
    /// The column whose amounts the flow picture shows.
    @State private var flowColumn = 0

    private var statement: TransactionStatement { transaction.statement }
    private var coin: String { transaction.coin }

    var body: some View {
        let layout = StatementLayout(statement: statement, only: only, title: rowTitle)
        VStack(alignment: .leading, spacing: 12) {
            Text(sentence)
                .font(.title3)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if statement.columns.count > StatementLayout.maxColumns {
                HStack(spacing: 10) {
                    NameText("Currency")
                    LampMenu(
                        options: [(String??.none, "First \(StatementLayout.maxColumns) of \(statement.columns.count)")]
                            + statement.columns.map { (String??.some($0), currency($0)) },
                        selection: $only)
                }
            }
            if !layout.rows.isEmpty {
                HStack(spacing: 14) {
                    LampSegments(options: [(false, "Table"), (true, "Flow")], selection: $showsFlow)
                    if showsFlow, layout.columns.count > 1 {
                        LampSegments(
                            options: layout.columns.enumerated().map { ($0.offset, currency($0.element)) },
                            selection: $flowColumn)
                    }
                }
            }
            if !layout.rows.isEmpty || !layout.spread.isEmpty {
                DisplayPanel {
                    VStack(alignment: .leading, spacing: 12) {
                        if !layout.rows.isEmpty {
                            if showsFlow {
                                StatementFlow(
                                    layout: layout, column: min(flowColumn, max(layout.columns.count - 1, 0)),
                                    unit: currency(layout.columns[safe: min(flowColumn, max(layout.columns.count - 1, 0))] ?? nil),
                                    selection: $selection)
                            } else {
                                grid(layout)
                            }
                        }
                        if !layout.rows.isEmpty, !layout.spread.isEmpty {
                            PanelRule()
                        }
                        if !layout.spread.isEmpty {
                            spread(layout)
                        }
                    }
                }
            }
            if let selection, !(selection.inputs.isEmpty && selection.outputs.isEmpty) {
                HStack(spacing: 8) {
                    Text(marked(selection)).font(.callout).foregroundStyle(.secondary)
                    Button("Show in the Lists") {
                        if let input = selection.inputs.min() {
                            scroll("input-\(input)")
                        } else if let output = selection.outputs.min() {
                            scroll("output-\(output)")
                        }
                    }
                    .buttonStyle(.displayLink)
                }
            }
            routes
            carried
            marks
            if statement.closingKind == .inputsLessOutputs {
                Text(
                    "Protocol outputs account for value in other transactions. "
                        + "The row \"Inputs less outputs\" is not a fee.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if !statement.complete {
                Note(text: "A value is missing from the source. The statement is not complete.", warning: true)
            }
        }
        .onChange(of: transaction.txid) {
            only = nil
            flowColumn = 0
        }
    }

    // MARK: The grid

    private func grid(_ layout: StatementLayout) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            if !layout.columns.isEmpty {
                HStack(spacing: 0) {
                    Color.clear.frame(width: Self.labelWidth, height: 1)
                    ForEach(Array(layout.columns.enumerated()), id: \.offset) { _, column in
                        NameText(currency(column))
                            .lineLimit(1)
                            .frame(maxWidth: .infinity)
                    }
                }
                .padding(.bottom, 4)
            }
            ForEach(Array(layout.rows.enumerated()), id: \.element.id) { position, row in
                if position > 0 { PanelRule() }
                line(row, layout)
            }
        }
    }

    private static let labelWidth: CGFloat = 230

    private func line(_ row: StatementRow, _ layout: StatementLayout) -> some View {
        let chosen = selection?.id == row.id
        let selectable = !(row.inputs.isEmpty && row.outputs.isEmpty)
        return HStack(spacing: 0) {
            Group {
                if let address = row.address {
                    RouteLink(text: address, route: .holder(address))
                } else {
                    Text(row.title)
                        .foregroundStyle(row.style == .closing || row.style == .rest ? Tone.label : Tone.value)
                        .lineLimit(1)
                }
            }
            .frame(width: Self.labelWidth, alignment: .leading)
            Button {
                selection = chosen
                    ? nil
                    : StatementSelection(id: row.id, inputs: Set(row.inputs), outputs: Set(row.outputs))
            } label: {
                HStack(spacing: 0) {
                    if row.style == .unchanged {
                        NameText("No change")
                            .frame(maxWidth: .infinity)
                    } else {
                        ForEach(Array(row.cells.enumerated()), id: \.offset) { column, sats in
                            cell(
                                sats, scale: layout.scale[column], style: row.style, chosen: chosen,
                                currency: layout.columns[column])
                        }
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(!selectable)
            .accessibilityLabel(spoken(row, layout))
            .help(selectable ? "Mark the inputs and outputs of this row in the lists below." : "")
        }
        .padding(.vertical, 5)
        .background(chosen ? Tone.present.opacity(0.08) : .clear)
        .overlay(alignment: .leading) {
            if chosen {
                Rectangle().fill(Tone.present).frame(width: 2).shadow(color: Tone.present.opacity(0.6), radius: 3)
            }
        }
    }

    private func cell(
        _ sats: Int64, scale: Int64, style: StatementRow.Style, chosen: Bool, currency id: String?
    ) -> some View {
        VStack(spacing: 2) {
            NetBar(fraction: Double(sats) / Double(scale), style: style, chosen: chosen)
                .frame(height: 9)
            Text(sats == 0 ? " " : StatementText.signed(sats))
                .font(.callout.monospacedDigit())
                .foregroundStyle(style == .closing || style == .rest ? Tone.label : Tone.value)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity)
        .padding(.horizontal, 6)
        .help(sats == 0 ? "" : "\(Format.coins(abs(sats))) \(currency(id)) \(sats < 0 ? "given" : "received")")
    }

    private func spoken(_ row: StatementRow, _ layout: StatementLayout) -> String {
        let parts = zip(row.cells, layout.columns).filter { $0.0 != 0 }.map { sats, column in
            let money = "\(Format.coins(abs(sats))) \(currency(column))"
            if row.style == .closing { return sats < 0 ? "minus \(money)" : money }
            return "\(sats < 0 ? "gave" : "received") \(money)"
        }
        return parts.isEmpty ? "\(row.title), no change" : "\(row.title) \(StatementText.list(parts))"
    }

    private func marked(_ selection: StatementSelection) -> String {
        let inputs = selection.inputs.count, outputs = selection.outputs.count
        var parts: [String] = []
        if inputs > 0 { parts.append("\(inputs) \(inputs == 1 ? "input" : "inputs")") }
        if outputs > 0 { parts.append("\(outputs) \(outputs == 1 ? "output" : "outputs")") }
        let count = inputs + outputs
        return "\(StatementText.list(parts)) of this row \(count == 1 ? "is" : "are") marked in the lists below."
    }

    // MARK: The spread

    private func spread(_ layout: StatementLayout) -> some View {
        let values = layout.spread
        let total = values.reduce(0, +)
        let median = values.sorted()[values.count / 2]
        let top = values.prefix(StatementLayout.keptReceivers).reduce(0, +)
        let name = currency(layout.columns.first ?? nil)
        // An address with less than 1 of each 400 gets less than two points of the bar.
        // Those addresses are one segment at the end, so that the bar stays clear.
        let single = values.filter { $0 * 400 >= total }
        let small = values.count - single.count
        let smallTotal = total - single.reduce(0, +)
        let share = Int((Double(top) / Double(max(total, 1)) * 100).rounded())
        let note: String = small > 0
            ? "One segment is one address. The segment with lines is the \(small) smallest together: "
                + "\(StatementText.brief(smallTotal)) \(name)."
            : "One segment is one address."
        let figures: [(String, String)] = [
            ("Addresses", "\(values.count)"),
            ("Largest", "\(StatementText.brief(values[0])) \(name)"),
            ("Median", "\(StatementText.brief(median)) \(name)"),
            ("\(StatementLayout.keptReceivers) largest", "\(share)% of the total"),
        ]
        return VStack(alignment: .leading, spacing: 8) {
            Canvas { context, size in
                var x: CGFloat = 0
                for (index, value) in single.enumerated() {
                    let width = CGFloat(value) / CGFloat(max(total, 1)) * size.width
                    context.fill(
                        Path(CGRect(x: x, y: 0, width: max(width - 0.6, 0.3), height: size.height)),
                        with: .color(Tone.record.opacity(index.isMultiple(of: 2) ? 0.9 : 0.55)))
                    x += width
                }
                if small > 0, size.width > x {
                    // The group of small addresses: an outline with lines across it.
                    let box = CGRect(x: x, y: 0, width: size.width - x, height: size.height)
                    var lines = Path()
                    var at = box.minX - box.height
                    while at < box.maxX {
                        lines.move(to: CGPoint(x: at, y: box.maxY))
                        lines.addLine(to: CGPoint(x: at + box.height, y: box.minY))
                        at += 5
                    }
                    context.drawLayer { layer in
                        layer.clip(to: Path(box))
                        layer.stroke(lines, with: .color(Tone.record.opacity(0.6)), lineWidth: 1)
                    }
                    context.stroke(Path(box.insetBy(dx: 0.5, dy: 0.5)), with: .color(Tone.record.opacity(0.8)), lineWidth: 1)
                }
            }
            .frame(height: 20)
            .accessibilityLabel("The amount of each of \(values.count) addresses, largest first.")
            HStack(alignment: .firstTextBaseline, spacing: 26) {
                ForEach(figures, id: \.0) { figure in
                    HStack(alignment: .firstTextBaseline, spacing: 7) {
                        NameText(figure.0)
                        Text(figure.1).font(.callout.monospacedDigit()).foregroundStyle(Tone.value)
                    }
                }
            }
            Text(note)
                .font(Tone.caption).foregroundStyle(Tone.label)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: Below the grid

    @ViewBuilder
    private var routes: some View {
        if !statement.routes.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                NameText("Asked for")
                ForEach(statement.routes, id: \.output) { route in
                    HStack(spacing: 6) {
                        Text(routeText(route)).font(.callout.monospacedDigit())
                        if transaction.traceRoots.contains(where: { $0.output == route.output }) {
                            Button("Follow") { follow(route.output) }
                                .buttonStyle(.displayLink)
                                .help("Start the transfer trace of output \(route.output).")
                        }
                    }
                }
            }
        }
    }

    private func routeText(_ route: StatementRoute) -> String {
        var parts = [StatementText.list(route.amounts.map(amount))]
        if let via = route.via { parts.append(model.currencyName(via)) }
        if let to = route.toCurrency { parts.append(model.currencyName(to)) }
        var text = parts.joined(separator: " → ")
        if let destination = route.destination {
            text += ", for \(model.displayID(destination))"
        }
        return text
    }

    @ViewBuilder
    private var carried: some View {
        let lines = statement.parties.filter { !$0.carried.isEmpty && !$0.net.isEmpty }
        if !lines.isEmpty {
            VStack(alignment: .leading, spacing: 2) {
                NameText("Passed through with no change")
                ForEach(Array(lines.enumerated()), id: \.offset) { _, party in
                    Text("\(StatementText.capital(name(party))): \(StatementText.list(party.carried.map(amount)))")
                        .font(.callout)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    @ViewBuilder
    private var marks: some View {
        let spent = statement.marks.filter(\.spent)
        let written = statement.marks.filter { !$0.spent }
        if !statement.marks.isEmpty {
            VStack(alignment: .leading, spacing: 6) {
                if !spent.isEmpty { markLine("Spends", spent) }
                if !written.isEmpty { markLine("Writes", written) }
            }
        }
    }

    private func markLine(_ title: String, _ marks: [StatementMark]) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            NameText(title)
                .frame(width: 56, alignment: .leading)
            FlowLayout(spacing: 6) {
                ForEach(Array(marks.enumerated()), id: \.offset) { _, mark in
                    Button {
                        selection = StatementSelection(
                            id: "mark-\(mark.spent)-\(mark.index)",
                            inputs: mark.spent ? [mark.index] : [], outputs: mark.spent ? [] : [mark.index])
                        scroll(mark.spent ? "input-\(mark.index)" : "output-\(mark.index)")
                    } label: {
                        Plate(markText(mark))
                    }
                    .buttonStyle(.plain)
                    .help("Go to \(mark.spent ? "input" : "output") \(mark.index) in the lists below.")
                }
            }
        }
    }

    private func markText(_ mark: StatementMark) -> String {
        mark.currencyId.map { "\(mark.label) · \(model.currencyName($0))" } ?? mark.label
    }

    // MARK: Words

    private var words: StatementWords {
        // Reads the setting, so that the words follow a change of it.
        _ = showsNames
        return StatementWords(
            statement: statement, coin: coin, tags: transaction.tags,
            coinbase: transaction.accounting.inputs.coinbase, feePool: transaction.feePoolCheckpoint,
            model: model)
    }

    private func currency(_ id: String?) -> String { words.currency(id) }
    private func amount(_ amount: StatementAmount) -> String { words.amount(amount) }
    private func name(_ party: StatementParty) -> String { words.name(party) }
    private func rowTitle(_ party: StatementParty) -> String { words.rowTitle(party) }
    private var sentence: String { words.sentence }
}

/// The words of a statement: the names of its parties and currencies, and the sentence
/// that says what the transaction did. The Transaction page and the Mempool page use the
/// same words.
@MainActor
struct StatementWords {
    let statement: TransactionStatement
    let coin: String
    let tags: [String]
    let coinbase: Bool
    let feePool: Bool
    let model: AppModel


    func currency(_ id: String?) -> String {
        id.map { model.currencyName($0) } ?? coin
    }

    func amount(_ amount: StatementAmount) -> String {
        "\(Format.coins(abs(amount.sats))) \(currency(amount.currencyId))"
    }

    /// A party in the middle of a sentence.
    func name(_ party: StatementParty) -> String {
        switch party.kind {
        case .address:
            // An output script can name more than one address. The lists below show each.
            return party.addresses.first.map { model.displayID($0) } ?? "an address"
        case .reserveDeposits:
            return party.currencyId.map { "the reserve deposits of \(model.currencyName($0))" }
                ?? "the reserve deposits"
        case .waitingTransfers: return "the waiting transfers"
        case .newCoins: return "new coins and fees"
        case .shieldedPool: return "the shielded pool"
        case .noAddress: return "an output with no address"
        }
    }

    func rowTitle(_ party: StatementParty) -> String {
        StatementText.capital(name(party).replacingOccurrences(of: "the ", with: "", options: .anchored))
    }

    var sentence: String {
        let parties = statement.parties
        let givers = parties.filter { $0.net.contains { $0.sats < 0 } }
        let receivers = parties.filter { party in !party.net.isEmpty && !party.net.contains { $0.sats < 0 } }
        let addresses = receivers.filter { $0.kind == .address || $0.kind == .noAddress }
        let pool = parties.first { $0.kind == .shieldedPool }
        let written = statement.marks.filter { !$0.spent }
        let oneColumn = statement.columns.count == 1
        let column = statement.columns.first ?? nil
        func net(_ party: StatementParty) -> Int64 { party.net.first { $0.currencyId == column }?.sats ?? 0 }
        func money(_ sats: Int64) -> String { "\(Format.coins(abs(sats))) \(currency(column))" }
        var parts: [String] = []

        if coinbase {
            let total = addresses.map(net).reduce(0, +)
            let to = addresses.count == 1 ? name(addresses[0]) : "\(addresses.count) addresses"
            parts.append("The block reward of \(money(total)) went to \(to).")
            if feePool { parts.append("One output records the fee pool.") }
            return parts.joined(separator: " ")
        }
        if tags.contains("Stake"), givers.isEmpty, receivers.isEmpty,
            let owner = parties.first, let stayed = owner.carried.first
        {
            return "This transaction staked the block. \(amount(stayed)) stayed at \(name(owner)). No value moved."
        }

        // The kind, from Carbide's tags.
        if let mark = written.first(where: { $0.label == "Import" }) {
            parts.append(mark.currencyId.map { "An import into \(model.currencyName($0))." } ?? "An import.")
        } else if let mark = written.first(where: { $0.label == "Export" }) {
            parts.append(mark.currencyId.map { "An export for \(model.currencyName($0))." } ?? "An export.")
        }

        let giver = givers.count == 1 ? name(givers[0]) : "\(givers.count) parties"
        if !statement.routes.isEmpty {
            let conversions = statement.routes.allSatisfy(\.conversion)
            let noun = conversions ? "conversion" : "transfer"
            let count = statement.routes.count
            var text = "\(StatementText.capital(giver)) started \(count) \(noun)\(count == 1 ? "" : "s")"
            if count <= 2 {
                let each = statement.routes.map { route in
                    var words = StatementText.list(route.amounts.map(amount))
                    if let to = route.toCurrency { words += " to \(model.currencyName(to))" }
                    if let via = route.via { words += " through \(model.currencyName(via))" }
                    return words
                }
                text += ": " + StatementText.list(each)
            }
            parts.append(text + ".")
            parts.append("The \(count == 1 ? "result is" : "results are") not in this transaction.")
        } else if !receivers.isEmpty, !givers.isEmpty {
            var clauses: [String] = []
            if oneColumn, addresses.count <= 2 {
                clauses += addresses.map { "\(money(net($0))) to \(name($0))" }
            } else if !addresses.isEmpty {
                let what = oneColumn
                    ? money(addresses.map(net).reduce(0, +))
                    : StatementText.list(statement.columns.map(currency))
                let to = addresses.count == 1 ? name(addresses[0]) : "\(addresses.count) addresses"
                clauses.append("\(what) to \(to)")
            }
            if let pool, receivers.contains(where: { $0.kind == .shieldedPool }) {
                clauses.append("\(money(net(pool))) into the shielded pool")
            }
            let verb = addresses.isEmpty ? "sent" : "paid"
            parts.append("\(StatementText.capital(giver)) \(verb) \(StatementText.list(clauses)).")
            if let pool {
                parts.append(net(pool) > 0 ? "The recipient in the pool is hidden." : "The sender in the pool is hidden.")
            }
            if addresses.count > StatementLayout.maxReceivers, oneColumn {
                let sorted = addresses.map(net).sorted()
                parts.append("Half of the addresses received less than \(money(sorted[sorted.count / 2])).")
            }
        } else if !written.isEmpty, !givers.isEmpty {
            let labels = written.map { $0.label.lowercased() }
            parts.append("\(StatementText.capital(giver)) wrote: \(StatementText.list(labels)).")
        } else if givers.isEmpty, receivers.isEmpty, parts.isEmpty {
            parts.append(
                tags.isEmpty ? "No value moved." : "\(tags.prefix(3).joined(separator: ", ")). No value moved.")
        }

        if statement.closingKind == .fee, let fee = statement.closing.first(where: { $0.currencyId == nil }) {
            parts.append("The fee was \(Format.coins(fee.sats)) \(coin).")
            if receivers.isEmpty, statement.routes.isEmpty, !givers.isEmpty { parts.append("No other value moved.") }
        }
        return parts.joined(separator: " ")
    }
}

/// One bar of the statement. A hollow bar to the left of the axis is an amount given. A
/// solid bar to the right is an amount received. A dashed bar has a hidden recipient.
private struct NetBar: View {
    /// The amount as a part of the largest amount in its column: -1 to 1.
    let fraction: Double
    let style: StatementRow.Style
    /// True for the row that the person marked: its received bars are the present.
    var chosen = false

    var body: some View {
        Canvas { context, size in
            let middle = size.width / 2
            context.rule(middle, 0, middle, size.height, Tone.axis)
            guard fraction != 0 else { return }
            let length = max(CGFloat(abs(fraction)) * (middle - 1), 2)
            let rect = CGRect(
                x: fraction < 0 ? middle - length : middle, y: 0.75, width: length, height: size.height - 1.5)
            switch style {
            case .closing, .rest:
                context.fill(Path(rect), with: .color(Tone.label.opacity(0.55)))
            case .hidden:
                context.fill(Path(rect), with: .color(Tone.record.opacity(0.15)))
                context.stroke(
                    Path(rect.insetBy(dx: 0.5, dy: 0.5)), with: .color(Tone.record),
                    style: StrokeStyle(lineWidth: 1, lineCap: .butt, dash: [4, 3]))
            default:
                if fraction < 0 {
                    context.stroke(
                        Path(rect.insetBy(dx: 0.5, dy: 0.5)), with: .color(chosen ? Tone.present : Tone.value),
                        lineWidth: 1)
                } else if chosen {
                    context.lit(Path(rect), Tone.present, strength: 0.5)
                } else {
                    context.fill(Path(rect), with: .color(Tone.record))
                }
            }
        }
        .accessibilityHidden(true)
    }
}

/// Places its views in lines, and starts a new line when the width is full.
struct FlowLayout: Layout {
    var spacing: CGFloat = 6

    private func lines(_ subviews: Subviews, width: CGFloat) -> (frames: [CGRect], size: CGSize) {
        var frames: [CGRect] = []
        var x: CGFloat = 0, y: CGFloat = 0, lineHeight: CGFloat = 0, widest: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                x = 0
                y += lineHeight + spacing
                lineHeight = 0
            }
            frames.append(CGRect(origin: CGPoint(x: x, y: y), size: size))
            x += size.width + spacing
            lineHeight = max(lineHeight, size.height)
            widest = max(widest, x - spacing)
        }
        return (frames, CGSize(width: widest, height: y + lineHeight))
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        lines(subviews, width: proposal.width ?? .infinity).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        for (subview, frame) in zip(subviews, lines(subviews, width: bounds.width).frames) {
            subview.place(
                at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), proposal: .unspecified)
        }
    }
}

private extension Array {
    subscript(safe index: Int) -> Element? {
        indices.contains(index) ? self[index] : nil
    }
}

/// The statement as a flow: the parties that gave on the left, the transaction in the
/// middle, the parties that received on the right. The width of a band is the amount.
///
/// All bands go through the transaction. The chain does not record which input paid which
/// output, thus the picture has no band from one party to a different party.
///
/// The band of the joined addresses opens: a click goes one level in, where that group is
/// on the left and its next largest addresses are on the right, with the remainder as a
/// new group. The picture moves across to the new level.
private struct StatementFlow: View {
    let layout: StatementLayout
    let column: Int
    let unit: String
    @Binding var selection: StatementSelection?
    @State private var pointed: String?
    /// How many levels into the joined addresses the picture is. 0 is the transaction.
    @State private var depth = 0
    /// The level that the picture comes from, while it moves.
    @State private var origin = 0
    /// 0 at the start of a move between levels, 1 at its end.
    @State private var progress: Double = 1
    /// The width of the names and the picture together, for the place of the pointer.
    @State private var rowWidth: CGFloat = 800

    fileprivate struct Node {
        let row: StatementRow
        let sats: Int64
        /// The thickness of the band.
        let thick: CGFloat
        /// The top of the place of the node, and the height of the place.
        let top: CGFloat
        let slot: CGFloat
        var middle: CGFloat { top + slot / 2 }
    }

    fileprivate struct Scene {
        var left: [Node] = []
        var right: [Node] = []
        var height: CGFloat = 60
        /// The node of the group that opens to the next level.
        var group: Node? { right.last { $0.row.style == .rest } }

        static let bar: CGFloat = 4
        static let core: CGFloat = 9

        /// The top of each band where it meets the bar in the middle.
        func inner(_ nodes: [Node]) -> [CGFloat] {
            let leftThick = left.reduce(0) { $0 + $1.thick }
            let rightThick = right.reduce(0) { $0 + $1.thick }
            let coreHeight = max(leftThick, rightThick)
            let coreTop = max((height - coreHeight) / 2, 0)
            var y = coreTop + (coreHeight - nodes.reduce(0) { $0 + $1.thick }) / 2
            return nodes.map { node in
                defer { y += node.thick }
                return y
            }
        }

        /// The band at a point of the picture: the band itself, not the row of its name.
        func band(at point: CGPoint, width: CGFloat) -> Node? {
            let mid = width / 2
            let gave = point.x < mid
            let nodes = gave ? left : right
            let outer = gave ? Self.bar : width - Self.bar
            let innerX = gave ? mid - Self.core / 2 : mid + Self.core / 2
            // How far along the band the point is, from its party (0) to the middle (1).
            let along = min(max((point.x - outer) / (innerX - outer), 0), 1)
            // The band is a curve with its control points at the half distance: find the
            // parameter of the curve for this distance.
            var low: CGFloat = 0, high: CGFloat = 1
            for _ in 0..<18 {
                let t = (low + high) / 2
                let x = 1.5 * t * (1 - t) * (1 - t) + 1.5 * t * t * (1 - t) + t * t * t
                if x < along { low = t } else { high = t }
            }
            let t = (low + high) / 2
            let ease = 3 * t * t - 2 * t * t * t
            var best: (node: Node, distance: CGFloat)?
            for (node, end) in zip(nodes, inner(nodes)) {
                let start = node.middle - node.thick / 2
                let top = start + (end - start) * ease
                // A thin band takes the pointer from a little above and below it.
                let reach = max((6 - node.thick) / 2, 0)
                guard point.y >= top - reach, point.y <= top + node.thick + reach else { continue }
                let distance = abs(point.y - (top + node.thick / 2))
                if best == nil || distance < best!.distance { best = (node, distance) }
            }
            return best?.node
        }
    }

    private static let bandSpace: CGFloat = 210
    private static let minThick: CGFloat = 1.5
    private static let minSlot: CGFloat = 22
    private static let gap: CGFloat = 5
    private static let leftWidth: CGFloat = 236
    private static let rightWidth: CGFloat = 270
    private static let step = StatementLayout.keptReceivers

    /// The nodes of one side, from the top.
    private func nodes(_ rows: [StatementRow], scale: CGFloat) -> [Node] {
        var top: CGFloat = 0
        return rows.map { row in
            let sats = abs(row.cells[column])
            let thick = max(CGFloat(sats) * scale, Self.minThick)
            let slot = max(thick, Self.minSlot)
            defer { top += slot + Self.gap }
            return Node(row: row, sats: sats, thick: thick, top: top, slot: slot)
        }
    }

    /// One row for a group of joined addresses.
    private func group(_ rows: ArraySlice<StatementRow>, id: String) -> StatementRow {
        StatementRow(
            id: id, title: "\(rows.count) more addresses", address: nil, style: .rest,
            cells: layout.columns.indices.map { index in rows.reduce(0) { $0 + $1.cells[index] } },
            inputs: rows.flatMap(\.inputs), outputs: rows.flatMap(\.outputs))
    }

    /// The deepest level: each level takes the next addresses from the joined ones.
    private var maxDepth: Int {
        layout.joined.isEmpty ? 0 : (layout.joined.count - 1) / Self.step + 1
    }

    private func scene(_ level: Int, height: CGFloat) -> Scene {
        var found = scene(level)
        found.height = height
        return found
    }

    /// The picture of a level.
    private func scene(_ level: Int) -> Scene {
        var gave: [StatementRow]
        var received: [StatementRow]
        if level == 0 {
            let rows = layout.rows.filter { $0.style != .unchanged && $0.cells[column] != 0 }
            gave = rows.filter { $0.cells[column] < 0 }
            received = rows.filter { $0.cells[column] > 0 }
        } else {
            let from = (level - 1) * Self.step
            let joined = layout.joined
            gave = [group(joined[min(from, joined.count)...], id: "group-\(level)")]
            let end = min(from + Self.step, joined.count)
            received = Array(joined[min(from, joined.count)..<end])
            if end < joined.count { received.append(group(joined[end...], id: "rest")) }
            received = received.filter { $0.cells[column] != 0 }
        }
        let total = max(
            gave.reduce(0) { $0 + abs($1.cells[column]) }, received.reduce(0) { $0 + abs($1.cells[column]) }, 1)
        let scale = Self.bandSpace / CGFloat(total)
        var scene = Scene(left: nodes(gave, scale: scale), right: nodes(received, scale: scale))
        scene.height = max(
            scene.left.last.map { $0.top + $0.slot } ?? 0, scene.right.last.map { $0.top + $0.slot } ?? 0, 60)
        return scene
    }

    /// Goes to a level. The picture moves across in the direction of the level.
    private func go(to level: Int) {
        let level = min(max(level, 0), maxDepth)
        guard level != depth else { return }
        origin = depth
        depth = level
        progress = 0
        pointed = nil
        let still = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
        withAnimation(still ? nil : .easeInOut(duration: 0.6)) { progress = 1 }
    }

    var body: some View {
        let level = min(depth, maxDepth)
        // Each level has the height of the tallest one seen, so that the page below the
        // picture does not move when the level changes.
        let tall = max(scene(level).height, scene(min(origin, maxDepth)).height, scene(0).height)
        let now = scene(level, height: tall)
        let before = scene(min(origin, maxDepth), height: tall)
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .top, spacing: 10) {
                labels(now.left, alignment: .trailing, level: level)
                    .frame(width: Self.leftWidth, height: now.height, alignment: .topTrailing)
                    .id("left-\(level)")
                    .transition(.opacity)
                GeometryReader { geometry in
                    let width = geometry.size.width
                    FlowPicture(
                        from: before, to: now, inward: level >= origin, progress: progress,
                        chosen: selection?.id, pointed: pointed)
                    .contentShape(Rectangle())
                    .onTapGesture(coordinateSpace: .local) { point in
                        if let found = node(at: point, width: width, in: now) { choose(found.row, level: level) }
                    }
                }
                .frame(height: now.height)
                .accessibilityLabel("The flow of \(unit) through the transaction.")
                labels(now.right, alignment: .leading, level: level)
                    .frame(width: Self.rightWidth, height: now.height, alignment: .topLeading)
                    .id("right-\(level)")
                    .transition(.opacity)
            }
            // One pointer area for the names and the bands. With an area for each, the light
            // of a band went off and on when the pointer went from its name to the band.
            .contentShape(Rectangle())
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { rowWidth = $0 }
            .onContinuousHover { phase in
                switch phase {
                case .active(let point):
                    // In the picture, the band under the pointer. At the names, the row.
                    let pictureLeft = Self.leftWidth + 10
                    let pictureWidth = rowWidth - Self.leftWidth - Self.rightWidth - 20
                    let found: String?
                    if point.x >= pictureLeft, point.x <= pictureLeft + pictureWidth {
                        found = now.band(
                            at: CGPoint(x: point.x - pictureLeft, y: point.y), width: pictureWidth)?.row.id
                    } else {
                        let side = point.x < pictureLeft ? now.left : now.right
                        found = side.first { point.y >= $0.top && point.y <= $0.top + $0.slot }?.row.id
                    }
                    if found != pointed { pointed = found }
                case .ended: pointed = nil
                }
            }
            // The keys to go back are on the picture, in the empty corner above the group,
            // so that the picture stays in its place when they come.
            .overlay(alignment: .topLeading) {
                if level > 0 {
                    VStack(alignment: .leading, spacing: 4) {
                        Button("Up one level") { go(to: level - 1) }
                        if level > 1 { Button("To the transaction") { go(to: 0) } }
                    }
                    .transition(.opacity)
                }
            }
            if !caption(level, now).isEmpty {
                Text(caption(level, now))
                    .font(Tone.caption).foregroundStyle(Tone.label)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .onChange(of: column) { depth = 0; origin = 0 }
    }

    private func place(_ level: Int) -> String {
        let first = StatementLayout.keptReceivers + (level - 1) * Self.step + 1
        let last = min(first + Self.step - 1, StatementLayout.keptReceivers + layout.joined.count)
        return "Level \(level) of \(maxDepth): addresses \(first) to \(last) by amount."
    }

    private func caption(_ level: Int, _ scene: Scene) -> String {
        var text = level == 0 ? "Click a band to mark its inputs and outputs in the lists." : place(level)
        if scene.group != nil {
            text += (text.isEmpty ? "" : " ") + "Click the \"more addresses\" band to open that group."
        }
        return text
    }

    private func choose(_ row: StatementRow, level: Int) {
        // The band of the joined addresses opens the next level. The band of the group on
        // the left goes back.
        if row.id == "rest", !layout.joined.isEmpty { return go(to: level + 1) }
        if row.id.hasPrefix("group-") { return go(to: level - 1) }
        guard !(row.inputs.isEmpty && row.outputs.isEmpty) else { return }
        selection = selection?.id == row.id
            ? nil : StatementSelection(id: row.id, inputs: Set(row.inputs), outputs: Set(row.outputs))
    }

    /// The node whose place has the point: the left half is the givers.
    private func node(at point: CGPoint, width: CGFloat, in scene: Scene) -> Node? {
        scene.band(at: point, width: width)
    }

    /// True for a row whose band does something on a click.
    private func acts(_ row: StatementRow) -> Bool {
        row.id == "rest" || row.id.hasPrefix("group-") || !(row.inputs.isEmpty && row.outputs.isEmpty)
    }

    private func labels(_ nodes: [Node], alignment: HorizontalAlignment, level: Int) -> some View {
        VStack(alignment: alignment, spacing: Self.gap) {
            ForEach(nodes, id: \.row.id) { node in
                let chosen = selection?.id == node.row.id
                HStack(spacing: 8) {
                    if alignment == .leading { amount(node, chosen: chosen) }
                    if let address = node.row.address {
                        RouteLink(text: address, route: .holder(address))
                    } else if acts(node.row) {
                        // The name does what a click on its band does.
                        Button(node.row.title) { choose(node.row, level: level) }
                            .buttonStyle(
                                DisplayLinkStyle(tone: node.row.style == .rest ? Tone.label : Tone.value))
                            .lineLimit(1)
                    } else {
                        Text(node.row.title).foregroundStyle(Tone.label).lineLimit(1)
                    }
                    if alignment == .trailing { amount(node, chosen: chosen) }
                }
                .frame(height: node.slot)
            }
        }
    }

    private func amount(_ node: Node, chosen: Bool) -> some View {
        Text(StatementText.brief(node.sats))
            .font(.callout.monospacedDigit())
            .foregroundStyle(chosen ? Tone.present : Tone.value)
            .lineLimit(1)
            .frame(minWidth: 78, alignment: .trailing)
            .help("\(Format.coins(node.sats)) \(unit)")
    }
}

/// The bands of the flow. While the picture moves between two levels, the level it leaves
/// goes out to one side and the new level comes in from the other side. The bar of the
/// group goes across with them: it is the right end of one level and the left end of the
/// next.
private struct FlowPicture: View, @preconcurrency Animatable {
    let from: StatementFlow.Scene
    let to: StatementFlow.Scene
    /// True when the move goes into a group, false when it goes back out.
    let inward: Bool
    var progress: Double
    let chosen: String?
    let pointed: String?

    var animatableData: Double {
        get { progress }
        set { progress = newValue }
    }

    private static let bar: CGFloat = 4
    private static let core: CGFloat = 9

    var body: some View {
        Canvas { context, size in
            let p = CGFloat(min(max(progress, 0), 1))
            guard p < 1 else { return draw(context, size, to, shift: 0, opacity: 1) }
            let way: CGFloat = inward ? 1 : -1
            draw(context, size, from, shift: -way * p * size.width, opacity: Double(1 - p))
            draw(context, size, to, shift: way * (1 - p) * size.width, opacity: Double(p))
            // The bar of the group, from its place in one level to its place in the other.
            let start = inward ? end(from.group, right: true, size) : end(from.left.first, right: false, size)
            let stop = inward ? end(to.left.first, right: false, size) : end(to.group, right: true, size)
            if let start, let stop {
                let rect = CGRect(
                    x: start.minX + (stop.minX - start.minX) * p, y: start.minY + (stop.minY - start.minY) * p,
                    width: Self.bar, height: start.height + (stop.height - start.height) * p)
                context.lit(Path(rect), Tone.present, strength: 0.8)
            }
        }
    }

    /// The bar at the end of a band, at its party.
    private func end(_ node: StatementFlow.Node?, right: Bool, _ size: CGSize) -> CGRect? {
        guard let node else { return nil }
        return CGRect(
            x: right ? size.width - Self.bar : 0, y: node.middle - node.thick / 2, width: Self.bar, height: node.thick)
    }

    private func color(_ row: StatementRow, gave: Bool) -> Color {
        if chosen == row.id { return Tone.present }
        switch row.style {
        case .closing: return Tone.mined
        case .rest: return Tone.label
        case .hidden: return Tone.staked
        default: return gave ? Tone.value : Tone.record
        }
    }

    private func draw(
        _ base: GraphicsContext, _ size: CGSize, _ scene: StatementFlow.Scene, shift: CGFloat, opacity: Double
    ) {
        var context = base
        context.opacity = opacity
        context.translateBy(x: shift, y: 0)
        let bar = Self.bar
        let core = Self.core
        let mid = size.width / 2
        let leftThick = scene.left.reduce(0) { $0 + $1.thick }
        let rightThick = scene.right.reduce(0) { $0 + $1.thick }
        let coreHeight = max(leftThick, rightThick)
        let coreTop = max((scene.height - coreHeight) / 2, 0)

        func band(_ node: StatementFlow.Node, at y: CGFloat, gave: Bool) {
            let outer = gave ? bar : size.width - bar
            let inner = gave ? mid - core / 2 : mid + core / 2
            let from = CGPoint(x: outer, y: node.middle - node.thick / 2)
            let bend = (outer + inner) / 2
            var path = Path()
            path.move(to: from)
            path.addCurve(
                to: CGPoint(x: inner, y: y), control1: CGPoint(x: bend, y: from.y), control2: CGPoint(x: bend, y: y))
            path.addLine(to: CGPoint(x: inner, y: y + node.thick))
            path.addCurve(
                to: CGPoint(x: outer, y: from.y + node.thick), control1: CGPoint(x: bend, y: y + node.thick),
                control2: CGPoint(x: bend, y: from.y + node.thick))
            path.closeSubpath()
            let tone = color(node.row, gave: gave)
            let strong = chosen == node.row.id || pointed == node.row.id
            context.fill(path, with: .color(tone.opacity(strong ? 0.6 : gave ? 0.16 : 0.34)))
            if gave || node.row.style == .hidden {
                // What a party gave is an outline, as in the table.
                context.stroke(
                    path, with: .color(tone.opacity(0.8)),
                    style: StrokeStyle(lineWidth: 1, dash: node.row.style == .hidden ? [4, 3] : []))
            }
            // The end of the band at its party.
            let end = CGRect(
                x: gave ? 0 : size.width - bar, y: node.middle - node.thick / 2, width: bar, height: node.thick)
            if strong {
                context.lit(Path(end), tone, strength: 0.6)
            } else {
                context.fill(Path(end), with: .color(tone))
            }
        }

        var y = coreTop + (coreHeight - leftThick) / 2
        for node in scene.left {
            band(node, at: y, gave: true)
            y += node.thick
        }
        y = coreTop + (coreHeight - rightThick) / 2
        for node in scene.right {
            band(node, at: y, gave: false)
            y += node.thick
        }
        // The middle: all that came in is all that went out.
        context.lit(
            Path(CGRect(x: mid - core / 2, y: coreTop, width: core, height: coreHeight)), Tone.present, strength: 0.5)
    }
}
