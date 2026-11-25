import CarbideCore
import SwiftUI

/// One band in the flow picture: a source of funds on the left or a destination on the right.
struct FlowItem: Identifiable {
    enum Kind {
        case input, created, payment, change, fee, hidden, protocolOutput
    }

    let id: Int
    let title: String
    let detail: String
    let sats: Int64
    let kind: Kind
}

/// A transaction reduced to what a person needs first: one sentence, and where the coins went.
///
/// Only native-coin amounts are drawn, because other currencies are not comparable on one
/// scale. Inputs join into a single trunk before splitting: a transaction does not record
/// which input paid which output, and the picture must not imply that it does.
struct TransactionFlowModel {
    let inputs: [FlowItem]
    let outputs: [FlowItem]
    let sentence: String
    /// Things the picture leaves out, said plainly.
    let notes: [String]

    private static let maxItems = 7

    init(transaction: TransactionDetail, coin: String) {
        let amount = { (sats: Int64) in "\(Format.coins(sats)) \(coin)" }
        let coinbase = transaction.inputs.contains(where: \.coinbase)
        let inputAddresses = Set(transaction.inputs.flatMap(\.addresses))
        let transparentInputs = transaction.inputs.filter { $0.shieldedPool == nil }
        let transparentOutputs = transaction.outputs.filter { $0.shieldedPool == nil }
        let publicOut = transparentOutputs.compactMap(\.valueSats).reduce(0, +)
        let knownInputs = transparentInputs.filter { !$0.coinbase && $0.valueSats != nil }
        let unknownInputs = transparentInputs.filter { !$0.coinbase && $0.valueSats == nil }.count
        // The fee as Carbide accounts for it, including the net shielded flow.
        let fee: Int64? = transaction.accounting.feeKind == .known ? transaction.accounting.feeSats : nil
        let publicIn = knownInputs.compactMap(\.valueSats).reduce(0, +)
        let shielded = transaction.privateInputs + transaction.privateOutputs > 0
        let staking = transaction.tags.contains("Stake")
        var notes: [String] = []

        var inputs: [FlowItem] = []
        if coinbase {
            inputs.append(
                FlowItem(
                    id: 0, title: "Newly created", detail: amount(publicOut), sats: publicOut,
                    kind: .created))
        } else {
            for input in knownInputs {
                inputs.append(
                    FlowItem(
                        id: Int(input.index),
                        title: input.addresses.first.map { Format.short($0, head: 8, tail: 6) }
                            ?? "Unknown source",
                        detail: amount(input.valueSats ?? 0), sats: input.valueSats ?? 0,
                        kind: .input))
            }
            let shieldedIn = publicOut + (fee ?? 0) - publicIn
            if transaction.privateInputs > 0, shieldedIn > 0 {
                inputs.append(
                    FlowItem(
                        id: 10_000, title: "Private address",
                        detail: fee == nil ? "amount hidden" : "\(amount(shieldedIn)) in total",
                        sats: shieldedIn, kind: .hidden))
            }
        }
        if unknownInputs > 0 {
            notes.append(
                "\(unknownInputs) input\(unknownInputs == 1 ? " has" : "s have") no recorded "
                    + "amount from this source. The app cannot calculate the fee.")
        }

        var outputs: [FlowItem] = []
        var valueless = 0
        for output in transparentOutputs {
            guard let sats = output.valueSats, sats > 0 else {
                valueless += 1
                continue
            }
            let address = output.addresses.first
            let name = address.map { Format.short($0, head: 8, tail: 6) }
            if let tag = output.tags.first {
                outputs.append(
                    FlowItem(
                        id: Int(output.index), title: tag,
                        detail: amount(sats), sats: sats, kind: .protocolOutput))
            } else if let address, inputAddresses.contains(address), !coinbase {
                outputs.append(
                    FlowItem(
                        id: Int(output.index), title: name ?? "Sender",
                        detail: "\(amount(sats)) · \(staking ? "back to its owner" : "change")",
                        sats: sats, kind: .change))
            } else {
                outputs.append(
                    FlowItem(
                        id: Int(output.index), title: name ?? (output.scriptType ?? "No address"),
                        detail: amount(sats), sats: sats, kind: .payment))
            }
        }
        if valueless > 0 {
            notes.append(
                "\(valueless) output\(valueless == 1 ? " carries" : "s carry") no \(coin) and "
                    + "\(valueless == 1 ? "is" : "are") listed below.")
        }

        if !coinbase, unknownInputs == 0 {
            let shieldedOut = publicIn - publicOut - (fee ?? 0)
            if transaction.privateOutputs > 0, shieldedOut > 0 {
                outputs.append(
                    FlowItem(
                        id: 10_001, title: "Private address",
                        detail: fee == nil ? "hidden amount, includes the fee" : "\(amount(shieldedOut)) in total",
                        sats: shieldedOut, kind: .hidden))
            }
            if let fee, fee > 0 {
                outputs.append(
                    FlowItem(id: 10_002, title: "Fee", detail: amount(fee), sats: fee, kind: .fee))
            }
        }
        if transaction.accounting.feeKind == .protocolAccounting {
            notes.append(
                "Protocol outputs account for value elsewhere, so the fee is not known. "
                    + "The picture shows the public native amounts only.")
        }

        self.inputs = Self.condense(inputs, label: "more inputs", coin: coin)
        self.outputs = Self.condense(outputs, label: "more outputs", coin: coin)
        self.notes = notes

        // The sentence.
        let payments = outputs.filter { $0.kind == .payment }
        let change = outputs.filter { $0.kind == .change }.map(\.sats).reduce(0, +)
        let protocolTags = transaction.tags.filter { !["Coinbase", "Stake"].contains($0) }
        var parts: [String] = []
        if coinbase {
            parts.append("Created \(amount(publicOut)) as the block reward.")
        } else if transaction.tags.contains("Stake") {
            parts.append("Staked \(amount(publicIn)) to make this block. The coins go back to the owner.")
        } else if shielded {
            parts.append("This transaction uses a private address. The diagram does not show hidden amounts.")
        } else if !protocolTags.isEmpty {
            parts.append("\(protocolTags.prefix(3).joined(separator: ", ")).")
            if publicIn > 0 { parts.append("The input was \(amount(publicIn)).") }
        } else if payments.count == 1, let payment = payments.first {
            parts.append("Paid \(amount(payment.sats)) to \(payment.title).")
        } else if payments.isEmpty, change > 0 {
            parts.append("Moved \(amount(change)) between addresses of the sender.")
        } else {
            let sent = payments.map(\.sats).reduce(0, +)
            parts.append("Paid \(amount(sent)) to \(payments.count) recipients.")
        }
        if !coinbase, !transaction.tags.contains("Stake"), change > 0, !payments.isEmpty {
            parts.append("\(amount(change)) went back to the sender as change.")
        }
        if let fee {
            parts.append("The fee was \(amount(fee)).")
        }
        sentence = parts.joined(separator: " ")
    }

    /// Keeps the largest items and folds the rest into one, so the picture stays readable.
    private static func condense(_ items: [FlowItem], label: String, coin: String) -> [FlowItem] {
        guard items.count > maxItems else { return items }
        let sorted = items.sorted { $0.sats > $1.sats }
        let kept = sorted.prefix(maxItems - 1)
        let rest = sorted.dropFirst(maxItems - 1)
        let total = rest.map(\.sats).reduce(0, +)
        return kept + [
            FlowItem(
                id: 20_000, title: "\(rest.count) \(label)",
                detail: "\(Format.coins(total)) \(coin)", sats: total, kind: .input)
        ]
    }
}

/// Inputs on the left, outputs on the right, band width proportional to amount.
struct TransactionFlowView: View {
    let flow: TransactionFlowModel

    private let gap: CGFloat = 12
    private let minBand: CGFloat = 3

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(flow.sentence)
                .font(.title3)
                .textSelection(.enabled)
            if !flow.inputs.isEmpty, !flow.outputs.isEmpty {
                Canvas { context, size in
                    draw(context, size)
                }
                .frame(height: canvasHeight)
                .accessibilityLabel("Flow of coins. \(flow.sentence)")
            }
            ForEach(flow.notes, id: \.self) { note in
                Text(note).font(.caption).foregroundStyle(.secondary)
            }
        }
    }

    private var canvasHeight: CGFloat {
        CGFloat(max(flow.inputs.count, flow.outputs.count, 3)) * 50 + 30
    }

    private func color(_ kind: FlowItem.Kind) -> Color {
        switch kind {
        case .input: return .accentColor.opacity(0.3)
        case .created: return .orange.opacity(0.45)
        case .payment: return .accentColor.opacity(0.55)
        case .change: return .secondary.opacity(0.25)
        case .fee: return .red.opacity(0.8)
        case .hidden: return .secondary.opacity(0.18)
        case .protocolOutput: return .purple.opacity(0.5)
        }
    }

    /// Vertical extents for a column of items, centred in the usable height.
    ///
    /// With `labelled`, each item gets a slot at least tall enough for its two-line label,
    /// and its band sits centred in that slot, so the text of small items never overlaps.
    private func stack(
        _ items: [FlowItem], usable: CGFloat, top: CGFloat, gap: CGFloat, scaleTo total: Double,
        labelled: Bool = true
    ) -> [(CGFloat, CGFloat)] {
        let slotMinimum: CGFloat = labelled ? 34 : 0
        let gaps = gap * CGFloat(max(items.count - 1, 0))
        var scale = (usable - gaps) / CGFloat(total)
        // Items too small for a label slot take the slot; the rest share what is left.
        let small = items.filter { CGFloat($0.sats) * scale < slotMinimum }
        let bigTotal = items.filter { CGFloat($0.sats) * scale >= slotMinimum }.map(\.sats).reduce(0, +)
        if labelled, !small.isEmpty, bigTotal > 0 {
            scale = max((usable - gaps - slotMinimum * CGFloat(small.count)) / CGFloat(bigTotal), 0)
        }
        let heights = items.map { max(CGFloat($0.sats) * scale, minBand) }
        let slots = heights.map { max($0, slotMinimum) }
        let used = slots.reduce(0, +) + gaps
        var y = top + max((usable - used) / 2, 0)
        return zip(heights, slots).map { height, slot in
            defer { y += slot + gap }
            let start = y + (slot - height) / 2
            return (start, start + height)
        }
    }

    private func band(_ x0: CGFloat, _ a: (CGFloat, CGFloat), _ x1: CGFloat, _ b: (CGFloat, CGFloat))
        -> Path
    {
        let mid = (x0 + x1) / 2
        return Path { path in
            path.move(to: CGPoint(x: x0, y: a.0))
            path.addCurve(
                to: CGPoint(x: x1, y: b.0), control1: CGPoint(x: mid, y: a.0),
                control2: CGPoint(x: mid, y: b.0))
            path.addLine(to: CGPoint(x: x1, y: b.1))
            path.addCurve(
                to: CGPoint(x: x0, y: a.1), control1: CGPoint(x: mid, y: b.1),
                control2: CGPoint(x: mid, y: a.1))
            path.closeSubpath()
        }
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let top: CGFloat = 16
        let usable = size.height - top - 8
        let total = Double(max(flow.inputs.map(\.sats).reduce(0, +), flow.outputs.map(\.sats).reduce(0, +), 1))
        let leftBar = size.width * 0.27
        let trunk = size.width * 0.5 - 13
        let rightBar = size.width * 0.73 - 12
        let barWidth: CGFloat = 12

        let left = stack(flow.inputs, usable: usable, top: top, gap: gap, scaleTo: total)
        let right = stack(flow.outputs, usable: usable, top: top, gap: gap, scaleTo: total)
        let trunkIn = stack(flow.inputs, usable: usable * 0.8, top: top + usable * 0.1, gap: 0, scaleTo: total, labelled: false)
        let trunkOut = stack(flow.outputs, usable: usable * 0.8, top: top + usable * 0.1, gap: 0, scaleTo: total, labelled: false)

        for (index, item) in flow.inputs.enumerated() {
            context.fill(
                band(leftBar + barWidth, left[index], trunk, trunkIn[index]),
                with: .color(color(item.kind)))
            context.fill(
                Path(CGRect(x: leftBar, y: left[index].0, width: barWidth, height: left[index].1 - left[index].0)),
                with: .color(item.kind == .created ? .orange : item.kind == .hidden ? .secondary : .accentColor))
            label(context, item, x: leftBar - 8, y: (left[index].0 + left[index].1) / 2, anchor: .trailing)
        }

        let trunkTop = min(trunkIn.first?.0 ?? top, trunkOut.first?.0 ?? top)
        let trunkBottom = max(trunkIn.last?.1 ?? top, trunkOut.last?.1 ?? top)
        context.fill(
            Path(roundedRect: CGRect(x: trunk, y: trunkTop, width: 26, height: trunkBottom - trunkTop), cornerRadius: 3),
            with: .color(.primary.opacity(0.85)))
        context.draw(
            Text("transaction").font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: trunk + 13, y: trunkTop - 8), anchor: .center)

        for (index, item) in flow.outputs.enumerated() {
            let shape = band(trunk + 26, trunkOut[index], rightBar, right[index])
            context.fill(shape, with: .color(color(item.kind)))
            if item.kind == .hidden {
                context.stroke(shape, with: .color(.secondary), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
            }
            let solid: Color =
                switch item.kind {
                case .fee: .red
                case .change, .hidden: .secondary
                case .protocolOutput: .purple
                default: .accentColor
                }
            context.fill(
                Path(CGRect(x: rightBar, y: right[index].0, width: barWidth, height: right[index].1 - right[index].0)),
                with: .color(solid))
            label(context, item, x: rightBar + barWidth + 8, y: (right[index].0 + right[index].1) / 2, anchor: .leading)
        }
    }

    private func label(_ context: GraphicsContext, _ item: FlowItem, x: CGFloat, y: CGFloat, anchor: UnitPoint) {
        context.draw(
            Text(item.title).font(.callout.weight(.semibold)).foregroundStyle(.primary),
            at: CGPoint(x: x, y: y - 8), anchor: anchor)
        context.draw(
            Text(item.detail).font(.caption).foregroundStyle(.secondary),
            at: CGPoint(x: x, y: y + 8), anchor: anchor)
    }
}
