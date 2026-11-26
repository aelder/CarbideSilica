import CarbideCore
import SwiftUI

/// When a future block will arrive, or what height a future moment will have, from
/// Carbide's calibrated block rate. Each result shows the calibration it came from.
///
/// Search can open it with a query: `BlockEstimator(query: "5000000")` runs at once.
struct BlockEstimator: View {
    var query: String?

    @Environment(AppModel.self) private var model
    @State private var text = ""
    @State private var result: Forecast?
    @State private var failure: String?
    @State private var running = false
    @State private var ran: String?

    init(query: String? = nil) {
        self.query = query
    }

    var body: some View {
        Card("Future block estimate") {
            HStack {
                TextField(
                    "Target", text: $text,
                    prompt: Text("Future height or date. Examples: 5000000, in 2 hours, 2028-11-08, unix:1857254400"))
                    .textFieldStyle(.roundedBorder)
                    .onSubmit { Task { await run() } }
                Button("Estimate") { Task { await run() } }
                    .disabled(running || text.trimmingCharacters(in: .whitespaces).isEmpty)
            }
            Text("A bare number is a block height. Use unix: or time: for Unix seconds. Relative dates are fixed when you submit them.")
                .font(.caption).foregroundStyle(.secondary)
            if running {
                ProgressView("Calibrating from the node's blocks…").controlSize(.small)
            } else if let failure {
                Note(text: failure, warning: true)
            } else if let result {
                ForecastResult(forecast: result)
            }
        }
        .task(id: query) {
            guard let query, !query.isEmpty, ran != query else { return }
            ran = query
            text = query
            await run()
        }
    }

    private func run() async {
        let query = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !query.isEmpty, !running else { return }
        running = true
        failure = nil
        defer { running = false }
        do {
            result = try await model.requireExplorer().forecast(query: query)
        } catch {
            result = nil
            failure = describe(error)
        }
    }
}

/// One estimate: the answer, a timeline of the calibration and the target, and the details.
private struct ForecastResult: View {
    let forecast: Forecast

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(headline).font(.title3)
            DisplayPanel(padding: 0) {
                ForecastTimeline(forecast: forecast)
                    .frame(height: 64)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
            }
            FieldRow(label: "Estimated height", value: Format.integer(forecast.height))
            FieldRow(label: "Estimated time", value: "\(Format.date(UInt64(max(forecast.time, 0)))) (\(forecast.time))")
            FieldRow(label: "Average", value: "\(forecast.secondsPerBlock.formatted(.number.precision(.fractionLength(6)))) seconds per block")
            FieldRow(
                label: "Calibration",
                value: "#\(Format.integer(forecast.start.height)) (\(Format.date(forecast.start.time))) to #\(Format.integer(forecast.tip.height)) (\(Format.date(forecast.tip.time)))")
            FieldRow(
                label: "Observed tip",
                value: "#\(Format.integer(forecast.tip.height)) at \(Format.date(UInt64(max(forecast.observedAt, 0))))")
            FieldRow(label: "Chain", value: forecast.system)
            FieldRow(label: "Source", value: forecast.sourceLabel)
            Text("This estimate assumes ordinary block production continues. Outages, changes in participation, and protocol updates can change the result.")
                .font(.caption).foregroundStyle(.secondary)
            Text("The rate comes from two blocks: the calibration start and the verified tip. An average of two endpoints does not detect interruptions between them.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private var headline: String {
        let date = Date(timeIntervalSince1970: TimeInterval(forecast.time))
        let relative = date.formatted(.relative(presentation: .named))
        if forecast.targetIsHeight {
            return "Block \(Format.integer(forecast.height)) is estimated at \(Format.date(UInt64(max(forecast.time, 0)))) (\(relative))."
        }
        return "At \(Format.date(UInt64(max(forecast.time, 0)))) (\(relative)) the estimated height is \(Format.integer(forecast.height))."
    }
}

/// The calibration span and the target on one time axis: the measured stretch as a solid
/// line, the projection to the target as a dashed line. The tip is the present.
private struct ForecastTimeline: View {
    let forecast: Forecast

    var body: some View {
        Canvas { context, size in
            let start = Double(forecast.start.time)
            let tip = Double(forecast.tip.time)
            let target = Double(forecast.time)
            let low = min(start, target), high = max(max(tip, target), low + 1)
            let x = { (time: Double) -> CGFloat in 8 + CGFloat((time - low) / (high - low)) * (size.width - 16) }
            let y: CGFloat = 22
            var measured = Path()
            measured.move(to: CGPoint(x: x(start), y: y))
            measured.addLine(to: CGPoint(x: x(tip), y: y))
            context.stroke(measured, with: .color(Tone.record), lineWidth: 2)
            var projected = Path()
            projected.move(to: CGPoint(x: x(tip), y: y))
            projected.addLine(to: CGPoint(x: x(target), y: y))
            context.stroke(projected, with: .color(Tone.value.opacity(0.7)), style: StrokeStyle(lineWidth: 1, dash: [4, 3]))
            let marks: [(Double, String, Color, UnitPoint)] = [
                (start, "Calibration start #\(Format.integer(forecast.start.height))", Tone.record, .topLeading),
                (tip, "Tip #\(Format.integer(forecast.tip.height))", Tone.present, .top),
                (target, "Estimate #\(Format.integer(forecast.height))", Tone.value, .topTrailing),
            ]
            for (time, label, color, anchor) in marks {
                let point = CGPoint(x: x(time), y: y)
                let dot = Path(ellipseIn: CGRect(x: point.x - 3.5, y: point.y - 3.5, width: 7, height: 7))
                // The tip is the present: it alone has light.
                context.lit(dot, color, strength: anchor == .top ? 0.6 : 0)
                // The tip's label goes above the line, so that it does not meet the target's label.
                if anchor == .top {
                    context.name(label, point.x, y - 8, .bottom, color: Tone.present)
                } else {
                    context.name(label, point.x, y + 10, anchor)
                }
            }
        }
        .accessibilityLabel("Calibration from block \(forecast.start.height) to the tip, then the estimate for block \(forecast.height)")
    }
}
