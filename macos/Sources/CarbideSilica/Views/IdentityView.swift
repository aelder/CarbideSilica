import CarbideCore
import SwiftUI

/// A VerusID: its addresses, authorities and registration.
struct IdentityView: View {
    let name: String

    @Environment(AppModel.self) private var model

    var body: some View {
        LoadView(key: "identity:\(name)") { explorer in
            try await explorer.identity(name: name)
        } content: { identity in
            Page {
                PageHeader(
                    kind: "Identity",
                    title: identity.fullyQualifiedName ?? identity.name,
                    subtitle: identity.identityAddress,
                    tags: tags(identity)
                )
                if identity.definesCurrency, let address = identity.identityAddress {
                    Button {
                        model.open(.currency(name: address))
                    } label: {
                        Label("Show the currency", systemImage: "circle.hexagongrid")
                    }
                }

                IdentityControlCard(identity: identity)

                Card("Identity") {
                    FieldRow(label: "Name", value: identity.name)
                    OptionalFieldRow(
                        label: "Identity address", value: identity.identityAddress,
                        monospaced: true)
                    OptionalFieldRow(
                        label: "Parent", value: identity.parent,
                        route: { .currency(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "System", value: identity.systemId,
                        route: { .currency(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "Private address", value: identity.privateAddress, monospaced: true)
                    if let timelock = identity.timelock, timelock > 0 {
                        FieldRow(label: "Timelock", value: Format.integer(timelock))
                    }
                }

                Card(
                    "Signing authority",
                    trailing: identity.minimumSignatures.map {
                        "\($0) of \(identity.primaryAddresses.count) required"
                    }
                ) {
                    ForEach(Array(identity.primaryAddresses.enumerated()), id: \.offset) {
                        index, address in
                        FieldRow(
                            label: "Primary address \(index + 1)", value: address,
                            route: .address(address: address), monospaced: true)
                    }
                    OptionalFieldRow(
                        label: "Revocation authority", value: identity.revocationAuthority,
                        route: { .identity(name: $0) }, monospaced: true)
                    OptionalFieldRow(
                        label: "Recovery authority", value: identity.recoveryAuthority,
                        route: { .identity(name: $0) }, monospaced: true)
                }

                // CIdentity flag 4: control of the identity is held by a token.
                if let flags = identity.flags, flags & 4 != 0 {
                    TokenControlCard(identity: identity)
                }

                IdentityHistoryCard(
                    name: identity.identityAddress ?? name, identityAddress: identity.identityAddress)

                ReverseLookupCard(identity: identity)

                Card("Latest update") {
                    if let height = identity.blockHeight {
                        FieldRow(
                            label: "Block", value: Format.integer(height),
                            route: .block(id: "\(height)"))
                    }
                    OptionalFieldRow(
                        label: "Transaction", value: identity.txid,
                        route: { .transaction(txid: $0) }, monospaced: true)
                }

                JSONSection(json: identity.json)
            }
            .task(id: identity.identityAddress) {
                await model.resolveNames([
                    identity.parent, identity.systemId, identity.revocationAuthority,
                    identity.recoveryAuthority,
                ])
            }
        }
        .navigationTitle("Identity")
    }

    private func tags(_ identity: IdentityDetail) -> [String] {
        var tags: [String] = []
        if let status = identity.status { tags.append(status.capitalized) }
        if identity.definesCurrency { tags.append("Currency defined") }
        if let flags = identity.flags, flags & 4 != 0 { tags.append("Tokenized control") }
        return tags
    }
}
