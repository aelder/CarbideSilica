import CarbideCore
import SwiftUI

/// Every currency the daemon lists, filtered by what kind it is.
///
/// Carbide's browser cycles each of thirteen option flags through Any, Show and Hide with
/// keys. Here each flag is a chip: on means the currency must have it.
struct CurrenciesView: View {
    @Environment(AppModel.self) private var model
    @State private var rows: [CurrencyRow] = []
    @State private var failure: String?
    @State private var loading = true
    @State private var search = ""
    @State private var required: Set<String> = []
    @State private var status = "Any status"

    private let statuses = ["Any status", "Launched", "Prelaunch", "Failed launch (refunding)", "Launch status unknown"]

    var body: some View {
        Page {
            Card("Currencies", trailing: loading ? nil : "\(filtered.count) of \(rows.count)") {
                if loading {
                    ProgressView("Loading currencies…").frame(maxWidth: .infinity, minHeight: 200)
                } else if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary)
                } else {
                    HStack {
                        TextField("Filter", text: $search, prompt: Text("Filter by name or ID"))
                            .textFieldStyle(.roundedBorder)
                            .frame(maxWidth: 280)
                        Picker("Status", selection: $status) {
                            ForEach(statuses, id: \.self) { Text($0).tag($0) }
                        }
                        .labelsHidden()
                        .frame(maxWidth: 230)
                        Spacer()
                        Button("Pools only") { required = ["Fractional"] }
                        Button("Clear") {
                            required = []
                            search = ""
                            status = statuses[0]
                        }
                    }
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 6) {
                            ForEach(allTags, id: \.self) { tag in
                                Toggle(tag, isOn: binding(for: tag))
                                    .toggleStyle(.button)
                                    .controlSize(.small)
                            }
                        }
                    }
                    VStack(spacing: 0) {
                        ForEach(filtered, id: \.currencyId) { row in
                            Divider()
                            CurrencyRowView(row: row)
                        }
                    }
                    if filtered.isEmpty {
                        Text("No currency matches the filters.").foregroundStyle(.secondary)
                    }
                }
            }
        }
        .navigationTitle("Currencies")
        .task(id: model.generation) { await load() }
    }

    private var allTags: [String] {
        var seen: [String] = []
        for tag in rows.flatMap(\.optionTags) where !seen.contains(tag) { seen.append(tag) }
        return seen.sorted()
    }

    private var filtered: [CurrencyRow] {
        let needle = search.trimmingCharacters(in: .whitespaces).lowercased()
        return rows.filter { row in
            (needle.isEmpty || row.name.lowercased().contains(needle) || row.currencyId.lowercased().contains(needle))
                && (status == statuses[0] || row.launchStatus == status)
                && required.isSubset(of: Set(row.optionTags))
        }
    }

    private func binding(for tag: String) -> Binding<Bool> {
        Binding(
            get: { required.contains(tag) },
            set: { on in
                if on { required.insert(tag) } else { required.remove(tag) }
            })
    }

    private func load() async {
        guard let explorer = model.explorer else { return }
        loading = true
        do {
            rows = try await explorer.currencies()
            failure = nil
        } catch {
            rows = []
            failure = describe(error)
        }
        loading = false
    }
}

private struct CurrencyRowView: View {
    let row: CurrencyRow
    @Environment(AppModel.self) private var model
    @State private var hovering = false

    var body: some View {
        Button {
            model.open(.currency(name: row.currencyId))
        } label: {
            HStack(spacing: 12) {
                Text(row.name).fontWeight(.semibold).frame(width: 170, alignment: .leading).lineLimit(1)
                HStack(spacing: 5) {
                    ForEach(row.optionTags.prefix(3), id: \.self) { TagChip(text: $0) }
                    if row.optionTags.count > 3 {
                        Text("+\(row.optionTags.count - 3)").font(.caption).foregroundStyle(.secondary)
                    }
                }
                Spacer(minLength: 8)
                if row.reserveCount > 0 {
                    Text("\(row.reserveCount) reserve\(row.reserveCount == 1 ? "" : "s")").font(.callout).foregroundStyle(.secondary)
                }
                Text(row.supply.map { Format.amount($0) } ?? "–")
                    .monospacedDigit().frame(width: 130, alignment: .trailing).lineLimit(1)
                Text(row.launchStatus)
                    .font(.callout)
                    .foregroundStyle(row.launchStatus == "Launched" ? AnyShapeStyle(.secondary) : AnyShapeStyle(.orange))
                    .frame(width: 110, alignment: .trailing)
                    .lineLimit(1)
            }
            .padding(.vertical, 7)
            .padding(.horizontal, 6)
            .background(hovering ? AnyShapeStyle(.quaternary) : AnyShapeStyle(.clear), in: RoundedRectangle(cornerRadius: 6))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
    }
}
