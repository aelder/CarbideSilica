import CarbideCore
import SwiftUI

/// Changes to the node's peer connections, each behind a confirmation, and the daemon's
/// log lines for a peer. These are the only things the app can change on the daemon.
struct PeerControlsCard: View {
    let node: NodeStatus

    @Environment(AppModel.self) private var model
    @State private var banned: [String] = []
    @State private var message: String?
    @State private var newNode = ""
    @State private var addingNode = false
    @State private var pending: PendingAction?
    @State private var logPeer: PeerRow?
    @State private var logLines: [String] = []

    private struct PendingAction: Identifiable {
        let id = UUID()
        let title: String
        let detail: String
        let action: PeerAction
    }

    var body: some View {
        if node.peersError == nil {
            Card("Manage peers", trailing: banned.isEmpty ? nil : "\(banned.count) banned") {
                HStack {
                    Button("Add Node…") { addingNode = true }
                    Button("Ping All") { run(.pingAll, done: "The node will ping all connected peers.") }
                    Button("Clear Ban List…") {
                        pending = PendingAction(
                            title: "Clear the ban list?",
                            detail: "All \(banned.count) banned addresses can connect again.",
                            action: .clearBans)
                    }
                    .disabled(banned.isEmpty)
                    Spacer()
                }
                if let message {
                    Text(message).font(.callout).foregroundStyle(.secondary)
                }
                VStack(spacing: 0) {
                    ForEach(node.peers, id: \.id) { peer in
                        Divider()
                        HStack(spacing: 10) {
                            Text(peer.address).font(.callout.monospaced()).lineLimit(1)
                            Spacer()
                            Button("Log") { showLog(peer) }
                            Button("Disconnect…") {
                                pending = PendingAction(
                                    title: "Disconnect \(peer.address)?",
                                    detail: "The node closes this connection. The peer can connect again.",
                                    action: .disconnect(address: peer.address))
                            }
                            Button("Ban…", role: .destructive) {
                                pending = PendingAction(
                                    title: "Ban \(peer.address)?",
                                    detail: "The node refuses connections from this address until the ban expires or you clear the ban list.",
                                    action: .ban(ip: peer.address))
                            }
                        }
                        .padding(.vertical, 6)
                    }
                }
                if let logPeer {
                    VStack(alignment: .leading, spacing: 6) {
                        HStack {
                            Text("Node log for \(logPeer.address)").font(.subheadline.weight(.semibold))
                            Spacer()
                            Button("Close") { self.logPeer = nil }
                        }
                        if logLines.isEmpty {
                            Text("The recent log has no lines for this peer.").foregroundStyle(.secondary)
                        } else {
                            ScrollView {
                                Text(logLines.joined(separator: "\n"))
                                    .font(.caption.monospaced())
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                            .frame(maxHeight: 220)
                        }
                    }
                    .padding(10)
                    .background(.background.tertiary, in: RoundedRectangle(cornerRadius: 6))
                }
                if !banned.isEmpty {
                    Text("Banned: \(banned.joined(separator: ", "))").font(.caption).foregroundStyle(.secondary)
                }
                Text("These commands change only the node connections. They do not change funds, keys or the chain.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            .task(id: model.generation) { await loadBanned() }
            .alert("Add a node", isPresented: $addingNode) {
                TextField("Address, such as 203.0.113.18:27485", text: $newNode)
                Button("Add") {
                    run(.addNode(address: newNode), done: "Added \(newNode) to the node list.")
                    newNode = ""
                }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("The node tries to keep a connection to this address.")
            }
            .confirmationDialog(
                pending?.title ?? "", isPresented: Binding(get: { pending != nil }, set: { if !$0 { pending = nil } }),
                titleVisibility: .visible
            ) {
                if let pending {
                    Button("Confirm", role: .destructive) { run(pending.action, done: "Done.") }
                }
            } message: {
                Text(pending?.detail ?? "")
            }
        }
    }

    private func run(_ action: PeerAction, done: String) {
        Task {
            do {
                try await model.requireExplorer().peerAction(action: action)
                message = done
            } catch {
                message = describe(error)
            }
            await loadBanned()
            await model.refreshNode()
        }
    }

    private func loadBanned() async {
        banned = (try? await model.explorer?.bannedPeers()) ?? []
    }

    private func showLog(_ peer: PeerRow) {
        logPeer = peer
        logLines = (try? model.explorer?.peerLog(address: peer.address)) ?? []
    }
}
