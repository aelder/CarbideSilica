import CarbideCore
import SwiftUI

/// Changes to the node's peer connections. Each change waits for a confirmation. These are
/// the only things the app can change on the node.
@MainActor
@Observable
final class PeerCommands {
    struct Pending: Identifiable {
        let id = UUID()
        let title: String
        let detail: String
        let button: String
        let action: PeerAction
        let done: String
    }

    var pending: Pending?
    var addingNode = false
    var newNode = ""
    private(set) var message: String?
    private(set) var busy = false

    func confirmClearBans(count: Int) {
        pending = Pending(
            title: "Clear the ban list?",
            detail: count == 1
                ? "The banned address can connect to the node again."
                : "All \(count) banned addresses can connect to the node again.",
            button: "Clear Ban List", action: .clearBans, done: "The node cleared its ban list.")
    }

    func confirmDisconnect(_ address: String) {
        pending = Pending(
            title: "Disconnect \(address)?",
            detail: "The node closes this connection. The peer can connect again.",
            button: "Disconnect", action: .disconnect(address: address),
            done: "The node disconnects \(address).")
    }

    func confirmBan(_ address: String) {
        pending = Pending(
            title: "Ban \(address)?",
            detail: "The node refuses connections from this IP address until the ban expires or you clear the ban list.",
            button: "Ban", action: .ban(ip: address), done: "The node banned \(address).")
    }

    func startAddNode() {
        newNode = ""
        addingNode = true
    }

    func run(_ action: PeerAction, done: String, model: AppModel, after: @escaping () async -> Void) {
        guard !busy else { return }
        busy = true
        Task {
            do {
                try await model.requireExplorer().peerAction(action: action)
                message = done
            } catch {
                message = describe(error)
            }
            busy = false
            await after()
            await model.refreshNode()
        }
    }
}

/// The commands for all peers: add a node, ping all peers and clear the ban list.
struct PeerControlsCard: View {
    let banned: Int
    let commands: PeerCommands
    let reload: () async -> Void

    @Environment(AppModel.self) private var model

    var body: some View {
        Card("Manage peers") {
            HStack {
                Button("Add Node…") { commands.startAddNode() }
                Button("Ping All") {
                    commands.run(
                        .pingAll, done: "The node sends a ping to all connected peers.", model: model,
                        after: reload)
                }
                Button("Clear Ban List…") { commands.confirmClearBans(count: banned) }
                    .disabled(banned == 0)
                if commands.busy { ProgressView().controlSize(.small) }
                Spacer()
            }
            .disabled(commands.busy)
            if let message = commands.message {
                Text(message).font(.callout).foregroundStyle(.secondary)
            }
            Text(
                "These commands change only the connections of the node. They do not change funds, keys or the chain. "
                    + "To disconnect or ban a peer, open the peer."
            )
            .font(.caption).foregroundStyle(.secondary)
        }
    }
}

/// The add-node prompt and the confirmation dialog, attached once to the page that shows
/// peers. The peer page uses the same commands.
struct PeerCommandDialogs: ViewModifier {
    @Bindable var commands: PeerCommands
    /// False while a sheet above this view shows the dialogs instead.
    var enabled = true
    let reload: () async -> Void
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        content
            .alert(
                "Add a node",
                isPresented: Binding(
                    get: { enabled && commands.addingNode }, set: { commands.addingNode = $0 })
            ) {
                TextField("Host name or IP address, with an optional port", text: $commands.newNode)
                Button("Add") {
                    let node = commands.newNode.trimmingCharacters(in: .whitespaces)
                    guard !node.isEmpty else { return }
                    commands.run(
                        .addNode(address: node), done: "The node added \(node) to its node list.",
                        model: model, after: reload)
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The node tries to keep a connection to this address.")
            }
            .confirmationDialog(
                commands.pending?.title ?? "",
                isPresented: Binding(
                    get: { enabled && commands.pending != nil }, set: { if !$0 { commands.pending = nil } }),
                titleVisibility: .visible,
                presenting: commands.pending
            ) { pending in
                Button(pending.button, role: .destructive) {
                    commands.run(pending.action, done: pending.done, model: model, after: reload)
                }
                Button("Cancel", role: .cancel) {}
            } message: { pending in
                Text(pending.detail)
            }
    }
}
