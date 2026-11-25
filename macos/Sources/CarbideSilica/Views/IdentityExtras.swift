import CarbideCore
import SwiftUI

/// For an identity whose control is a token: where the token was issued, and who holds it
/// now. Carbide follows the token in passes of 16 transfers. A pass that reaches the
/// limit stops, and the next pass continues from the same output.
struct TokenControlCard: View {
    let identityAddress: String
    let viewingPast: Bool

    @Environment(AppModel.self) private var model
    @State private var result: TokenControl?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Tokenized control") {
            Text("The holder of the control token controls this identity.")
                .foregroundStyle(.secondary)
            if viewingPast {
                Note(text: "The trail shows the holder now, not at the viewed version.")
            }
            if let result {
                trail(result)
                status(result)
            }
            if running {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Checking…").foregroundStyle(.secondary)
                }
            } else if let failure {
                Note(text: failure, warning: true)
                Button("Try Again") { Task { await run(resume: result?.frontier) } }
            }
        }
        .task(id: "\(identityAddress)|\(model.generation)") {
            result = nil
            await run(resume: nil)
        }
    }

    /// The trail as a line: where the token was issued, one tick per transfer followed,
    /// and where it is now.
    private func trail(_ result: TokenControl) -> some View {
        let hops = Int(result.hops)
        let shown = min(hops, 40)
        return VStack(alignment: .leading, spacing: 4) {
            HStack(spacing: 4) {
                Text("Issued").font(.caption.weight(.semibold))
                ForEach(0..<shown, id: \.self) { _ in
                    Circle().fill(Color.accentColor.opacity(0.7)).frame(width: 6, height: 6)
                }
                if hops > shown {
                    Text("+\(hops - shown)").font(.caption).foregroundStyle(.secondary)
                }
                Image(systemName: "arrow.right").foregroundStyle(.secondary)
                Text(endWord(result.state)).font(.caption.weight(.semibold))
                    .foregroundStyle(endColor(result.state))
            }
            Text("Each dot is one transfer of the token that the trail followed.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private func status(_ result: TokenControl) -> some View {
        switch result.state {
        case .unspent(let holder):
            if let holder {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    ConfidenceMark(level: .verified, label: "Current token holder")
                    IDLink(id: holder, route: .holder(holder), shorten: false)
                }
                Text("\(hopText(result.hops)) checked.").foregroundStyle(.secondary)
            } else {
                ConfidenceMark(level: .observed, label: "No single holder")
                Text("The unspent output has no single holder address. \(hopText(result.hops)) checked.")
            }
        case .paused:
            ConfidenceMark(level: .observed, label: "Not finished")
            Text("\(hopText(result.hops)) checked. The trail continues past this pass.")
            Button("Follow up to \(result.hopsPerPass) more") {
                Task { await run(resume: result.frontier) }
            }
            .disabled(running)
        case .unresolved(let reason):
            ConfidenceMark(level: .unknown, label: "Unresolved")
            Text(result.frontier == nil ? reason : "\(hopText(result.hops)) checked. \(reason).")
            Button("Retry") { Task { await run(resume: result.frontier) } }
                .disabled(running)
        }
        if let frontier = result.frontier {
            HStack(spacing: 6) {
                Text("Last output checked").foregroundStyle(.secondary)
                RouteLink(text: frontier.txid, route: .transaction(txid: frontier.txid))
                Text("output \(frontier.output)").foregroundStyle(.secondary)
            }
            .font(.callout)
        }
    }

    private func hopText(_ hops: UInt32) -> String {
        "\(hops) transfer\(hops == 1 ? "" : "s")"
    }

    private func endWord(_ state: TokenControlState) -> String {
        switch state {
        case .unspent(let holder): holder == nil ? "unspent, no single holder" : "current holder"
        case .paused: "not finished"
        case .unresolved: "stopped"
        }
    }

    private func endColor(_ state: TokenControlState) -> Color {
        switch state {
        case .unspent: .green
        case .paused: .orange
        case .unresolved: .red
        }
    }

    private func run(resume: TokenFrontier?) async {
        running = true
        failure = nil
        defer { running = false }
        do {
            let next = try await model.requireExplorer().tokenControl(
                identityAddress: identityAddress, resume: resume)
            result = next
            if case .unspent(let holder) = next.state {
                await model.resolveNames([holder])
            }
        } catch is CancellationError {
        } catch {
            failure = describe(error)
        }
    }
}
