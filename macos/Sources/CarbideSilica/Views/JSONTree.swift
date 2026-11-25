import AppKit
import CarbideCore
import SwiftUI
import UniformTypeIdentifiers

/// A JSON value with its members in document order and numbers kept as written, so a copied
/// value is exactly what the node sent.
indirect enum JSONNode: Sendable {
    case object([(String, JSONNode)])
    case array([JSONNode])
    case string(String)
    case number(String)
    case bool(Bool)
    case null

    var isContainer: Bool {
        switch self {
        case .object, .array: true
        default: false
        }
    }

    var children: [(String, JSONNode)] {
        switch self {
        case .object(let members): members
        case .array(let items): items.enumerated().map { (String($0.offset), $0.element) }
        default: []
        }
    }

    /// Compact JSON for a subtree, or the raw scalar: a string without quotes, as the
    /// terminal's `y` copies it.
    var copyText: String {
        switch self {
        case .string(let text): text
        case .number(let text): text
        case .bool(let flag): flag ? "true" : "false"
        case .null: "null"
        default: compact
        }
    }

    var compact: String {
        switch self {
        case .object(let members):
            "{" + members.map { "\(JSONNode.quote($0.0)):\($0.1.compact)" }.joined(separator: ",") + "}"
        case .array(let items):
            "[" + items.map(\.compact).joined(separator: ",") + "]"
        case .string(let text): JSONNode.quote(text)
        case .number(let text): text
        case .bool(let flag): flag ? "true" : "false"
        case .null: "null"
        }
    }

    /// Indented JSON with the numbers as written.
    func pretty(_ indent: String = "") -> String {
        let inner = indent + "  "
        switch self {
        case .object(let members):
            if members.isEmpty { return "{}" }
            return "{\n" + members.map { inner + JSONNode.quote($0.0) + ": " + $0.1.pretty(inner) }
                .joined(separator: ",\n") + "\n" + indent + "}"
        case .array(let items):
            if items.isEmpty { return "[]" }
            return "[\n" + items.map { inner + $0.pretty(inner) }.joined(separator: ",\n") + "\n" + indent + "]"
        default:
            return compact
        }
    }

    static func quote(_ text: String) -> String {
        var result = "\""
        for scalar in text.unicodeScalars {
            switch scalar {
            case "\"": result += "\\\""
            case "\\": result += "\\\\"
            case "\n": result += "\\n"
            case "\r": result += "\\r"
            case "\t": result += "\\t"
            case let c where c.value < 0x20: result += String(format: "\\u%04x", c.value)
            default: result.unicodeScalars.append(scalar)
            }
        }
        return result + "\""
    }
}

/// A small JSON reader that keeps member order and number text.
struct JSONReader {
    private let bytes: [UInt8]
    private var index = 0

    struct Failure: Error {}

    static func parse(_ text: String) throws -> JSONNode {
        var reader = JSONReader(bytes: Array(text.utf8))
        reader.skipSpace()
        let value = try reader.value()
        reader.skipSpace()
        guard reader.index == reader.bytes.count else { throw Failure() }
        return value
    }

    private init(bytes: [UInt8]) { self.bytes = bytes }

    private mutating func skipSpace() {
        while index < bytes.count, [0x20, 0x0a, 0x0d, 0x09].contains(bytes[index]) { index += 1 }
    }

    private mutating func value() throws -> JSONNode {
        guard index < bytes.count else { throw Failure() }
        switch bytes[index] {
        case UInt8(ascii: "{"):
            index += 1
            var members: [(String, JSONNode)] = []
            skipSpace()
            if index < bytes.count, bytes[index] == UInt8(ascii: "}") { index += 1; return .object(members) }
            while true {
                skipSpace()
                let key = try string()
                skipSpace()
                try expect(":")
                skipSpace()
                members.append((key, try value()))
                skipSpace()
                if try separator("}") { return .object(members) }
            }
        case UInt8(ascii: "["):
            index += 1
            var items: [JSONNode] = []
            skipSpace()
            if index < bytes.count, bytes[index] == UInt8(ascii: "]") { index += 1; return .array(items) }
            while true {
                skipSpace()
                items.append(try value())
                skipSpace()
                if try separator("]") { return .array(items) }
            }
        case UInt8(ascii: "\""):
            return .string(try string())
        case UInt8(ascii: "t"):
            try literal("true"); return .bool(true)
        case UInt8(ascii: "f"):
            try literal("false"); return .bool(false)
        case UInt8(ascii: "n"):
            try literal("null"); return .null
        default:
            let start = index
            while index < bytes.count, "+-0123456789.eE".utf8.contains(bytes[index]) { index += 1 }
            guard index > start else { throw Failure() }
            return .number(String(decoding: bytes[start..<index], as: UTF8.self))
        }
    }

    /// True at the closing bracket, false after a comma.
    private mutating func separator(_ close: Character) throws -> Bool {
        guard index < bytes.count else { throw Failure() }
        if bytes[index] == UInt8(ascii: ",") { index += 1; return false }
        if bytes[index] == close.asciiValue { index += 1; return true }
        throw Failure()
    }

    private mutating func expect(_ character: Character) throws {
        guard index < bytes.count, bytes[index] == character.asciiValue else { throw Failure() }
        index += 1
    }

    private mutating func literal(_ word: String) throws {
        let word = Array(word.utf8)
        guard index + word.count <= bytes.count, Array(bytes[index..<index + word.count]) == word else {
            throw Failure()
        }
        index += word.count
    }

    private mutating func string() throws -> String {
        try expect("\"")
        var scalars = String.UnicodeScalarView()
        var run = index
        func flush() { scalars.append(contentsOf: String(decoding: bytes[run..<index], as: UTF8.self).unicodeScalars) }
        while index < bytes.count {
            let byte = bytes[index]
            if byte == UInt8(ascii: "\"") {
                flush()
                index += 1
                return String(scalars)
            }
            if byte == UInt8(ascii: "\\") {
                flush()
                index += 1
                guard index < bytes.count else { throw Failure() }
                let escape = bytes[index]
                index += 1
                switch escape {
                case UInt8(ascii: "n"): scalars.append("\n")
                case UInt8(ascii: "t"): scalars.append("\t")
                case UInt8(ascii: "r"): scalars.append("\r")
                case UInt8(ascii: "b"): scalars.append("\u{08}")
                case UInt8(ascii: "f"): scalars.append("\u{0C}")
                case UInt8(ascii: "u"):
                    var code = try hex4()
                    if (0xD800...0xDBFF).contains(code), index + 1 < bytes.count,
                        bytes[index] == UInt8(ascii: "\\"), bytes[index + 1] == UInt8(ascii: "u")
                    {
                        index += 2
                        let low = try hex4()
                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
                    }
                    scalars.append(Unicode.Scalar(code) ?? "\u{FFFD}")
                default: scalars.append(Unicode.Scalar(escape))
                }
                run = index
                continue
            }
            index += 1
        }
        throw Failure()
    }

    private mutating func hex4() throws -> UInt32 {
        guard index + 4 <= bytes.count,
            let code = UInt32(String(decoding: bytes[index..<index + 4], as: UTF8.self), radix: 16)
        else { throw Failure() }
        index += 4
        return code
    }
}

/// One visible line of the tree.
private struct JSONRow: Identifiable {
    let pointer: String
    let depth: Int
    let label: String
    let node: JSONNode
    var id: String { pointer }
}

/// The node's JSON as a tree: collapse and expand nodes, follow links, copy a value or a
/// subtree, and save the document.
///
/// `showNames` shows names for identity and currency IDs; the `identityaddress` field stays
/// raw. `showDates` adds the date a height or a timestamp stands for, as Carbide does.
struct JSONTreeView: View {
    let json: String
    var showNames: Bool?
    var showDates: Bool?
    /// The file name offered by Save.
    var filename = "carbide.json"

    @Environment(AppModel.self) private var model
    @State private var root: JSONNode?
    @State private var failed = false
    @State private var expanded: Set<String> = [""]
    @State private var openStrings: Set<String> = []
    @State private var marks: [String: JsonMark] = [:]
    @State private var selected: String?
    @AppStorage(DisplayKey.showsNames) private var showsNamesSetting = true
    @AppStorage(DisplayKey.showsDates) private var showsDatesSetting = false

    private var names: Bool { showNames ?? showsNamesSetting }
    private var dates: Bool { showDates ?? showsDatesSetting }

    private static let previewLength = 120
    private static let rowLimit = 5000

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            toolbar
            if let root {
                let rows = visibleRows(root)
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(rows.prefix(Self.rowLimit)) { row in
                        line(row)
                    }
                }
                .font(.callout.monospaced())
                if rows.count > Self.rowLimit {
                    Text("The view shows the first \(Self.rowLimit) lines. Collapse a node, or save the document to read all of it.")
                        .font(.caption).foregroundStyle(.secondary)
                }
                if let selected {
                    Text("Path \(selected.isEmpty ? "/" : selected)")
                        .font(.caption.monospaced()).foregroundStyle(.secondary).textSelection(.enabled)
                }
            } else if failed {
                ScrollView(.horizontal) {
                    Text(json).font(.callout.monospaced()).textSelection(.enabled)
                }
            } else {
                ProgressView().controlSize(.small)
            }
        }
        .task(id: json) {
            let text = json
            let parsed = await Task.detached { try? JSONReader.parse(text) }.value
            root = parsed
            failed = parsed == nil
            expanded = [""]
        }
        .task(id: "\(json.hashValue)|\(dates)|\(model.generation)") {
            guard let explorer = model.explorer else { return }
            let found = await explorer.jsonMarks(json: json, dates: dates)
            var byPointer: [String: JsonMark] = [:]
            for mark in found { byPointer[mark.pointer] = mark }
            marks = byPointer
            if names {
                await model.resolveNames(found.filter(\.nameable).compactMap(\.target))
            }
        }
    }

    private var toolbar: some View {
        HStack(spacing: 12) {
            Button("Expand All") { if let root { expanded = Self.allContainers(root) } }
            Button("Collapse All") { expanded = [""] }
            Spacer()
            Button {
                copy(json)
            } label: {
                Label("Copy All", systemImage: "doc.on.doc")
            }
            Button {
                save()
            } label: {
                Label("Save…", systemImage: "square.and.arrow.down")
            }
        }
        .controlSize(.small)
        .buttonStyle(.borderless)
    }

    private func visibleRows(_ root: JSONNode) -> [JSONRow] {
        var rows: [JSONRow] = []
        func walk(_ node: JSONNode, pointer: String, label: String, depth: Int) {
            rows.append(JSONRow(pointer: pointer, depth: depth, label: label, node: node))
            guard node.isContainer, expanded.contains(pointer), rows.count <= Self.rowLimit else { return }
            for (key, child) in node.children {
                walk(child, pointer: pointer + "/" + Self.escape(key), label: key, depth: depth + 1)
            }
        }
        walk(root, pointer: "", label: "", depth: 0)
        return rows
    }

    @ViewBuilder private func line(_ row: JSONRow) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 4) {
            Group {
                if row.node.isContainer {
                    Button {
                        toggle(row.pointer)
                    } label: {
                        Image(systemName: expanded.contains(row.pointer) ? "chevron.down" : "chevron.right")
                            .font(.caption2)
                            .frame(width: 12)
                    }
                    .buttonStyle(.borderless)
                    .accessibilityLabel(expanded.contains(row.pointer) ? "Collapse" : "Expand")
                } else {
                    Spacer().frame(width: 12)
                }
            }
            if !row.label.isEmpty {
                Text(row.label + ":").foregroundStyle(.secondary)
            }
            value(row)
            Spacer(minLength: 0)
        }
        .padding(.leading, CGFloat(row.depth) * 16)
        .padding(.vertical, 1)
        .background(selected == row.pointer ? Color.accentColor.opacity(0.12) : .clear)
        .contentShape(Rectangle())
        .onTapGesture { selected = row.pointer }
        .contextMenu { menu(row) }
    }

    @ViewBuilder private func value(_ row: JSONRow) -> some View {
        switch row.node {
        case .object(let members):
            Text("{\(members.count)}").foregroundStyle(.orange)
        case .array(let items):
            Text("[\(items.count)]").foregroundStyle(.orange)
        case .string(let text):
            stringValue(text, row: row)
        case .number(let text):
            HStack(spacing: 6) {
                Text(text).foregroundStyle(.blue).textSelection(.enabled)
                date(row)
            }
        case .bool(let flag):
            Text(flag ? "true" : "false").foregroundStyle(.purple)
        case .null:
            Text("null").foregroundStyle(.tertiary)
        }
    }

    @ViewBuilder private func stringValue(_ text: String, row: JSONRow) -> some View {
        let mark = marks[row.pointer]
        let long = text.count > Self.previewLength
        let open = openStrings.contains(row.pointer)
        let friendly = names && row.label.lowercased() == "friendlyname" ? Self.friendly(text) : text
        let shown = long && !open ? String(friendly.prefix(Self.previewLength)) + "…" : friendly
        HStack(alignment: .firstTextBaseline, spacing: 6) {
            if let mark, let target = mark.target, let route = Self.route(mark, target: target) {
                Button(display(text, mark: mark, label: row.label)) { model.open(route) }
                    .buttonStyle(.link)
                    .help(target)
            } else {
                Text(JSONNode.quote(shown))
                    .foregroundStyle(.green)
                    .textSelection(.enabled)
                    .lineLimit(open ? nil : 1)
            }
            if long {
                Button(open ? "Collapse" : "Show all \(text.count) characters") {
                    if open { openStrings.remove(row.pointer) } else { openStrings.insert(row.pointer) }
                }
                .buttonStyle(.link)
                .font(.caption)
            }
            date(row)
        }
    }

    /// The name for a nameable ID when names are on, else the raw value.
    private func display(_ text: String, mark: JsonMark, label: String) -> String {
        if names, label.lowercased() == "friendlyname" { return Self.friendly(text) }
        guard names, mark.nameable, let name = model.names[text] else { return text }
        if mark.link == .currency, name.hasSuffix("@") { return String(name.dropLast()) }
        return name
    }

    @ViewBuilder private func date(_ row: JSONRow) -> some View {
        if dates, let date = marks[row.pointer]?.date {
            Text((marks[row.pointer]?.estimated == true ? "est. " : "") + Format.date(date))
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder private func menu(_ row: JSONRow) -> some View {
        Button(row.node.isContainer ? "Copy Subtree as JSON" : "Copy Value") { copy(row.node.copyText) }
        if row.node.isContainer {
            Button("Copy Subtree as Pretty JSON") { copy(row.node.pretty()) }
            Button(expanded.contains(row.pointer) ? "Collapse" : "Expand") { toggle(row.pointer) }
        }
        Button("Copy Path") { copy(row.pointer.isEmpty ? "/" : row.pointer) }
        if let mark = marks[row.pointer], let target = mark.target, let route = Self.route(mark, target: target) {
            Divider()
            Button("Open") { model.open(route) }
        }
    }

    private func toggle(_ pointer: String) {
        if expanded.contains(pointer) { expanded.remove(pointer) } else { expanded.insert(pointer) }
    }

    private func copy(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
    }

    private func save() {
        let panel = NSSavePanel()
        panel.allowedContentTypes = [.json]
        panel.nameFieldStringValue = filename
        panel.canCreateDirectories = true
        guard panel.runModal() == .OK, let url = panel.url else { return }
        do {
            // The bridge's documents are already pretty. Others are indented here.
            let text = json.contains("\n") ? json : (root?.pretty() ?? json)
            try text.write(to: url, atomically: true, encoding: .utf8)
        } catch {
            let alert = NSAlert(error: error)
            alert.runModal()
        }
    }

    private static func route(_ mark: JsonMark, target: String) -> Route? {
        switch mark.link {
        case .transaction: .transaction(txid: target)
        case .block: .block(id: target)
        case .address: .address(address: target)
        case .identity: .identity(name: target)
        case .currency: .currency(name: target)
        case nil: nil
        }
    }

    /// Port of `friendly_identity_name` in `src/app.rs`: a friendly name omits the assumed
    /// `.VRSC` root and keeps every other namespace, the casing and the `@`.
    static func friendly(_ name: String) -> String {
        let marker = name.hasSuffix("@") ? "@" : ""
        let body = marker.isEmpty ? name : String(name.dropLast())
        guard body.count > 5, body.suffix(5).lowercased() == ".vrsc" else { return name }
        return String(body.dropLast(5)) + marker
    }

    private static func escape(_ key: String) -> String {
        key.replacingOccurrences(of: "~", with: "~0").replacingOccurrences(of: "/", with: "~1")
    }

    private static func allContainers(_ root: JSONNode) -> Set<String> {
        var found: Set<String> = []
        func walk(_ node: JSONNode, _ pointer: String) {
            guard node.isContainer, found.count < 2000 else { return }
            found.insert(pointer)
            for (key, child) in node.children { walk(child, pointer + "/" + escape(key)) }
        }
        walk(root, "")
        return found
    }
}
