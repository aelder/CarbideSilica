import AppKit
import SwiftUI

/// The mark of the app for work in progress: a pattern of circuit traces in an octagon that
/// draws itself from the middle column out, with a bright point that goes from place to
/// place, then clears and starts again.
///
/// It is Core Animation: the system runs the animation, and the app does no work for a frame.
struct CircuitLoader: NSViewRepresentable {
    var size: CGFloat = 72

    func makeNSView(context: Context) -> CircuitView { CircuitView(side: size) }
    func updateNSView(_ view: CircuitView, context: Context) {}
    func sizeThatFits(_ proposal: ProposedViewSize, nsView: CircuitView, context: Context) -> CGSize? {
        CGSize(width: size, height: size)
    }

    final class CircuitView: NSView {
        private let side: CGFloat
        override var isFlipped: Bool { true }
        override var intrinsicContentSize: NSSize { NSSize(width: side, height: side) }

        /// True draws the full pattern with no movement. For a picture of the mark.
        nonisolated(unsafe) static var alwaysStill = false

        /// One cycle: the pattern draws itself, stays, and goes out.
        private static let cycle: CFTimeInterval = 3.6
        private static let colors: [CGColor] = [
            CGColor(red: 0.82, green: 0.93, blue: 0.86, alpha: 1),
            CGColor(red: 0.5, green: 0.86, blue: 0.62, alpha: 1),
            CGColor(red: 0.56, green: 0.74, blue: 0.98, alpha: 1),
        ]

        init(side: CGFloat) {
            self.side = side
            super.init(frame: NSRect(x: 0, y: 0, width: side, height: side))
            wantsLayer = true
            build()
        }

        required init?(coder: NSCoder) { nil }

        /// A small generator with a fixed start, so that the pattern is the same each time.
        private struct Steps {
            var state: UInt64
            mutating func next() -> UInt64 {
                state = state &* 6364136223846793005 &+ 1442695040888963407
                return state >> 33
            }
            mutating func below(_ limit: Int) -> Int { Int(next() % UInt64(max(limit, 1))) }
        }

        private func octagon() -> CGPath {
            let cut = side * 0.29
            let path = CGMutablePath()
            path.addLines(between: [
                CGPoint(x: cut, y: 0), CGPoint(x: side - cut, y: 0), CGPoint(x: side, y: cut),
                CGPoint(x: side, y: side - cut), CGPoint(x: side - cut, y: side), CGPoint(x: cut, y: side),
                CGPoint(x: 0, y: side - cut), CGPoint(x: 0, y: cut),
            ])
            path.closeSubpath()
            return path
        }

        private func build() {
            guard let layer else { return }
            let still = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion || Self.alwaysStill
            // The number of grid lines follows the size: a small mark has few traces.
            let cells = side < 24 ? 5 : side < 48 ? 9 : min(Int(side / 4.5), 30)
            let step = side / CGFloat(cells)
            let width = max(side / 110, side < 24 ? 1 : 0.9)
            let bands = side < 24 ? 3 : 7
            var random = Steps(state: 0x5EED_C0DE)

            let traces = CALayer()
            traces.frame = bounds
            let mask = CAShapeLayer()
            mask.path = octagon()
            traces.mask = mask
            layer.addSublayer(traces)

            // The traces, in bands by their distance from the middle column. A band of the
            // middle shows first, as on the screen of the ship.
            let paths = (0..<bands).map { _ in (0..<Self.colors.count).map { _ in CGMutablePath() } }
            let middle = CGFloat(cells) / 2
            func path(at x: CGFloat, _ color: Int) -> CGMutablePath {
                let band = min(Int(abs(x - middle) / middle * CGFloat(bands)), bands - 1)
                return paths[band][color % Self.colors.count]
            }
            func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x * step, y: y * step) }

            // Bundles of parallel lines from the middle to the four corners.
            if side >= 24 {
                for (dx, dy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] as [(CGFloat, CGFloat)] {
                    for lane in -1...1 {
                        let shift = CGFloat(lane) * 0.8
                        let from = 1.2 + CGFloat(random.below(3))
                        let to = middle * (0.75 + 0.25 * CGFloat(random.below(100)) / 100)
                        let line = path(at: middle + dx * from, 0)
                        line.move(to: point(middle + dx * from - dy * shift, middle + dy * from + dx * shift))
                        line.addLine(to: point(middle + dx * to - dy * shift, middle + dy * to + dx * shift))
                    }
                }
            }

            // Traces: a run along the grid, then a turn, as the lines of a circuit board.
            let straight: [(CGFloat, CGFloat)] = [(1, 0), (-1, 0), (0, 1), (0, -1)]
            let count = side < 24 ? 7 : Int(Double(cells) * 2.2)
            for index in 0..<count {
                var x = CGFloat(random.below(cells + 1))
                var y = CGFloat(random.below(cells + 1))
                let line = path(at: x, index)
                line.move(to: point(x, y))
                var direction = straight[random.below(4)]
                for turn in 0..<(2 + random.below(3)) {
                    let length = CGFloat(2 + random.below(max(cells / 3, 2)))
                    x = min(max(x + direction.0 * length, 0), CGFloat(cells))
                    y = min(max(y + direction.1 * length, 0), CGFloat(cells))
                    line.addLine(to: point(x, y))
                    if turn == 0, random.below(4) == 0 {
                        // A short step at 45 degrees between two runs.
                        let slant: CGFloat = random.below(2) == 0 ? 1 : -1
                        x = min(max(x + 1, 0), CGFloat(cells))
                        y = min(max(y + slant, 0), CGFloat(cells))
                        line.addLine(to: point(x, y))
                    }
                    // Turn a quarter.
                    direction = direction.0 == 0 ? (random.below(2) == 0 ? (1, 0) : (-1, 0))
                        : (random.below(2) == 0 ? (0, 1) : (0, -1))
                }
            }

            // Rows of short parallel lines, as the legs of a part.
            if side >= 24 {
                for index in 0..<(cells / 2) {
                    let x = CGFloat(random.below(cells - 3))
                    let y = CGFloat(1 + random.below(cells - 2))
                    let upright = random.below(2) == 0
                    let line = path(at: x, index + 1)
                    for leg in 0..<(4 + random.below(4)) {
                        let offset = CGFloat(leg) * 0.5
                        if upright {
                            line.move(to: point(x + offset, y))
                            line.addLine(to: point(x + offset, y + 1))
                        } else {
                            line.move(to: point(y, x + offset))
                            line.addLine(to: point(y + 1, x + offset))
                        }
                    }
                }
            }

            let cycle = Self.cycle
            let drawn = 2.3
            for band in 0..<bands {
                let start = drawn * Double(band) / Double(bands) * 0.8
                for (index, color) in Self.colors.enumerated() {
                    let shape = CAShapeLayer()
                    shape.frame = bounds
                    shape.path = paths[band][index]
                    shape.fillColor = nil
                    shape.strokeColor = color
                    shape.lineWidth = width
                    shape.shadowColor = color
                    shape.shadowOpacity = 0.7
                    shape.shadowRadius = max(side / 60, 0.8)
                    shape.shadowOffset = .zero
                    traces.addSublayer(shape)
                    guard !still else { continue }

                    // First some short pieces of each trace, then all of it.
                    shape.lineDashPattern = [NSNumber(value: Double(step) * 0.5), NSNumber(value: Double(step) * 1.4)]
                    let early = start + Double(index) * 0.12
                    let full = min(early + 0.9, drawn)
                    let opacity = CAKeyframeAnimation(keyPath: "opacity")
                    opacity.values = [0, 0, 1, 1, 0, 0]
                    opacity.keyTimes = [0, early / cycle, (early + 0.05) / cycle, 3.1 / cycle, 3.45 / cycle, 1].map {
                        NSNumber(value: $0)
                    }
                    opacity.calculationMode = .linear
                    // The spaces of the dashes close: the pieces become the full trace.
                    let dash = CAKeyframeAnimation(keyPath: "lineDashPattern")
                    let pieces: [NSNumber] = [NSNumber(value: Double(step) * 0.5), NSNumber(value: Double(step) * 1.4)]
                    let more: [NSNumber] = [NSNumber(value: Double(step) * 1.2), NSNumber(value: Double(step) * 0.7)]
                    let whole: [NSNumber] = [NSNumber(value: Double(side) * 4), NSNumber(value: 0)]
                    dash.values = [pieces, pieces, more, whole, whole]
                    dash.keyTimes = [0, early / cycle, ((early + full) / 2) / cycle, full / cycle, 1].map {
                        NSNumber(value: $0)
                    }
                    dash.calculationMode = .discrete
                    let group = CAAnimationGroup()
                    group.animations = [opacity, dash]
                    group.duration = cycle
                    group.repeatCount = .infinity
                    group.isRemovedOnCompletion = false
                    shape.add(group, forKey: "draw")
                }
            }

            // The bright point that goes from place to place while the pattern draws.
            guard !still, side >= 24 else { return }
            let point = CALayer()
            let radius = max(side / 34, 1.6)
            point.bounds = CGRect(x: 0, y: 0, width: radius * 2, height: radius * 2)
            point.cornerRadius = radius
            point.backgroundColor = CGColor(red: 0.9, green: 1, blue: 0.94, alpha: 1)
            point.shadowColor = CGColor(red: 0.5, green: 0.95, blue: 0.75, alpha: 1)
            point.shadowOpacity = 1
            point.shadowRadius = radius * 2
            point.shadowOffset = .zero
            layer.addSublayer(point)
            let places = (0..<9).map { _ in
                NSValue(point: NSPoint(
                    x: side * (0.12 + 0.76 * CGFloat(random.below(100)) / 100),
                    y: side * (0.12 + 0.76 * CGFloat(random.below(100)) / 100)))
            }
            let move = CAKeyframeAnimation(keyPath: "position")
            move.values = places
            move.calculationMode = .discrete
            let seen = CAKeyframeAnimation(keyPath: "opacity")
            seen.values = [1, 1, 0, 0]
            seen.keyTimes = [0, NSNumber(value: drawn / cycle), NSNumber(value: (drawn + 0.1) / cycle), 1]
            let group = CAAnimationGroup()
            group.animations = [move, seen]
            group.duration = cycle
            group.repeatCount = .infinity
            group.isRemovedOnCompletion = false
            point.add(group, forKey: "point")
        }
    }
}

/// The look of work in progress on a page: the circuit mark, with the words below it. A
/// small control gets a small mark. Progress with a known part is a thin bar.
struct CircuitProgressStyle: ProgressViewStyle {
    func makeBody(configuration: Configuration) -> some View {
        Mark(configuration: configuration)
    }

    private struct Mark: View {
        let configuration: Configuration
        @Environment(\.controlSize) private var controlSize

        var body: some View {
            if let part = configuration.fractionCompleted {
                GeometryReader { geometry in
                    ZStack(alignment: .leading) {
                        Rectangle().fill(Tone.axis.opacity(0.5))
                        Rectangle().fill(Tone.record).frame(width: geometry.size.width * CGFloat(min(max(part, 0), 1)))
                    }
                }
                .frame(height: 3)
            } else if controlSize == .small || controlSize == .mini {
                CircuitLoader(size: 16)
            } else {
                VStack(spacing: 12) {
                    CircuitLoader(size: 84)
                    configuration.label
                        .font(Tone.small)
                        .tracking(1.1)
                        .textCase(.uppercase)
                        .foregroundStyle(Tone.label)
                }
            }
        }
    }
}
