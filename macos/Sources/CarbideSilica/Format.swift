import Foundation

/// Display formatting shared by the views.
enum Format {
    /// Satoshis as a coin amount with up to eight decimals, e.g. `3.00046564`.
    static func coins(_ sats: Int64) -> String {
        let value = Decimal(sats) / Decimal(100_000_000)
        return value.formatted(.number.precision(.fractionLength(0...8)))
    }

    static func amount(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(0...8)))
    }

    static func integer(_ value: UInt64) -> String {
        value.formatted(.number)
    }

    static func date(_ unix: UInt64) -> String {
        Date(timeIntervalSince1970: TimeInterval(unix))
            .formatted(date: .abbreviated, time: .standard)
    }

    static func age(_ unix: UInt64, now: Date = .now) -> String {
        let seconds = Int(now.timeIntervalSince1970) - Int(unix)
        switch seconds {
        case ..<0: return "just now"
        case ..<60: return "\(seconds)s ago"
        case ..<3600: return "\(seconds / 60)m ago"
        case ..<86_400: return "\(seconds / 3600)h \(seconds % 3600 / 60)m ago"
        default: return "\(seconds / 86_400)d ago"
        }
    }

    static func bytes(_ count: UInt64) -> String {
        ByteCountFormatter.string(fromByteCount: Int64(count), countStyle: .binary)
    }

    /// A long hash or address shortened in the middle.
    static func short(_ text: String, head: Int = 10, tail: Int = 8) -> String {
        guard text.count > head + tail + 1 else { return text }
        return "\(text.prefix(head))…\(text.suffix(tail))"
    }

    static func difficulty(_ value: Double) -> String {
        let units = ["", "K", "M", "G", "T", "P", "E"]
        var value = value
        var unit = 0
        while value >= 1000, unit < units.count - 1 {
            value /= 1000
            unit += 1
        }
        return value.formatted(.number.precision(.fractionLength(0...2))) + " " + units[unit]
    }

    /// A reserve weight such as `0.25` as `25%`.
    static func percent(_ value: Double) -> String {
        value.formatted(.percent.precision(.fractionLength(0...2)))
    }
}
