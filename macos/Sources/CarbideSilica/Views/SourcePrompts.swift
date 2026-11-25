import CarbideCore
import SwiftUI

/// Shows the dialog for a primary request that the local node cannot answer, when a
/// disabled public API could. Apply it once, to the window's root view.
struct PermissionPrompt: ViewModifier {
    @Environment(AppModel.self) private var model

    func body(content: Content) -> some View {
        content.sheet(
            isPresented: Binding(
                get: { !model.permissionRequests.isEmpty },
                // A sheet dismissed without a choice cancels the request.
                set: { if !$0, !model.permissionRequests.isEmpty { model.answerPermission(.cancel) } }
            )
        ) {
            if let request = model.permissionRequests.first {
                PermissionDialog(request: request)
                    .id(request.id)
            }
        }
    }
}

/// The terminal's API permission modal: the local failure, the disabled APIs, and Retry,
/// Enable for this session, or Cancel.
private struct PermissionDialog: View {
    let request: PermissionRequest
    @Environment(AppModel.self) private var model
    @State private var choice = 0

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Use a public API for this request?")
                .font(.title3.weight(.semibold))
            Text("The \(chainName(request.chain)) node cannot answer the request \(request.method).")
            Note(text: request.failure, warning: true)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
            if request.endpoints.count == 1 {
                LabeledContent("API", value: request.endpoints[0])
            } else {
                Picker("API", selection: $choice) {
                    ForEach(Array(request.endpoints.enumerated()), id: \.offset) { index, endpoint in
                        Text(endpoint).tag(index)
                    }
                }
                .pickerStyle(.radioGroup)
            }
            Text(
                "If you enable the API, the app sends this request and later requests for this chain to it until the app quits. The operator of the API can see what you look up. Sources can save the setting."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button("Cancel", role: .cancel) { model.answerPermission(.cancel) }
                    .keyboardShortcut(.cancelAction)
                Spacer()
                Button("Try the Node Again") { model.answerPermission(.retry) }
                Button("Enable API") { model.answerPermission(.enable(index: UInt32(choice))) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(request.endpoints.isEmpty)
            }
        }
        .padding(20)
        .frame(width: 520)
    }
}

/// One line that says where a page's data came from. For a retained observation it says
/// when the source reported it and why no source answered now.
struct ProvenanceNote: View {
    let provenance: Provenance

    var body: some View {
        if provenance.retained {
            VStack(alignment: .leading, spacing: 4) {
                Note(
                    text: "Stored observation from \(provenance.source), reported \(Format.date(provenance.observedAt)). No source answered now, so the page can be out of date.",
                    warning: true)
                if let failure = provenance.failure {
                    Text(failure).font(.caption).foregroundStyle(.secondary)
                }
                if let caution = provenance.caution {
                    Text(caution).font(.caption).foregroundStyle(.secondary)
                }
            }
        } else {
            Text("From \(provenance.source)")
                .font(.caption)
                .foregroundStyle(.secondary)
                .help("Answered at \(Format.date(provenance.observedAt))")
        }
    }
}
