import CarbideCore
import SwiftUI

/// The currency's state at one exact block, found by height, Unix seconds or a date, and
/// the imports and exports around that block.
///
/// This follows Carbide's currency viewer: a bare number up to the tip is a height and a
/// larger one is Unix seconds; `height:`, `#` and `unix:` remove doubt. A future state is
/// refused. A time request reports which blocks it checked, because block times are not
/// in order and the nearest checked block is not proof of the nearest block.
struct CurrencyStateCard: View {
    let currency: CurrencyDetail
    @Binding var chosenHeight: UInt64?

    @Environment(AppModel.self) private var model
    @State private var query = ""
    @State private var loading = false
    @State private var failure: String?
    @State private var state: CurrencyStateAt?
    @State private var showJSON = false
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    private var currencyKey: String { currency.currencyId ?? currency.name }

    var body: some View {
        let _ = (showsNames, showsDates)
        Card("State at a chosen block") {
            Text("Enter a block height, Unix seconds or a date and time in UTC. A date without a time means 00:00 UTC. Use # or height: for a height, and unix: for a time.")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                TextField(
                    "Block or time", text: $query,
                    prompt: Text("3962166, #3962166, 2024-06-01 12:00 or unix:1717243200")
                )
                .textFieldStyle(.roundedBorder)
                .frame(maxWidth: 420)
                .onSubmit { Task { await seek(query) } }
                Button("Show State") { Task { await seek(query) } }
                    .disabled(loading || query.trimmingCharacters(in: .whitespaces).isEmpty)
                Button("Latest Block") {
                    if let tip = model.status?.blocks {
                        query = "#\(tip)"
                        Task { await seek(query) }
                    }
                }
                .disabled(loading || model.status == nil)
                if loading { ProgressView().controlSize(.small) }
            }
            if let failure {
                Note(text: failure, warning: true)
            }
            if let state {
                stateDetail(state)
                CurrencyActivitySection(state: state, currencyName: currency.name)
                    .id("\(state.currencyId)@\(state.height)")
            } else if !loading && failure == nil {
                Text("No state is selected. Enter a block or time, or use Show Exact State on the slider above.")
                    .foregroundStyle(.secondary)
            }
        }
        .onChange(of: chosenHeight) {
            guard let height = chosenHeight else { return }
            chosenHeight = nil
            query = "#\(height)"
            Task { await seek(query) }
        }
    }

    @ViewBuilder
    private func stateDetail(_ state: CurrencyStateAt) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            ProvenanceNote(provenance: state.provenance)
            FieldRow(label: "Request", value: state.interpreted)
            FieldRow(
                label: "Block", value: heightText(state.height, known: state.blockTime, model: model),
                route: .block(id: state.blockHash))
            FieldRow(label: "Block hash", value: state.blockHash, route: .block(id: state.blockHash), monospaced: true)
            if let time = state.blockTime {
                FieldRow(label: "Block time", value: Format.date(time))
            }
            if let coverage = state.coverage {
                FieldRow(label: "Requested time", value: Format.date(UInt64(max(coverage.targetTime, 0))))
                FieldRow(label: "Checked blocks", value: checkedText(coverage))
                Note(text: coverageSentence(coverage), warning: !coverage.exact && !coverage.wholeChain)
            }
            FieldRow(label: "Source", value: "\(state.dataSource.sourceWord) · \(observationSource(state.source))")
            if let tip = state.tipHeight {
                FieldRow(label: "Tip when read", value: Format.integer(tip))
            }
            FieldRow(label: "Launch status", value: state.launchLabel)
            if let supply = state.supply {
                FieldRow(label: "Supply", value: "\(Format.amount(supply)) \(currency.name)")
            }
            if !state.reserves.isEmpty {
                reserveTable(state)
                PoolBatchView(currencyName: currency.name, point: point(state), name: reserveName)
            }
            DisclosureGroup(isExpanded: $showJSON) {
                ScrollView(.horizontal) {
                    Text(state.json)
                        .font(.callout.monospaced())
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            } label: {
                HStack {
                    Text(showJSON ? "Hide the state" : "Show the state from the node, with its source and block")
                        .foregroundStyle(.secondary)
                    Spacer()
                    CopyButton(text: "source: \(state.source)\nblock: \(state.height) \(state.blockHash)\n\(state.json)")
                }
            }
        }
        .task(id: state.height) {
            await model.resolveNames(state.reserves.map(\.currencyId))
        }
    }

    private func reserveTable(_ state: CurrencyStateAt) -> some View {
        Grid(alignment: .leading, horizontalSpacing: 18, verticalSpacing: 6) {
            GridRow {
                Text("Reserve")
                Text("In reserve").gridColumnAlignment(.trailing)
                Text("Price").gridColumnAlignment(.trailing)
                Text("Last batch in").gridColumnAlignment(.trailing)
                Text("Last batch out").gridColumnAlignment(.trailing)
            }
            .font(.caption.weight(.medium))
            .foregroundStyle(.secondary)
            Divider()
            ForEach(state.reserves, id: \.currencyId) { reserve in
                GridRow {
                    RouteLink(text: reserve.currencyId, route: .currency(name: reserve.currencyId))
                    Text(reserve.reserves.map(Format.amount) ?? "–")
                    Text(reserve.priceInReserve.map(Format.amount) ?? "–")
                    Text(Format.amount(reserve.reserveIn))
                    Text(Format.amount(reserve.reserveOut))
                }
                .monospacedDigit()
            }
        }
    }

    private func reserveName(_ id: String) -> String {
        knownName(id, model: model, fallback: currency.reserves.first { $0.currencyId == id }?.name)
    }

    /// The state as a history point, so the batch picture can draw it.
    private func point(_ state: CurrencyStateAt) -> CurrencyPoint {
        CurrencyPoint(
            height: state.height, time: state.blockTime, supply: state.supply,
            reserves: state.reserves.map {
                ReservePoint(
                    currencyId: $0.currencyId, reserves: $0.reserves, priceInReserve: $0.priceInReserve,
                    lastIn: $0.reserveIn, lastOut: $0.reserveOut, lastFees: $0.conversionFees)
            })
    }

    private func checkedText(_ coverage: TimeCoverage) -> String {
        let window = "\(Format.integer(coverage.checkedFirst)) to \(Format.integer(coverage.checkedLast))"
        return "\(window), \(coverage.probes) blocks checked"
    }

    private func coverageSentence(_ coverage: TimeCoverage) -> String {
        let seconds = coverage.differenceSeconds
        let direction = seconds == 0 ? "" : seconds > 0 ? " after the requested time" : " before the requested time"
        let gap = seconds == 0 ? "The block time matches the requested time exactly." : "The block time is \(abs(seconds)) seconds\(direction)."
        if coverage.exact {
            return gap
        }
        if coverage.wholeChain {
            return "\(gap) This is the nearest block."
        }
        return "\(gap) This is the nearest of the checked blocks. Block times are not in order, so a block outside the checked range can possibly be nearer."
    }

    private func seek(_ text: String) async {
        let text = text.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty, !loading else { return }
        loading = true
        failure = nil
        defer { loading = false }
        do {
            let found = try await model.requireExplorer().currencyStateAt(currency: currencyKey, query: text)
            state = found
        } catch is CancellationError {
        } catch {
            // The earlier state stays, with its source and block.
            failure = describe(error)
        }
    }
}

/// Imports and exports at the chosen state, in windows of up to 1,000 blocks that end at
/// or before the state's block.
///
/// Carbide's batches are evidence about a whole export. A single transfer is only found
/// from the export's declared input range, so each record can open that range.
struct CurrencyActivitySection: View {
    let state: CurrencyStateAt
    let currencyName: String

    @Environment(AppModel.self) private var model
    @State private var page: ActivityPage?
    /// Windows loaded before, kept with their source and block.
    @State private var retained: [String: ActivityPage] = [:]
    @State private var fromRetained = false
    @State private var loading = false
    @State private var failure: String?
    @State private var selected: ActivityRecord?
    @State private var inputs: ExportInputs?
    @State private var inputsFailure: String?
    @State private var inputsLoading = false
    @AppStorage(DisplayKey.showsNames) private var showsNames = true
    @AppStorage(DisplayKey.showsDates) private var showsDates = false

    var body: some View {
        let _ = showsNames
        VStack(alignment: .leading, spacing: 10) {
            Divider().padding(.top, 6)
            Text("Imports and exports").font(.headline)
            Text("The app asks the node with getimports and getexports. A public API can refuse these requests.")
                .font(.caption).foregroundStyle(.secondary)
            HStack {
                Button("Check This Block") {
                    load(BlockWindow(first: state.height, last: state.height))
                }
                Button("Load Preceding 1,000 Blocks") {
                    load(activityWindowEndingAt(height: state.height))
                }
                Spacer()
                Button {
                    if let older = page?.older { load(older) }
                } label: {
                    Label("Older", systemImage: "chevron.left")
                }
                .disabled(page?.older == nil || loading)
                Button {
                    if let newer = page?.newer { load(newer) }
                } label: {
                    Label("Newer", systemImage: "chevron.right")
                }
                .disabled(page?.newer == nil || loading)
                .help("Newer windows stop at the state's block, \(Format.integer(state.height)).")
                if loading { ProgressView().controlSize(.small) }
            }
            .disabled(loading)
            if let failure {
                Note(text: failure, warning: true)
            }
            if let page {
                pageView(page)
            }
        }
    }

    @ViewBuilder
    private func pageView(_ page: ActivityPage) -> some View {
        let imports = page.records.filter { $0.kind == .import }.count
        let exports = page.records.count - imports
        Text(
            "Blocks \(Format.integer(page.window.first)) to \(Format.integer(page.window.last)): \(imports) import\(imports == 1 ? "" : "s") and \(exports) export\(exports == 1 ? "" : "s"). Source \(observationSource(page.source)), anchored at block \(Format.integer(page.anchorHeight))."
                + (fromRetained ? " This window is from an earlier request in this session." : "")
        )
        .font(.callout)
        if page.truncated {
            Note(text: "The window holds more records than Carbide keeps. The app shows the newest 80.", warning: true)
        }
        if page.records.isEmpty {
            Text("No imports or exports were found in this window. This does not prove that none exist outside it.")
                .foregroundStyle(.secondary)
        } else {
            ActivityStrip(page: page, selected: selected) { record in
                select(record)
            }
            .frame(height: 64)
            Text("Each mark is an import (above the line, filled) or an export (below the line, open) at its block. Click a mark to select it.")
                .font(.caption).foregroundStyle(.secondary)
            VStack(spacing: 0) {
                ForEach(Array(page.records.enumerated()), id: \.offset) { _, record in
                    Divider()
                    recordRow(record)
                    if record == selected {
                        inputsView(record)
                    }
                }
            }
        }
    }

    private func recordRow(_ record: ActivityRecord) -> some View {
        HStack(spacing: 12) {
            Label(
                record.kind == .import ? "Import" : "Export",
                systemImage: record.kind == .import ? "arrow.down.circle.fill" : "arrow.up.circle"
            )
            .foregroundStyle(record.kind == .import ? Color.accentColor : .purple)
            .frame(width: 90, alignment: .leading)
            RouteLink(text: heightText(record.height, model: model), route: .block(id: "\(record.height)"), shorten: false)
                .frame(width: showsDates ? 190 : 90, alignment: .leading)
            RouteLink(text: record.output.txid, route: .transaction(txid: record.output.txid))
            Text("output \(record.output.output)").foregroundStyle(.secondary)
            Spacer()
            if let export = record.export, record.kind == .import {
                Text("from \(knownName(export.systemId, model: model))")
                    .foregroundStyle(.secondary)
                    .help("Source export \(export.txid):\(export.output) on \(export.systemId)")
            }
            Button(record == selected ? "Hide Inputs" : "Inspect Source Export") {
                if record == selected {
                    selected = nil
                } else {
                    select(record)
                }
            }
            .disabled(record.export == nil)
        }
        .padding(.vertical, 6)
    }

    @ViewBuilder
    private func inputsView(_ record: ActivityRecord) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            if let export = record.export {
                HStack {
                    Text("Source export on \(knownName(export.systemId, model: model))")
                        .foregroundStyle(.secondary)
                    if export.systemId == model.status?.chainId {
                        RouteLink(text: export.txid, route: .transaction(txid: export.txid))
                    } else {
                        Text(Format.short(export.txid)).font(.callout.monospaced()).textSelection(.enabled)
                            .help(export.txid)
                    }
                    Text("output \(export.output)").foregroundStyle(.secondary)
                    if export.systemId != model.status?.chainId {
                        OtherChainNote(systemId: export.systemId)
                    }
                }
            }
            if inputsLoading {
                ProgressView().controlSize(.small)
            } else if let inputsFailure {
                Note(text: inputsFailure, warning: true)
            } else if let inputs {
                Text("The export spent \(inputs.inputs.count) output\(inputs.inputs.count == 1 ? "" : "s") in its declared input range. Each is a transfer that went into the batch.")
                    .font(.callout)
                if inputs.truncated {
                    Note(text: "The range holds more inputs than Carbide lists. The app shows the first 64.", warning: true)
                }
                ForEach(Array(inputs.inputs.enumerated()), id: \.offset) { index, input in
                    HStack(spacing: 10) {
                        Text("Input \(index + 1)").foregroundStyle(.secondary).frame(width: 70, alignment: .leading)
                        if input.systemId == model.status?.chainId {
                            RouteLink(text: input.txid, route: .transaction(txid: input.txid))
                        } else {
                            Text(Format.short(input.txid)).font(.callout.monospaced()).textSelection(.enabled)
                        }
                        Text("output \(input.output)").foregroundStyle(.secondary)
                        Spacer()
                        if input.systemId == model.status?.chainId {
                            Button("Follow Transfer") {
                                model.openTrace(txid: input.txid, output: input.output)
                            }
                            .help("Opens the transaction and follows the transfer from output \(input.output).")
                        } else {
                            OtherChainNote(systemId: input.systemId)
                        }
                    }
                }
                Text("Follow Transfer opens the transaction and starts the transfer trace at that output.")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(.leading, 102)
        .padding(.bottom, 8)
    }

    private func select(_ record: ActivityRecord) {
        selected = record
        inputs = nil
        inputsFailure = nil
        guard record.export != nil else { return }
        inputsLoading = true
        Task {
            do {
                let found = try await model.requireExplorer().exportInputs(record: record)
                if selected == record { inputs = found }
            } catch {
                if selected == record { inputsFailure = describe(error) }
            }
            if selected == record { inputsLoading = false }
        }
    }

    private func load(_ window: BlockWindow) {
        let key = "\(window.first)-\(window.last)"
        selected = nil
        if let kept = retained[key] {
            page = kept
            fromRetained = true
            failure = nil
            return
        }
        loading = true
        failure = nil
        Task {
            defer { loading = false }
            do {
                let found = try await model.requireExplorer().currencyActivity(
                    currency: state.currencyId, first: window.first, last: window.last,
                    anchorHeight: state.height, anchorHash: state.blockHash)
                retained[key] = found
                page = found
                fromRetained = false
                await model.resolveNames(found.records.compactMap { $0.export?.systemId })
            } catch {
                failure = describe(error)
            }
        }
    }
}

/// The window as a line of blocks, with each import above it and each export below it.
private struct ActivityStrip: View {
    let page: ActivityPage
    let selected: ActivityRecord?
    let choose: (ActivityRecord) -> Void

    var body: some View {
        GeometryReader { geometry in
            Canvas { context, size in
                draw(context, size)
            }
            .contentShape(Rectangle())
            .onTapGesture(coordinateSpace: .local) { point in
                if let record = nearest(point, size: geometry.size) { choose(record) }
            }
        }
        .accessibilityLabel("\(page.records.count) imports and exports between blocks \(page.window.first) and \(page.window.last)")
    }

    private func x(_ height: UInt64, _ size: CGSize) -> CGFloat {
        let span = max(Double(page.window.last - page.window.first), 1)
        return 8 + CGFloat(Double(height - page.window.first) / span) * (size.width - 16)
    }

    private func nearest(_ point: CGPoint, size: CGSize) -> ActivityRecord? {
        let middle = size.height / 2 - 6
        var best: ActivityRecord?
        var distance: CGFloat = 14
        for record in page.records {
            let y = record.kind == .import ? middle - 12 : middle + 12
            let gap = hypot(x(record.height, size) - point.x, y - point.y)
            if gap < distance {
                distance = gap
                best = record
            }
        }
        return best
    }

    private func draw(_ context: GraphicsContext, _ size: CGSize) {
        let middle = size.height / 2 - 6
        context.stroke(
            Path { path in
                path.move(to: CGPoint(x: 8, y: middle))
                path.addLine(to: CGPoint(x: size.width - 8, y: middle))
            }, with: .color(.secondary.opacity(0.5)), lineWidth: 1)
        for record in page.records {
            let px = x(record.height, size)
            let isImport = record.kind == .import
            let y = isImport ? middle - 12 : middle + 12
            let color: Color = isImport ? .accentColor : .purple
            context.stroke(
                Path { path in
                    path.move(to: CGPoint(x: px, y: middle))
                    path.addLine(to: CGPoint(x: px, y: y))
                }, with: .color(color.opacity(0.5)), lineWidth: 1)
            let box = CGRect(x: px - 4, y: y - 4, width: 8, height: 8)
            if isImport {
                context.fill(Path(ellipseIn: box), with: .color(color))
            } else {
                context.stroke(Path(ellipseIn: box), with: .color(color), lineWidth: 1.6)
            }
            if record == selected {
                context.stroke(Path(ellipseIn: box.insetBy(dx: -3, dy: -3)), with: .color(.primary), lineWidth: 1.5)
            }
        }
        context.draw(
            Text(Format.integer(page.window.first)).font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: 8, y: size.height - 4), anchor: .bottomLeading)
        context.draw(
            Text(Format.integer(page.window.last)).font(.caption2).foregroundStyle(.secondary),
            at: CGPoint(x: size.width - 8, y: size.height - 4), anchor: .bottomTrailing)
    }
}
