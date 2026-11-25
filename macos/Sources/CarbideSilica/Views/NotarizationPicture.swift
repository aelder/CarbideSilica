import CarbideCore
import SwiftUI

/// The state of a link between two systems, from the record of its notarizations.
enum LinkState {
    /// The wait is in the range of the record.
    case current
    /// The wait is longer than each interval of the record.
    case late
    /// The wait is more than 100 times the longest interval of the record.
    case stopped

    /// A link with fewer intervals than this has no state.
    static let minimumIntervals = 5
    static let stoppedFactor: UInt64 = 100

    init?(wait: UInt64?, intervals: [UInt64]) {
        guard let wait, intervals.count >= Self.minimumIntervals, let longest = intervals.max() else { return nil }
        if wait > longest * Self.stoppedFactor {
            self = .stopped
        } else if wait > longest {
            self = .late
        } else {
            self = .current
        }
    }

    var word: String {
        switch self {
        case .current: "Current"
        case .late: "Late"
        case .stopped: "Stopped"
        }
    }

    var rule: String {
        switch self {
        case .current: "Current: the wait is not longer than the longest interval of the record."
        case .late: "Late: the wait is longer than each interval of the record."
        case .stopped: "Stopped: the wait is more than \(Self.stoppedFactor) times the longest interval of the record."
        }
    }

    var color: Color {
        switch self {
        case .current: .primary
        case .late: .orange
        case .stopped: .red
        }
    }
}

enum NotarizationMath {
    /// The blocks between the entries of a record in sequence. The record is newest first.
    static func intervals(_ record: [RecordPoint]) -> [UInt64] {
        zip(record, record.dropFirst()).compactMap { newer, older in
            newer.height > older.height ? newer.height - older.height : nil
        }
    }

    /// A number of blocks as a length of time, at one block for each minute. Days stay
    /// days for two years, because the wait of a link is compared in days.
    static func span(blocks: UInt64) -> String {
        switch blocks {
        case 1: "1 minute"
        case ..<120: "\(blocks) minutes"
        case ..<2880: "\(Int((Double(blocks) / 60).rounded())) hours"
        case ..<1_051_200: "\(Int((Double(blocks) / 1440).rounded())) days"
        default: "\((Double(blocks) / 525_600).formatted(.number.precision(.fractionLength(0...1)))) years"
        }
    }

    static func median(_ values: [UInt64]) -> UInt64? {
        guard !values.isEmpty else { return nil }
        let sorted = values.sorted()
        return sorted[sorted.count / 2]
    }
}

/// The intervals between earlier notarizations as dots on one scale, and the wait now as
/// one mark. Each step to the right on the scale is ten times more.
struct NotarizationRhythm: View {
    let intervals: [UInt64]
    let wait: UInt64?
    let state: LinkState?

    var body: some View {
        Canvas { context, size in
            draw(context, size)
        }
        .frame(height: 112)
        .accessibilityLabel(spoken)
    }

    private var spoken: String {
        var text = "\(intervals.count) intervals between earlier notarizations."
        if let usual = NotarizationMath.median(intervals), let longest = intervals.max() {
            text += " The usual interval is \(Format.integer(usual)) blocks. The longest is \(Format.integer(longest))."
        }
        if let wait {
            text += " The wait now is \(Format.integer(wait)) blocks."
        }
        return text
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let left: CGFloat = 16
        let right = size.width - 56
        let base = size.height - 22
        let top = Double(max(intervals.max() ?? 1, wait ?? 1, 100))
        let decades = max(Int(ceil(log10(top))), 2)
        let x = { (blocks: UInt64) -> CGFloat in
            left + CGFloat(log10(Double(max(blocks, 1))) / Double(decades)) * (right - left)
        }
        context.fill(
            Path(CGRect(x: left, y: base, width: right - left, height: 1)), with: .color(.secondary.opacity(0.6)))
        for power in 0...decades {
            let value = UInt64(pow(10, Double(power)))
            let at = x(value)
            context.fill(Path(CGRect(x: at, y: base - 3, width: 1, height: 7)), with: .color(.secondary.opacity(0.6)))
            context.draw(
                Text("\(Format.integer(value)) \(value == 1 ? "block" : "blocks")")
                    .font(.caption).foregroundStyle(.secondary),
                at: CGPoint(x: at, y: base + 13), anchor: .center)
        }
        // The dots, in columns.
        let column: CGFloat = 6
        let room = Int((base - 26) / 5.6)
        var heights: [Int: Int] = [:]
        var dots = Path()
        for interval in intervals {
            let slot = Int((x(interval) / column).rounded())
            let level = heights[slot, default: 0]
            heights[slot] = level + 1
            guard level < room else { continue }
            let centre = CGPoint(x: CGFloat(slot) * column, y: base - 5 - CGFloat(level) * 5.6)
            dots.addEllipse(in: CGRect(x: centre.x - 2.3, y: centre.y - 2.3, width: 4.6, height: 4.6))
        }
        context.fill(dots, with: .color(.accentColor))
        if let usual = NotarizationMath.median(intervals) {
            context.draw(
                Text("usual: \(Format.integer(usual)) blocks").font(.callout).foregroundStyle(.primary),
                at: CGPoint(x: max(x(usual), 60), y: 8), anchor: .center)
        }
        if let wait {
            let at = x(wait)
            let color = state?.color ?? .primary
            context.fill(Path(CGRect(x: at - 1.25, y: 18, width: 2.5, height: base - 18)), with: .color(color))
            let onLeft = at > size.width / 2
            context.draw(
                Text("now: \(Format.integer(wait)) blocks").font(.callout.weight(.semibold)).foregroundStyle(color),
                at: CGPoint(x: at + (onLeft ? -8 : 8), y: 26), anchor: onLeft ? .trailing : .leading)
        }
    }
}

/// One mark of the graph: one notarization, or entries in sequence that commit to the
/// same block of the other system.
struct NotarizationMark: Identifiable {
    var rows: [BrowserNotarization]
    /// 0 is the chain that the node gives as the best one.
    var lane: Int
    var column: Int
    var parent: UInt32?
    var id: UInt32 { rows.first?.index ?? 0 }
    var confirmed: Bool { rows.contains(where: \.confirmed) }
    var newest: BrowserNotarization? { rows.last }
}

enum NotarizationPicture {
    static func observerHeight(_ row: BrowserNotarization) -> UInt64? {
        row.roots.first { !$0.foreign }?.height ?? row.includedHeight
    }

    static func otherHeight(_ row: BrowserNotarization) -> UInt64? {
        row.roots.first { $0.foreign }?.height
    }

    /// The marks of a view. The best chain is lane 0. Each other fork is a new lane from
    /// the mark where it leaves the entries that are placed.
    static func marks(_ view: NotarizationBrowser) -> [NotarizationMark] {
        let byIndex = Dictionary(view.rows.map { ($0.index, $0) }, uniquingKeysWith: { first, _ in first })
        var forks = view.forks
        if let best = view.bestFork, forks.indices.contains(Int(best)) {
            forks.insert(forks.remove(at: Int(best)), at: 0)
        }
        if forks.isEmpty { forks = [view.rows.map(\.index)] }
        var marks: [NotarizationMark] = []
        var place: [UInt32: Int] = [:]
        var lane = 0
        for fork in forks {
            var previous: Int?
            var added = false
            for index in fork {
                if let known = place[index] {
                    previous = known
                    continue
                }
                guard let row = byIndex[index] else { continue }
                if let at = previous, marks[at].lane == lane, !marks[at].confirmed,
                    let height = otherHeight(row), marks[at].newest.flatMap(otherHeight) == height
                {
                    marks[at].rows.append(row)
                    place[index] = at
                    continue
                }
                marks.append(
                    NotarizationMark(
                        rows: [row], lane: lane, column: previous.map { marks[$0].column + 1 } ?? 0,
                        parent: previous.map { marks[$0].id }))
                place[index] = marks.count - 1
                previous = marks.count - 1
                added = true
            }
            if added || lane == 0 { lane += 1 }
        }
        // Rows that no fork has.
        for row in view.rows where place[row.index] == nil {
            marks.append(NotarizationMark(rows: [row], lane: lane, column: 0, parent: nil))
        }
        return marks
    }

    /// The rows in the order of the graph: each lane from the newest entry back.
    static func ordered(_ marks: [NotarizationMark]) -> [(lane: Int, rows: [BrowserNotarization])] {
        Dictionary(grouping: marks, by: \.lane).sorted { $0.key < $1.key }.map { lane, marks in
            (lane, marks.sorted { $0.column > $1.column }.flatMap { $0.rows.reversed() })
        }
    }
}

/// The entries of a view as a graph. The best chain is the straight line. A side branch
/// is a second line that leaves the first at one mark.
struct NotarizationGraph: View {
    let marks: [NotarizationMark]
    let observer: String
    let other: String
    /// The text below a mark: its age, or what it repeats.
    let note: (NotarizationMark) -> String
    @Binding var selected: UInt32?

    private let left: CGFloat = 118
    private let laneHeight: CGFloat = 84
    private let top: CGFloat = 72

    var body: some View {
        let lanes = (marks.map(\.lane).max() ?? 0) + 1
        let columns = (marks.map(\.column).max() ?? 0) + 1
        let height = y(lanes - 1) + (lanes > 1 ? 68 : 22)
        GeometryReader { geometry in
            let step = min(max((geometry.size.width - left - 90) / CGFloat(max(columns - 1, 1)), 150), 260)
            let width = max(left + CGFloat(columns - 1) * step + 90, geometry.size.width)
            ScrollView(.horizontal) {
                ZStack(alignment: .topLeading) {
                    Canvas { context, _ in
                        lines(context, step: step)
                    }
                    ForEach(0..<lanes, id: \.self) { lane in
                        Text(lane == 0 ? "Best chain" : lanes > 2 ? "Side branch \(lane)" : "Side branch")
                            .font(.callout.weight(.semibold))
                            .position(x: 44, y: y(lane))
                    }
                    ForEach(marks) { mark in
                        markView(mark).position(point(mark, step: step))
                        labels(mark)
                            .frame(width: step - 8)
                            .position(x: point(mark, step: step).x, y: y(mark.lane) + (mark.lane == 0 ? -42 : 40))
                    }
                }
                .frame(width: width, height: height, alignment: .topLeading)
            }
        }
        .frame(height: height)
    }

    /// The labels of the best chain are above its marks. The labels of a side branch are below.
    private func y(_ lane: Int) -> CGFloat { top + CGFloat(lane) * laneHeight }

    private func point(_ mark: NotarizationMark, step: CGFloat) -> CGPoint {
        // A mark of a side branch is a part of a step back, so that its line is a curve.
        CGPoint(x: left + (CGFloat(mark.column) - (mark.lane > 0 ? 0.3 : 0)) * step, y: y(mark.lane))
    }

    private func lines(_ context: GraphicsContext, step: CGFloat) {
        for mark in marks {
            guard let parent = marks.first(where: { $0.id == mark.parent }) else { continue }
            let from = point(parent, step: step)
            let to = point(mark, step: step)
            var path = Path()
            path.move(to: from)
            if from.y == to.y {
                path.addLine(to: to)
            } else {
                path.addCurve(
                    to: to, control1: CGPoint(x: from.x + 30, y: from.y), control2: CGPoint(x: to.x - 40, y: to.y))
            }
            context.stroke(
                path, with: .color(mark.lane == 0 ? .secondary : .orange),
                style: StrokeStyle(lineWidth: 1.8, dash: [5, 4]))
        }
    }

    private func markView(_ mark: NotarizationMark) -> some View {
        let color: Color = mark.confirmed ? .accentColor : mark.lane == 0 ? .primary : .orange
        return Button {
            selected = selected == mark.id ? nil : mark.id
        } label: {
            ZStack {
                Circle().fill(mark.confirmed ? Color.accentColor : Color(nsColor: .windowBackgroundColor))
                Circle().stroke(color, lineWidth: selected == mark.id ? 3.4 : 1.8)
                if mark.rows.count > 1 {
                    Text("\(mark.rows.count)").font(.caption.weight(.bold)).foregroundStyle(color)
                }
            }
            .frame(width: 20, height: 20)
            .contentShape(Circle().inset(by: -6))
        }
        .buttonStyle(.plain)
        .help("Shows the rows of this mark")
        .accessibilityLabel(spoken(mark))
    }

    private func labels(_ mark: NotarizationMark) -> some View {
        VStack(spacing: 1) {
            if let row = mark.newest {
                Text("\(observer) \(NotarizationPicture.observerHeight(row).map(Format.integer) ?? "–")")
                    .monospacedDigit()
                Text("\(other) \(NotarizationPicture.otherHeight(row).map(Format.integer) ?? "–")")
                    .monospacedDigit().foregroundStyle(.secondary)
            }
            Text(note(mark)).foregroundStyle(.secondary)
        }
        .font(.callout)
        .lineLimit(1)
        .minimumScaleFactor(0.8)
        .multilineTextAlignment(.center)
    }

    private func spoken(_ mark: NotarizationMark) -> String {
        guard let row = mark.newest else { return "" }
        let heights =
            "\(observer) block \(NotarizationPicture.observerHeight(row).map(Format.integer) ?? "not known"), \(other) block \(NotarizationPicture.otherHeight(row).map(Format.integer) ?? "not known")"
        let kind = mark.confirmed ? "Confirmed" : mark.lane == 0 ? "On the best chain" : "On a side branch"
        return "\(kind). \(heights). \(note(mark))"
    }
}
