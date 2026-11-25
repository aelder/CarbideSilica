import SwiftUI

/// A future block height or moment, opened from search (`estimate:`, or a height or date
/// beyond the tip). The page holds the same estimator as the Charts screen.
struct EstimateView: View {
    /// An estimator target: a height, or `unix:` and seconds.
    let query: String

    var body: some View {
        Page {
            BlockEstimator(query: query).id(query)
        }
        .navigationTitle("Estimate")
    }
}
