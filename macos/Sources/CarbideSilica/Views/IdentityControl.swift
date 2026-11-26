import CarbideCore
import SwiftUI

/// Who holds each power of an identity, and what follows when its key is lost or stolen.
///
/// The control of an identity has four parts: the keys that spend and update, the identity
/// that can revoke, the identity that can recover, and the lock on the funds. They are a
/// table, because each identity has the same four. The two outcomes are in words and in
/// view together. An identity that is its own authority is called out, because that is the
/// difference between a recoverable identity and one that is only as safe as its keys.
struct IdentityControlCard: View {
    let identity: IdentityState

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        let _ = showsNames
        Card("Control", trailing: "at the viewed version") {
            Text(sentence)
                .font(.title3)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
            Grid(alignment: .leadingFirstTextBaseline, horizontalSpacing: 18, verticalSpacing: 7) {
                GridRow {
                    NameText("Power")
                    NameText("Held by")
                    NameText("What it does")
                }
                Divider()
                GridRow {
                    NameText("Spend and update")
                    VStack(alignment: .leading, spacing: 2) {
                        Text(keys.count <= 1 ? "1 key" : "\(required) of \(keys.count) keys")
                        ForEach(keys, id: \.self) { key in
                            RouteLink(text: key, route: .holder(key))
                        }
                    }
                    Text("Moves funds and changes the identity.").foregroundStyle(.secondary)
                }
                Divider()
                GridRow {
                    NameText("Revoke")
                    holder(identity.revocationAuthority, isSelf: selfRevoke)
                    Text("Stops all use of the identity at once.").foregroundStyle(.secondary)
                }
                Divider()
                GridRow {
                    NameText("Recover")
                    holder(identity.recoveryAuthority, isSelf: selfRecover)
                    Text("Sets new keys for a revoked identity.").foregroundStyle(.secondary)
                }
                Divider()
                GridRow {
                    NameText("Lock")
                    Text(lockHolder)
                    Text(lockEffect).foregroundStyle(.secondary)
                }
            }
            if identity.revoked {
                Note(text: "This version is revoked. The keys cannot spend or sign until a recovery.", warning: true)
            }
            VStack(alignment: .leading, spacing: 4) {
                outcome("If the key is lost", lost, warning: selfRecover)
                outcome("If the key is stolen", stolen, warning: selfRevoke)
            }
        }
    }

    private var name: String { identity.name }
    private var keys: [String] { identity.primaryAddresses }
    private var required: Int { Int(identity.minimumSignatures ?? 1) }
    private var selfRevoke: Bool { identity.revocationAuthority == nil || identity.revocationIsSelf }
    private var selfRecover: Bool { identity.recoveryAuthority == nil || identity.recoveryIsSelf }

    private func display(_ id: String?) -> String {
        guard let id else { return "itself" }
        return model.displayID(id)
    }

    @ViewBuilder
    private func holder(_ id: String?, isSelf: Bool) -> some View {
        if !isSelf, let id {
            RouteLink(text: id, route: .identity(name: id))
        } else {
            Text("The identity itself")
        }
    }

    private func outcome(_ title: String, _ text: String, warning: Bool) -> some View {
        (Text(title + ": ").fontWeight(.semibold) + Text(text))
            .foregroundStyle(warning ? Tone.mined : Color.primary)
            .fixedSize(horizontal: false, vertical: true)
            .textSelection(.enabled)
    }

    // MARK: Words

    /// A number of blocks as a length of time, at one block for each minute.
    static func span(blocks: UInt64) -> String {
        switch blocks {
        case ..<120: "\(blocks) minutes"
        case ..<2880: "\(Int((Double(blocks) / 60).rounded())) hours"
        case ..<86_400: "\(Int((Double(blocks) / 1440).rounded())) days"
        default: "\((Double(blocks) / 525_600).formatted(.number.precision(.fractionLength(0...1)))) years"
        }
    }

    private var lockHolder: String {
        switch identity.timelock {
        case .unlocked: "None"
        case .delay(let blocks): "Delay of \(Format.integer(blocks)) blocks"
        case .unlocksAt(let height): "Until block \(Format.integer(height))"
        }
    }

    private var lockEffect: String {
        switch identity.timelock {
        case .unlocked: "Funds can move at once."
        case .delay(let blocks):
            "Funds cannot move until about \(Self.span(blocks: blocks)) after an unlock is requested."
        case .unlocksAt: "Funds cannot move before that block."
        }
    }

    private var sentence: String {
        var parts: [String] = [
            keys.count <= 1
                ? "One key controls \(name)."
                : "\(required) of the \(keys.count) keys must sign to control \(name)."
        ]
        if selfRevoke, selfRecover {
            parts.append("It is its own authority: no other identity can revoke or recover it.")
        } else if !selfRevoke, !selfRecover, identity.revocationAuthority == identity.recoveryAuthority {
            parts.append("\(display(identity.revocationAuthority)) can revoke and recover it.")
        } else {
            let revoke = selfRevoke ? "Only its own keys can revoke it" : "\(display(identity.revocationAuthority)) can revoke it"
            let recover = selfRecover ? "no other identity can recover it" : "\(display(identity.recoveryAuthority)) can recover it"
            parts.append("\(revoke) and \(recover).")
        }
        switch identity.timelock {
        case .unlocked: break
        case .delay(let blocks):
            parts.append(
                "The funds are locked: an unlock takes \(Format.integer(blocks)) blocks, about \(Self.span(blocks: blocks)).")
        case .unlocksAt(let height):
            parts.append("The funds are locked until block \(Format.integer(height)).")
        }
        return parts.joined(separator: " ")
    }

    private var lost: String {
        selfRecover
            ? "No other identity can set new keys. The identity and its funds are lost."
            : "\(display(identity.recoveryAuthority)) can set new keys after a revocation. \(name) keeps its name, its funds and its history."
    }

    private var stolen: String {
        var text: String
        if selfRevoke, selfRecover {
            text = "No other identity can stop the thief."
        } else {
            text = selfRevoke
                ? "Only a holder of the keys can revoke \(name)"
                : "\(display(identity.revocationAuthority)) can revoke \(name), and the stolen keys then cannot spend or sign"
            text += selfRecover ? ". No other identity can recover it." : ". Then \(display(identity.recoveryAuthority)) can set new keys."
        }
        switch identity.timelock {
        case .unlocked: break
        case .delay(let blocks):
            text += " A thief must request an unlock and then wait about \(Self.span(blocks: blocks)) to move funds."
        case .unlocksAt(let height):
            text += " A thief cannot move funds before block \(Format.integer(height))."
        }
        return text
    }
}

/// The history of an identity as lanes: the keys, the revoke authority, the recover
/// authority and the lock, with one step for each version.
///
/// The updates of an identity frequently come in groups of some days with years between
/// them. Thus the lanes give each version the same width, and a time line above shows the
/// true block of each one. A bar is the versions for which one value was in effect: an
/// authority that did not change is one bar. A red band is a revoked version. Select a
/// step to view the identity as it was after that update.
struct VersionStrip: View {
    let versions: [IdentityState]
    let tip: UInt64?
    @Binding var selection: Int

    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsDates) private var showsDates = false
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    private let labelWidth: CGFloat = 72
    private let timeHeight: CGFloat = 34
    private let laneHeight: CGFloat = 24
    private static let lanes = ["Keys", "Revoke", "Recover", "Lock"]
    /// The holders of a lane, in order of their first version.
    private static let palette: [Color] = Tone.series

    var body: some View {
        let _ = showsNames
        DisplayPanel(padding: 10) {
            VStack(alignment: .leading, spacing: 6) {
                HStack(alignment: .top, spacing: 0) {
                    VStack(alignment: .leading, spacing: 0) {
                        NameText("Time").frame(height: timeHeight, alignment: .top)
                        ForEach(Self.lanes, id: \.self) { lane in
                            Plate(lane).frame(height: laneHeight)
                        }
                    }
                    .frame(width: labelWidth, alignment: .leading)
                    ZStack(alignment: .topLeading) {
                        Canvas { context, size in
                            draw(context, size)
                        }
                        .accessibilityHidden(true)
                        // One button for each version: the selection, the help text and the
                        // accessible name.
                        HStack(spacing: 0) {
                            ForEach(Array(versions.enumerated()), id: \.offset) { index, version in
                                Button {
                                    selection = index
                                } label: {
                                    Color.clear.contentShape(Rectangle())
                                }
                                .buttonStyle(.plain)
                                .frame(maxWidth: .infinity)
                                .help("Version \(index + 1): \(heightAndTime(version.height, version.time, model: model)). \(version.changes.joined(separator: ". "))")
                                .accessibilityLabel("Version \(index + 1), \(spoken(version))")
                            }
                        }
                        .padding(.top, timeHeight)
                    }
                    .frame(height: timeHeight + laneHeight * CGFloat(Self.lanes.count))
                }
                PanelRule()
                HStack {
                    Text(edge(versions.first)).font(Tone.figureSmall).monospacedDigit().foregroundStyle(Tone.label)
                    Spacer()
                    Text(selectedText).font(Tone.caption.weight(.semibold)).foregroundStyle(Tone.present)
                    Spacer()
                    Text(edge(versions.last)).font(Tone.figureSmall).monospacedDigit().foregroundStyle(Tone.label)
                }
                Text(
                    "Each step is one version, and the line at the top shows its true block. A change of colour in a lane "
                        + "is a change of holder. In the lock lane, a solid bar is a lock with a delay and a striped bar is a "
                        + "lock to a set block. A red band is a revoked version. Select a step to view that version.")
                    .font(Tone.caption).foregroundStyle(Tone.label)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    // MARK: The values of the lanes

    private func keys(_ version: IdentityState) -> String {
        let count = version.primaryAddresses.count
        let first = version.primaryAddresses.first.map { Format.short($0, head: 6, tail: 4) } ?? "no key"
        return count <= 1 ? first : "\(version.minimumSignatures ?? 1) of \(count): \(first)"
    }

    private func authority(_ id: String?, isSelf: Bool) -> String {
        guard !isSelf, let id else { return "itself" }
        return model.displayID(id)
    }

    private func values(_ lane: Int) -> [String] {
        versions.map { version in
            switch lane {
            case 0: keys(version) + "|" + version.primaryAddresses.joined(separator: ",")
            case 1: authority(version.revocationAuthority, isSelf: version.revocationAuthority == nil || version.revocationIsSelf)
            case 2: authority(version.recoveryAuthority, isSelf: version.recoveryAuthority == nil || version.recoveryIsSelf)
            default: version.timelockLabel
            }
        }
    }

    private func spoken(_ version: IdentityState) -> String {
        version.revoked ? "revoked" : (version.changes.first ?? "updated")
    }

    // MARK: Drawing

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let count = versions.count
        guard count > 0 else { return }
        let step = size.width / CGFloat(count)
        // The time line: a tick at the true block of each version, and a line to its step.
        let first = versions.first?.height ?? 0
        let last = max(tip ?? versions.last?.height ?? first, versions.last?.height ?? first)
        func time(_ height: UInt64) -> CGFloat {
            last > first ? 4 + CGFloat(Double(height - first) / Double(last - first)) * (size.width - 8) : size.width / 2
        }
        context.rule(4, 8, size.width - 4, 8, Tone.axis)
        var leaders = Path()
        var ticks = Path()
        for (index, version) in versions.enumerated() {
            let x = time(version.height)
            ticks.addRect(CGRect(x: x - 0.5, y: 3, width: 1, height: 10))
            leaders.move(to: CGPoint(x: x, y: 13))
            leaders.addLine(to: CGPoint(x: (CGFloat(index) + 0.5) * step, y: timeHeight - 3))
        }
        context.fill(ticks, with: .color(Tone.value))
        context.stroke(leaders, with: .color(Tone.axis), lineWidth: 0.6)

        for lane in 0..<Self.lanes.count {
            let y = timeHeight + CGFloat(lane) * laneHeight
            let lock = lane == 3
            context.fill(
                Path(CGRect(x: 0, y: y + 3, width: size.width, height: laneHeight - 6)),
                with: .color(Tone.grid))
            let values = values(lane)
            var seen: [String] = []
            var start = 0
            for index in 1...count where index == count || values[index] != values[start] {
                let value = values[start]
                let bar = CGRect(
                    x: CGFloat(start) * step, y: y + 3, width: max(CGFloat(index - start) * step - 1, 1.5),
                    height: laneHeight - 6)
                if lock {
                    switch versions[start].timelock {
                    case .unlocked: break
                    case .delay: context.fill(Path(bar), with: .color(Tone.record.opacity(0.9)))
                    case .unlocksAt:
                        context.fill(Path(bar), with: .color(Tone.record.opacity(0.25)))
                        var stripes = Path()
                        var x = bar.minX - bar.height
                        while x < bar.maxX {
                            stripes.move(to: CGPoint(x: x, y: bar.maxY))
                            stripes.addLine(to: CGPoint(x: x + bar.height, y: bar.minY))
                            x += 6
                        }
                        var clipped = context
                        clipped.clip(to: Path(bar))
                        clipped.stroke(stripes, with: .color(Tone.record), lineWidth: 2)
                    }
                } else {
                    if !seen.contains(value) { seen.append(value) }
                    let color = Self.palette[min(seen.firstIndex(of: value) ?? 0, Self.palette.count - 1)]
                    context.fill(Path(bar), with: .color(color.opacity(0.85)))
                    let label = String(value.split(separator: "|").first ?? "")
                    let text = context.resolve(
                        Text(label).font(.caption.weight(.semibold)).foregroundStyle(Tone.screen))
                    if text.measure(in: size).width + 10 < bar.width {
                        context.draw(text, at: CGPoint(x: bar.minX + 5, y: bar.midY), anchor: .leading)
                    }
                }
                start = index
            }
        }
        let lanesHeight = laneHeight * CGFloat(Self.lanes.count)
        for (index, version) in versions.enumerated() where version.revoked {
            context.fill(
                Path(CGRect(x: CGFloat(index) * step, y: timeHeight, width: step, height: lanesHeight)),
                with: .color(Tone.alert.opacity(0.3)))
        }
        // Thin gaps between the steps, and the selected step.
        if count <= 80 {
            for index in 1..<max(count, 1) {
                context.fill(
                    Path(CGRect(x: CGFloat(index) * step - 0.5, y: timeHeight, width: 1, height: lanesHeight)),
                    with: .color(Tone.screen.opacity(0.7)))
            }
        }
        if versions.indices.contains(selection) {
            let column = CGRect(x: CGFloat(selection) * step, y: timeHeight + 1, width: step, height: lanesHeight - 2)
            // The selected version is the present: a lit outline.
            let outline = Path(column.insetBy(dx: 1, dy: 0)).strokedPath(StrokeStyle(lineWidth: 2))
            context.lit(outline, Tone.present, strength: 0.7)
        }
    }

    private func edge(_ version: IdentityState?) -> String {
        guard let version else { return "" }
        guard !showsDates, let time = version.time else { return model.heightText(version.height, time: version.time) }
        return Date(timeIntervalSince1970: TimeInterval(time)).formatted(date: .abbreviated, time: .omitted)
    }

    private var selectedText: String {
        guard versions.indices.contains(selection) else { return "" }
        let version = versions[selection]
        return "Version \(selection + 1) of \(versions.count): \(version.revoked ? "Revoked" : version.changes.first ?? "Updated")"
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
                .buttonStyle(.displayLink)
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
                Rectangle().fill(isFirst ? Color.clear : Tone.axis)
                    .frame(width: 1, height: 8)
                Rectangle().fill(version.revoked ? Tone.alert : selected ? Tone.present : Tone.record)
                    .frame(width: selected ? 11 : 8, height: selected ? 11 : 8)
                    .shadow(color: selected ? Tone.present.opacity(0.7) : .clear, radius: selected ? 3 : 0)
                Rectangle().fill(isLast ? Color.clear : Tone.axis)
                    .frame(width: 1)
                    .frame(maxHeight: .infinity)
            }
            .frame(width: 13)

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    Button("Version \(number)", action: select)
                        .buttonStyle(.displayLink)
                        .fontWeight(.semibold)
                    if selected {
                        Plate("Viewed")
                    }
                    Text(when).foregroundStyle(.secondary)
                    RouteLink(
                        text: model.heightText(version.height, time: version.time),
                        route: .block(id: version.blockHash ?? "\(version.height)"), shorten: false)
                    if version.onActiveChain == false {
                        Label("Not on the active chain", systemImage: "exclamationmark.triangle")
                            .font(.caption).foregroundStyle(Tone.mined)
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
