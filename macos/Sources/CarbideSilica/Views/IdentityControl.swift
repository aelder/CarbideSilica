import CarbideCore
import SwiftUI

/// Who holds each of an identity's three powers: spending, revoking and recovering.
///
/// The scenarios show what the powers are for, using this identity's real keys and
/// authorities. An identity that is its own authority is called out, because that is the
/// difference between a recoverable identity and one that is only as safe as its keys.
struct IdentityControlCard: View {
    let identity: IdentityDetail

    @Environment(AppModel.self) private var model
    @State private var scenario = Scenario.everyday

    enum Scenario: String, CaseIterable, Identifiable {
        case everyday = "Everyday use"
        case stolen = "If keys are stolen"
        case recovery = "Recovery"
        var id: String { rawValue }
    }

    var body: some View {
        Card("Who controls it") {
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

            Label(sentence, systemImage: warning ? "exclamationmark.triangle" : "info.circle")
                .foregroundStyle(warning ? AnyShapeStyle(.orange) : AnyShapeStyle(.primary))
                .font(.callout)
        }
    }

    private var name: String { identity.name }
    private var keys: [String] { identity.primaryAddresses }
    private var required: Int { Int(identity.minimumSignatures ?? 1) }
    private var selfRevoke: Bool {
        identity.revocationAuthority == nil || identity.revocationAuthority == identity.identityAddress
    }
    private var selfRecover: Bool {
        identity.recoveryAuthority == nil || identity.recoveryAuthority == identity.identityAddress
    }

    private func display(_ id: String?) -> String {
        guard let id else { return "itself" }
        return model.names[id] ?? Format.short(id, head: 8, tail: 6)
    }

    private var warning: Bool {
        (scenario == .stolen && selfRevoke) || (scenario == .recovery && selfRecover)
    }

    private var sentence: String {
        switch scenario {
        case .everyday:
            if keys.count <= 1 { return "One key controls \(name). Whoever holds it can spend and update." }
            return "Any \(required) of the \(keys.count) keys sign together to spend or update \(name)."
        case .stolen:
            if selfRevoke {
                return "\(name) is its own revocation authority. Whoever holds its keys also holds "
                    + "the power to revoke, so stolen keys can't be shut out by anyone else."
            }
            return "\(display(identity.revocationAuthority)) can revoke \(name). That freezes it: "
                + "the stolen keys can no longer spend or sign."
        case .recovery:
            if selfRecover {
                return "\(name) is its own recovery authority. If its keys are lost, nothing else "
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
            case .stolen: selfRevoke ? ("No outside revoker", .orange) : ("Revoked · frozen", .red)
            case .recovery: selfRecover ? ("No outside recovery", .orange) : ("Active again", .green)
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

/// Every recorded version of an identity, newest first, with what changed each time.
struct IdentityHistoryCard: View {
    let name: String
    @State private var showAll = false

    private let collapsedCount = 8

    var body: some View {
        LoadView(key: "identity-history:\(name)") { explorer in
            try await explorer.identityHistory(name: name)
        } content: { versions in
            Card("History", trailing: "\(versions.count) version\(versions.count == 1 ? "" : "s")") {
                let newestFirst = Array(versions.reversed())
                let shown = showAll ? newestFirst : Array(newestFirst.prefix(collapsedCount))
                VStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(shown.enumerated()), id: \.element.height) { index, version in
                        HistoryRow(
                            version: version,
                            isFirst: index == 0, isLast: index == shown.count - 1)
                    }
                }
                if newestFirst.count > collapsedCount {
                    Button(showAll ? "Show fewer" : "Show all \(newestFirst.count)") {
                        showAll.toggle()
                    }
                    .buttonStyle(.link)
                }
            }
        }
        .frame(minHeight: 80)
    }
}

private struct HistoryRow: View {
    let version: IdentityVersion
    let isFirst: Bool
    let isLast: Bool

    var body: some View {
        HStack(alignment: .top, spacing: 12) {
            // The line and its dot.
            VStack(spacing: 0) {
                Rectangle().fill(isFirst ? Color.clear : Color.secondary.opacity(0.4))
                    .frame(width: 2, height: 8)
                Circle().fill(Color.accentColor).frame(width: 10, height: 10)
                Rectangle().fill(isLast ? Color.clear : Color.secondary.opacity(0.4))
                    .frame(width: 2)
                    .frame(maxHeight: .infinity)
            }
            .frame(width: 10)

            VStack(alignment: .leading, spacing: 3) {
                HStack(spacing: 8) {
                    Text(when).fontWeight(.semibold)
                    RouteLink(
                        text: "block \(Format.integer(version.height))",
                        route: .block(id: version.blockHash ?? "\(version.height)"), shorten: false)
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
        guard let time = version.time else { return "Date not loaded" }
        return Date(timeIntervalSince1970: TimeInterval(time))
            .formatted(date: .abbreviated, time: .omitted)
    }
}
