import CarbideCore
import Observation
import SwiftUI

/// The currency browser's list, filters and selection, kept for the session so that they
/// survive going back and visits to other places in the sidebar.
@MainActor
@Observable
final class CurrencyBrowserState {
    static let shared = CurrencyBrowserState()

    var rows: [CurrencyListing] = []
    var source: String?
    /// Where the list came from: live, or stored from the last answer.
    var provenance: Provenance?
    var failure: String?
    var loading = false
    /// The connection the rows came from. A new connection loads a new list.
    var generation: Int?
    var filter = defaultCurrencyFilter()
    /// The currency opened last, so the list shows where the user was.
    var selected: String?
    var showFlags = false
}

/// Requests to open a currency page with its notarization browser on one perspective.
/// A notarization bookmark saves the observing chain, and this carries it to the page.
@MainActor
@Observable
final class NotarizationRequests {
    static let shared = NotarizationRequests()

    /// Currency name or ID, as routed, to the observing system's ID.
    private(set) var observers: [String: String] = [:]

    func request(currency: String, observer: String) {
        observers[currency] = observer
    }

    /// Returns and forgets a request for any of the currency's names.
    func take(_ keys: [String?]) -> String? {
        for key in keys.compactMap({ $0 }) {
            if let observer = observers.removeValue(forKey: key) {
                return observer
            }
        }
        return nil
    }
}

extension LaunchState {
    var word: String {
        switch self {
        case .launched: "Launched"
        case .prelaunch: "Prelaunch"
        case .failed: "Failed launch (refunding)"
        case .unknown: "Unknown"
        }
    }

    static let all: [LaunchState] = [.launched, .prelaunch, .failed, .unknown]
}

extension DataSource {
    var sourceWord: String {
        switch self {
        case .localDaemon: "Local node"
        case .publicApi: "Public API"
        }
    }
}

/// A currency or system name for display: a resolved name without the trailing "@",
/// else the given fallback, else a shortened ID. When names are off in the View menu, the
/// ID. Views that call it read `DisplayKey.showsNames` with `@AppStorage` to redraw.
@MainActor
func knownName(_ id: String, model: AppModel, fallback: String? = nil) -> String {
    guard Display.showsNames else { return id }
    if id == model.status?.chainId { return model.coin }
    if let name = model.names[id] {
        return name.hasSuffix("@") ? String(name.dropLast()) : name
    }
    if let fallback, !fallback.isEmpty { return fallback }
    return Format.short(id, head: 6, tail: 4)
}

/// Carbide's source IDs are `kind:digest`. Show the kind and a short digest.
func observationSource(_ source: String) -> String {
    guard let (kind, id) = source.split(separator: ":", maxSplits: 1).map(String.init).pair else {
        return source
    }
    let word = kind == "local-daemon" ? "Local node" : kind == "public-api" ? "Public API" : kind
    return "\(word) \(Format.short(id, head: 6, tail: 4))"
}

private extension Array {
    var pair: (Element, Element)? { count == 2 ? (self[0], self[1]) : nil }
}

/// A height as a number, or as a date when dates are on in the View menu.
@MainActor
func heightText(_ height: UInt64, known: UInt64? = nil, model: AppModel) -> String {
    HeightDates.shared.text(height, known: known, model: model)
}

/// Names the chain of a block or transaction that is on another chain. The app shows one
/// chain at a time, so the object stays text. When a node for that chain is configured on
/// this Mac, the person can make it the current chain.
struct OtherChainNote: View {
    let systemId: String
    var fallback: String?
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @State private var confirming: SourceRow?

    var body: some View {
        let name = showsNames ? knownName(systemId, model: model, fallback: fallback) : systemId
        HStack(spacing: 6) {
            Text("on \(name)").font(.caption).foregroundStyle(Tone.label)
            if let row = node {
                Button("Switch Chain…") { confirming = row }
                    .buttonStyle(.displayLink)
                    .font(.caption)
                    .help("Make \(name) the current chain. The app then shows its blocks and transactions.")
            }
        }
        .confirmationDialog(
            "Switch to \(name)?",
            isPresented: Binding(get: { confirming != nil }, set: { if !$0 { confirming = nil } }),
            presenting: confirming
        ) { row in
            Button("Switch Chain") { model.switchChain(to: row) }
        } message: { _ in
            Text("The app connects to the \(name) node and closes the pages that are open now.")
        }
    }

    /// A configured node for the chain, which `switchChain(to:)` can open.
    private var node: SourceRow? {
        model.explorer?.sourcesForSystem(systemId: systemId).first {
            $0.kind == .localNode && $0.configFile != nil && !$0.current
        }
    }
}
