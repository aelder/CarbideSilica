import CarbideCore
import SwiftUI

/// A VerusID at one of its versions: who controls it, how it was registered, what it is
/// associated with, and its transactions.
///
/// The route names only the identity. The version being viewed is state of this page:
/// it starts at the current version, and the version strip, the history list and the Go
/// menu's Previous Version and Next Version step through the others.
struct IdentityView: View {
    let name: String

    @State private var refreshes = 0

    var body: some View {
        LoadView(key: "identity-timeline:\(name)|\(refreshes)") { explorer in
            try await explorer.identityTimeline(name: name)
        } content: { timeline in
            if timeline.versions.isEmpty {
                ContentUnavailableView(
                    "No versions", systemImage: "person.crop.circle.badge.questionmark",
                    description: Text("The source returned this identity without a version."))
            } else {
                IdentityPage(timeline: timeline, refresh: { refreshes += 1 })
            }
        }
        .navigationTitle("Identity")
    }
}

private struct IdentityPage: View {
    let timeline: IdentityTimeline
    let refresh: () -> Void

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false
    @State private var viewed: Int
    @State private var evidence: EvidenceState = .loading
    @State private var offers: IdentityOffers?
    @State private var activity: AddressOverview?
    /// Set when a later tip shows that the current version's block left the active chain.
    @State private var reorganized = false
    /// Incremented by the menu command for the historical reverse request.
    @State private var historicalRequests = 0

    init(timeline: IdentityTimeline, refresh: @escaping () -> Void) {
        self.timeline = timeline
        self.refresh = refresh
        _viewed = State(initialValue: timeline.versions.count - 1)
    }

    private var stale: Bool { timeline.stale || reorganized }

    /// As the terminal: offer badges show only while the node's tip is healthy.
    private var tipHealthy: Bool { model.status != nil && model.statusError == nil }

    private var state: IdentityState { timeline.versions[viewed] }
    private var latestIndex: Int { timeline.versions.count - 1 }
    private var isLatest: Bool { viewed == latestIndex }

    var body: some View {
        Page {
            header
            ProvenanceNote(provenance: timeline.provenance)
            VersionStrip(versions: timeline.versions, tip: timeline.tipHeight, selection: $viewed)
            if !isLatest {
                pastVersionNote
            }

            IdentityControlCard(identity: state)
            propertiesCard
            balancesCard

            if state.tokenizedControl {
                TokenControlCard(
                    identityAddress: timeline.identityAddress, viewingPast: !isLatest)
            }

            IdentityEvidenceCard(
                name: timeline.identityAddress, evidence: $evidence,
                tip: timeline.tipHeight, requests: historicalRequests)

            if let first = timeline.versions.first, let txid = first.txid {
                IdentityOriginCard(
                    txid: txid, height: first.height, time: first.time,
                    identityAddress: timeline.identityAddress)
                RegistrationPaymentsCard(txid: txid)
            }

            IdentityHistoryCard(timeline: timeline, selection: $viewed)

            Card("Transactions") {
                AddressActivityList(address: timeline.identityAddress)
            }

            JSONSection(json: state.json, filename: "id-\(timeline.identityAddress).json")
        }
        .publishAdjacent(AdjacentObjects(
                noun: "Version",
                previous: viewed > 0 ? { viewed -= 1 } : nil,
                next: isLatest ? nil : { viewed += 1 }))
        .publishInvestigations(InvestigationActions(sharedAuthorities: { historicalRequests += 1 }))
        .publishPlace(PlaceActions(refresh: refresh))
        .task(id: "\(timeline.identityAddress)|\(model.generation)") {
            await load()
        }
        .task(id: "\(timeline.identityAddress)|\(model.generation)|offers") {
            // The terminal rechecks offers every 30 seconds while the identity is open.
            while !Task.isCancelled {
                if let explorer = model.explorer {
                    offers = try? await explorer.identityOffers(identityAddress: timeline.identityAddress)
                }
                try? await Task.sleep(for: .seconds(30))
            }
        }
        .task(id: model.status?.blocks) {
            await recheckStale()
        }
        .task(id: "\(timeline.identityAddress)|names") {
            var ids: [String?] = [timeline.identityAddress]
            for version in timeline.versions {
                ids += [version.parent, version.systemId, version.revocationAuthority,
                        version.recoveryAuthority]
            }
            await model.resolveNames(ids)
        }
    }

    // MARK: - Header

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            PageHeader(
                kind: "Identity",
                title: timeline.name,
                subtitle: timeline.identityAddress,
                tags: [])
            HStack(spacing: 6) {
                statusChip
                if state.definesCurrency {
                    Button {
                        model.open(.currency(name: timeline.identityAddress))
                    } label: {
                        Label("Currency defined", systemImage: "circle.hexagongrid")
                            .font(.caption.weight(.medium))
                    }
                    .buttonStyle(.link)
                    .help("Open the currency that this identity defines")
                }
                if state.tokenizedControl {
                    TagChip(text: "Tokenized control")
                } else if state.flags == 0 {
                    Text("No flags set").font(.caption).foregroundStyle(.secondary)
                }
                if let offers, tipHealthy {
                    if offers.buy { TagChip(text: "Buy offers", color: .purple) }
                    if offers.sell { TagChip(text: "Sell offers", color: .purple) }
                }
            }
            versionStepper
        }
    }

    private var statusChip: some View {
        let stale = stale && isLatest
        let word = stale ? "Stale after reorg" : state.status.capitalized
        let good = !stale && state.status.lowercased() == "active"
        return Label(word, systemImage: good ? "checkmark.circle" : "exclamationmark.triangle")
            .font(.caption.weight(.semibold))
            .foregroundStyle(good ? .green : .red)
            .padding(.horizontal, 8)
            .padding(.vertical, 3)
            .background((good ? Color.green : Color.red).opacity(0.12), in: Capsule())
    }

    private var versionStepper: some View {
        HStack(spacing: 10) {
            Button {
                viewed = max(viewed - 1, 0)
            } label: {
                Label("Previous version", systemImage: "chevron.left")
            }
            .disabled(viewed == 0)
            .help("Previous version (Option-Command-[)")

            Text("Version \(viewed + 1) of \(timeline.versions.count)")
                .monospacedDigit()
                .fontWeight(.semibold)
            Text(isLatest ? "current" : "past").foregroundStyle(isLatest ? .green : .orange)

            Button {
                viewed = min(viewed + 1, latestIndex)
            } label: {
                Label("Next version", systemImage: "chevron.right")
            }
            .disabled(isLatest)
            .help("Next version (Option-Command-])")

            if !isLatest {
                Button("Show the current version") { viewed = latestIndex }
                    .buttonStyle(.link)
            }
        }
        .labelStyle(.iconOnly)
    }

    private var pastVersionNote: some View {
        Note(
            text: "This page shows the identity as it was after the update at "
                + "\(heightAndTime(state.height, state.time, model: model)). Balances, evidence and "
                + "transactions are for the identity now.",
            warning: true)
    }

    // MARK: - Properties

    private var propertiesCard: some View {
        Card("Identity at version \(viewed + 1)") {
            ValueRow(label: "Name", value: state.name)
            if let fqn = timeline.fullyQualifiedName {
                ValueRow(label: "Fully qualified", value: fqn)
            }
            ValueRow(label: "Version", value: state.formatVersion.map { "\($0)" } ?? "unknown")
            ValueRow(label: "Status", value: statusText, warning: statusWarning)
            ValueRow(label: "Timelock", value: timelockText)
            if let parent = state.parent {
                IDRow(label: "Parent", id: parent, route: .identity(name: parent))
            }
            if let system = state.systemId {
                IDRow(label: "System", id: system, route: .currency(name: system))
            }
            authorityRow("Revocation", state.revocationAuthority, isSelf: state.revocationIsSelf)
            authorityRow("Recovery", state.recoveryAuthority, isSelf: state.recoveryIsSelf)
            if let privateAddress = state.privateAddress {
                ValueRow(label: "Private address", value: privateAddress)
            }
            ValueRow(label: registeredLabel, value: registeredText)
            ValueRow(label: "Updated", value: updatedText)
            ValueRow(label: "Viewing update", value: heightAndTime(state.height, state.time, model: model))
            if state.onActiveChain == false {
                Note(
                    text: "The block of this update is not on the active chain. A reorganization "
                        + "removed it.",
                    warning: true)
            }
            ValueRow(
                label: "Versions",
                value: "\(timeline.versions.count)"
                    + (timeline.historyComplete ? "" : " (the history is not complete)"))
            if let note = timeline.historyNote {
                Note(text: note, warning: true)
            }
            if let txid = state.txid {
                IDRow(
                    label: "Update transaction", id: txid, route: .transaction(txid: txid),
                    note: state.output.map { "output \($0)" })
            }
        }
    }

    @ViewBuilder
    private func authorityRow(_ label: String, _ id: String?, isSelf: Bool) -> some View {
        if isSelf {
            ValueRow(label: label, value: "self")
        } else if let id {
            IDRow(label: label, id: id, route: .identity(name: id))
        } else {
            ValueRow(label: label, value: "unknown")
        }
    }

    private var statusText: String {
        if stale && isLatest { return "Stale after reorg. The page is refreshing." }
        return state.status
    }

    private var statusWarning: Bool {
        (stale && isLatest) || state.status.lowercased() != "active"
    }

    private var timelockText: String {
        switch state.timelock {
        case .unlocked: "Unlocked"
        case .delay(let blocks):
            "Locked. An unlock takes effect \(Format.integer(blocks)) block\(blocks == 1 ? "" : "s") after it is requested."
        case .unlocksAt(let height): "Locked until \(model.heightText(height))"
        }
    }

    private var registeredLabel: String {
        if case .loaded(let report) = evidence, report.imported { return "Imported" }
        return "Registered"
    }

    private var registeredText: String {
        guard let first = timeline.versions.first else { return "unknown" }
        guard timeline.historyComplete else { return "History not complete" }
        return heightAndTime(first.height, first.time, model: model)
    }

    private var updatedText: String {
        guard let first = timeline.versions.first, let last = timeline.versions.last else {
            return "unknown"
        }
        guard timeline.historyComplete else { return "History not complete" }
        return last.height == first.height ? "never" : heightAndTime(last.height, last.time, model: model)
    }

    // MARK: - Balances

    private var balancesCard: some View {
        Card("Balances and activity", trailing: "now") {
            if let activity {
                if let error = activity.balanceError {
                    Note(text: "The balances did not load: \(error)", warning: true)
                } else if activity.balances.isEmpty {
                    Text("No balances.").foregroundStyle(.secondary)
                } else {
                    BalanceList(balances: activity.balances)
                }
                Text(activitySummary(activity)).foregroundStyle(.secondary)
            } else {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Loading balances…").foregroundStyle(.secondary)
                }
            }
        }
    }

    private func activitySummary(_ overview: AddressOverview) -> String {
        if let error = overview.historyError { return "The activity did not load: \(error)" }
        var text = "\(Format.integer(overview.transactionCount)) transaction"
            + (overview.transactionCount == 1 ? "" : "s")
        if let first = overview.firstTime { text += ", first \(Format.date(first))" }
        if let latest = overview.latestTime { text += ", latest \(Format.date(latest))" }
        return text + "."
    }

    // MARK: - Loading

    /// When the tip changes, checks that the current version's block is still on the active
    /// chain. If it is not, the page says so and loads the identity again.
    private func recheckStale() async {
        guard !timeline.provenance.retained, let explorer = model.explorer,
            let hash = timeline.versions.last?.blockHash
        else { return }
        let checked = await explorer.blocksActive(hashes: [hash])
        guard !Task.isCancelled, checked.first?.active == false else { return }
        reorganized = true
        try? await Task.sleep(for: .seconds(2))
        if !Task.isCancelled { refresh() }
    }

    private func load() async {
        guard let explorer = model.explorer else { return }
        let address = timeline.identityAddress
        evidence = .loading
        async let overview = try? explorer.addressOverview(address: address)
        async let report = Self.evidence(explorer, address)
        activity = await overview
        if let overview = activity {
            await model.resolveNames(overview.balances.map(\.currencyId))
        }
        evidence = await report
    }

    private static func evidence(_ explorer: Explorer, _ address: String) async -> EvidenceState {
        do {
            return .loaded(try await explorer.identityEvidence(name: address))
        } catch {
            return .failed(describe(error))
        }
    }
}

/// The evidence for the current version, shared by the header and the evidence section.
enum EvidenceState {
    case loading
    case failed(String)
    case loaded(IdentityEvidenceReport)
}

/// Balances by currency, the chain's own coin first.
struct BalanceList: View {
    let balances: [AddressBalance]
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            ForEach(balances, id: \.currencyId) { balance in
                HStack(spacing: 12) {
                    Button(label(balance)) {
                        model.open(.currency(name: balance.native ? model.coin : balance.currencyId))
                    }
                    .buttonStyle(.link)
                    .lineLimit(1)
                    .help(balance.currencyId)
                    .frame(width: 170, alignment: .leading)
                    Text(Format.coins(balance.amountSats)).monospacedDigit()
                    Text(label(balance)).foregroundStyle(.secondary)
                    Spacer(minLength: 0)
                }
            }
        }
    }

    private func label(_ balance: AddressBalance) -> String {
        if balance.native { return model.coin }
        if showsNames, let name = model.names[balance.currencyId] {
            return name.hasSuffix("@") ? String(name.dropLast()) : name
        }
        return balance.name
    }
}
