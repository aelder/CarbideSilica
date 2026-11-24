import CarbideCore
import SwiftUI

/// Which identities shared this one's addresses or authorities during a past window of blocks.
struct ReverseLookupCard: View {
    let identity: IdentityDetail

    @Environment(AppModel.self) private var model
    @State private var endHeight = ""
    @State private var found: [SharedIdentity]?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Shared keys and authorities") {
            HStack {
                Text("In the 1,000 blocks ending at").foregroundStyle(.secondary)
                TextField("Block", text: $endHeight, prompt: Text(model.status.map { "\($0.blocks)" } ?? "height"))
                    .textFieldStyle(.roundedBorder)
                    .frame(width: 130)
                Button("Look up") { Task { await run() } }.disabled(running)
                Spacer()
            }
            if running {
                ProgressView().controlSize(.small)
            } else if let failure {
                Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.secondary)
            } else if let found {
                if found.isEmpty {
                    Text("No other identity used these addresses or authorities in this range. Other ranges can give different results.")
                        .foregroundStyle(.secondary)
                } else {
                    VStack(spacing: 0) {
                        ForEach(Array(found.enumerated()), id: \.offset) { index, shared in
                            if index > 0 { Divider() }
                            HStack(spacing: 12) {
                                Text(shared.relation).foregroundStyle(.secondary).frame(width: 170, alignment: .leading)
                                RouteLink(
                                    text: shared.name,
                                    route: .identity(name: shared.identityAddress ?? shared.name), shorten: false)
                                Spacer()
                                Text("via \(model.names[shared.via] ?? Format.short(shared.via))")
                                    .font(.callout).foregroundStyle(.secondary)
                            }
                            .padding(.vertical, 7)
                        }
                    }
                }
            } else {
                Text("Find identities that used the same primary address, revocation authority or recovery authority. This search needs a node with the identity index.")
                    .font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private func run() async {
        guard let height = UInt64(endHeight.trimmingCharacters(in: .whitespaces)) ?? model.status?.blocks else { return }
        running = true
        failure = nil
        defer { running = false }
        do {
            found = try await model.requireExplorer().identitiesSharing(
                name: identity.identityAddress ?? identity.name, endHeight: height)
        } catch {
            found = nil
            failure = describe(error)
        }
    }
}

/// For an identity whose control is a token: who holds that token now.
struct TokenControlCard: View {
    let identity: IdentityDetail

    @Environment(AppModel.self) private var model
    @State private var holder: TokenHolder?
    @State private var failure: String?
    @State private var running = false

    var body: some View {
        Card("Tokenized control") {
            Text("The holder of the control token controls this identity.")
                .foregroundStyle(.secondary)
            if let holder {
                HStack(alignment: .firstTextBaseline, spacing: 10) {
                    ConfidenceMark(level: holder.confidence, label: holder.holder == nil ? "Unresolved" : "Current holder")
                    if let address = holder.holder {
                        RouteLink(text: address, route: .holder(address), shorten: false)
                    }
                }
                Text(holder.summary).font(.callout)
                if let txid = holder.txid {
                    HStack(spacing: 6) {
                        Text("Last seen in").foregroundStyle(.secondary)
                        RouteLink(text: txid, route: .transaction(txid: txid))
                        if let output = holder.output { Text("output \(output)").foregroundStyle(.secondary) }
                    }
                    .font(.callout)
                }
            } else if running {
                HStack(spacing: 10) {
                    ProgressView().controlSize(.small)
                    Text("Tracing the token…").foregroundStyle(.secondary)
                }
            } else {
                if let failure {
                    Label(failure, systemImage: "exclamationmark.triangle").foregroundStyle(.orange)
                }
                Button("Find the token holder") { Task { await run() } }
            }
        }
    }

    private func run() async {
        guard let address = identity.identityAddress else { return }
        running = true
        failure = nil
        defer { running = false }
        do {
            let result = try await model.requireExplorer().tokenHolder(identityAddress: address)
            holder = result
            await model.resolveNames([result.holder])
        } catch {
            failure = describe(error)
        }
    }
}
