import SwiftUI

/// The design of the app: the display of a ship's computer.
///
/// The app is a black display. Colour is a signal and not decoration: violet is staked,
/// amber is mined, warm white is the present and the selection, pale blue is a link and a
/// part of a record that is done, red is an alert. Lines are thin and flat. Light is for
/// the things that are live. Names and figures are in Futura, the typeface of the displays
/// in "2001: A Space Odyssey". Long text and IDs stay in the system typefaces, because a
/// person must read them and compare them.
enum Tone {
    // MARK: Colours

    // The colours are those of the display on film stock. The measures are from a film
    // simulation of the app: the white is a pale grey-green at four fifths of full light,
    // a signal colour is a pale core with its colour in the light around it, and the black
    // is a little above zero.
    static let screen = Color(red: 0.012, green: 0.012, blue: 0.016)
    /// The border of a panel and the rule above a card.
    static let rule = Color(red: 0.75, green: 0.8, blue: 0.75).opacity(0.15)
    static let grid = Color(red: 0.75, green: 0.8, blue: 0.75).opacity(0.09)
    static let axis = Color(red: 0.75, green: 0.8, blue: 0.75).opacity(0.36)
    /// Names of things: axis labels, readout names.
    static let label = Color(red: 0.62, green: 0.67, blue: 0.64)
    /// Figures and text that a person reads.
    static let value = Color(red: 0.75, green: 0.8, blue: 0.75)
    /// The present and the selection: the lightest tone of the display.
    static let present = Color(red: 0.8, green: 0.84, blue: 0.77)
    static let staked = Color(red: 0.72, green: 0.68, blue: 0.8)
    static let mined = Color(red: 0.74, green: 0.67, blue: 0.44)
    /// A link, and the part of a record that is done or passed.
    static let record = Color(red: 0.5, green: 0.74, blue: 0.71)
    static let good = Color(red: 0.26, green: 0.65, blue: 0.48)
    static let alert = Color(red: 0.82, green: 0.33, blue: 0.24)
    /// The colour of the light around a staked mark and around a mined mark. On film the
    /// core of a strong colour goes pale, and the colour is in the light around it.
    static let stakedGlow = Color(red: 0.56, green: 0.3, blue: 0.86)
    static let minedGlow = Color(red: 1, green: 0.5, blue: 0.12)
    /// Series of a chart, in order, when a chart has more than the named signals.
    static let series: [Color] = [record, mined, staked, good, Color(red: 0.78, green: 0.5, blue: 0.56), value]

    // MARK: Type

    static func futura(_ size: CGFloat, bold: Bool = false) -> Font {
        .custom(bold ? "Futura-Bold" : "Futura-Medium", size: size)
    }

    /// Axis labels and readout names. Use with capitals and `tracking(1.1)`.
    static let small = futura(9.5)
    /// The lettering of a plate.
    static let plate = futura(8.5, bold: true)
    /// A figure of a readout.
    static let figure = futura(17)
    static let figureSmall = futura(11.5)
    /// The heading of a card.
    static let heading = futura(13)
    /// The title of a page.
    static let title = futura(26)
    /// Text in a panel that a person reads as a sentence.
    static let caption = Font.system(size: 11.5)
}

/// Four small corners that frame a panel.
struct CornerMarks: Shape {
    func path(in rect: CGRect) -> Path {
        let arm: CGFloat = 9
        var path = Path()
        for (corner, dx, dy) in [
            (CGPoint(x: rect.minX, y: rect.minY), 1.0, 1.0), (CGPoint(x: rect.maxX, y: rect.minY), -1.0, 1.0),
            (CGPoint(x: rect.minX, y: rect.maxY), 1.0, -1.0), (CGPoint(x: rect.maxX, y: rect.maxY), -1.0, -1.0),
        ] {
            path.move(to: CGPoint(x: corner.x + arm * dx, y: corner.y))
            path.addLine(to: corner)
            path.addLine(to: CGPoint(x: corner.x, y: corner.y + arm * dy))
        }
        return path
    }
}

/// A display panel: the frame of each chart and diagram. It has a thin border and four
/// corner marks. `padding` is the room between the frame and the content; a chart that
/// draws to the frame gives 0 and sets its own.
struct DisplayPanel<Content: View>: View {
    var padding: CGFloat = 16
    @ViewBuilder let content: Content

    var body: some View {
        content
            .padding(padding)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Tone.screen, in: RoundedRectangle(cornerRadius: 4))
            .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Tone.rule, lineWidth: 1))
            .overlay(CornerMarks().stroke(Tone.value.opacity(0.55), lineWidth: 1).padding(5).allowsHitTesting(false))
            .environment(\.colorScheme, .dark)
    }
}

/// A thin rule across a panel, between its parts.
struct PanelRule: View {
    var body: some View {
        Rectangle().fill(Tone.rule).frame(height: 1)
    }
}

/// A name and a figure in one line: "TARGET 25".
struct Readout: View {
    let label: String
    let value: String
    var tone: Color = Tone.value

    init(_ label: String, _ value: String, tone: Color = Tone.value) {
        self.label = label
        self.value = value
        self.tone = tone
    }

    var body: some View {
        HStack(alignment: .lastTextBaseline, spacing: 7) {
            Text(label.uppercased()).font(Tone.small).tracking(1.1).foregroundStyle(Tone.label)
            Text(value).font(Tone.figure).monospacedDigit().foregroundStyle(tone)
        }
        .lineLimit(1)
        .fixedSize()
    }
}

/// A name in capitals on a field of its colour, or in an outline when it has no colour.
struct Plate: View {
    let text: String
    var fill: Color?

    init(_ text: String, fill: Color? = nil) {
        self.text = text
        self.fill = fill
    }

    var body: some View {
        Text(text.uppercased())
            .font(Tone.plate)
            .tracking(0.8)
            .foregroundStyle(fill == nil ? Tone.label : Tone.screen)
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(fill ?? .clear)
            .overlay(Rectangle().strokeBorder(fill == nil ? Tone.label.opacity(0.7) : .clear, lineWidth: 1))
            .lineLimit(1)
            .fixedSize()
    }
}

/// A small name in spaced capitals: the name of a field, an axis or a column.
struct NameText: View {
    let text: String

    init(_ text: String) {
        self.text = text
    }

    var body: some View {
        // A name is one line. In a narrow column it gets smaller, and it does not break.
        Text(text.uppercased()).font(Tone.small).tracking(1.1).foregroundStyle(Tone.label)
            .lineLimit(1)
            .minimumScaleFactor(0.7)
    }
}

// MARK: - Drawing in a Canvas

extension GraphicsContext {
    /// A straight line.
    func rule(
        _ x0: CGFloat, _ y0: CGFloat, _ x1: CGFloat, _ y1: CGFloat, _ color: Color = Tone.axis,
        width: CGFloat = 1, dash: [CGFloat] = []
    ) {
        stroke(
            Path { path in
                path.move(to: CGPoint(x: x0, y: y0))
                path.addLine(to: CGPoint(x: x1, y: y1))
            }, with: .color(color), style: StrokeStyle(lineWidth: width, lineCap: .butt, dash: dash))
    }

    /// A small name in spaced capitals.
    func name(
        _ text: String, _ x: CGFloat, _ y: CGFloat, _ anchor: UnitPoint = .center, color: Color = Tone.label
    ) {
        draw(
            Text(text.uppercased()).font(Tone.small).tracking(1.1).foregroundStyle(color),
            at: CGPoint(x: x, y: y), anchor: anchor)
    }

    /// A figure, such as a time or an amount on a chart.
    func figure(
        _ text: String, _ x: CGFloat, _ y: CGFloat, _ anchor: UnitPoint = .center, color: Color = Tone.value
    ) {
        draw(
            Text(text).font(Tone.figureSmall).monospacedDigit().foregroundStyle(color),
            at: CGPoint(x: x, y: y), anchor: anchor)
    }

    /// A name on a field of its colour, or in an outline. The plate is 46 wide by default
    /// and 16 high, and its left middle is at (x, y).
    func plate(_ text: String, x: CGFloat = 18, y: CGFloat, width: CGFloat = 46, fill: Color?) {
        let box = CGRect(x: x, y: y - 8, width: width, height: 16)
        if let fill {
            self.fill(Path(box), with: .color(fill))
        } else {
            stroke(Path(box.insetBy(dx: 0.5, dy: 0.5)), with: .color(Tone.label), lineWidth: 1)
        }
        draw(
            Text(text.uppercased()).font(Tone.plate).tracking(0.8)
                .foregroundStyle(fill == nil ? Tone.label : Tone.screen),
            at: CGPoint(x: box.midX, y: box.midY), anchor: .center)
    }

    /// A shape with a soft light around it: for the things that are live or selected.
    func lit(_ shape: Path, _ color: Color, strength: Double = 0.55) {
        if strength > 0 {
            // Halation: a wide red-orange edge, as film gives to a strong light.
            drawLayer { layer in
                layer.addFilter(.blur(radius: 7))
                layer.fill(shape, with: .color(LampTone.halation.opacity(strength * 0.55)))
            }
            drawLayer { layer in
                layer.addFilter(.blur(radius: 3))
                layer.fill(shape, with: .color(color.opacity(strength * 0.8)))
            }
        }
        fill(shape, with: .color(color))
    }
}

// MARK: - Links

/// A link in the display: pale blue text with no underline. It replaces the system link
/// style, which has the system's blue.
struct DisplayLinkStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .foregroundStyle(Tone.record)
            .opacity(configuration.isPressed ? 0.6 : 1)
            .contentShape(Rectangle())
    }
}

extension ButtonStyle where Self == DisplayLinkStyle {
    static var displayLink: DisplayLinkStyle { DisplayLinkStyle() }
}

// MARK: - Illuminated keys

/// A key with a lamp behind it, as on a console of the 1960s. It is dark when it is off.
/// The pointer on it gives it a cool blue light. The key of the open place has a warm
/// amber-white light. The light comes up quickly and goes down more slowly, as the
/// filament of a lamp does.
struct LampKey<Label: View>: View {
    /// True for the key of the open place.
    let chosen: Bool
    var height: CGFloat = 30
    let action: () -> Void
    /// The legend of the key. The argument is true while the key has light: the legend
    /// is then dark on the light.
    @ViewBuilder let label: (Bool) -> Label

    @State private var hovering = false

    private enum Lamp { case off, blue, amber }
    private var lamp: Lamp { chosen ? .amber : hovering ? .blue : .off }

    var body: some View {
        Button(action: action) {
            label(lamp != .off)
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 10)
                .frame(height: height)
                .background {
                    ZStack {
                        Rectangle().fill(Color(white: 0.045))
                        light(center: LampTone.blueCentre, edge: LampTone.blueEdge).opacity(lamp == .blue ? 1 : 0)
                        light(center: LampTone.amberCentre, edge: LampTone.amberEdge).opacity(lamp == .amber ? 1 : 0)
                    }
                }
                .overlay(Rectangle().strokeBorder(Tone.axis.opacity(lamp == .off ? 1 : 0), lineWidth: 1))
                // The light that goes out from the key: a small soft light of its own colour.
                .shadow(color: LampTone.blueEdge.opacity(lamp == .blue ? 0.3 : 0), radius: 4)
                // Halation: on film, a strong light has a red-orange edge. It is strong at
                // the edge of the key and goes down quickly.
                .shadow(color: LampTone.halation.opacity(lamp == .amber ? 0.75 : lamp == .blue ? 0.3 : 0), radius: 5)
                .shadow(color: LampTone.halation.opacity(lamp == .off ? 0 : 0.28), radius: 12)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        // Quick up, slower down.
        .animation(lamp == .off ? .easeOut(duration: 0.34) : .easeOut(duration: 0.1), value: lamp)
    }

    /// The face of a key with a lamp behind it: most light in the middle, less at the edges,
    /// and a thin dark line where the face meets its frame.
    private func light(center: Color, edge: Color) -> some View {
        Rectangle()
            .fill(
                EllipticalGradient(
                    colors: [center, edge], center: .center, startRadiusFraction: 0.05, endRadiusFraction: 0.75)
            )
            .overlay(Rectangle().strokeBorder(Color.black.opacity(0.22), lineWidth: 1))
    }
}

/// The colours of the lamps and of a legend on a lit key.
enum LampTone {
    static let blueCentre = Color(red: 0.62, green: 0.73, blue: 0.73)
    static let blueEdge = Color(red: 0.42, green: 0.57, blue: 0.6)
    static let amberCentre = Color(red: 0.71, green: 0.74, blue: 0.64)
    static let amberEdge = Color(red: 0.62, green: 0.53, blue: 0.44)
    /// A legend on a lit key: a dark red-brown, as a legend on film is.
    static let legend = Color(red: 0.33, green: 0.24, blue: 0.2)
    /// The colour of halation: the red-orange edge that film gives to a strong light.
    static let halation = Color(red: 0.95, green: 0.33, blue: 0.14)
}

// MARK: - Film grain

/// Film grain over the display: a fixed pattern of fine noise that makes the black a little
/// less flat. It does not move, so it costs nothing after the first frame.
struct FilmGrain: View {
    /// How strong the grain is. It must stay near the limit of what a person sees.
    var strength: Double = 0.05

    var body: some View {
        Image(decorative: Self.pattern, scale: 2)
            .resizable(resizingMode: .tile)
            .opacity(strength)
            .blendMode(.plusLighter)
            .allowsHitTesting(false)
            .accessibilityHidden(true)
    }

    /// One tile of noise, made one time. The values are near the middle with a few bright
    /// points, as grain is.
    private static let pattern: CGImage = {
        let side = 384
        var generator = SystemRandomNumberGenerator()
        var bytes = [UInt8](repeating: 0, count: side * side)
        for index in bytes.indices {
            // The sum of three random values is near a normal distribution.
            let sum = Int(UInt8.random(in: 0...255, using: &generator))
                + Int(UInt8.random(in: 0...255, using: &generator))
                + Int(UInt8.random(in: 0...255, using: &generator))
            bytes[index] = UInt8(sum / 3)
        }
        let provider = CGDataProvider(data: Data(bytes) as CFData)!
        return CGImage(
            width: side, height: side, bitsPerComponent: 8, bitsPerPixel: 8, bytesPerRow: side,
            space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGBitmapInfo(rawValue: CGImageAlphaInfo.none.rawValue),
            provider: provider, decode: nil, shouldInterpolate: false, intent: .defaultIntent)!
    }()
}

// MARK: - Illuminated buttons and segments

/// The face of a small key with a lamp: dark with an outline when it is off, a cool blue
/// light under the pointer, a cream light with halation when it is pressed or selected.
struct LampFace: View {
    enum Lamp { case off, blue, amber }
    let lamp: Lamp

    var body: some View {
        ZStack {
            Rectangle().fill(Color(white: 0.045))
            face(LampTone.blueCentre, LampTone.blueEdge).opacity(lamp == .blue ? 1 : 0)
            face(LampTone.amberCentre, LampTone.amberEdge).opacity(lamp == .amber ? 1 : 0)
        }
        .overlay(Rectangle().strokeBorder(Tone.axis.opacity(lamp == .off ? 1 : 0), lineWidth: 1))
        .shadow(color: LampTone.blueEdge.opacity(lamp == .blue ? 0.3 : 0), radius: 3)
        .shadow(color: LampTone.halation.opacity(lamp == .amber ? 0.7 : lamp == .blue ? 0.25 : 0), radius: 4)
        .shadow(color: LampTone.halation.opacity(lamp == .off ? 0 : 0.22), radius: 9)
        // Quick up, slower down, as the filament of a lamp.
        .animation(lamp == .off ? .easeOut(duration: 0.34) : .easeOut(duration: 0.1), value: lamp)
    }

    private func face(_ center: Color, _ edge: Color) -> some View {
        Rectangle()
            .fill(EllipticalGradient(colors: [center, edge], center: .center, startRadiusFraction: 0.05, endRadiusFraction: 0.75))
            .overlay(Rectangle().strokeBorder(Color.black.opacity(0.22), lineWidth: 1))
    }
}

/// The style of a button of the display: a small square key with a lamp.
struct LampButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        Key(configuration: configuration)
    }

    private struct Key: View {
        let configuration: Configuration
        @Environment(\.isEnabled) private var enabled
        @State private var hovering = false

        private var lamp: LampFace.Lamp {
            guard enabled else { return .off }
            return configuration.isPressed ? .amber : hovering ? .blue : .off
        }

        var body: some View {
            configuration.label
                .font(Tone.futura(10.5))
                .tracking(1.2)
                .textCase(.uppercase)
                .labelStyle(.titleAndIcon)
                .lineLimit(1)
                .foregroundStyle(lamp == .off ? Tone.value.opacity(enabled ? 0.9 : 0.35) : LampTone.legend)
                .padding(.horizontal, 10)
                .padding(.vertical, 5)
                .background(LampFace(lamp: lamp).opacity(enabled ? 1 : 0.5))
                .contentShape(Rectangle())
                .onHover { hovering = $0 }
        }
    }
}

extension ButtonStyle where Self == LampButtonStyle {
    static var lamp: LampButtonStyle { LampButtonStyle() }
}

/// A row of keys of which one is on: the display's form of a segmented control.
struct LampSegments<Value: Hashable>: View {
    let options: [(value: Value, title: String)]
    @Binding var selection: Value

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(options.enumerated()), id: \.offset) { _, option in
                Segment(title: option.title, chosen: option.value == selection) { selection = option.value }
            }
        }
        .accessibilityElement(children: .contain)
    }

    private struct Segment: View {
        let title: String
        let chosen: Bool
        let action: () -> Void
        @State private var hovering = false

        private var lamp: LampFace.Lamp { chosen ? .amber : hovering ? .blue : .off }

        var body: some View {
            Button(action: action) {
                Text(title.uppercased())
                    .font(Tone.futura(10.5))
                    .tracking(1.2)
                    .lineLimit(1)
                    .foregroundStyle(lamp == .off ? Tone.value.opacity(0.9) : LampTone.legend)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 5)
                    .background(LampFace(lamp: lamp))
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .onHover { hovering = $0 }
            .accessibilityLabel(title)
            .accessibilityAddTraits(chosen ? .isSelected : [])
        }
    }
}
