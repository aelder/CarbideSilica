import CarbideCore
import SwiftUI

/// Carbide's registration and association evidence: the registration transaction and name
/// commitment, funding candidates, the referral chain, shared addresses and authorities,
/// and import provenance. Each link keeps its source outpoint or request.
struct IdentityEvidenceCard: View {
    let name: String
    @Binding var evidence: EvidenceState
    let tip: UInt64?
    /// Incremented by the menu command for the historical reverse request.
    var requests = 0

    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Registration and associations", trailing: trailing) {
            switch evidence {
            case .loading:
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Registration and association evidence loading...").foregroundStyle(.secondary)
                }
            case .failed(let message):
                Note(text: "Evidence unavailable: \(message)", warning: true)
                Button("Try Again") { Task { await reload() } }
            case .loaded(let report):
                EvidenceBody(name: name, report: report)
                    .task(id: report.links.count) {
                        await model.resolveNames(
                            report.links.filter { $0.kind == .identity }.map(\.target))
                    }
            }
            Divider()
            HistoricalLookup(name: name, evidence: $evidence, tip: tip, requests: requests)
        }
    }

    private var trailing: String? {
        guard case .loaded(let report) = evidence else { return nil }
        return "\(report.links.count) evidence links"
    }

    private func reload() async {
        evidence = .loading
        do {
            evidence = .loaded(try await model.requireExplorer().identityEvidence(name: name))
        } catch {
            evidence = .failed(describe(error))
        }
    }
}

private struct EvidenceBody: View {
    let name: String
    let report: IdentityEvidenceReport

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            if let registration = report.registration {
                HStack(spacing: 8) {
                    NameText(report.imported ? "Imported" : "Registration")
                    Text(registration).font(.body.monospaced()).textSelection(.enabled)
                    if let link = report.links.first(where: {
                        $0.label == "Registration transaction" || $0.label == "Identity import transaction"
                    }) {
                        RouteLink(text: link.target, route: .transaction(txid: link.target))
                    }
                }
            }
            if !referralChain.isEmpty {
                ReferralChain(identity: name, chain: referralChain)
            }
            if report.links.isEmpty {
                Text("No additional associations to display").foregroundStyle(.secondary)
            }
            ForEach(groups, id: \.title) { group in
                if group.title == Self.earlierAuthorities {
                    // The lanes at the top of the page show who held each power at each
                    // version. The rows stay as evidence, closed at the start.
                    DisclosureGroup("\(group.title) (\(group.links.count) rows)") {
                        VStack(alignment: .leading, spacing: 0) {
                            Text("The lanes at the top of the page show the same authorities for each version.")
                                .font(.callout).foregroundStyle(.secondary).padding(.vertical, 4)
                            ForEach(Array(group.links.enumerated()), id: \.offset) { index, link in
                                if index > 0 { Divider() }
                                EvidenceRow(link: link)
                            }
                        }
                    }
                    .font(.headline)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        NameText(group.title).padding(.bottom, 4)
                        ForEach(Array(group.links.enumerated()), id: \.offset) { index, link in
                            if index > 0 { Divider() }
                            EvidenceRow(link: link)
                        }
                    }
                }
            }
            notes
            storage
        }
    }

    /// The referrer, then each ancestor in order of depth.
    private var referralChain: [String] {
        var chain: [String] = []
        if let direct = report.links.first(where: { $0.label == "Referral" }) {
            chain.append(direct.target)
        }
        chain += report.links.filter { $0.label == "Referral ancestor" }.map(\.target)
        return chain
    }

    private static let earlierAuthorities = "Authorities in earlier versions"

    private struct Group {
        let title: String
        let links: [EvidenceLinkRow]
    }

    private var groups: [Group] {
        let order: [(String, (String) -> Bool)] = [
            ("Registration", { label in
                ["Registration transaction", "Name commitment", "Parent", "Name ID",
                 "Reservation output destination", "Commitment control destination",
                 "Identity import transaction"].contains(label)
            }),
            ("Funding candidates", { $0.hasSuffix("funding candidate") }),
            ("Referral chain", { $0 == "Referral" || $0 == "Referral ancestor" }),
            (Self.earlierAuthorities, { $0.hasPrefix("Historical recovery") || $0.hasPrefix("Historical revocation") }),
            ("Shared now", { $0.hasPrefix("Shared ") }),
            ("Shared in a past window", { $0.hasPrefix("Historical shared") }),
        ]
        var used = Set<Int>()
        var result: [Group] = []
        for (title, matches) in order {
            var links: [EvidenceLinkRow] = []
            for (index, link) in report.links.enumerated() where !used.contains(index) && matches(link.label) {
                used.insert(index)
                links.append(link)
            }
            if !links.isEmpty { result.append(Group(title: title, links: links)) }
        }
        let rest = report.links.enumerated().filter { !used.contains($0.offset) }.map(\.element)
        if !rest.isEmpty { result.append(Group(title: "Other", links: rest)) }
        return result
    }

    @ViewBuilder
    private var notes: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(Array(report.notes.enumerated()), id: \.offset) { _, note in
                Text(note).font(.callout).foregroundStyle(.secondary)
            }
            Text("Association is evidence, not ownership; private funding may be invisible")
                .font(.callout).foregroundStyle(.secondary)
            if !report.historyComplete {
                Note(text: "Registration evidence awaits complete identity history", warning: true)
            }
        }
    }

    @ViewBuilder
    private var storage: some View {
        if report.retainedOnly {
            Note(text: "The live refresh failed. These links come from stored evidence.", warning: true)
        }
        if report.saved {
            Text(storedText).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
        }
    }

    private var storedText: String {
        var text = "Stored with the version at \(model.heightText(report.versionHeight))"
        if let block = report.blockHash { text += ", anchored to block \(Format.short(block))" }
        if let source = report.sourceId { text += " from source \(Format.short(source, head: 16, tail: 6))" }
        return text + ". A reorganization that removes that block makes the stored links invalid."
    }
}

/// The identity and who referred it, back as far as Carbide follows: four ancestors.
private struct ReferralChain: View {
    let identity: String
    let chain: [String]

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    chip { IDLink(id: identity, route: .identity(name: identity)) }
                    ForEach(Array(chain.enumerated()), id: \.offset) { depth, referrer in
                        VStack(spacing: 0) {
                            Image(systemName: "arrow.left").foregroundStyle(Tone.axis)
                            NameText(depth == 0 ? "referred by" : "who was referred by")
                        }
                        chip { IDLink(id: referrer, route: .identity(name: referrer)) }
                    }
                }
                .padding(.vertical, 2)
            }
            Text("Each box is an identity. An arrow points from an identity to the identity named as its referrer in its registration.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private func chip(@ViewBuilder _ content: () -> some View) -> some View {
        content()
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .overlay(Rectangle().strokeBorder(Tone.axis, lineWidth: 1))
    }
}

/// An evidence source, such as "i5w5…ZcV e8e77a…:4": the system by name, a transaction
/// outpoint as a short link, and long IDs shortened. The full text is in the tooltip and
/// the copy button.
private struct EvidenceSource: View {
    let source: String

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    private enum Piece: Hashable {
        case text(String)
        case outpoint(txid: String, rest: String)
    }

    var body: some View {
        HStack(spacing: 4) {
            NameText("Source")
            ForEach(Array(pieces.enumerated()), id: \.offset) { _, piece in
                switch piece {
                case .text(let text):
                    Text(text).foregroundStyle(.secondary).lineLimit(1)
                case .outpoint(let txid, let rest):
                    Button(Format.short(txid, head: 6, tail: 6) + rest) {
                        model.open(.transaction(txid: txid))
                    }
                    .buttonStyle(.displayLink)
                    .lineLimit(1)
                    .help(txid)
                }
            }
            CopyButton(text: source)
        }
        .font(.caption)
        .help(source)
        .task(id: source) {
            await model.resolveNames(words.filter(isIdentityID))
        }
    }

    private var words: [String] { source.split(separator: " ").map(String.init) }

    private func isIdentityID(_ word: String) -> Bool {
        word.hasPrefix("i") && word.count == 34
    }

    /// The words of the source, with outpoints and IDs made short. At most six, so the line
    /// stays short.
    private var pieces: [Piece] {
        var result: [Piece] = []
        for word in words.prefix(6) {
            let trimmed = word.trimmingCharacters(in: CharacterSet(charactersIn: ";,"))
            let parts = trimmed.split(separator: ":", maxSplits: 1).map(String.init)
            if let txid = parts.first, txid.count == 64, txid.allSatisfy({ $0.isHexDigit }) {
                result.append(.outpoint(txid: txid, rest: parts.count > 1 ? ":\(parts[1])" : ""))
            } else if isIdentityID(trimmed) {
                result.append(.text(showsNames ? model.displayID(trimmed) : Format.short(trimmed, head: 8, tail: 6)))
            } else if let equals = trimmed.firstIndex(of: "="), isIdentityID(String(trimmed[trimmed.index(after: equals)...])) {
                let id = String(trimmed[trimmed.index(after: equals)...])
                result.append(.text(trimmed[..<equals] + "=" + model.displayID(id)))
            } else {
                result.append(.text(trimmed.count > 28 ? Format.short(trimmed, head: 14, tail: 6) : trimmed))
            }
        }
        if words.count > 6 { result.append(.text("…")) }
        return result
    }
}

private struct EvidenceRow: View {
    let link: EvidenceLinkRow

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            ConfidenceMark(level: link.confidence, label: link.confidence == .candidate ? "Candidate" : "Observed")
                .font(.caption)
                .frame(width: 110, alignment: .leading)
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 8) {
                    Text(link.label)
                    IDLink(id: link.target, route: route)
                }
                if !link.detail.isEmpty {
                    Text(link.detail).font(.callout).foregroundStyle(.secondary).textSelection(.enabled)
                }
                EvidenceSource(source: link.source)
            }
            Spacer(minLength: 0)
        }
        .padding(.vertical, 6)
    }

    private var route: Route {
        switch link.kind {
        case .identity: .identity(name: link.target)
        case .address: .address(address: link.target)
        case .transaction: .transaction(txid: link.target)
        }
    }
}

/// Carbide's `v`: a reverse request over the 1,000 blocks that end at a chosen height.
private struct HistoricalLookup: View {
    let name: String
    @Binding var evidence: EvidenceState
    let tip: UInt64?
    let requests: Int

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsDates) private var showsDates = false
    @State private var endHeight = ""
    @State private var running = false
    @State private var result: HistoricalEvidence?
    @State private var failure: String?

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            NameText("Historical reverse request")
                .onChange(of: requests) { Task { await run() } }
            Text("Find identities that used a known authority or primary address of this identity in one window of 1,000 blocks. The request needs a source with the identity index. The links found are stored with this version and its block.")
                .font(.callout).foregroundStyle(.secondary)
            HStack {
                NameText("Window ends at block")
                TextField("Block", text: $endHeight, prompt: Text(tip.map { "\($0)" } ?? "height"))
                    .displayField()
                    .frame(width: 130)
                    .onSubmit { Task { await run() } }
                Button("Request") { Task { await run() } }.disabled(running)
                if running { ProgressView().controlSize(.small) }
                Spacer()
            }
            if let failure {
                Note(text: failure, warning: true)
            }
            if let result {
                Text(summary(result))
                if result.indexMissing {
                    Note(text: "The source has no identity index. It cannot answer historical reverse requests. Start the node with -idindex=1, or enable a public API in Settings.", warning: true)
                } else if result.unavailable > 0 {
                    Note(text: "\(result.unavailable) of \(result.requests) requests did not get an answer.", warning: true)
                }
            }
        }
    }

    private func summary(_ result: HistoricalEvidence) -> String {
        let window = "From \(model.heightText(result.fromHeight)) to \(model.heightText(result.toHeight))"
        switch result.found {
        case 0: return "\(window): no new links. Other windows can give different results."
        case 1: return "\(window): 1 new link. It is in the list above."
        default: return "\(window): \(result.found) new links. They are in the list above."
        }
    }

    private func run() async {
        let typed = endHeight.trimmingCharacters(in: .whitespaces)
        guard let height = typed.isEmpty ? tip : UInt64(typed) else {
            failure = "Enter a block height."
            return
        }
        running = true
        failure = nil
        defer { running = false }
        do {
            let found = try await model.requireExplorer().identityHistoricalEvidence(
                name: name, endHeight: height)
            result = found
            evidence = .loaded(found.report)
        } catch {
            failure = describe(error)
        }
    }
}
