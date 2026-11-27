//! The sounds of a key of the display: the pointer on it, a press and a release
//! (`ClickSound.swift`).
//!
//! The sounds are from Josh Comeau's Button Sounds (MIT licence; the licence is with the
//! files in `resources/sounds`): "halo-shift" for the press and the release, and
//! "mx-master-3", more quietly, for the pointer on a key. One file holds some recordings of
//! a press and of a release. A sound plays one of them at random, and not the one that it
//! played before, so that two sounds in sequence are not the same.
//!
//! On Linux the files are decoded once (rodio with Symphonia's MP3 decoder) and played
//! through the default output of ALSA, which PipeWire and PulseAudio also serve. One audio
//! thread owns the output. When the computer has no audio output, the app is silent and
//! says so once on standard error.

use std::sync::mpsc;
use std::thread;

use crate::settings::{self, key};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum File {
    Click,
    Touch,
}

#[derive(Clone, Copy, Debug)]
enum Cue {
    Warm,
    Press { file: File, volume: f32 },
    Release { file: File, volume: f32 },
}

// Sounds of an interface must be quiet. The pointer on a key is the quietest.
const CLICK_VOLUME: f32 = 0.32;
// The recordings of this file are less loud than those of the click. At this volume the
// pointer sound is a third as loud as a press.
const TOUCH_VOLUME: f32 = 0.28;

thread_local! {
    static SENDER: mpsc::Sender<Cue> = start();
}

fn enabled() -> bool {
    settings::bool(key::INTERFACE_SOUNDS, true)
}

fn send(cue: Cue) {
    if !enabled() {
        return;
    }
    SENDER.with(|sender| {
        let _ = sender.send(cue);
    });
}

/// Cuts the files at the start of the app, so that the first sound is not late.
pub fn warm() {
    send(Cue::Warm);
}

/// The press of a key.
pub fn press() {
    send(Cue::Press { file: File::Click, volume: CLICK_VOLUME });
}

/// The release of a key.
pub fn release() {
    send(Cue::Release { file: File::Click, volume: CLICK_VOLUME });
}

/// The press of a link: the sound of a key, less loud.
pub fn link() {
    send(Cue::Press { file: File::Click, volume: CLICK_VOLUME * 0.45 });
}

/// The pointer goes on a row of a list: the sound of the pointer on a key, less loud.
pub fn row() {
    send(Cue::Press { file: File::Touch, volume: TOUCH_VOLUME * 0.55 });
}

/// The pointer goes on a key.
pub fn hover() {
    send(Cue::Press { file: File::Touch, volume: TOUCH_VOLUME });
}

/// The recordings of one file.
#[derive(Default)]
struct Voice {
    presses: Vec<rodio::buffer::SamplesBuffer<f32>>,
    releases: Vec<rodio::buffer::SamplesBuffer<f32>>,
    last_press: Option<usize>,
    last_release: Option<usize>,
}

fn start() -> mpsc::Sender<Cue> {
    let (sender, receiver) = mpsc::channel::<Cue>();
    let _ = thread::Builder::new().name("sounds".into()).spawn(move || audio_thread(receiver));
    sender
}

fn audio_thread(receiver: mpsc::Receiver<Cue>) {
    let mut output: Option<(rodio::OutputStream, rodio::OutputStreamHandle)> = None;
    let mut failed = false;
    let mut click = Voice::default();
    let mut touch = Voice::default();
    let mut prepared = false;
    while let Ok(cue) = receiver.recv() {
        if failed {
            continue;
        }
        if !prepared {
            prepared = true;
            click = cut("halo-shift", 7);
            touch = cut("mx-master-3", 4);
        }
        if output.is_none() {
            match rodio::OutputStream::try_default() {
                Ok(stream) => output = Some(stream),
                Err(error) => {
                    eprintln!("The app cannot play its interface sounds. The audio output did not open: {error}");
                    failed = true;
                    continue;
                }
            }
        }
        let Some((_, handle)) = &output else { continue };
        let (voice, press, volume) = match cue {
            Cue::Warm => continue,
            Cue::Press { file, volume } => (file, true, volume),
            Cue::Release { file, volume } => (file, false, volume),
        };
        let voice = if voice == File::Click { &mut click } else { &mut touch };
        let (list, last) = if press {
            (&voice.presses, &mut voice.last_press)
        } else {
            (&voice.releases, &mut voice.last_release)
        };
        if list.is_empty() {
            continue;
        }
        let mut index = rand::random_range(0..list.len());
        if Some(index) == *last {
            index = (index + 1) % list.len();
        }
        *last = Some(index);
        use rodio::Source;
        let _ = handle.play_raw(list[index].clone().amplify(volume));
    }
}

/// Cuts a file into its recordings. In a file a press starts at each full second from 1
/// and is 200 ms long. Its release follows and is 800 ms long.
fn cut(name: &str, pairs: usize) -> Voice {
    use rodio::Source;
    let mut voice = Voice::default();
    let path = super::fonts::resources().join("sounds").join(format!("{name}.mp3"));
    let Ok(file) = std::fs::File::open(&path) else { return voice };
    let Ok(decoder) = rodio::Decoder::new(std::io::BufReader::new(file)) else { return voice };
    let channels = decoder.channels();
    let rate = decoder.sample_rate();
    let samples: Vec<f32> = decoder.convert_samples::<f32>().collect();
    let frame = |seconds: f64| ((seconds * rate as f64) as usize) * channels as usize;
    for pair in 1..=pairs {
        for (press, start, length) in [(true, pair as f64, 0.2), (false, pair as f64 + 0.2, 0.8)] {
            let first = frame(start).min(samples.len());
            let end = (first + frame(length)).min(samples.len());
            if end <= first {
                continue;
            }
            let piece = rodio::buffer::SamplesBuffer::new(channels, rate, samples[first..end].to_vec());
            if press {
                voice.presses.push(piece);
            } else {
                voice.releases.push(piece);
            }
        }
    }
    voice
}

#[cfg(test)]
mod tests {
    #[test]
    fn cuts_each_file_into_its_recordings() {
        let click = super::cut("halo-shift", 7);
        assert_eq!((click.presses.len(), click.releases.len()), (7, 7));
        let touch = super::cut("mx-master-3", 4);
        assert_eq!((touch.presses.len(), touch.releases.len()), (4, 4));
    }
}
