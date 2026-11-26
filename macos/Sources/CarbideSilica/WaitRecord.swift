import CarbideCore
import Foundation
import Observation

/// The chain's record of recent waits for a block. The chain pulse and the Mempool page
/// set the present wait against the same record, thus the app reads it once.
@MainActor
@Observable
final class WaitRecord {
    static let shared = WaitRecord()
    /// The length of one counting window in the record.
    static let span: Double = 25 * 60

    private(set) var history: PulseHistory?
    private(set) var problem: String?
    private var readAt: Date?
    /// When a read started. A read that a cancelled task started can stay open, thus a
    /// read that is older than one minute does not stop a new one.
    private var readingSince: Date?
    private var generation = -1

    /// Reads the record at the start and each 10 minutes. The waits of a day change slowly.
    func keep(_ model: AppModel, live: Bool) async {
        while !Task.isCancelled {
            guard let explorer = model.explorer, let tip = model.status?.blocks, live else {
                try? await Task.sleep(for: .seconds(2))
                continue
            }
            if generation != model.generation {
                generation = model.generation
                history = nil
                problem = nil
                readAt = nil
                readingSince = nil
            }
            let free = readingSince.map { Date.now.timeIntervalSince($0) > 60 } ?? true
            if free, readAt.map({ Date.now.timeIntervalSince($0) >= 600 }) ?? true {
                readingSince = .now
                // A public API answers slowly and sees each request. Ask it for less.
                let count: UInt32 = model.status?.source == .publicApi ? 180 : 1440
                let asked = generation
                do {
                    let found = try await explorer.pulseHistory(
                        tip: tip, count: count, windowSeconds: UInt32(Self.span))
                    if asked == model.generation {
                        history = found
                        problem = nil
                    }
                } catch {
                    if asked == model.generation { problem = describe(error) }
                }
                readAt = .now
                readingSince = nil
            }
            try? await Task.sleep(for: .seconds(10))
        }
    }

    /// The share of recent waits that are shorter than `seconds`, from 0 to 1.
    func shareShorter(than seconds: Double) -> Double? {
        guard let waits = history?.waits, !waits.isEmpty else { return nil }
        var low = 0
        var high = waits.count
        while low < high {
            let middle = (low + high) / 2
            if Double(waits[middle]) < seconds { low = middle + 1 } else { high = middle }
        }
        return Double(low) / Double(waits.count)
    }
}
