import AppKit

/// A developer aid for the pictures in the documentation. When the environment variable
/// `CARBIDE_PICTURE_DIR` names a folder, the signal SIGUSR1 saves a picture of the main
/// window in that folder. The app draws its own window, so macOS asks for no permission.
/// Without the variable this does nothing.
enum WindowPicture {
    nonisolated(unsafe) private static var source: DispatchSourceSignal?
    nonisolated(unsafe) private static var count = 0

    static func arm() {
        guard let folder = ProcessInfo.processInfo.environment["CARBIDE_PICTURE_DIR"], !folder.isEmpty else { return }
        signal(SIGUSR1, SIG_IGN)
        let signals = DispatchSource.makeSignalSource(signal: SIGUSR1, queue: .main)
        signals.setEventHandler {
            MainActor.assumeIsolated { save(to: folder) }
        }
        signals.resume()
        source = signals
    }

    @MainActor
    private static func save(to folder: String) {
        let windows = NSApp.windows.filter { $0.isVisible && $0.styleMask.contains(.titled) }
        guard let window = windows.max(by: { $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height }),
            let view = window.contentView?.superview ?? window.contentView,
            let picture = view.bitmapImageRepForCachingDisplay(in: view.bounds)
        else { return }
        view.cacheDisplay(in: view.bounds, to: picture)
        guard let data = picture.representation(using: .png, properties: [:]) else { return }
        count += 1
        let file = URL(fileURLWithPath: folder).appendingPathComponent("picture-\(count).png")
        try? data.write(to: file)
    }
}
