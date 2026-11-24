import CarbideCore
import SwiftUI

/// References from this transaction's outputs to other outputs: the notarization, evidence,
/// finalization and import/export links Carbide follows with F3. Each names its other end.
struct ProtocolLinksCard: View {
    let links: [ProtocolLink]
    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Refers to", trailing: "\(links.count) link\(links.count == 1 ? "" : "s")") {
            VStack(spacing: 0) {
                ForEach(Array(links.enumerated()), id: \.offset) { index, link in
                    if index > 0 { Divider() }
                    HStack(spacing: 10) {
                        Text("Output \(link.output)").foregroundStyle(.secondary).frame(width: 76, alignment: .leading)
                        Text(link.label).fontWeight(.medium)
                        Image(systemName: "arrow.right").foregroundStyle(.secondary)
                        if link.targetSystem == model.status?.chainId {
                            RouteLink(text: link.targetTxid, route: .transaction(txid: link.targetTxid))
                        } else {
                            // The other end is on another system, which this app has no source for.
                            Text(Format.short(link.targetTxid)).font(.body.monospaced()).textSelection(.enabled)
                            Text("on \(model.names[link.targetSystem] ?? Format.short(link.targetSystem, head: 6, tail: 4))")
                                .foregroundStyle(.secondary)
                        }
                        Text("output \(link.targetOutput)").foregroundStyle(.secondary)
                        Spacer()
                    }
                    .padding(.vertical, 7)
                }
            }
        }
        .task(id: links.count) {
            await model.resolveNames(links.map(\.targetSystem))
        }
    }
}

/// Who signed the notarization evidence in one output, using Carbide's witness investigation.
struct WitnessCard: View {
    let txid: String
    let root: TraceRoot

    @Environment(AppModel.self) private var model
    @State private var report: WitnessReport?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Who signed this", trailing: "\(root.label) · output \(root.output)") {
            if let report {
                if report.signatures.isEmpty {
                    Text("No signatures were found in the evidence that could be reached.")
                        .foregroundStyle(.secondary)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(report.signatures.enumerated()), id: \.offset) { index, signature in
                            if index > 0 { Divider() }
                            HStack(spacing: 12) {
                                Label(
                                    signature.confirms ? "Confirms" : "Rejects",
                                    systemImage: signature.confirms ? "checkmark.circle.fill" : "xmark.circle.fill"
                                )
                                .foregroundStyle(signature.confirms ? .green : .red)
                                .frame(width: 100, alignment: .leading)
                                RouteLink(text: signature.identity, route: .identity(name: signature.identity))
                                Spacer()
                                Text("\(signature.keyCount) key\(signature.keyCount == 1 ? "" : "s")")
                                    .foregroundStyle(.secondary)
                                if let height = signature.height {
                                    Text("at \(Format.integer(height))").foregroundStyle(.secondary).monospacedDigit()
                                }
                                RouteLink(text: signature.targetTxid, route: .transaction(txid: signature.targetTxid))
                            }
                            .padding(.vertical, 7)
                        }
                    }
                }
                ForEach(report.finalizations, id: \.self) { Text($0).font(.callout).foregroundStyle(.secondary) }
                ForEach(report.issues, id: \.self) { issue in
                    Label(issue, systemImage: "exclamationmark.triangle").font(.callout).foregroundStyle(.secondary)
                }
                Text(
                    "Checked \(report.transactionsChecked) transactions and \(report.outputsChecked) outputs. "
                        + "This lists reachable evidence. It is not a quorum count, and signatures are not verified here."
                )
                .font(.caption).foregroundStyle(.secondary)
            } else if running {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Following the evidence…").foregroundStyle(.secondary)
                }
            } else {
                if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                HStack {
                    Text("Find the identities that signed for or against this notarization.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("See signers") { Task { await run() } }
                }
            }
        }
    }

    private func run() async {
        running = true
        failure = nil
        defer { running = false }
        do {
            let found = try await model.requireExplorer().witnesses(txid: txid, output: root.output)
            report = found
            await model.resolveNames(found.signatures.map(\.identity))
        } catch {
            failure = describe(error)
        }
    }
}

/// Raw output data as hex and text side by side, sixteen bytes to a row.
struct BytesView: View {
    let hex: String

    private var bytes: [UInt8] {
        var result: [UInt8] = []
        var index = hex.startIndex
        while let next = hex.index(index, offsetBy: 2, limitedBy: hex.endIndex) {
            if let byte = UInt8(hex[index..<next], radix: 16) { result.append(byte) }
            index = next
        }
        return result
    }

    var body: some View {
        let data = bytes
        let shown = Array(data.prefix(1024))
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(stride(from: 0, to: shown.count, by: 16)), id: \.self) { offset in
                let row = shown[offset..<min(offset + 16, shown.count)]
                HStack(spacing: 14) {
                    Text(String(format: "%04x", offset)).foregroundStyle(.tertiary)
                    Text(row.map { String(format: "%02x", $0) }.joined(separator: " "))
                        .frame(width: 350, alignment: .leading)
                    Text(String(row.map { (32...126).contains($0) ? Character(UnicodeScalar($0)) : "." }))
                        .foregroundStyle(.secondary)
                }
            }
            Text(data.count > shown.count ? "\(data.count) bytes; showing the first \(shown.count)." : "\(data.count) bytes.")
                .font(.caption).foregroundStyle(.secondary).padding(.top, 4)
        }
        .font(.callout.monospaced())
        .textSelection(.enabled)
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 6))
    }
}
