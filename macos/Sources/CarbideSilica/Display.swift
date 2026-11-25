import CarbideCore
import Foundation
import Observation

/// `UserDefaults` keys for the display settings. Views that show these values read them with
/// `@AppStorage(DisplayKey.showsNames) private var showsNames = true`, so they redraw when the
/// setting changes.
enum DisplayKey {
    /// Show names in place of identity and currency IDs. On by default.
    static let showsNames = "showsNames"
    /// Show dates for block heights, confirmation counts and Unix timestamps. Off by default.
    static let showsDates = "showsDates"
}

/// The display settings, as the terminal's Shift-I and Shift-D toggles.
///
/// The values are read when called. Code that must redraw on a change uses `@AppStorage`
/// with the keys in `DisplayKey`.
enum Display {
    static var showsNames: Bool {
        UserDefaults.standard.object(forKey: DisplayKey.showsNames) as? Bool ?? true
    }

    static var showsDates: Bool {
        UserDefaults.standard.bool(forKey: DisplayKey.showsDates)
    }

    static var usesUTC: Bool {
        UserDefaults.standard.bool(forKey: SettingsKey.useUTC)
    }

    static func toggleNames() {
        UserDefaults.standard.set(!showsNames, forKey: DisplayKey.showsNames)
    }

    static func toggleDates() {
        UserDefaults.standard.set(!showsDates, forKey: DisplayKey.showsDates)
    }

    static func toggleUTC() {
        UserDefaults.standard.set(!usesUTC, forKey: SettingsKey.useUTC)
    }

    /// The name to show for an identity or currency ID, or the ID when names are off or the
    /// name is not known. A currency name has no trailing "@".
    @MainActor
    static func name(for id: String, in model: AppModel, currency: Bool = false) -> String {
        guard showsNames, let name = model.names[id] else { return id }
        return currency && name.hasSuffix("@") ? String(name.dropLast()) : name
    }
}

/// Dates for block heights, loaded in small batches, as the terminal's `height_display`.
///
/// Past heights use the block's own time. Heights above the tip are estimated and shown
/// with "est.". A height whose date is not available keeps its number.
@MainActor
@Observable
final class HeightDates {
    static let shared = HeightDates()

    private var dates: [UInt64: HeightDate] = [:]
    // Bookkeeping changes while views draw, so it is not observed.
    @ObservationIgnored private var pending: Set<UInt64> = []
    @ObservationIgnored private var requested: Set<UInt64> = []
    @ObservationIgnored private var generation = -1
    @ObservationIgnored private var scheduled = false

    /// The text for a height under the current setting. `known` is the block's time when the
    /// caller has it. Loads missing dates in the background.
    func text(_ height: UInt64, known: UInt64? = nil, model: AppModel) -> String {
        guard Display.showsDates else { return Format.integer(height) }
        if let known, known > 0 { return Format.date(known) }
        reset(for: model)
        if let date = dates[height] {
            guard let time = date.time, time > 0 else {
                return "\(Format.integer(height)) (date not available)"
            }
            return (date.estimated ? "est. " : "") + Format.date(UInt64(time))
        }
        if !requested.contains(height), pending.count < 256 {
            pending.insert(height)
            schedule(model)
        }
        return "\(Format.integer(height)) (loading date)"
    }

    /// A confirmation count, or the date of the block that holds the transaction when dates
    /// are on and the block time is known. The terminal never subtracts from the tip.
    func confirmations(_ count: Int64, blockTime: UInt64?) -> String {
        if Display.showsDates, let blockTime, blockTime > 0 {
            return "included \(Format.date(blockTime))"
        }
        return count.formatted()
    }

    /// A Unix timestamp as a date when dates are on, otherwise as the number.
    func timestamp(_ unix: UInt64) -> String {
        Display.showsDates ? Format.date(unix) : String(unix)
    }

    private func reset(for model: AppModel) {
        guard generation != model.generation else { return }
        generation = model.generation
        pending = []
        requested = []
        // Clearing the observed dates while a view draws would redraw it at once. The old
        // dates are replaced when the new answers arrive.
        Task { @MainActor in self.dates = [:] }
    }

    private func schedule(_ model: AppModel) {
        guard !scheduled else { return }
        scheduled = true
        Task {
            // Collect the heights that one redraw asks for, then ask for them together.
            try? await Task.sleep(for: .milliseconds(150))
            scheduled = false
            guard let explorer = model.explorer else { return }
            let batch = Array(pending.prefix(128))
            pending.subtract(batch)
            requested.formUnion(batch)
            let generation = model.generation
            let found = await explorer.heightDates(heights: batch)
            guard generation == model.generation else { return }
            for date in found { dates[date.height] = date }
            if !pending.isEmpty { schedule(model) }
        }
    }
}

/// The start-up arguments, read once with Carbide's own rules (`src/cli.rs`):
/// `-chain=`, `-testnet`, `-conf=`, `-datadir=`, `-rpcconnect=`, `-rpcport=`, `-rpcuser=`,
/// `-rpcpassword=`, `-pollinterval=` and `-open=`, or the same flags with two dashes.
enum Launch {
    static let arguments: StartArguments = startArguments(arguments: CommandLine.arguments)

    /// Seconds between dashboard refreshes. `-pollinterval` sets it, as in Carbide (minimum
    /// 1 second). Without it the app keeps its own default of 15 seconds.
    static var refreshInterval: Duration {
        .seconds(Int(arguments.pollIntervalSeconds.map { max($0, 1) } ?? 15))
    }
}
