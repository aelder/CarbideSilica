import CarbideCore
import SwiftUI

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
