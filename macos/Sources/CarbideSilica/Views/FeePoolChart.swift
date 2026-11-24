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
