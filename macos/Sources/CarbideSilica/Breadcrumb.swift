import CarbideCore
import SwiftUI

/// The trail from the place in the sidebar to the open page, as the terminal's path line.
/// Each step is a link that goes back to it. The pages below stay in the navigation stack,
/// so going back restores their selection and scroll position.
struct Breadcrumb: View {
    @Environment(AppModel.self) private var model
    @AppStorage(DisplayKey.showsNames) private var showsNames = true

    var body: some View {
        HStack(spacing: 4) {
            crumb(placeTitle, depth: 0)
            ForEach(Array(model.path.enumerated()), id: \.offset) { index, route in
                Image(systemName: "chevron.compact.right")
                    .foregroundStyle(.tertiary)
                    .accessibilityHidden(true)
                crumb(route.crumbTitle(names: showsNames ? model.names : [:]), depth: index + 1)
            }
            Spacer(minLength: 0)
        }
        .font(.callout)
        .lineLimit(1)
        .padding(.horizontal, 16)
        .padding(.vertical, 5)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.bar)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Path")
    }

    @ViewBuilder
    private func crumb(_ title: String, depth: Int) -> some View {
        if depth == model.path.count {
            Text(title).truncationMode(.middle)
        } else {
            Button(title) { model.path = Array(model.path.prefix(depth)) }
                .buttonStyle(.link)
                .truncationMode(.middle)
        }
    }

    private var placeTitle: String {
        SidebarPlace.all.first { $0.item == model.sidebar }?.title ?? "Carbide Silica"
    }
}

extension Route {
    /// Only objects that Carbide's bookmark file can hold.
    var isBookmarkable: Bool {
        switch self {
        case .block, .transaction, .identity, .currency, .address: true
        case .currencies, .estimate: false
        }
    }

    /// A short label for the path line. `names` is empty when names are turned off.
    func crumbTitle(names: [String: String]) -> String {
        switch self {
        case .identity(let name):
            return names[name] ?? title
        case .currency(let name):
            return names[name].map { $0.hasSuffix("@") ? String($0.dropLast()) : $0 } ?? title
        default:
            return title
        }
    }
}
