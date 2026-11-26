import CarbideCore
import SwiftUI

// Small pieces shared by the identity and address pages.

/// A link to an identity, address, currency or transaction. It shows the resolved name
/// when the app-wide "Show Names for IDs" setting is on.
struct IDLink: View {
    let id: String
    let route: Route
    var shorten = true

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    private var isCurrency: Bool {
        if case .currency = route { return true }
        return false
    }

    var body: some View {
        Group {
            if showsNames, let name = model.names[id] {
                Button(isCurrency && name.hasSuffix("@") ? String(name.dropLast()) : name) {
                    model.open(route)
                }
                .buttonStyle(.displayLink)
                .lineLimit(1)
                .help(id)
            } else {
                Button(shorten ? Format.short(id) : id) { model.open(route) }
                    .buttonStyle(.displayLink)
                    .font(.body.monospaced())
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .help(id)
            }
        }
        .cursorStop(token: "\(route)", copy: id) { model.open(route) }
    }
}

/// A label and a linked ID, with a copy button.
struct IDRow: View {
    let label: String
    let id: String
    let route: Route
    var note: String?

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            NameText(label)
                .frame(width: 170, alignment: .leading)
            IDLink(id: id, route: route, shorten: false)
            if let note {
                Text(note).font(.callout).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            CopyButton(text: id)
        }
    }
}

/// A label and a plain value.
struct ValueRow: View {
    let label: String
    let value: String
    var warning = false

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            NameText(label)
                .frame(width: 170, alignment: .leading)
            if warning {
                Label(value, systemImage: "exclamationmark.triangle").foregroundStyle(Tone.mined)
            } else {
                Text(value).textSelection(.enabled)
            }
            Spacer(minLength: 0)
        }
    }
}

/// Loads data for one section and shows progress or an error in the section, not over
/// the whole page.
struct SectionLoader<Value: Sendable, Content: View>: View {
    let key: String
    var loadingText = "Loading…"
    let load: (Explorer) async throws -> Value
    @ViewBuilder let content: (Value) -> Content

    @Environment(AppModel.self) private var model
    @State private var phase: Phase = .loading
    @State private var attempt = 0

    private enum Phase {
        case loading
        case failed(String)
        case loaded(Value)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            switch phase {
            case .loading:
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text(loadingText).foregroundStyle(.secondary)
                }
            case .failed(let message):
                Note(text: message, warning: true)
                Button("Try Again") { attempt += 1 }
            case .loaded(let value):
                content(value)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .task(id: "\(key)|\(model.generation)|\(attempt)") {
            phase = .loading
            do {
                phase = .loaded(try await load(try model.requireExplorer()))
            } catch is CancellationError {
            } catch {
                phase = .failed(describe(error))
            }
        }
    }
}

extension AppModel {
    /// A currency's display name: the chain's own coin, a resolved name without "@", or a
    /// shortened ID.
    func currencyName(_ id: String) -> String {
        if id == status?.chainId || id == coin { return coin }
        guard Display.showsNames else { return Format.short(id, head: 8, tail: 6) }
        if let name = names[id] { return name.hasSuffix("@") ? String(name.dropLast()) : name }
        return Format.short(id, head: 8, tail: 6)
    }

    /// An identity or currency ID as the "Show Names for IDs" setting says: the name, or a
    /// shortened ID.
    func displayID(_ id: String) -> String {
        if Display.showsNames, let name = names[id] { return name }
        return Format.short(id, head: 8, tail: 6)
    }

    /// A block height as the "Show Dates for Heights" setting says. `time` is the block's
    /// time when it is known.
    func heightText(_ height: UInt64, time: UInt64? = nil) -> String {
        Display.showsDates
            ? HeightDates.shared.text(height, known: time, model: self)
            : "block \(Format.integer(height))"
    }

    /// An amount in satoshis with its ticker.
    func amountText(_ sats: Int64, currency: String) -> String {
        "\(Format.coins(sats)) \(currencyName(currency))"
    }
}

/// A height and the date of its block, as Carbide's `format_identity_height_time`: the date
/// alone when dates are shown for heights, otherwise "block 123, date".
@MainActor
func heightAndTime(_ height: UInt64, _ time: UInt64?, model: AppModel) -> String {
    if Display.showsDates { return HeightDates.shared.text(height, known: time, model: model) }
    guard let time else { return "block \(Format.integer(height)), time not available" }
    return "block \(Format.integer(height)), \(Format.date(time))"
}
