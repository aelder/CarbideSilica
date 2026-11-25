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
                Picker("Currency", selection: $only) {
                    Text("First \(StatementLayout.maxColumns) of \(statement.columns.count)")
                        .tag(String??.none)
                    ForEach(statement.columns, id: \.self) { column in
                        Text(currency(column)).tag(String??.some(column))
                    }
                }
                .fixedSize()
            }
            if !layout.rows.isEmpty {
                grid(layout)
            }
            if !layout.spread.isEmpty {
                spread(layout)
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
                    .buttonStyle(.link)
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
        .onChange(of: transaction.txid) { only = nil }
    }

    // MARK: The grid

    private func grid(_ layout: StatementLayout) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            if !layout.columns.isEmpty {
                HStack(spacing: 0) {
                    Color.clear.frame(width: Self.labelWidth, height: 1)
                    ForEach(Array(layout.columns.enumerated()), id: \.offset) { _, column in
                        Text(currency(column))
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .frame(maxWidth: .infinity)
                    }
                }
                .padding(.bottom, 4)
            }
            ForEach(Array(layout.rows.enumerated()), id: \.element.id) { position, row in
                if position > 0 { Divider() }
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
                        .foregroundStyle(row.style == .closing || row.style == .rest ? .secondary : .primary)
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
                        Text("No change").font(.callout).foregroundStyle(.secondary)
                            .frame(maxWidth: .infinity)
                    } else {
                        ForEach(Array(row.cells.enumerated()), id: \.offset) { column, sats in
                            cell(sats, scale: layout.scale[column], style: row.style, currency: layout.columns[column])
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
        .background(chosen ? Color.accentColor.opacity(0.1) : .clear)
    }

    private func cell(_ sats: Int64, scale: Int64, style: StatementRow.Style, currency id: String?) -> some View {
        VStack(spacing: 2) {
            NetBar(fraction: Double(sats) / Double(scale), style: style)
                .frame(height: 9)
            Text(sats == 0 ? " " : StatementText.signed(sats))
                .font(.callout.monospacedDigit())
                .foregroundStyle(style == .closing || style == .rest ? .secondary : .primary)
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
        return VStack(alignment: .leading, spacing: 4) {
            Canvas { context, size in
                var x: CGFloat = 0
                for (index, value) in values.enumerated() {
                    let width = CGFloat(value) / CGFloat(max(total, 1)) * size.width
                    context.fill(
                        Path(CGRect(x: x, y: 0, width: max(width - 0.6, 0.3), height: size.height)),
                        with: .color(.accentColor.opacity(index.isMultiple(of: 2) ? 0.9 : 0.55)))
                    x += width
                }
            }
            .frame(height: 20)
            .accessibilityLabel("The amount of each of \(values.count) addresses, largest first.")
            Text(
                "One segment for each of \(values.count) addresses, largest first. "
                    + "Largest: \(StatementText.brief(values[0])) \(name). "
                    + "Median: \(StatementText.brief(median)) \(name). "
                    + "The \(StatementLayout.keptReceivers) largest received "
                    + "\(Int((Double(top) / Double(max(total, 1)) * 100).rounded())) of each 100.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    // MARK: Below the grid

    @ViewBuilder
    private var routes: some View {
        if !statement.routes.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text("Asked for").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                ForEach(statement.routes, id: \.output) { route in
                    HStack(spacing: 6) {
                        Text(routeText(route)).font(.callout.monospacedDigit())
                        if transaction.traceRoots.contains(where: { $0.output == route.output }) {
                            Button("Follow") { follow(route.output) }
                                .buttonStyle(.link)
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
                Text("Passed through with no change").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
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
            Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                .frame(width: 48, alignment: .leading)
            FlowLayout(spacing: 6) {
                ForEach(Array(marks.enumerated()), id: \.offset) { _, mark in
                    Button {
                        selection = StatementSelection(
                            id: "mark-\(mark.spent)-\(mark.index)",
                            inputs: mark.spent ? [mark.index] : [], outputs: mark.spent ? [] : [mark.index])
                        scroll(mark.spent ? "input-\(mark.index)" : "output-\(mark.index)")
                    } label: {
                        TagChip(text: markText(mark), color: .secondary)
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

    private func currency(_ id: String?) -> String {
        id.map { model.currencyName($0) } ?? coin
    }

    private func amount(_ amount: StatementAmount) -> String {
        "\(Format.coins(abs(amount.sats))) \(currency(amount.currencyId))"
    }

    /// A party in the middle of a sentence.
    private func name(_ party: StatementParty) -> String {
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

    private func rowTitle(_ party: StatementParty) -> String {
        StatementText.capital(name(party).replacingOccurrences(of: "the ", with: "", options: .anchored))
    }

    private var sentence: String {
        _ = showsNames
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

        if transaction.accounting.inputs.coinbase {
            let total = addresses.map(net).reduce(0, +)
            let to = addresses.count == 1 ? name(addresses[0]) : "\(addresses.count) addresses"
            parts.append("The block reward of \(money(total)) went to \(to).")
            if transaction.feePoolCheckpoint { parts.append("One output records the fee pool.") }
            return parts.joined(separator: " ")
        }
        if transaction.tags.contains("Stake"), givers.isEmpty, receivers.isEmpty,
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
                transaction.tags.isEmpty ? "No value moved." : "\(transaction.tags.prefix(3).joined(separator: ", ")). No value moved.")
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

    var body: some View {
        Canvas { context, size in
            let middle = size.width / 2
            context.fill(
                Path(CGRect(x: middle - 0.5, y: 0, width: 1, height: size.height)),
                with: .color(.secondary.opacity(0.7)))
            guard fraction != 0 else { return }
            let length = max(CGFloat(abs(fraction)) * (middle - 1), 2)
            let rect = CGRect(
                x: fraction < 0 ? middle - length : middle, y: 0.75, width: length, height: size.height - 1.5)
            switch style {
            case .closing, .rest:
                context.fill(Path(rect), with: .color(.secondary.opacity(0.55)))
            case .hidden:
                context.fill(Path(rect), with: .color(.accentColor.opacity(0.2)))
                context.stroke(
                    Path(rect), with: .color(.accentColor), style: StrokeStyle(lineWidth: 1.3, dash: [4, 3]))
            default:
                if fraction < 0 {
                    context.stroke(Path(rect), with: .color(.primary), lineWidth: 1.3)
                } else {
                    context.fill(Path(rect), with: .color(.accentColor))
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
