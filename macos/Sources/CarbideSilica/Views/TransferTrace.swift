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
/// Carbide's DeFi trail, and draws it as a route with one verdict.
struct TransferTraceCard: View {
    let txid: String
    let root: TraceRoot

    @Environment(AppModel.self) private var model
    @State private var trace: TransferTrace?
    @State private var failure: String?
    @State private var running = false
    @State private var showEvidence = false

    var body: some View {
        Card("Transfer trace", trailing: "\(root.label) · output \(root.output)") {
            if let trace {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    ConfidenceMark(level: trace.confidence)
                    Text(trace.verdict).font(.title3)
                }
                Canvas { context, size in
                    draw(context, size, trace)
                }
                .frame(height: 96)
                .accessibilityLabel(trace.verdict)

                ForEach(Array(trace.legs.enumerated()), id: \.offset) { index, leg in
                    LegRow(leg: leg, number: trace.legs.count > 1 ? index + 1 : nil)
                }

                if let stop = trace.stop {
                    Text(stop).foregroundStyle(.secondary)
                }
                if let search = trace.search {
                    HStack {
                        Text(
                            "Searched blocks \(Format.integer(search.first)) to \(Format.integer(search.last)) "
                                + "(\(search.windows) window\(search.windows == 1 ? "" : "s")). "
                                + "The transfer can be in a block outside this range."
                        )
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        if search.moreOlder {
                            Button("Search older blocks") {
                                Task { await run(scanFrom: search.first) }
                            }
                            .disabled(running)
                        }
                    }
                }

                DisclosureGroup("Evidence", isExpanded: $showEvidence) {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(Array(trace.steps.enumerated()), id: \.offset) { _, step in
                            VStack(alignment: .leading, spacing: 3) {
                                HStack(spacing: 8) {
                                    Text(step.label).fontWeight(.semibold)
                                    RouteLink(text: step.txid, route: .transaction(txid: step.txid))
                                    Text("output \(step.output)").foregroundStyle(.secondary)
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
            } else if running {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Tracing the transfer…")
                        .foregroundStyle(.secondary)
                }
            } else {
                if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                HStack {
                    Text("Trace this transfer through its export, its import and its result.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Trace") { Task { await run(scanFrom: nil) } }
                        .keyboardShortcut(.defaultAction)
                }
            }
        }
    }

    private func run(scanFrom: UInt64?) async {
        running = true
        failure = nil
        defer { running = false }
        do {
            let result = try await model.requireExplorer().traceTransfer(
                txid: txid, output: root.output, scanOlderFrom: scanFrom)
            trace = result
            let ids = result.legs.flatMap { leg in
                leg.principal.map(\.currency) + leg.resultAmounts.map(\.currency) + [leg.pool].compactMap { $0 }
            }
            await model.resolveNames(ids)
        } catch {
            failure = describe(error)
        }
    }

    /// Sent → Exported → Imported → Result, filled as far as the trace got.
    private func draw(_ context: GraphicsContext, _ size: CGSize, _ trace: TransferTrace) {
        let labels = ["Sent", "Exported", "Imported", resultLabel(trace)]
        let reached = reachedCount(trace)
        let y: CGFloat = 34
        let xs = (0..<4).map { size.width * (0.1 + 0.8 * CGFloat($0) / 3) }
        for index in 0..<3 {
            let done = index + 1 < reached
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: xs[index] + 14, y: y))
                    path.addLine(to: CGPoint(x: xs[index + 1] - 14, y: y))
                }, with: .color(done ? .accentColor : .secondary.opacity(0.5)),
                style: StrokeStyle(lineWidth: done ? 3 : 1.5, dash: done ? [] : [5, 5]))
        }
        for index in 0..<4 {
            let done = index < reached
            let final = index == 3 && done
            let radius: CGFloat = index == reached - 1 ? 15 : 12
            let circle = Path(
                ellipseIn: CGRect(x: xs[index] - radius, y: y - radius, width: radius * 2, height: radius * 2))
            let color: Color = final ? .green : .accentColor
            if done {
                context.fill(circle, with: .color(color))
                context.stroke(
                    Path { path in
                        path.move(to: CGPoint(x: xs[index] - 5, y: y))
                        path.addLine(to: CGPoint(x: xs[index] - 1, y: y + 4))
                        path.addLine(to: CGPoint(x: xs[index] + 6, y: y - 4))
                    }, with: .color(.white), style: StrokeStyle(lineWidth: 2.2, lineCap: .round, lineJoin: .round))
            } else {
                context.stroke(
                    circle, with: .color(.secondary), style: StrokeStyle(lineWidth: 1.6, dash: [4, 3]))
            }
            context.draw(
                Text(labels[index]).font(.callout.weight(done ? .semibold : .regular))
                    .foregroundStyle(done ? .primary : .secondary),
                at: CGPoint(x: xs[index], y: y + 34), anchor: .center)
        }
    }

    private func resultLabel(_ trace: TransferTrace) -> String {
        trace.legs.last?.result ?? "Received"
    }

    private func reachedCount(_ trace: TransferTrace) -> Int {
        if trace.confidence == .verified { return 4 }
        let stage = trace.legs.last?.stage ?? ""
        if stage.hasPrefix("Imported") { return 3 }
        if stage.hasPrefix("Exported") || trace.steps.contains(where: { $0.label == "Export" }) { return 2 }
        return 1
    }
}

/// What one leg of the transfer asked for and what it produced.
private struct LegRow: View {
    let leg: TraceLeg
    let number: Int?

    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 10) {
                if let number {
                    Text("Leg \(number)").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                }
                ConfidenceMark(level: leg.confidence, label: leg.stage)
            }
            HStack(alignment: .top, spacing: 28) {
                column("Sent", amounts: leg.principal)
                if let pool = leg.pool {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Through").font(.caption).foregroundStyle(.secondary)
                        RouteLink(text: pool, route: .currency(name: pool))
                    }
                }
                if let fee = leg.conversionFee {
                    column("Conversion fee", amounts: [fee])
                }
                if let result = leg.result {
                    column(result, amounts: leg.resultAmounts)
                }
                if let height = leg.importHeight {
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Imported at").font(.caption).foregroundStyle(.secondary)
                        RouteLink(text: "block \(Format.integer(height))", route: .block(id: "\(height)"), shorten: false)
                    }
                }
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 8))
    }

    private func column(_ title: String, amounts: [CurrencyAmount]) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).font(.caption).foregroundStyle(.secondary)
            if amounts.isEmpty {
                Text("–").foregroundStyle(.secondary)
            }
            ForEach(amounts, id: \.currency) { amount in
                HStack(spacing: 5) {
                    Text(Format.amount(amount.amount)).monospacedDigit().fontWeight(.medium)
                    RouteLink(text: amount.currency, route: .currency(name: amount.currency))
                }
            }
        }
    }
}
