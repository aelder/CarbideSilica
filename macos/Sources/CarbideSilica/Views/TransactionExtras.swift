import CarbideCore
import SwiftUI

/// References between this transaction's outputs and other outputs: the notarization,
/// evidence, finalization and import/export links Carbide follows with F3, forward and
/// reverse, each with Carbide's resolution status.
struct ProtocolLinksCard: View {
    let txid: String
    /// The forward references decoded from the transaction, shown before the check ends.
    let forward: [ProtocolLink]

    @Environment(AppModel.self) private var model
    @State private var report: LinkReport?
    @State private var failure: String?

    private var links: [ProtocolLink] { report?.links ?? forward }

    var body: some View {
        Card("References", trailing: trailing) {
            let forwardLinks = links.filter { !$0.reverse }
            let reverseLinks = links.filter(\.reverse)
            if forwardLinks.isEmpty {
                Text("No explicit references to other transactions are decoded from this transaction.")
                    .foregroundStyle(.secondary)
            } else {
                LinkList(links: forwardLinks, checking: report == nil && failure == nil)
            }
            Text("Referenced by").font(.headline).padding(.top, 4)
            if let report {
                if !reverseLinks.isEmpty {
                    LinkList(links: reverseLinks, checking: false)
                } else if let reason = report.reverseUnavailable {
                    Note(text: reason, warning: true)
                } else {
                    Text("The cache holds no reference to this transaction from another transaction.")
                        .foregroundStyle(.secondary)
                }
                Text("Reverse references come from Carbide's cache. They include only transactions that the app or the terminal has already loaded.")
                    .font(.caption).foregroundStyle(.secondary)
            } else if let failure {
                Note(text: failure, warning: true)
            } else {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Checking the other side of each reference…").foregroundStyle(.secondary)
                }
            }
        }
        .task(id: "\(txid)|\(model.generation)") {
            report = nil
            failure = nil
            do {
                let found = try await model.requireExplorer().protocolLinks(txid: txid)
                report = found
                await model.resolveNames(found.links.map(\.targetSystem))
            } catch is CancellationError {
            } catch {
                failure = describe(error)
            }
        }
    }

    private var trailing: String {
        "\(links.count) link\(links.count == 1 ? "" : "s")"
    }
}

private struct LinkList: View {
    let links: [ProtocolLink]
    let checking: Bool

    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 0) {
            ForEach(Array(links.enumerated()), id: \.offset) { index, link in
                if index > 0 { Divider() }
                VStack(alignment: .leading, spacing: 4) {
                    HStack(spacing: 10) {
                        Text("Output \(link.output)").foregroundStyle(.secondary)
                            .frame(width: 76, alignment: .leading)
                        Text(link.label).fontWeight(.medium)
                        Image(systemName: link.reverse ? "arrow.left" : "arrow.right").foregroundStyle(.secondary)
                        if link.targetSystem == model.status?.chainId {
                            RouteLink(text: link.targetTxid, route: .transaction(txid: link.targetTxid))
                        } else {
                            // The other end is on another system. Open it with that chain selected.
                            Text(Format.short(link.targetTxid)).font(.body.monospaced()).textSelection(.enabled)
                                .help(link.targetTxid)
                        }
                        Text("output \(link.targetOutput) on").foregroundStyle(.secondary)
                        RouteLink(text: link.targetSystem, route: .currency(name: link.targetSystem))
                        Spacer()
                        CopyButton(text: "\(link.targetTxid):\(link.targetOutput)")
                    }
                    HStack(spacing: 10) {
                        Spacer().frame(width: 76)
                        status(link)
                        Text(link.field).font(.caption.monospaced()).foregroundStyle(.tertiary)
                    }
                }
                .padding(.vertical, 7)
            }
        }
    }

    @ViewBuilder private func status(_ link: ProtocolLink) -> some View {
        if let status = link.status {
            if link.resolved {
                ConfidenceMark(level: .verified, label: status)
            } else if link.problem {
                Label(status, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout.weight(.semibold))
                    .foregroundStyle(.red)
            } else {
                ConfidenceMark(level: .unknown, label: status)
            }
        } else if checking {
            ConfidenceMark(level: .observed, label: "Checking")
        } else {
            ConfidenceMark(level: .unknown, label: "Unresolved")
        }
    }
}

/// Who signed the notarization evidence in one output, using Carbide's witness investigation
/// (the terminal's F4). Signatures are grouped by identity, signing height, statement,
/// signing system and signed target.
struct WitnessCard: View {
    let txid: String
    let root: TraceRoot
    /// Incremented by the page to start the investigation from a menu command.
    var startRequests = 0

    @Environment(AppModel.self) private var model
    @State private var report: WitnessReport?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Signers", trailing: "\(root.label) · output \(root.output)") {
            if let report {
                summary(report)
                if report.signatures.isEmpty {
                    Text("No witness signatures are in the evidence that the search reached.")
                        .foregroundStyle(.secondary)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(report.signatures.enumerated()), id: \.offset) { index, signature in
                            if index > 0 { Divider() }
                            SignatureGroup(signature: signature)
                        }
                    }
                }
                if !report.finalizations.isEmpty {
                    Text("Observed finalization fields").font(.headline).padding(.top, 4)
                    ForEach(report.finalizations, id: \.self) {
                        Text($0).font(.callout.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                    }
                }
                if !report.issues.isEmpty {
                    Text("Investigation incomplete").font(.headline).foregroundStyle(.orange).padding(.top, 4)
                    ForEach(report.issues, id: \.self) { Note(text: $0, warning: true).font(.callout) }
                }
                Text(
                    "These groups are not a quorum count. The investigation covers reachable evidence. "
                        + "It does not find every witness broadcast, and it does not verify signatures, proofs, quorum membership or consensus."
                )
                .font(.caption).foregroundStyle(.secondary)
                DisclosureGroup("Decoded report") {
                    JSONTreeView(json: report.json)
                        .padding(.top, 6)
                }
            } else if running {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Searching the evidence…").foregroundStyle(.secondary)
                }
            } else {
                if let failure {
                    Note(text: failure, warning: true)
                }
                HStack {
                    Text("Find the identities that signed for or against this notarization.")
                        .foregroundStyle(.secondary)
                    Spacer()
                    Button("Find signers") { Task { await run(refresh: false) } }
                }
            }
        }
        .onChange(of: startRequests) {
            if report == nil, !running { Task { await run(refresh: false) } }
        }
    }

    private func summary(_ report: WitnessReport) -> some View {
        HStack {
            Text(
                "\(report.signatures.count) signature group\(report.signatures.count == 1 ? "" : "s") · "
                    + "\(report.transactionsChecked) transactions · \(report.outputsChecked) outputs · "
                    + "\(report.multipartSets) multipart set\(report.multipartSets == 1 ? "" : "s") decoded"
            )
            .foregroundStyle(.secondary)
            Spacer()
            Button("Refresh") { Task { await run(refresh: true) } }
                .disabled(running)
                .help("Request every reachable transaction again, instead of the answers already received.")
        }
    }

    private func run(refresh: Bool) async {
        running = true
        failure = nil
        defer { running = false }
        do {
            let found = try await model.requireExplorer().witnesses(txid: txid, output: root.output, refresh: refresh)
            report = found
            await model.resolveNames(
                found.signatures.flatMap { [$0.identity, $0.system, $0.targetSystem] })
        } catch {
            failure = describe(error)
        }
    }
}

/// One identity's signatures over one target, with the key signatures and the outputs that
/// hold the evidence.
private struct SignatureGroup: View {
    let signature: WitnessSignature

    @Environment(AppModel.self) private var model
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 12) {
                Label(
                    signature.confirms ? "Confirming" : "Rejecting",
                    systemImage: signature.confirms ? "checkmark.circle.fill" : "xmark.circle.fill"
                )
                .foregroundStyle(signature.confirms ? .green : .red)
                .frame(width: 110, alignment: .leading)
                RouteLink(text: signature.identity, route: .identity(name: signature.identity))
                Text("on").foregroundStyle(.secondary)
                RouteLink(text: signature.system, route: .currency(name: signature.system))
                Spacer()
                Text(signature.height.map { "signing height \(Format.integer($0))" } ?? "signing height unknown")
                    .foregroundStyle(.secondary).monospacedDigit()
                Button {
                    expanded.toggle()
                } label: {
                    Text("\(signature.keyCount) distinct key signature\(signature.keyCount == 1 ? "" : "s")")
                }
                .buttonStyle(.link)
            }
            HStack(spacing: 6) {
                Text("Signed target").foregroundStyle(.secondary)
                if signature.targetSystem == model.status?.chainId {
                    RouteLink(text: signature.targetTxid, route: .transaction(txid: signature.targetTxid))
                } else {
                    Text(Format.short(signature.targetTxid)).font(.body.monospaced()).textSelection(.enabled)
                }
                Text("output \(signature.targetOutput) on").foregroundStyle(.secondary)
                RouteLink(text: signature.targetSystem, route: .currency(name: signature.targetSystem))
            }
            .font(.callout)
            if expanded {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(signature.keySignatures, id: \.self) { key in
                        Text(key).font(.caption.monospaced()).textSelection(.enabled).lineLimit(1).truncationMode(.middle)
                    }
                    ForEach(Array(signature.origins.enumerated()), id: \.offset) { _, origin in
                        HStack(spacing: 6) {
                            Text("Evidence").foregroundStyle(.secondary)
                            if origin.output.systemId == model.status?.chainId {
                                RouteLink(text: origin.output.txid, route: .transaction(txid: origin.output.txid))
                            } else {
                                Text(Format.short(origin.output.txid)).font(.body.monospaced())
                            }
                            Text("output \(origin.output.output) · \(origin.source)").foregroundStyle(.secondary)
                        }
                        .font(.callout)
                    }
                }
                .padding(.leading, 122)
            }
        }
        .padding(.vertical, 7)
    }
}

/// Raw output data as hex and text side by side, sixteen bytes to a row. One cursor marks the
/// same byte in both columns. Move it with the arrow keys, Page Up, Page Down, Home and End,
/// or click a byte.
struct BytesView: View {
    let hex: String
    /// True when the pushes did not decode and the bytes are the script after `OP_RETURN`.
    var scriptBody = false

    @State private var cursor = 0
    @FocusState private var focused: Bool
    @Environment(\.pageCursor) private var pageCursor

    private static let columns = 16
    private static let pageRows = 16
    private static let limit = 8192

    private var bytes: [UInt8] {
        var result: [UInt8] = []
        result.reserveCapacity(hex.count / 2)
        var index = hex.startIndex
        while let next = hex.index(index, offsetBy: 2, limitedBy: hex.endIndex), index != next {
            if let byte = UInt8(hex[index..<next], radix: 16) { result.append(byte) }
            index = next
        }
        return result
    }

    var body: some View {
        let data = bytes
        let shown = Array(data.prefix(Self.limit))
        VStack(alignment: .leading, spacing: 6) {
            Text(scriptBody
                ? "The pushes did not decode. These are the script bytes after OP_RETURN."
                : "The payload bytes of every push, joined in order.")
                .font(.caption).foregroundStyle(.secondary)
            HStack(spacing: 6) {
                Text("Copy all the bytes as hexadecimal").font(.caption).foregroundStyle(.secondary)
                CopyButton(text: hex)
            }
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(Array(stride(from: 0, to: shown.count, by: Self.columns)), id: \.self) { offset in
                    row(offset: offset, bytes: shown)
                }
            }
            status(data: data, shownCount: shown.count)
        }
        .font(.callout.monospaced())
        .padding(10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 6))
        .focusable()
        .focused($focused)
        .focusEffectDisabled()
        .onKeyPress(keys: [.leftArrow, .rightArrow, .upArrow, .downArrow, .pageUp, .pageDown, .home, .end]) { press in
            move(press.key, count: shown.count)
            return .handled
        }
        // While the bytes have the keyboard, the arrow keys move the byte cursor.
        .onChange(of: focused) { _, now in pageCursor?.suspended = now }
        .onDisappear { pageCursor?.suspended = false }
        .accessibilityLabel("Bytes. The cursor is at byte \(cursor).")
    }

    private func row(offset: Int, bytes: [UInt8]) -> some View {
        let end = min(offset + Self.columns, bytes.count)
        return HStack(spacing: 14) {
            Text(String(format: "%04x", offset)).foregroundStyle(.tertiary)
            HStack(spacing: 0) {
                ForEach(offset..<end, id: \.self) { index in
                    Text(String(format: "%02x", bytes[index]))
                        .padding(.horizontal, 2)
                        .background(index == cursor ? Color.accentColor.opacity(0.35) : .clear)
                        .onTapGesture { select(index) }
                }
            }
            .frame(width: CGFloat(Self.columns) * 22, alignment: .leading)
            HStack(spacing: 0) {
                ForEach(offset..<end, id: \.self) { index in
                    Text(Self.printable(bytes[index]))
                        .background(index == cursor ? Color.accentColor.opacity(0.35) : .clear)
                        .foregroundStyle(.secondary)
                        .onTapGesture { select(index) }
                }
            }
        }
    }

    private func status(data: [UInt8], shownCount: Int) -> some View {
        let byte = cursor < shownCount ? data[cursor] : 0
        return VStack(alignment: .leading, spacing: 2) {
            if shownCount > 0 {
                Text("Cursor at offset \(cursor) (0x\(String(format: "%04x", cursor))): 0x\(String(format: "%02x", byte)), decimal \(byte), text \(Self.printable(byte))")
                    .textSelection(.enabled)
            }
            Text(data.count > shownCount
                ? "\(data.count) bytes. The view shows the first \(shownCount)."
                : "\(data.count) bytes. Dots stand for bytes that are not printable.")
                .foregroundStyle(.secondary)
        }
        .font(.caption)
        .padding(.top, 4)
    }

    private func select(_ index: Int) {
        cursor = index
        focused = true
    }

    private func move(_ key: KeyEquivalent, count: Int) {
        guard count > 0 else { return }
        let last = count - 1
        switch key {
        case .leftArrow: cursor = max(cursor - 1, 0)
        case .rightArrow: cursor = min(cursor + 1, last)
        case .upArrow: cursor = max(cursor - Self.columns, 0)
        case .downArrow: cursor = min(cursor + Self.columns, last)
        case .pageUp: cursor = max(cursor - Self.columns * Self.pageRows, 0)
        case .pageDown: cursor = min(cursor + Self.columns * Self.pageRows, last)
        case .home: cursor = 0
        case .end: cursor = last
        default: break
        }
    }

    static func printable(_ byte: UInt8) -> String {
        (32...126).contains(byte) ? String(UnicodeScalar(byte)) : "."
    }
}
