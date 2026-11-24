import CarbideCore
import SwiftUI

/// Two systems drawn as parallel lanes, with each notarization a tie between them.
///
/// A healthy link is a regular ladder that reaches the present. A stall is a ladder that
/// stops short. A notarization on a losing fork is drawn in red. This chain's lane runs to
/// its current tip, so the distance since the last tie is visible.
struct NotarizationLanesCard: View {
    let currency: CurrencyDetail

    @Environment(AppModel.self) private var model
    @State private var data: NotarizationData?
    @State private var hovered: UInt32?

    var body: some View {
        // A stack, not a Group: an empty Group has no view for `.task` to attach to,
        // so the load would never start.
        VStack(spacing: 0) {
            if let data, !data.rows.isEmpty {
                Card("Notarizations", trailing: "\(data.rows.count) recorded") {
                    Label(summary(data), systemImage: stale(data) ? "exclamationmark.triangle" : "link")
                        .foregroundStyle(stale(data) ? AnyShapeStyle(.orange) : AnyShapeStyle(.primary))

                    Canvas { context, size in
                        draw(context, size, data)
                    }
                    .frame(height: 150)
                    .accessibilityLabel(summary(data))

                    HStack(spacing: 18) {
                        legend(color: .accentColor, dashed: false, text: "Confirmed")
                        legend(color: .secondary, dashed: true, text: "Pending")
                        if data.forkCount > 1 {
                            legend(color: .red, dashed: false, text: "On a losing fork")
                        }
                        Spacer()
                        Text("The other system's lane is unchecked: no source for it is connected.")
                            .font(.caption).foregroundStyle(.secondary)
                    }

                    VStack(spacing: 0) {
                        ForEach(data.rows.reversed(), id: \.index) { row in
                            Divider()
                            HStack(spacing: 14) {
                                ConfidenceMark(
                                    level: row.confirmed ? .verified : row.onBestChain ? .observed : .unknown,
                                    label: row.confirmed ? "Confirmed" : row.onBestChain ? "Pending" : "Losing fork"
                                )
                                .frame(width: 120, alignment: .leading)
                                if let height = local(row) {
                                    RouteLink(
                                        text: "\(model.coin) \(Format.integer(height))",
                                        route: .block(id: "\(height)"), shorten: false)
                                }
                                if let other = row.otherHeight {
                                    Text("⇄ \(otherName(data)) \(Format.integer(other))")
                                        .foregroundStyle(.secondary).monospacedDigit()
                                }
                                Spacer()
                                if let time = row.blockTime {
                                    Text(Format.date(time)).font(.callout).foregroundStyle(.secondary)
                                }
                                if let txid = row.txid {
                                    RouteLink(text: txid, route: .transaction(txid: txid))
                                }
                            }
                            .padding(.vertical, 7)
                        }
                    }
                }
            }
        }
        .task(id: "\(currency.currencyId ?? currency.name)|\(model.generation)") {
            await load()
        }
    }

    /// The notarized system is the currency itself for a gateway or chain, and its parent
    /// for a converter such as a bridge currency.
    private func load() async {
        guard let explorer = model.explorer else { return }
        let candidates = [currency.currencyId, currency.parent].compactMap { $0 }
        // Take whichever has the fuller record: a bridge currency carries one entry of its
        // own, while its gateway carries the running series.
        var best: NotarizationData?
        for candidate in candidates where candidate != model.status?.chainId {
            if let found = try? await explorer.notarizations(currency: candidate),
                found.rows.count > (best?.rows.count ?? 0)
            {
                best = found
            }
        }
        data = best
        await model.resolveNames([best?.otherSystemId])
    }

    private func otherName(_ data: NotarizationData) -> String {
        guard let id = data.otherSystemId else { return "other system" }
        let name = model.names[id] ?? Format.short(id, head: 6, tail: 4)
        return name.hasSuffix("@") ? String(name.dropLast()) : name
    }

    /// This chain's height for a notarization: the height it commits to, or failing that
    /// the height of the block that contains it.
    private func local(_ row: NotarizationRow) -> UInt64? {
        row.localHeight ?? row.blockHeight
    }

    private func lastConfirmed(_ data: NotarizationData) -> NotarizationRow? {
        data.rows.last(where: \.confirmed)
    }

    private func blocksSinceConfirmed(_ data: NotarizationData) -> UInt64? {
        guard let height = lastConfirmed(data).flatMap(local), let tip = model.status?.blocks, tip > height
        else { return nil }
        return tip - height
    }

    /// More than a day without a confirmed notarization is worth pointing out.
    private func stale(_ data: NotarizationData) -> Bool {
        (blocksSinceConfirmed(data) ?? 0) > 1440
    }

    private func summary(_ data: NotarizationData) -> String {
        let pending = data.rows.filter { !$0.confirmed && $0.onBestChain }.count
        guard let confirmed = lastConfirmed(data), let height = local(confirmed) else {
            return "No confirmed notarization is recorded. \(pending) pending."
        }
        var text = "Last confirmed at \(model.coin) block \(Format.integer(height))"
        if let behind = blocksSinceConfirmed(data) {
            let days = Double(behind) / 1440
            text += ", \(Format.integer(behind)) blocks ago"
            text += days >= 1 ? " (about \(Int(days.rounded())) days)." : "."
        } else {
            text += "."
        }
        if pending > 0 {
            text += " \(pending) later notarization\(pending == 1 ? " is" : "s are") still pending."
        }
        return text
    }

    private func legend(color: Color, dashed: Bool, text: String) -> some View {
        HStack(spacing: 6) {
            Canvas { context, size in
                context.stroke(
                    Path { path in
                        path.move(to: CGPoint(x: 0, y: size.height / 2))
                        path.addLine(to: CGPoint(x: size.width, y: size.height / 2))
                    }, with: .color(color), style: StrokeStyle(lineWidth: 2.4, dash: dashed ? [4, 3] : []))
            }
            .frame(width: 22, height: 10)
            Text(text).font(.caption).foregroundStyle(.secondary)
        }
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize, _ data: NotarizationData) {
        let rows = data.rows.filter { local($0) != nil }
        guard let firstLocal = rows.compactMap(local).min() else { return }
        let tip = max(model.status?.blocks ?? 0, rows.compactMap(local).max() ?? firstLocal)
        let lastLocal = rows.compactMap(local).max() ?? firstLocal
        let others = rows.compactMap(\.otherHeight)
        let firstOther = others.min() ?? 0
        let lastOther = max(others.max() ?? 1, firstOther + 1)

        let left: CGFloat = 96
        let right = size.width - 16
        let top: CGFloat = 36
        let bottom: CGFloat = 112
        let pad = Double(max(tip - firstLocal, 1)) * 0.04
        let xLocal = { (height: UInt64) -> CGFloat in
            left + CGFloat((Double(height) - Double(firstLocal) + pad) / (Double(tip - firstLocal) + pad * 2))
                * (right - left)
        }
        // The other lane spans the same horizontal stretch as the notarized part of this one.
        let xOther = { (height: UInt64) -> CGFloat in
            let fraction = Double(height - firstOther) / Double(lastOther - firstOther)
            return xLocal(firstLocal) + CGFloat(fraction) * (xLocal(lastLocal) - xLocal(firstLocal))
        }

        for (y, name, unchecked) in [(top, model.coin, false), (bottom, otherName(data), true)] {
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: left - 8, y: y))
                    path.addLine(to: CGPoint(x: right, y: y))
                }, with: .color(.secondary.opacity(unchecked ? 0.16 : 0.3)),
                style: StrokeStyle(lineWidth: 12, lineCap: .round))
            context.draw(
                Text(name).font(.callout.weight(.semibold)).foregroundStyle(unchecked ? .secondary : .primary),
                at: CGPoint(x: 0, y: y), anchor: .leading)
        }

        for row in rows {
            guard let local = local(row) else { continue }
            let color: Color = !row.onBestChain ? .red : row.confirmed ? .accentColor : .secondary
            let a = CGPoint(x: xLocal(local), y: top)
            let b = CGPoint(x: row.otherHeight.map(xOther) ?? a.x, y: bottom)
            context.stroke(
                Path { path in
                    path.move(to: a)
                    path.addLine(to: b)
                }, with: .color(color),
                style: StrokeStyle(lineWidth: row.confirmed ? 2.6 : 1.6, dash: row.confirmed ? [] : [5, 4]))
            for point in [a, b] {
                context.fill(
                    Path(ellipseIn: CGRect(x: point.x - 5, y: point.y - 5, width: 10, height: 10)),
                    with: .color(color))
            }
        }

        // This chain's tip, and the stretch since the last confirmed tie.
        let tipX = xLocal(tip)
        if let confirmed = lastConfirmed(data).flatMap(local), stale(data) {
            let from = xLocal(confirmed)
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: from, y: top - 16))
                    path.addLine(to: CGPoint(x: tipX, y: top - 16))
                }, with: .color(.orange), style: StrokeStyle(lineWidth: 1.5, dash: [3, 4]))
            context.draw(
                Text("not confirmed since").font(.caption2.weight(.semibold)).foregroundStyle(.orange),
                at: CGPoint(x: (from + tipX) / 2, y: top - 26), anchor: .center)
        }
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: tipX, y: top - 12))
                path.addLine(to: CGPoint(x: tipX, y: top + 12))
            }, with: .color(.primary), lineWidth: 1.5)
        context.draw(
            Text("now").font(.caption2).foregroundStyle(.primary),
            at: CGPoint(x: tipX, y: top + 22), anchor: .center)
    }
}
