import CarbideCore
import SwiftUI

/// Who holds each of an identity's three powers: spending, revoking and recovering.
///
/// The scenarios show what the powers are for, using this identity's real keys and
/// authorities. An identity that is its own authority is called out, because that is the
/// difference between a recoverable identity and one that is only as safe as its keys.
struct IdentityControlCard: View {
    let identity: IdentityState

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @State private var scenario = Scenario.everyday

    enum Scenario: String, CaseIterable, Identifiable {
        case everyday = "Everyday use"
        case stolen = "Stolen keys"
        case recovery = "Recovery"
        var id: String { rawValue }
    }

    var body: some View {
        Card("Control", trailing: "at the viewed version") {
            Picker("Scenario", selection: $scenario) {
                ForEach(Scenario.allCases) { Text($0.rawValue).tag($0) }
            }
            .pickerStyle(.segmented)
            .labelsHidden()
            .frame(maxWidth: 420)

            Canvas { context, size in
                draw(context, size)
            }
            .frame(height: 230)
            .accessibilityLabel(sentence)

            Note(text: sentence, warning: warning).font(.callout)
            Text("Rounded ends are the primary addresses. The boxes on the right are the identities that can revoke and recover. A dashed line means that the identity holds that power itself.")
                .font(.caption).foregroundStyle(.secondary)
        }
    }

    private var name: String { identity.name }
    private var keys: [String] { identity.primaryAddresses }
    private var required: Int { Int(identity.minimumSignatures ?? 1) }
    private var selfRevoke: Bool {
        identity.revocationAuthority == nil || identity.revocationIsSelf
    }
    private var selfRecover: Bool {
        identity.recoveryAuthority == nil || identity.recoveryIsSelf
    }

    private func display(_ id: String?) -> String {
        guard let id else { return "itself" }
        return model.displayID(id)
    }

    private var warning: Bool {
        (scenario == .stolen && selfRevoke) || (scenario == .recovery && selfRecover)
    }

    private var sentence: String {
        switch scenario {
        case .everyday:
            if keys.count <= 1 { return "One key controls \(name). The holder of the key can spend funds and update the identity." }
            return "\(required) of the \(keys.count) keys must sign to spend funds or update \(name)."
        case .stolen:
            if selfRevoke {
                return "\(name) is its own revocation authority. A person with the keys can also revoke. "
                    + "No other identity can stop stolen keys."
            }
            return "\(display(identity.revocationAuthority)) can revoke \(name). After revocation, "
                + "the stolen keys cannot spend or sign."
        case .recovery:
            if selfRecover {
                return "\(name) is its own recovery authority. If the keys are lost, no other identity "
                    + "can restore control."
            }
            return "\(display(identity.recoveryAuthority)) can assign new keys. \(name) keeps its "
                + "name, its funds and its history."
        }
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let centre = CGRect(x: size.width / 2 - 85, y: size.height / 2 - 34, width: 170, height: 68)
        let shownKeys = Array(keys.prefix(4))
        let keyX: CGFloat = 0
        let keyWidth = min(size.width * 0.24, 200)
        let sideWidth = min(size.width * 0.26, 210)
        let sideX = size.width - sideWidth

        // Keys on the left.
        for (index, key) in shownKeys.enumerated() {
            let y = size.height * (CGFloat(index) + 0.5) / CGFloat(max(shownKeys.count, 1))
            let box = CGRect(x: keyX, y: y - 17, width: keyWidth, height: 34)
            let signing = scenario == .everyday && index < required
            let color: Color =
                switch scenario {
                case .everyday: signing ? .accentColor : .secondary
                case .stolen: .red
                case .recovery: selfRecover ? .secondary : .green
                }
            line(context, from: CGPoint(x: box.maxX, y: y), to: CGPoint(x: centre.minX, y: centre.midY),
                 color: color, strong: scenario != .everyday || signing, dashed: scenario == .stolen)
            node(context, box, color: color, strong: scenario != .everyday || signing, capsule: true,
                 title: scenario == .stolen ? "Stolen key" : scenario == .recovery && !selfRecover ? "New key" : Format.short(key, head: 6, tail: 5),
                 subtitle: nil)
        }
        if keys.count > shownKeys.count {
            context.draw(
                Text("+ \(keys.count - shownKeys.count) more keys").font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: keyX + keyWidth / 2, y: size.height - 4), anchor: .bottom)
        }

        // Authorities on the right.
        let revoke = CGRect(x: sideX, y: size.height * 0.25 - 26, width: sideWidth, height: 52)
        let recover = CGRect(x: sideX, y: size.height * 0.75 - 26, width: sideWidth, height: 52)
        let revokeActive = scenario == .stolen
        let recoverActive = scenario == .recovery
        line(context, from: CGPoint(x: centre.maxX, y: centre.midY), to: CGPoint(x: revoke.minX, y: revoke.midY),
             color: revokeActive ? .red : .secondary, strong: revokeActive, dashed: selfRevoke)
        line(context, from: CGPoint(x: centre.maxX, y: centre.midY), to: CGPoint(x: recover.minX, y: recover.midY),
             color: recoverActive ? .green : .secondary, strong: recoverActive, dashed: selfRecover)
        node(context, revoke, color: revokeActive ? .red : .secondary, strong: revokeActive, capsule: false,
             title: selfRevoke ? "Itself" : display(identity.revocationAuthority), subtitle: "can revoke",
             dashed: selfRevoke)
        node(context, recover, color: recoverActive ? .green : .secondary, strong: recoverActive, capsule: false,
             title: selfRecover ? "Itself" : display(identity.recoveryAuthority), subtitle: "can recover",
             dashed: selfRecover)

        // The identity.
        let state: (String, Color) =
            switch scenario {
            case .everyday: (keys.count <= 1 ? "1 key" : "\(required) of \(keys.count) keys", .accentColor)
            case .stolen: selfRevoke ? ("No external revoker", .orange) : ("Revoked · frozen", .red)
            case .recovery: selfRecover ? ("No external recovery", .orange) : ("Active again", .green)
            }
        let shape = Path(roundedRect: centre, cornerRadius: 10)
        context.fill(shape, with: .color(state.1.opacity(0.14)))
        context.stroke(shape, with: .color(state.1), lineWidth: 2.4)
        context.draw(
            Text(Format.short(name, head: 14, tail: 6)).font(.headline),
            at: CGPoint(x: centre.midX, y: centre.midY - 9), anchor: .center)
        context.draw(
            Text(state.0).font(.caption.weight(.semibold)).foregroundStyle(state.1),
            at: CGPoint(x: centre.midX, y: centre.midY + 12), anchor: .center)
    }

    private func line(_ context: GraphicsContext, from: CGPoint, to: CGPoint, color: Color, strong: Bool, dashed: Bool) {
        context.stroke(
            Path { path in
                path.move(to: from)
                path.addLine(to: to)
            }, with: .color(color.opacity(strong ? 1 : 0.5)),
            style: StrokeStyle(lineWidth: strong ? 2.6 : 1.2, dash: dashed ? [5, 4] : []))
    }

    private func node(
        _ context: GraphicsContext, _ box: CGRect, color: Color, strong: Bool, capsule: Bool,
        title: String, subtitle: String?, dashed: Bool = false
    ) {
        let shape = Path(roundedRect: box, cornerRadius: capsule ? box.height / 2 : 8)
        context.fill(shape, with: .color(Color(nsColor: .windowBackgroundColor)))
        context.stroke(
            shape, with: .color(color.opacity(strong ? 1 : 0.6)),
            style: StrokeStyle(lineWidth: strong ? 2.2 : 1.4, dash: dashed ? [5, 4] : []))
        context.draw(
            Text(title).font(.callout.weight(.semibold)).foregroundStyle(strong ? color : .primary),
            at: CGPoint(x: box.midX, y: box.midY - (subtitle == nil ? 0 : 8)), anchor: .center)
        if let subtitle {
            context.draw(
                Text(subtitle).font(.caption2).foregroundStyle(.secondary),
                at: CGPoint(x: box.midX, y: box.midY + 10), anchor: .center)
        }
    }
}

/// The versions of an identity on a line of block heights. Each mark is one update.
/// Select a mark to view the identity as it was after that update.
struct VersionStrip: View {
    let versions: [IdentityState]
    let tip: UInt64?
    @Binding var selection: Int

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            GeometryReader { geometry in
                let width = geometry.size.width
                let exact = versions.map { x($0.height, width) }
                let marks = Self.spread(exact, width: width)
                ZStack(alignment: .topLeading) {
                    // The axis of block heights, a tick at each update's real height, and a
                    // leader from the tick to its mark when marks were spread apart.
                    Rectangle()
                        .fill(Color.secondary.opacity(0.35))
                        .frame(width: width, height: 2)
                        .offset(y: 37)
                    Path { path in
                        for (real, shown) in zip(exact, marks) {
                            path.move(to: CGPoint(x: real, y: 32))
                            path.addLine(to: CGPoint(x: real, y: 42))
                            if abs(real - shown) > 1 {
                                path.move(to: CGPoint(x: real, y: 32))
                                path.addLine(to: CGPoint(x: shown, y: 24))
                            }
                        }
                    }
                    .stroke(Color.secondary.opacity(0.6), lineWidth: 1)
                    ForEach(Array(versions.enumerated()), id: \.offset) { index, version in
                        mark(index, version)
                            .position(x: marks[index], y: 12)
                    }
                }
            }
            .frame(height: 44)
            HStack {
                Text(edge(versions.first)).font(.caption).foregroundStyle(.secondary)
                Spacer()
                Text(selectedText).font(.caption.weight(.semibold))
                Spacer()
                Text(edge(versions.last)).font(.caption).foregroundStyle(.secondary)
            }
            Text("Each mark is an update. Its tick on the line is its block height. Marks for updates close together are spread apart, with a line to their ticks. A red mark is a revoked version. A blue mark changed who controls the identity. A grey mark changed other data. Select a mark to view that version.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .padding(.horizontal, 8)
    }

    /// Moves marks apart so that each has room to be clicked, keeping their order and
    /// keeping each as near its real position as the width allows.
    static func spread(_ positions: [CGFloat], width: CGFloat, gap: CGFloat = 26, inset: CGFloat = 12) -> [CGFloat] {
        guard !positions.isEmpty else { return [] }
        let room = width - 2 * inset
        if CGFloat(positions.count - 1) * gap > room {
            // More marks than fit with the gap: equal spacing over the whole width.
            let step = positions.count > 1 ? room / CGFloat(positions.count - 1) : 0
            return positions.indices.map { inset + CGFloat($0) * step }
        }
        var result = positions
        for index in result.indices.dropFirst() {
            result[index] = max(result[index], result[index - 1] + gap)
        }
        if let last = result.indices.last, result[last] > width - inset {
            result[last] = width - inset
            for index in result.indices.reversed().dropFirst() {
                result[index] = min(result[index], result[index + 1] - gap)
            }
        }
        return result
    }

    private func x(_ height: UInt64, _ width: CGFloat) -> CGFloat {
        guard let first = versions.first?.height, let last = versions.last?.height, last > first
        else { return width / 2 }
        let inset: CGFloat = 12
        return inset + (width - 2 * inset) * CGFloat(height - first) / CGFloat(last - first)
    }

    private func mark(_ index: Int, _ version: IdentityState) -> some View {
        let selected = index == selection
        let color = kind(version).color
        return Button {
            selection = index
        } label: {
            ZStack {
                if selected {
                    Circle().stroke(color, lineWidth: 2).frame(width: 22, height: 22)
                }
                Circle().fill(color).frame(width: selected ? 13 : 10, height: selected ? 13 : 10)
            }
            .frame(width: 24, height: 24)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help("Version \(index + 1): \(heightAndTime(version.height, version.time, model: model)). \(version.changes.joined(separator: ". "))")
        .accessibilityLabel("Version \(index + 1), \(kind(version).word)")
    }

    private func kind(_ version: IdentityState) -> (color: Color, word: String) {
        if version.revoked { return (.red, "revoked") }
        let control = version.changes.contains {
            $0.hasPrefix("Registered") || $0.hasPrefix("Signing keys") || $0.hasPrefix("Signatures")
                || $0.hasPrefix("Revocation") || $0.hasPrefix("Recovery")
        }
        return control ? (.accentColor, "control changed") : (.secondary, "other data changed")
    }

    private func edge(_ version: IdentityState?) -> String {
        guard let version else { return "" }
        guard !showsDates, let time = version.time else { return model.heightText(version.height, time: version.time) }
        return Date(timeIntervalSince1970: TimeInterval(time)).formatted(date: .abbreviated, time: .omitted)
    }

    private var selectedText: String {
        guard versions.indices.contains(selection) else { return "" }
        let version = versions[selection]
        return "Version \(selection + 1): \(version.changes.first ?? "Updated") (\(kind(version).word))"
    }
}

/// Every version of an identity, newest first, with what changed each time. Select a row
/// to view that version.
struct IdentityHistoryCard: View {
    let timeline: IdentityTimeline
    @Binding var selection: Int
    @State private var showAll = false

    private let collapsedCount = 8

    var body: some View {
        Card("History", trailing: "\(timeline.versions.count) version\(timeline.versions.count == 1 ? "" : "s")") {
            let newestFirst = Array(timeline.versions.enumerated().reversed())
            let shown = showAll ? newestFirst : Array(newestFirst.prefix(collapsedCount))
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(shown.enumerated()), id: \.element.offset) { position, entry in
                    HistoryRow(
                        number: entry.offset + 1,
                        version: entry.element,
                        selected: entry.offset == selection,
                        isFirst: position == 0, isLast: position == shown.count - 1,
                        select: { selection = entry.offset })
                }
            }
            if newestFirst.count > collapsedCount {
                Button(showAll ? "Show fewer" : "Show all \(newestFirst.count)") {
                    showAll.toggle()
                }
                .buttonStyle(.link)
            }
            if !timeline.historyComplete {
                Note(text: "The source did not return every earlier version.", warning: true)
            }
        }
    }
}

private struct HistoryRow: View {
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsDates) private var showsDates = false
    let number: Int
    let version: IdentityState
    let selected: Bool
    let isFirst: Bool
    let isLast: Bool
    let select: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            // The line and its dot.
            VStack(spacing: 0) {
                Rectangle().fill(isFirst ? Color.clear : Color.secondary.opacity(0.4))
                    .frame(width: 2, height: 8)
                Circle().fill(version.revoked ? Color.red : Color.accentColor)
                    .frame(width: selected ? 13 : 10, height: selected ? 13 : 10)
                Rectangle().fill(isLast ? Color.clear : Color.secondary.opacity(0.4))
                    .frame(width: 2)
                    .frame(maxHeight: .infinity)
            }
            .frame(width: 13)

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    Button("Version \(number)", action: select)
                        .buttonStyle(.link)
                        .fontWeight(.semibold)
                    if selected {
                        Text("viewed").font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                    }
                    Text(when).foregroundStyle(.secondary)
                    RouteLink(
                        text: model.heightText(version.height, time: version.time),
                        route: .block(id: version.blockHash ?? "\(version.height)"), shorten: false)
                    if version.onActiveChain == false {
                        Label("Not on the active chain", systemImage: "exclamationmark.triangle")
                            .font(.caption).foregroundStyle(.orange)
                    }
                }
                ForEach(version.changes, id: \.self) { change in
                    Text(change).foregroundStyle(.secondary)
                }
            }
            .padding(.bottom, 14)

            Spacer(minLength: 8)
            if let txid = version.txid {
                RouteLink(text: txid, route: .transaction(txid: txid))
                    .padding(.top, 4)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    private var when: String {
        guard let time = version.time else { return "Date not available" }
        return Date(timeIntervalSince1970: TimeInterval(time))
            .formatted(date: .abbreviated, time: .omitted)
    }
}
