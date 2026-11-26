import AVFoundation
import Foundation

/// The sounds of a key of the display: the pointer on it, a press and a release.
///
/// The sounds are from Josh Comeau's Button Sounds (MIT licence; the licence is with the
/// files in the app): "halo-shift" for the press and the release, and "mx-master-3", more
/// quietly, for the pointer on a key. One file holds some recordings of a press and of a
/// release. A sound plays one of them at random, and not the one that it played before, so
/// that two sounds in sequence are not the same.
@MainActor
final class ClickSound {
    static let shared = ClickSound()

    /// The setting that turns the sounds off.
    static let settingKey = "interfaceSounds"

    /// One file of recordings, cut into players.
    private final class Voice {
        let file: String
        let pairs: Int
        let volume: Float
        var presses: [AVAudioPlayer] = []
        var releases: [AVAudioPlayer] = []
        var lastPress = -1
        var lastRelease = -1
        var prepared = false

        init(file: String, pairs: Int, volume: Float) {
            self.file = file
            self.pairs = pairs
            self.volume = volume
        }
    }

    // Sounds of an interface must be quiet. The pointer on a key is the quietest.
    private let click = Voice(file: "halo-shift", pairs: 7, volume: 0.32)
    // The recordings of this file are less loud than those of the click. At this volume the
    // pointer sound is a third as loud as a press.
    private let touch = Voice(file: "mx-master-3", pairs: 4, volume: 0.28)

    private var enabled: Bool {
        UserDefaults.standard.object(forKey: Self.settingKey) as? Bool ?? true
    }

    /// Cuts the files at the start of the app, so that the first sound is not late.
    func warm() {
        guard enabled else { return }
        prepare(click)
        prepare(touch)
    }

    /// The press of a key.
    func press() {
        guard enabled else { return }
        prepare(click)
        play(click.presses, last: &click.lastPress, volume: click.volume)
    }

    /// The release of a key.
    func release() {
        guard enabled else { return }
        prepare(click)
        play(click.releases, last: &click.lastRelease, volume: click.volume)
    }

    /// The press of a link: the sound of a key, less loud.
    func link() {
        guard enabled else { return }
        prepare(click)
        play(click.presses, last: &click.lastPress, volume: click.volume * 0.45)
    }

    /// The pointer goes on a row of a list: the sound of the pointer on a key, less loud.
    func row() {
        guard enabled else { return }
        prepare(touch)
        play(touch.presses, last: &touch.lastPress, volume: touch.volume * 0.55)
    }

    /// The pointer goes on a key.
    func hover() {
        guard enabled else { return }
        prepare(touch)
        play(touch.presses, last: &touch.lastPress, volume: touch.volume)
    }

    private func play(_ players: [AVAudioPlayer], last: inout Int, volume: Float) {
        guard !players.isEmpty else { return }
        var index = Int.random(in: 0..<players.count)
        if index == last { index = (index + 1) % players.count }
        last = index
        let player = players[index]
        player.volume = volume
        player.currentTime = 0
        player.play()
    }

    /// Cuts a file into its recordings, one time. In a file a press starts at each full
    /// second from 1 and is 200 ms long. Its release follows and is 800 ms long.
    private func prepare(_ voice: Voice) {
        guard !voice.prepared else { return }
        voice.prepared = true
        guard let source = Self.sourceFile(voice.file), let file = try? AVAudioFile(forReading: source) else { return }
        let format = file.processingFormat
        let rate = format.sampleRate
        let folder = FileManager.default.temporaryDirectory.appendingPathComponent("CarbideSilicaSounds", isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        for pair in 1...voice.pairs {
            for (press, start, length) in [(true, Double(pair), 0.2), (false, Double(pair) + 0.2, 0.8)] {
                let first = AVAudioFramePosition(start * rate)
                let count = min(AVAudioFrameCount(length * rate), AVAudioFrameCount(max(file.length - first, 0)))
                guard count > 0, let buffer = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: count) else { continue }
                file.framePosition = first
                guard (try? file.read(into: buffer, frameCount: count)) != nil, buffer.frameLength > 0 else { continue }
                let piece = folder.appendingPathComponent("\(voice.file)-\(press ? "press" : "release")-\(pair).caf")
                try? FileManager.default.removeItem(at: piece)
                guard let out = try? AVAudioFile(forWriting: piece, settings: format.settings),
                    (try? out.write(from: buffer)) != nil
                else { continue }
                guard let player = try? AVAudioPlayer(contentsOf: piece) else { continue }
                player.volume = voice.volume
                player.prepareToPlay()
                if press { voice.presses.append(player) } else { voice.releases.append(player) }
            }
        }
    }

    /// A sound file in the app, or next to the sources when the app runs with no bundle.
    private static func sourceFile(_ name: String) -> URL? {
        if let inApp = Bundle.main.url(forResource: name, withExtension: "mp3", subdirectory: "Sounds") {
            return inApp
        }
        let nearSources = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Resources/Sounds/\(name).mp3")
        return FileManager.default.fileExists(atPath: nearSources.path) ? nearSources : nil
    }
}

import SwiftUI

/// A button with no form of its own that gives the sound of a key: the press when the
/// pointer goes down on it, and the release when the pointer comes up.
struct PressSoundStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .onChange(of: configuration.isPressed) { _, pressed in
                if pressed { ClickSound.shared.press() } else { ClickSound.shared.release() }
            }
    }
}
