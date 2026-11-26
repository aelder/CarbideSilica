import CarbideCore
import SwiftUI

/// Peer data beside `NodeStatus`: the ban list, the one-time traffic totals and peer
/// locations from Carbide's location database.
@MainActor
@Observable
final class PeerFeed {
    static let shared = PeerFeed()

    private(set) var banned: [BannedPeer] = []
    private(set) var bannedError: String?
    private(set) var totals: NetworkTotals?
    private(set) var totalsError: String?
    private(set) var locations: PeerLocations?
    /// The place of this node, when the peers report a public address for it and the
    /// location data has that address.
    private(set) var home: PeerLocation?
    private(set) var installing = false
    private(set) var installMessage: String?
    /// The peer list's selection, kept for the session.
    var selection: String?
    /// The lowest and highest response time of each peer since the page opened, in
    /// milliseconds, by peer ID.
    private(set) var responseRange: [Int64: ClosedRange<Double>] = [:]

    /// Records the response time of each peer. The record of a peer that left is removed.
    func note(_ peers: [PeerRow]) {
        var kept: [Int64: ClosedRange<Double>] = [:]
        for peer in peers {
            let known = responseRange[peer.id]
            guard let ms = peer.pingMs else {
                kept[peer.id] = known
                continue
            }
            kept[peer.id] = min(known?.lowerBound ?? ms, ms)...max(known?.upperBound ?? ms, ms)
        }
        if kept != responseRange { responseRange = kept }
    }

    func loadBans(_ model: AppModel) async {
        guard let explorer = model.explorer else { return }
        do {
            banned = try await explorer.banList()
            bannedError = nil
        } catch {
            banned = []
            bannedError = describe(error)
        }
    }

    /// The terminal reads the totals once when Peers opens. So does the app.
    func loadTotals(_ model: AppModel) async {
        guard let explorer = model.explorer else { return }
        do {
            totals = try await explorer.networkTotals()
            totalsError = nil
        } catch {
            totals = nil
            totalsError = describe(error)
        }
    }

    func loadLocations(_ model: AppModel, addresses: [String]) async {
        guard let explorer = model.explorer else { return }
        locations = await explorer.peerLocations(addresses: addresses)
        // Each peer reports the address that it sees for this node. The place of that
        // address comes from the same local data: the app makes no request for it.
        var counts: [String: Int] = [:]
        for peer in model.node?.peers ?? [] {
            guard let data = peer.json.data(using: .utf8),
                let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let seen = object["addrlocal"] as? String, !seen.isEmpty
            else { continue }
            counts[seen, default: 0] += 1
        }
        guard let own = counts.max(by: { $0.value < $1.value })?.key else {
            home = nil
            return
        }
        let found = await explorer.peerLocations(addresses: [own]).locations.first
        home = found?.place == .located && found?.latitude != nil && found?.longitude != nil ? found : nil
    }

    /// Downloads the location database. Only the Download button in the confirmation calls it.
    func install(_ model: AppModel, addresses: [String]) async {
        guard let explorer = model.explorer, !installing else { return }
        installing = true
        installMessage = nil
        defer { installing = false }
        do {
            let release = try await explorer.installLocationDatabase()
            installMessage = "The app installed the location data for \(release)."
        } catch {
            installMessage = describe(error)
        }
        await loadLocations(model, addresses: addresses)
    }

    func location(of address: String) -> PeerLocation? {
        locations?.locations.first { $0.address == address }
    }
}

/// Connected peers on a map of the world, placed by Carbide's location database.
///
/// The app draws the map itself, from the coast points of the terminal's map, and makes no
/// request to a map service. The map opens on the area that holds all placed peers. Private
/// and unresolved peers have no place on a map. The terminal draws them in empty sea areas.
/// The app lists them below the map, so that each mark is a real position.
struct PeerMapCard: View {
    let peers: [PeerRow]
    let feed: PeerFeed
    let open: (PeerRow) -> Void

    @Environment(AppModel.self) private var model
    @State private var confirmingInstall = false

    var body: some View {
        Card("Connected node map", trailing: stateText) {
            // The map shows even with no pin, so the peers listed under it have their place.
            if let locations = feed.locations {
                map(locations)
            }
            if let locations = feed.locations {
                stateNote(locations)
                unplaced(locations)
            } else {
                Text("Reading the location data.").foregroundStyle(.secondary)
            }
            if let message = feed.installMessage {
                Text(message).font(.callout).foregroundStyle(.secondary)
            }
            // When the data is current, its date and the attribution are one line.
            if feed.locations?.state != .current { attribution }
        }
        .confirmationDialog(
            "Download the location data?", isPresented: $confirmingInstall, titleVisibility: .visible
        ) {
            Button("Download") {
                Task { await feed.install(model, addresses: peers.map(\.address)) }
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text(installText)
        }
    }

    /// Where the data goes and what is known of its size, before the download starts.
    private var installText: String {
        let locations = feed.locations
        var text =
            "The app downloads this month's DB-IP City Lite and ASN Lite databases from download.db-ip.com "
            + "and keeps them in \(locations?.directory ?? "the geoip folder beside the cache"). "
            + "The terminal app uses the same files. "
        if let installed = locations?.installedBytes, installed > 0 {
            text += "The data installed now uses \(Format.bytes(installed)). The new release replaces it. "
        }
        text += "Carbide does not request the size of the download before it starts. "
            + "The databases are large, and the download can take several minutes. "
        if locations?.defaultFolder ?? true {
            text += "This folder is in the default cache folder on the startup disk. "
                + "To keep the data on another disk, choose a cache folder in Settings first. "
                + "The location data follows the cache folder."
        } else {
            text += "This folder is in the cache folder that you chose in Settings."
        }
        return text
    }

    private var stateText: String? {
        guard let locations = feed.locations else { return nil }
        let placed = located(locations).count
        return "\(placed) of \(peers.count) placed"
    }

    private func located(_ locations: PeerLocations) -> [(PeerRow, PeerLocation)] {
        peers.compactMap { peer in
            guard let location = locations.locations.first(where: { $0.address == peer.address }),
                location.place == .located, location.latitude != nil, location.longitude != nil
            else { return nil }
            return (peer, location)
        }
    }

    @ViewBuilder
    private func map(_ locations: PeerLocations) -> some View {
        let pins = located(locations)
        if !pins.isEmpty {
            PeerOutlineMap(pins: pins, home: feed.home, open: open)
        }
    }

    @ViewBuilder
    private func stateNote(_ locations: PeerLocations) -> some View {
        switch locations.state {
        case .notInstalled:
            HStack {
                Text("The location data is not installed. The app cannot place peers on the map.")
                Spacer()
                installButton("Install Location Data…")
            }
        case .updateAvailable:
            HStack {
                Text("The location data is from \(locations.release ?? "an earlier month"). A newer release is available.")
                    .foregroundStyle(.secondary)
                Spacer()
                installButton("Update Location Data…")
            }
        case .unavailable:
            HStack {
                Note(text: "The location data is not available: \(locations.problem ?? "unknown error").", warning: true)
                Spacer()
                installButton("Install Location Data Again…")
            }
        case .current:
            HStack(alignment: .firstTextBaseline, spacing: 10) {
                Text("Location data from \(locations.release ?? "this month").")
                    .font(.caption).foregroundStyle(Tone.label)
                attribution
            }
        }
    }

    /// DB-IP's licence (CC BY 4.0) requires this attribution wherever its data appears.
    private var attribution: some View {
        Link("IP geolocation by DB-IP (CC BY 4.0)", destination: URL(string: "https://db-ip.com")!)
            .buttonStyle(.displayLink)
            .font(.caption)
    }

    private func installButton(_ title: String) -> some View {
        HStack(spacing: 6) {
            if feed.installing {
                ProgressView().controlSize(.small)
                Text("Downloading…").foregroundStyle(.secondary)
            } else {
                Button(title) { confirmingInstall = true }
            }
        }
    }

    /// Peers that have no position: private addresses and addresses the data cannot place.
    @ViewBuilder
    private func unplaced(_ locations: PeerLocations) -> some View {
        let privatePeers = peers.filter { locations.place(of: $0.address) == .private }
        let unresolved = peers.filter { locations.place(of: $0.address) == .unresolved }
        if !privatePeers.isEmpty {
            peerLine("Private or local address", privatePeers)
        }
        if !unresolved.isEmpty {
            peerLine(
                locations.state == .current || locations.state == .updateAvailable
                    ? "No location found for these addresses"
                    : "Not placed, because the location data is not available",
                unresolved)
        }
    }

    /// Peers without a public location, one per line, each a link to its page.
    private func peerLine(_ label: String, _ list: [PeerRow]) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            NameText("\(label) (\(list.count))")
            ForEach(list, id: \.id) { peer in
                HStack(spacing: 8) {
                    Button(peer.address) { open(peer) }.buttonStyle(.displayLink).font(.callout.monospaced())
                    Text("Peer \(peer.id)").foregroundStyle(.secondary)
                }
            }
        }
        .lineLimit(1)
    }

    /// "City, Country (CC)", as the terminal's peer overview.
    static func placeName(_ location: PeerLocation) -> String {
        let place = [location.city, location.country].compactMap { $0 }.joined(separator: ", ")
        switch (place.isEmpty, location.countryCode) {
        case (true, let code?): return code
        case (false, let code?): return "\(place) (\(code))"
        case (false, nil): return place
        case (true, nil): return "unknown"
        }
    }
}

extension PeerLocations {
    func place(of address: String) -> PeerPlace {
        locations.first { $0.address == address }?.place ?? .unresolved
    }
}

/// One peer: the terminal's groups (connection, traffic and latency, chain progress), its
/// location, the node's log lines for it, and its raw JSON.
struct PeerDetailView: View {
    let peerID: Int64
    let commands: PeerCommands

    @Environment(AppModel.self) private var model
    @Environment(\.dismiss) private var dismiss
    @State private var showsJSON = false
    @State private var log: [String] = []
    @State private var logError: String?

    private var peer: PeerRow? { model.node?.peers.first { $0.id == peerID } }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                LampSegments(options: [(false, "Overview"), (true, "JSON")], selection: $showsJSON)
                Spacer()
                if let peer {
                    Button("Disconnect…") { commands.confirmDisconnect(peer.address) }
                    Button("Ban…") { commands.confirmBan(peer.address) }
                }
                Button("Done") { dismiss() }
                    .keyboardShortcut(.cancelAction)
            }
            .padding(14)
            Divider()
            if let peer {
                ScrollView {
                    VStack(alignment: .leading, spacing: 18) {
                        header(peer)
                        if showsJSON {
                            Text(peer.json)
                                .font(.callout.monospaced())
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            CopyButton(text: peer.json)
                        } else {
                            location(peer)
                            ForEach(peerFieldGroups(json: peer.json), id: \.title) { group in
                                Card(group.title) {
                                    ForEach(group.fields, id: \.key) { field in
                                        FieldRow(label: field.label, value: display(field))
                                    }
                                }
                            }
                            logSection(peer)
                        }
                    }
                    .padding(20)
                }
            } else {
                Text("This peer is no longer connected.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(minWidth: 620, idealWidth: 700, minHeight: 520, idealHeight: 680)
        .task(id: peerID) { loadLog() }
    }

    private func header(_ peer: PeerRow) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("Peer \(peer.id)").font(.title2.weight(.semibold))
            Text(peer.address).font(.title3.monospaced()).textSelection(.enabled)
            Plate(peer.inbound ? "Inbound" : "Outbound", fill: peer.inbound ? Tone.record : nil)
            Spacer()
        }
    }

    @ViewBuilder
    private func location(_ peer: PeerRow) -> some View {
        if let location = PeerFeed.shared.location(of: peer.address) {
            Card("Location") {
                switch location.place {
                case .located:
                    FieldRow(label: "Location", value: PeerMapCard.placeName(location))
                    FieldRow(label: "ASN", value: location.asn.map { "AS\($0)" } ?? "unknown")
                    FieldRow(label: "Network operator", value: location.organization ?? "unknown")
                    Text("IP geolocation by DB-IP (CC BY 4.0). The location is approximate.")
                        .font(.caption).foregroundStyle(.secondary)
                case .private:
                    Text("A private or local address has no public location.").foregroundStyle(.secondary)
                case .unresolved:
                    Text("The location data has no location for this address, or the data is not installed.")
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

    private func logSection(_ peer: PeerRow) -> some View {
        Card("Node log", trailing: "lines with this peer's IP address") {
            if let logError {
                Note(text: logError, warning: true)
                Text("The app reads the debug.log file beside the node configuration file.")
                    .font(.caption).foregroundStyle(.secondary)
            } else if log.isEmpty {
                Text("No recent line of the node log contains this peer's IP address.")
                    .foregroundStyle(.secondary)
            } else {
                ScrollView {
                    Text(log.joined(separator: "\n"))
                        .font(.caption.monospaced())
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .frame(maxHeight: 220)
            }
            Button("Read Again") { loadLog() }
        }
    }

    private func loadLog() {
        guard let peer, let explorer = model.explorer else { return }
        do {
            log = try explorer.peerLog(address: peer.address)
            logError = nil
        } catch {
            log = []
            logError = describe(error)
        }
    }

    private func display(_ field: PeerField) -> String {
        switch field.kind {
        case .time:
            return UInt64(field.value).map(Format.date) ?? field.value
        case .bytes:
            return UInt64(field.value).map { "\(Format.bytes($0)) (\($0.formatted()) bytes)" } ?? field.value
        case .seconds:
            guard let seconds = Double(field.value) else { return field.value }
            return seconds < 1 ? "\(Int((seconds * 1000).rounded())) ms" : "\(seconds.formatted(.number.precision(.fractionLength(2)))) s"
        case .text:
            return field.value
        }
    }
}
