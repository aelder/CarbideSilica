import CarbideCore
import SwiftUI

/// Charts of block intervals, mining and staking difficulty, the share of staked blocks and
/// the fee pool, over a range of header time that can be paged and sought. Below them, the
/// estimator for future blocks.
///
/// Other screens open the charts at a block with `ChartsView(anchorHeight:)`. Without an
/// anchor the charts open at the chain tip. A block on a side branch also needs its hash,
/// `ChartsView(anchorHeight:anchorHash:)`: the chart then follows parent hashes from it.
struct ChartsView: View {
    var anchorHeight: UInt64?
    var anchorHash: String?

    @Environment(AppModel.self) private var model
    @State private var state = ChartState()
    @FocusState private var plotFocused: Bool

    init(anchorHeight: UInt64? = nil, anchorHash: String? = nil) {
        self.anchorHeight = anchorHeight
        self.anchorHash = anchorHash
    }

    var body: some View {
        Page(keys: false) {
            Card("Charts", trailing: state.rangeLabel) {
                controls
                if state.viaAPI {
                    Note(text: "A public API supplies blocks one at a time and slowly. The charts open with a 2-hour span. A longer span can take many minutes. A local node loads 7 days in seconds.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                coverage
                if let provenance = state.page?.provenance, provenance.retained {
                    ProvenanceNote(provenance: provenance)
                }
                plotArea
                notes
            }
            SeekCard(state: state)
            SelectedBlockCard(state: state)
            FeePoolSection(state: state)
            SampleTable(state: state)
            BlockEstimator()
        }
        .navigationTitle("Charts")
        .onAppear {
            state.prepare(anchor: anchorHeight, hash: anchorHash, viaAPI: model.status?.source == .publicApi)
            // The keys work while the plot has focus, so it takes focus when the page opens.
            plotFocused = true
        }
        // The plot exists only once a range has loaded. Give it focus then, once.
        .onChange(of: state.page != nil) { _, shown in
            if shown { plotFocused = true }
        }
        .publishPlace(PlaceActions(refresh: { state.refresh() }))
        .onChange(of: model.generation) {
            state.reconnected(viaAPI: model.status?.source == .publicApi)
        }
        // A new request id cancels the previous load. Leaving the view cancels it too.
        .task(id: "\(state.requestID)|\(model.generation)") {
            guard let explorer = try? model.requireExplorer() else { return }
            await state.run(explorer)
        }
        .task(id: "\(state.feePoolKey)|\(model.generation)") {
            guard let explorer = try? model.requireExplorer() else { return }
            await state.loadFeePool(explorer)
        }
    }

    // MARK: Controls

    private var controls: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                NameText("Span")
                LampSegments(
                    options: ChartState.Span.allCases.map { ($0, $0.title) },
                    selection: Binding(get: { state.span }, set: { state.setSpan($0) }))
                Spacer(minLength: 16)
            }
            HStack(spacing: 8) {
                Button {
                    state.older()
                } label: {
                    Label("Older range", systemImage: "chevron.left")
                }
                .disabled(!state.canPageOlder)
                .help("Load the range before this one ( [ or Page Up )")
                Button {
                    state.newer()
                } label: {
                    Label("Newer range", systemImage: "chevron.right")
                }
                .disabled(!state.canPageNewer)
                .help("Load the range after this one ( ] or Page Down )")
                // The tip moves on after a range loads, so this stays available.
                Button("Latest") { state.latest() }
                    .help("Load the range that ends at the current chain tip")
                Spacer()
                if state.loading {
                    ProgressView().controlSize(.small)
                    Text(state.loadingText)
                        .font(.caption).foregroundStyle(.secondary).monospacedDigit()
                    Button("Stop") { state.stop() }
                } else if state.fee.loading {
                    ProgressView().controlSize(.small)
                    Text("Reading fee pool checkpoints")
                        .font(.caption).foregroundStyle(.secondary)
                }
                Button {
                    state.refresh()
                } label: {
                    Label("Refresh", systemImage: "arrow.clockwise")
                }
                .help("Load this range again and check that its newest block is on the active chain (⌘R)")
            }
            .buttonStyle(.lamp)
            .controlSize(.small)
        }
    }

    // MARK: Coverage

    @ViewBuilder
    private var coverage: some View {
        if let page = state.page, let first = page.samples.first, let last = page.samples.last {
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 24) {
                    endpoint("Start", first)
                    endpoint("End", last)
                    VStack(alignment: .leading, spacing: 3) {
                        NameText("Covers")
                        HStack(alignment: .lastTextBaseline, spacing: 5) {
                            Text(ChartFormat.duration(Int64(last.time) - Int64(first.time)))
                                .font(Tone.figureSmall).foregroundStyle(Tone.value)
                            Text("of header time").font(Tone.caption).foregroundStyle(Tone.label)
                        }
                        .monospacedDigit()
                    }
                }
                Text(state.coverageShort)
                    .font(.caption).foregroundStyle(Tone.label)
                    .help(state.coverageSentence)
            }
        }
    }

    private func endpoint(_ label: String, _ sample: ChartSample) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            NameText(label)
            HStack(alignment: .lastTextBaseline, spacing: 6) {
                Button("#\(Format.integer(sample.height))") { model.open(.block(id: sample.hash)) }
                    .buttonStyle(.displayLink)
                    .font(Tone.figureSmall)
                    .help(sample.hash)
                Text(Format.date(sample.time)).font(Tone.caption).foregroundStyle(Tone.value).monospacedDigit()
            }
        }
    }

    // MARK: Plot

    @ViewBuilder
    private var plotArea: some View {
        if let failure = state.failure {
            Note(text: failure, warning: true)
        }
        if let stopped = state.stopped {
            Note(text: stopped, warning: true)
        }
        if let page = state.page, !page.samples.isEmpty {
            ChartStack(
                samples: page.samples,
                spanSeconds: state.span.seconds,
                fee: state.fee,
                coin: state.fee.current?.currencies.nativeLabel ?? model.coin,
                selected: state.selectedSample?.hash,
                select: { state.select(hash: $0) },
                open: { model.open(.block(id: $0)) })
            .focusable()
            .focusEffectDisabled()
            .focused($plotFocused)
            .onKeyPress { press in state.handle(press, open: { model.open(.block(id: $0)) }) }
            .simultaneousGesture(TapGesture().onEnded { plotFocused = true })
            Text(
                plotFocused
                    ? "Keys: ← → or j k select a block, Home End ends, [ ] ranges, − + span, g go to, Return open"
                    : "Click the panels to use the keys."
            )
            .font(.caption2).foregroundStyle(Tone.label)
        } else if state.loading {
            ProgressView("Loading block headers…").frame(maxWidth: .infinity, minHeight: 300)
        } else if state.failure == nil {
            Text("No blocks in this range.")
                .foregroundStyle(.secondary)
                .frame(maxWidth: .infinity, minHeight: 300)
        }
    }

    /// What the marks mean, and Carbide's statements about each measure.
    @ViewBuilder
    private var notes: some View {
        if let page = state.page {
            DisclosureGroup("What the marks mean") {
                VStack(alignment: .leading, spacing: 4) {
                    Text("All panels share one time axis. One line shows the selected block in each panel. A grey band is one of the longest gaps of the range: the longest hundredth, and 12 at most.")
                    Text("Block rate: each point is one period. The band is the mean with limits of 2.66 times the mean change between periods. The dashed line is the target. A red point is outside the limits.")
                    Text("Difficulty: each dot is one block of that kind, or a bar from the lowest to the highest value where the dots would merge. The line is MA100, the mean over the trailing 100 chain heights, so it turns after the dots turn. A gap means that the trailing 100 heights are incomplete or hold a block of the kind without a value. The axes are fitted to the values and do not start at zero.")
                    Text("Staking difficulty = fixed initial Verus LWMA posLimit / observed block PoS target. The target comes from the nonce and must equal postarget.")
                    Text("Staked share: the violet line is the share over 100 blocks and the blue line is the share over 1,000 blocks. The band is three standard deviations of chance for 100 blocks. A share line needs a complete window of known blocks.")
                    Text("Fee pool: a tick at the top is a block where the pool rose. This is a balance chart. It does not separate inflow from payout.")
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.top, 4)
            }
            VStack(alignment: .leading, spacing: 4) {
                if page.negativeIntervals > 0 {
                    Text(
                        page.negativeIntervals == 1
                            ? "1 block in this range has a timestamp before its parent's. Select it to see its signed interval."
                            : "\(page.negativeIntervals) blocks in this range have a timestamp before their parent's. Select a block to see its signed interval.")
                }
                if page.unavailableStake > 0 {
                    Text("\(page.unavailableStake) staked blocks have no staking difficulty. Select one to see why.")
                }
                if let note = state.seekNote {
                    Text(note)
                }
            }
            .font(.caption)
            .foregroundStyle(.secondary)
        }
    }
}

// MARK: - State

/// The chart screen's state and its loading. Loading runs in steps so that a new request,
/// or leaving the view, stops it after the current step.
@MainActor
@Observable
final class ChartState {
    enum Span: CaseIterable, Identifiable {
        case twoHours, sixHours, day, week
        var id: Self { self }
        var seconds: UInt64 {
            switch self {
            case .twoHours: 7_200
            case .sixHours: 21_600
            case .day: 86_400
            case .week: 604_800
            }
        }
        var title: String {
            switch self {
            case .twoHours: "2 hours"
            case .sixHours: "6 hours"
            case .day: "1 day"
            case .week: "7 days"
            }
        }
    }

    enum Metric: Int, CaseIterable, Identifiable {
        case intervals = 1, mining, staking, ratio, feePool
        var id: Int { rawValue }
        var title: String {
            switch self {
            case .intervals: "Block intervals"
            case .mining: "Mining difficulty"
            case .staking: "Staking difficulty"
            case .ratio: "PoW ratio"
            case .feePool: "Fee pool balances"
            }
        }
        var shortTitle: String {
            switch self {
            case .intervals: "Intervals"
            case .mining: "Mining"
            case .staking: "Staking"
            case .ratio: "PoW ratio"
            case .feePool: "Fee pool"
            }
        }
        /// Trailing heights each sample needs. Carbide uses the same values.
        var context: UInt32 {
            switch self {
            case .intervals, .mining, .staking: 100
            case .ratio: 1_000
            case .feePool: 2
            }
        }
    }

    var span = Span.day
    var metric = Metric.intervals
    var page: ChartPage?
    var loading = false
    var headersRead: UInt32 = 0
    var failure: String?
    var stopped: String?
    var selected: String?
    var seekNote: String?
    var viaAPI = false
    var fee = FeePoolState()

    private(set) var requestID = 0
    private var request: ChartRequest?
    private var loadedContext: UInt32 = 0
    private var prepared = false

    // MARK: Requests

    func prepare(anchor: UInt64?, hash: String? = nil, viaAPI: Bool) {
        guard !prepared else { return }
        prepared = true
        self.viaAPI = viaAPI
        if viaAPI { span = .twoHours }
        // A local node can read each block's checkpoint. A public API gets the 100 samples.
        fee.detail = !viaAPI
        begin(end: anchor, anchor: hash)
    }

    private func begin(
        end: UInt64?, anchor: String?, newer: Bool = false, focus: ChartBlock? = nil, context: UInt32? = nil
    ) {
        request = ChartRequest(
            spanSeconds: span.seconds,
            endHeight: end,
            anchorHash: anchor,
            newer: newer,
            // The 1,000-block share needs 1,000 earlier blocks. A public API gets the short context.
            context: context ?? (viaAPI ? 100 : 1_000),
            focusHeight: focus?.height,
            focusHash: focus?.hash,
            chunk: viaAPI ? 25 : 500)
        failure = nil
        stopped = nil
        loading = true
        headersRead = 0
        requestID += 1
    }

    func run(_ explorer: Explorer) async {
        guard let request, loading else { return }
        let id = requestID
        let load = explorer.chartLoad(request: request)
        var published = Date.distantPast
        do {
            while true {
                try Task.checkCancellation()
                let progress = try await load.step()
                guard id == requestID, !Task.isCancelled else { return }
                headersRead = progress.headers
                if progress.done || Date.now.timeIntervalSince(published) > 0.4 {
                    if let next = await load.page(), id == requestID {
                        page = next
                        if progress.done { loadedContext = request.context }
                    }
                    published = .now
                }
                if progress.done {
                    stopped = progress.stopped
                    break
                }
            }
        } catch is CancellationError {
            return
        } catch {
            guard id == requestID else { return }
            failure = describe(error)
        }
        if id == requestID { loading = false }
    }

    /// A new connection can be another chain or source. Load the latest range again.
    func reconnected(viaAPI: Bool) {
        self.viaAPI = viaAPI
        fee.reset(keepCheckpoints: false)
        page = nil
        latest()
    }

    func stop() {
        requestID += 1
        loading = false
        stopped = "Loading stopped. The chart shows what loaded before."
    }

    func setSpan(_ value: Span) {
        guard value != span else { return }
        span = value
        seekNote = nil
        begin(end: page?.last, anchor: page?.samples.last?.hash)
    }

    func setMetric(_ value: Metric) {
        guard value != metric else { return }
        metric = value
        // A longer context extends the same range. The range and the selection stay.
        if value.context > loadedContext, let page {
            begin(end: page.last, anchor: page.samples.last?.hash)
        }
        if let sample = selectedSample, self.value(sample) == nil { selected = nil }
    }

    var canPageOlder: Bool { !loading && (page?.first ?? 0) > 0 }
    var canPageNewer: Bool { !loading && page.map { $0.last < $0.tip } ?? false }

    func older() {
        guard canPageOlder, let page else { return }
        seekNote = nil
        begin(end: page.first - 1, anchor: nil)
    }

    func newer() {
        guard canPageNewer, let page else { return }
        seekNote = nil
        begin(end: page.last, anchor: page.samples.last?.hash, newer: true)
    }

    func refresh() {
        begin(end: page?.last, anchor: page?.samples.last?.hash)
        fee.reset(keepCheckpoints: false)
    }

    func latest() {
        seekNote = nil
        selected = nil
        begin(end: nil, anchor: nil)
    }

    /// Loads a range centred on a sought block, and selects that block.
    func show(_ outcome: SeekOutcome) {
        selected = outcome.block.hash
        seekNote = outcome.note
        begin(end: outcome.end.height, anchor: outcome.end.hash, focus: outcome.block)
    }

    // MARK: Selection

    /// Each block can be selected: the panels show all measures at once.
    func value(_ sample: ChartSample) -> Double? { 1 }

    private var candidates: [ChartSample] {
        (page?.samples ?? []).filter { value($0) != nil }
    }

    var selectedSample: ChartSample? {
        guard let samples = page?.samples else { return nil }
        if let selected {
            if let found = samples.first(where: { $0.hash == selected }) { return found }
            if loading { return nil }
        }
        return samples.last { value($0) != nil }
    }

    func select(hash: String) {
        selected = hash
        seekNote = nil
    }

    /// Moves the selection to an older (negative) or newer (positive) sample.
    func move(_ delta: Int) {
        let list = candidates
        guard !list.isEmpty else { return }
        let index = selectedSample.flatMap { current in list.firstIndex { $0.hash == current.hash } } ?? list.count - 1
        selected = list[min(max(index + delta, 0), list.count - 1)].hash
        seekNote = nil
    }

    func moveToEnd(newest: Bool) {
        let list = candidates
        selected = (newest ? list.last : list.first)?.hash
        seekNote = nil
    }

    // MARK: Keys

    func handle(_ press: KeyPress, open: (String) -> Void) -> KeyPress.Result {
        switch press.key {
        case .downArrow, .leftArrow: move(-1); return .handled
        case .upArrow, .rightArrow: move(1); return .handled
        case .home: moveToEnd(newest: false); return .handled
        case .end: moveToEnd(newest: true); return .handled
        case .pageUp: older(); return .handled
        case .pageDown: newer(); return .handled
        case .return:
            if let hash = selectedSample?.hash { open(hash) }
            return .handled
        default: break
        }
        switch press.characters {
        case "j": move(-1)
        case "k": move(1)
        case "[": older()
        case "]": newer()
        case "-": step(span: -1)
        case "+", "=": step(span: 1)
        case "g": focusSeek += 1
        case "v": fee.cycleCurrency()
        case "z": fee.cycleScale()
        case "t": fee.detail.toggle()
        default: return .ignored
        }
        return .handled
    }

    /// Incremented to move focus to the seek field.
    var focusSeek = 0

    private func step(span delta: Int) {
        let all = Span.allCases
        guard let index = all.firstIndex(of: span) else { return }
        setSpan(all[min(max(index + delta, 0), all.count - 1)])
    }

    // MARK: Text

    var rangeLabel: String? {
        guard let page else { return nil }
        return "blocks \(Format.integer(page.first)) to \(Format.integer(page.last))"
    }

    /// What the current load reads, in words.
    var loadingText: String {
        if headersRead > 0 { return "Read \(Format.integer(UInt64(headersRead))) headers" }
        if let page, let request, request.context > loadedContext, request.endHeight == page.last {
            return "Reading \(Format.integer(UInt64(request.context - 1))) earlier blocks"
        }
        return "Reading the range's block headers"
    }

    /// What a reader needs: how many blocks, and anything that limits the range.
    var coverageShort: String {
        guard let page else { return "" }
        var parts = ["\(Format.integer(UInt64(page.samples.count))) blocks"]
        if page.limited {
            parts.append("partial: the 20,000-header limit was reached")
        } else if !page.complete && loading {
            parts.append("loading")
        } else if page.first == 0 && !page.complete {
            parts.append("starts at genesis")
        }
        if !page.status.isEmpty, page.status != "Loading" { parts.append(page.status) }
        return parts.joined(separator: " · ") + "."
    }

    var coverageSentence: String {
        guard let page else { return "" }
        var parts = [
            "\(Format.integer(UInt64(page.samples.count))) blocks shown",
            "\(Format.integer(UInt64(page.headers))) headers read, with \(Format.integer(UInt64(page.context))) earlier blocks for averages and intervals",
            "span \(span.title)",
        ]
        if page.limited {
            parts.append("partial: the 20,000-header limit was reached")
        } else if !page.complete && loading {
            parts.append("loading")
        } else if page.first == 0 && !page.complete {
            parts.append("starts at genesis")
        }
        if page.stored > 0 {
            parts.append("\(Format.integer(UInt64(page.stored))) headers from Carbide's cache")
        }
        if !page.status.isEmpty, page.status != "Loading" { parts.append(page.status) }
        return parts.joined(separator: " · ") + "."
    }

    // MARK: Fee pool

    /// Changes when the fee pool checkpoints must load again.
    var feePoolKey: String {
        guard let page, !loading else { return "idle" }
        return "\(page.first)|\(page.last)|\(fee.detail)|\(fee.generation)"
    }

    func loadFeePool(_ explorer: Explorer) async {
        guard let page, !page.samples.isEmpty else { return }
        let samples = page.samples
        fee.loading = true
        fee.error = nil
        defer { fee.loading = false }
        if fee.current == nil {
            do {
                fee.current = try await explorer.feePoolCurrent()
            } catch is CancellationError {
                return
            } catch {
                fee.currentError = describe(error)
            }
        }
        let selectedIndex = selected.flatMap { hash in samples.firstIndex { $0.hash == hash } }
        let order = feePoolOrder(
            count: UInt32(samples.count), selected: selectedIndex.map(UInt32.init), detail: fee.detail
        ).map(Int.init)
        fee.scheduled = Set(order.map { samples[$0].hash })
        fee.requested = order.count
        let missing = order.filter { fee.checkpoints[samples[$0].hash] == nil }
        // The endpoints and the selected block first, one at a time, then larger groups.
        var start = 0
        while start < missing.count {
            if Task.isCancelled { return }
            let size = start < 4 ? (start < 2 ? 2 : 1) : (viaAPI ? 4 : 16)
            let batch = missing[start..<min(start + size, missing.count)]
            start += size
            let blocks = batch.map { ChartBlock(height: samples[$0].height, hash: samples[$0].hash, time: samples[$0].time) }
            do {
                let found = try await explorer.feePoolCheckpoints(blocks: blocks)
                if Task.isCancelled { return }
                for checkpoint in found { fee.checkpoints[checkpoint.hash] = checkpoint }
            } catch is CancellationError {
                return
            } catch {
                fee.error = describe(error)
                return
            }
        }
    }

    /// Reads the checkpoint at one sought block, so it is part of the sample set.
    func includeCheckpoint(_ sample: ChartSample, explorer: Explorer) async {
        guard fee.checkpoints[sample.hash] == nil else { return }
        let block = ChartBlock(height: sample.height, hash: sample.hash, time: sample.time)
        if let found = try? await explorer.feePoolCheckpoints(blocks: [block]).first {
            fee.checkpoints[found.hash] = found
            fee.scheduled.insert(found.hash)
        }
    }
}

/// The fee pool's checkpoints and display options.
struct FeePoolState {
    enum Currency: CaseIterable { case both, native, reserve }
    enum Scale: CaseIterable { case zero, fit, log }

    var current: FeePoolCurrent?
    var currentError: String?
    var checkpoints: [String: FeePoolCheckpoint] = [:]
    var scheduled: Set<String> = []
    var requested = 0
    var loading = false
    var error: String?
    var currency = Currency.both
    var scale = Scale.zero
    var detail = false
    /// Changed to load the checkpoints again.
    var generation = 0

    var hasReserve: Bool { current?.currencies.reserveId != nil }

    mutating func cycleCurrency() {
        guard hasReserve else { currency = .native; return }
        let all = Currency.allCases
        currency = all[((all.firstIndex(of: currency) ?? 0) + 1) % all.count]
    }

    mutating func cycleScale() {
        let all = Scale.allCases
        scale = all[((all.firstIndex(of: scale) ?? 0) + 1) % all.count]
    }

    mutating func reset(keepCheckpoints: Bool) {
        current = nil
        currentError = nil
        if !keepCheckpoints { checkpoints = [:] }
        generation += 1
    }
}

// MARK: - Seek

/// Go to a block height, Unix seconds, or a date and time, with a preview before submitting.
private struct SeekCard: View {
    @Bindable var state: ChartState
    @Environment(AppModel.self) private var model
    @State private var text = ""
    @State private var now = Int64(Date.now.timeIntervalSince1970)
    @State private var working = false
    @State private var problem: String?
    @State private var future = false
    @State private var task: Task<Void, Never>?
    @FocusState private var focused: Bool

    var body: some View {
        Card("Go to a block or a time") {
            HStack {
                TextField(
                    "Target", text: $text,
                    prompt: Text("Height, Unix seconds or date. Examples: 4000000, 2026-11-07 14:30, height:3000000"))
                    .displayField()
                    .focused($focused)
                    .onSubmit { submit(nil) }
                    .onChange(of: text) { old, new in
                        if old.isEmpty && !new.isEmpty { now = Int64(Date.now.timeIntervalSince1970) }
                        problem = nil
                        future = false
                    }
                if working {
                    ProgressView().controlSize(.small)
                    Button("Cancel") { cancel() }
                } else {
                    Button("Go") { submit(nil) }
                        .disabled(text.trimmingCharacters(in: .whitespaces).isEmpty)
                }
            }
            Text(chartSeekPreview(input: text, now: now, tip: state.page?.tip ?? model.status?.blocks))
                .font(.callout)
                .foregroundStyle(.secondary)
            if working {
                Text("Locating the block. A date search reads up to 129 blocks around the best match.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let problem {
                Note(text: problem, warning: true)
            }
            if future {
                Button("Show the latest block") { submit(.latest) }
            }
            Text("Dates are in UTC unless you give an offset. A date without a time means midnight. height: and unix: remove doubt about a number. A target in the loaded range is selected at once. Other targets load a range of the current span centred on them.")
                .font(.caption).foregroundStyle(.secondary)
        }
        .onChange(of: state.focusSeek) { focused = true }
    }

    private func cancel() {
        task?.cancel()
        task = nil
        working = false
    }

    private func submit(_ forced: SeekTarget?) {
        problem = nil
        future = false
        let target: SeekTarget
        if let forced {
            target = forced
        } else {
            do {
                target = try chartSeekParse(input: text, now: now)
            } catch {
                problem = describe(error)
                return
            }
        }
        if let (hash, note) = loadedMatch(target) {
            state.select(hash: hash)
            state.seekNote = note
            if state.metric == .feePool, let sample = state.page?.samples.first(where: { $0.hash == hash }),
                let explorer = try? model.requireExplorer()
            {
                Task { await state.includeCheckpoint(sample, explorer: explorer) }
            }
            return
        }
        guard let explorer = try? model.requireExplorer() else { return }
        let span = state.span.seconds
        working = true
        task = Task {
            do {
                let outcome = try await explorer.chartSeek(target: target, spanSeconds: span)
                guard !Task.isCancelled else { return }
                working = false
                if outcome.future {
                    problem = outcome.note
                    future = true
                } else {
                    state.show(outcome)
                }
            } catch {
                guard !Task.isCancelled else { return }
                working = false
                problem = describe(error)
            }
        }
    }

    /// A target inside the loaded range, selected without a request. Ported from Carbide's
    /// `chart_ui::seek::loaded`.
    private func loadedMatch(_ target: SeekTarget) -> (String, String)? {
        guard let page = state.page, !page.samples.isEmpty else { return nil }
        switch target {
        case .number(let value) where value > page.tip:
            return loadedMatch(.time(unix: Int64(clamping: value)))
        case .number(let value), .height(let value):
            guard let sample = page.samples.first(where: { $0.height == value }) else { return nil }
            return (sample.hash, "Block \(Format.integer(sample.height)) is in the loaded range.")
        case .time(let unix):
            let times = page.samples.map { Int64($0.time) }
            guard let low = times.min(), let high = times.max(), (low...high).contains(unix) else { return nil }
            let best = page.samples.min { a, b in
                let da = abs(Int64(a.time) - unix), db = abs(Int64(b.time) - unix)
                return (da, a.time, a.height) < (db, b.time, b.height)
            }
            guard let best else { return nil }
            let difference = Int64(best.time) - unix
            return (
                best.hash,
                "Requested \(ChartFormat.iso(unix)). Matched block \(Format.integer(best.height)) at \(ChartFormat.iso(Int64(best.time))), a difference of \(difference >= 0 ? "+" : "")\(difference) s. This is the nearest of \(page.samples.count) loaded blocks."
            )
        case .latest:
            return nil
        }
    }
}

// MARK: - Selected block

/// The selected block's exact figures, with controls to move the selection and open it.
private struct SelectedBlockCard: View {
    let state: ChartState
    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Selected block") {
            if let sample = state.selectedSample {
                HStack {
                    Button {
                        state.move(-1)
                    } label: {
                        Label("Older", systemImage: "chevron.left")
                    }
                    Button {
                        state.move(1)
                    } label: {
                        Label("Newer", systemImage: "chevron.right")
                    }
                    Button("Open block") { model.open(.block(id: sample.hash)) }
                    Spacer()
                }
                .buttonStyle(.lamp)
                .controlSize(.small)
                FieldRow(label: "Height", value: Format.integer(sample.height), route: .block(id: sample.hash))
                FieldRow(label: "Hash", value: sample.hash, route: .block(id: sample.hash), monospaced: true)
                FieldRow(label: "Time", value: "\(Format.date(sample.time)) (\(sample.time))")
                FieldRow(label: "Produced by", value: ChartFormat.production(sample.production))
                ForEach(rows(sample)) { row in
                    FieldRow(label: row.label, value: row.value, monospaced: row.monospaced)
                }
                if !sample.linked {
                    Note(text: "This block does not link to a loaded parent. Its interval and averages are not available.")
                        .font(.caption).foregroundStyle(.secondary)
                }
            } else if state.loading {
                Text("Loading.").foregroundStyle(.secondary)
            } else {
                Text("No applicable block in this range. A missing value is not zero.")
                    .foregroundStyle(.secondary)
            }
        }
    }

    private struct Row: Identifiable {
        let label: String
        let value: String
        var monospaced = false
        var id: String { label }
    }

    private func rows(_ sample: ChartSample) -> [Row] {
        var rows: [Row] = []
        rows.append(Row(label: "Interval", value: sample.interval.map { ChartFormat.signedSeconds($0) } ?? "Not available. The parent is not loaded or not linked."))
        rows.append(Row(label: "Mining difficulty", value: sample.mining.map(ChartFormat.exact) ?? "Not a mined block, or no value"))
        rows.append(Row(label: "Mining MA100", value: average(sample.miningAverage, kind: "mined")))
        rows.append(Row(label: "Staking difficulty", value: sample.staking.map(ChartFormat.exact) ?? "Not a staked block, or no value"))
        rows.append(Row(label: "Staking MA100", value: average(sample.stakingAverage, kind: "staked")))
        if let target = sample.posTarget { rows.append(Row(label: "PoS target", value: target, monospaced: true)) }
        if let problem = sample.problem { rows.append(Row(label: "Problem", value: problem)) }
        rows.append(Row(label: "100 blocks", value: ChartFormat.window(sample.window100, share: sample.share100)))
        rows.append(Row(label: "1,000 blocks", value: ChartFormat.window(sample.window1000, share: sample.share1000)))
        return rows
    }

    private func average(_ value: ChartAverage?, kind: String) -> String {
        guard let value else {
            return "Not available. The trailing 100 heights are incomplete, or hold a block without a value."
        }
        return "\(ChartFormat.exact(value.value)) from \(value.samples) \(kind) blocks"
    }
}

// MARK: - Table

/// The same samples as a table, for exact reading and copying.
private struct SampleTable: View {
    let state: ChartState
    @State private var expanded = false

    var body: some View {
        if let page = state.page, !page.samples.isEmpty {
            Card("Samples as a table") {
                DisclosureGroup(isExpanded: $expanded) {
                    Table(Array(page.samples.reversed())) {
                        TableColumn(ChartFormat.columnName("Height")) { Text(String($0.height)).monospacedDigit() }
                            .width(min: 70, ideal: 80)
                        TableColumn(ChartFormat.columnName("Time")) { Text(Format.date($0.time)).monospacedDigit() }
                            .width(min: 150, ideal: 180)
                        TableColumn(ChartFormat.columnName("Produced")) { Text(ChartFormat.production($0.production)) }
                            .width(min: 60, ideal: 70)
                        TableColumn(ChartFormat.columnName("Interval")) { Text($0.interval.map { ChartFormat.signedSeconds($0) } ?? "–").monospacedDigit() }
                            .width(min: 60, ideal: 70)
                        TableColumn(ChartFormat.columnName("Mining")) { Text($0.mining.map { Format.difficulty($0) } ?? "–").monospacedDigit() }
                            .width(min: 60, ideal: 70)
                        TableColumn(ChartFormat.columnName("Staking")) { Text($0.staking.map { Format.difficulty($0) } ?? "–").monospacedDigit() }
                            .width(min: 60, ideal: 70)
                        TableColumn(ChartFormat.columnName("Staked, 100")) { Text($0.share100.map(Format.percent) ?? "–").monospacedDigit() }
                            .width(min: 70, ideal: 80)
                        TableColumn(ChartFormat.columnName("Staked, 1,000")) { Text($0.share1000.map(Format.percent) ?? "–").monospacedDigit() }
                            .width(min: 70, ideal: 90)
                    }
                    .frame(height: 320)
                } label: {
                    Text(expanded ? "Hide" : "Show all \(page.samples.count) blocks of this range")
                        .foregroundStyle(.secondary)
                }
            }
        }
    }

}

extension ChartSample: Identifiable {
    public var id: String { hash }
}

// MARK: - Formatting

enum ChartFormat {
    static func production(_ value: BlockProduction) -> String {
        switch value {
        case .mined: "Mined"
        case .staked: "Staked"
        case .unknown: "Unknown"
        }
    }

    static func signedSeconds(_ value: Int64) -> String {
        "\(value > 0 ? "+" : "")\(value) s"
    }

    /// A full-precision figure for the details, not a rounded axis label.
    static func exact(_ value: Double) -> String {
        value.formatted(.number.precision(.fractionLength(0...8)).grouping(.automatic))
    }

    static func window(_ counts: ProductionCounts, share: Double?) -> String {
        let total = counts.mined + counts.staked + counts.unknown
        var text = "\(counts.mined) mined, \(counts.staked) staked"
        if total < counts.window {
            text += ". Partial history: \(total) of \(counts.window) blocks. No trend."
        } else if counts.unknown > 0 {
            text += ", \(counts.unknown) unknown. No trend."
        } else if let share {
            text += ". \(share.formatted(.percent.precision(.fractionLength(1)))) staked."
        }
        return text
    }

    static func duration(_ seconds: Int64) -> String {
        let seconds = max(seconds, 0)
        let days = seconds / 86_400, hours = seconds % 86_400 / 3_600, minutes = seconds % 3_600 / 60
        if days > 0 { return "\(days) d \(hours) h \(minutes) min" }
        if hours > 0 { return "\(hours) h \(minutes) min" }
        return "\(minutes) min \(seconds % 60) s"
    }

    /// The name of a table column, in spaced capitals like the names on the charts.
    static func columnName(_ name: String) -> Text {
        Text(name.uppercased()).font(Tone.small).tracking(1.1).foregroundStyle(Tone.label)
    }

    static func iso(_ unix: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(unix)).formatted(.iso8601)
    }

    /// A short axis time that follows the UTC setting.
    static func axisTime(_ unix: UInt64, span: UInt64) -> String {
        let date = Date(timeIntervalSince1970: TimeInterval(unix))
        var style = span > 2 * 86_400
            ? Date.FormatStyle().month(.abbreviated).day().hour().minute()
            : Date.FormatStyle().hour().minute()
        if UserDefaults.standard.bool(forKey: SettingsKey.useUTC) { style.timeZone = .gmt }
        return date.formatted(style)
    }
}
