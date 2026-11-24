import CarbideCore
import SwiftUI

/// Waiting transactions as tiles: sized by bytes, darker the longer they have waited, and
/// dashed when they depend on another waiting transaction.
struct MempoolView: View {
    @Environment(AppModel.self) private var model
    @State private var rows: [MempoolRow] = []
    @State private var failure: String?
    @State private var loaded = false
    @State private var hovered: String?

    var body: some View {
        Page {
            Card("Waiting to be confirmed", trailing: summary) {
                if let failure {
                    Label(
                        model.status?.source == .publicApi
                            ? "The mempool is only available from a local daemon." : failure,
                        systemImage: "info.circle"
                    )
                    .foregroundStyle(.secondary)
                } else if !loaded {
                    ProgressView().frame(maxWidth: .infinity, minHeight: 200)
                } else if rows.isEmpty {
                    VStack(spacing: 6) {
                        Image(systemName: "tray").font(.largeTitle).foregroundStyle(.secondary)
                        Text("Nothing is waiting.").font(.title3)
                        Text(
                            model.syncFraction == nil
                                ? "Every transaction the node has seen is in a block."
                                : "A node that is still syncing doesn't collect waiting transactions."
                        )
                        .foregroundStyle(.secondary)
                    }
                    .frame(maxWidth: .infinity, minHeight: 200)
                } else {
                    TimelineView(.periodic(from: .now, by: 1)) { timeline in
                        GeometryReader { geometry in
                            let tiles = Self.layout(rows, in: geometry.size)
                            Canvas { context, _ in
                                draw(context, tiles, now: timeline.date.timeIntervalSince1970)
                            }
                            .contentShape(Rectangle())
                            .onContinuousHover { phase in
                                if case .active(let point) = phase {
                                    hovered = tiles.first { $0.1.contains(point) }?.0.txid
                                } else {
                                    hovered = nil
                                }
                            }
                            .onTapGesture(coordinateSpace: .local) { point in
                                if let tile = tiles.first(where: { $0.1.contains(point) }) {
                                    model.open(.transaction(txid: tile.0.txid))
                                }
                            }
                        }
                    }
                    .frame(height: 280)
                    Text(caption)
                        .font(.callout)
                        .foregroundStyle(hovered == nil ? .secondary : .primary)
                        .frame(maxWidth: .infinity, minHeight: 20, alignment: .leading)
                }
            }
        }
        .navigationTitle("Mempool")
        .task(id: model.generation) {
            while !Task.isCancelled {
                await load()
                try? await Task.sleep(for: .seconds(3))
            }
        }
    }

    private var summary: String? {
        guard loaded, failure == nil, !rows.isEmpty else { return nil }
        let bytes = rows.map(\.size).reduce(0, +)
        return "\(rows.count) transaction\(rows.count == 1 ? "" : "s") · \(Format.bytes(bytes))"
    }

    private var caption: String {
        guard let row = rows.first(where: { $0.txid == hovered }) else {
            return "Each tile is a waiting transaction, sized by its bytes and darker the longer "
                + "it has waited. Click one to open it."
        }
        var parts = [row.tags.first ?? "Transaction", Format.bytes(row.size)]
        if let time = row.time { parts.append("waiting \(Format.age(time).replacingOccurrences(of: " ago", with: ""))") }
        if let fee = row.feeSats { parts.append("fee \(Format.coins(fee)) \(model.coin)") }
        if row.depends > 0 { parts.append("depends on \(row.depends) other waiting") }
        return parts.joined(separator: " · ")
    }

    private func load() async {
        guard let explorer = model.explorer else { return }
        do {
            rows = try await explorer.mempool()
            failure = nil
        } catch {
            rows = []
            failure = describe(error)
        }
        loaded = true
    }

    /// A strip treemap: rows of tiles, each row's height proportional to the bytes in it.
    static func layout(_ rows: [MempoolRow], in size: CGSize) -> [(MempoolRow, CGRect)] {
        let total = CGFloat(rows.map(\.size).reduce(0, +))
        guard total > 0, size.width > 0 else { return [] }
        let strips = min(max(Int((Double(rows.count) / 3).rounded(.up)), 1), 5)
        let perStrip = total / CGFloat(strips)
        var groups: [[MempoolRow]] = [[]]
        var filled: CGFloat = 0
        for row in rows {
            if filled >= perStrip, groups.count < strips {
                groups.append([])
                filled = 0
            }
            groups[groups.count - 1].append(row)
            filled += CGFloat(row.size)
        }
        var tiles: [(MempoolRow, CGRect)] = []
        var y: CGFloat = 0
        for group in groups where !group.isEmpty {
            let sum = CGFloat(group.map(\.size).reduce(0, +))
            let height = sum / total * size.height
            var x: CGFloat = 0
            for row in group {
                let width = CGFloat(row.size) / sum * size.width
                tiles.append((row, CGRect(x: x, y: y, width: width, height: height)))
                x += width
            }
            y += height
        }
        return tiles
    }

    private func draw(_ context: GraphicsContext, _ tiles: [(MempoolRow, CGRect)], now: Double) {
        for (row, frame) in tiles {
            let box = frame.insetBy(dx: 1.5, dy: 1.5)
            guard box.width > 0, box.height > 0 else { continue }
            let age = row.time.map { max(now - Double($0), 0) } ?? 0
            let shape = Path(roundedRect: box, cornerRadius: 4)
            context.fill(shape, with: .color(.accentColor.opacity(min(0.25 + age / 90 * 0.7, 0.95))))
            if row.depends > 0 {
                context.stroke(shape, with: .color(.primary), style: StrokeStyle(lineWidth: 1.6, dash: [4, 3]))
            }
            if hovered == row.txid {
                context.stroke(shape, with: .color(.primary), lineWidth: 2)
            }
            if box.width > 110, box.height > 30 {
                context.draw(
                    Text(row.tags.first ?? "Transaction").font(.caption.weight(.semibold)),
                    at: CGPoint(x: box.minX + 9, y: box.minY + 9), anchor: .topLeading)
            }
        }
    }
}
