use std::io::{BufWriter, Write};
use std::time::{Duration, Instant};

/// Optional diagnostics. The UI sends only static stage names and durations.
#[derive(Default)]
pub(super) struct Timing(Option<std::sync::mpsc::SyncSender<Sample>>);

struct Sample {
    stage: &'static str,
    elapsed: Duration,
}

impl Timing {
    pub(super) fn open() -> anyhow::Result<Self> {
        let Some(path) = std::env::var_os("VCARBIDE_UI_TIMING") else {
            return Ok(Self::default());
        };
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut output = BufWriter::new(options.open(path)?);
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Sample>(256);
        std::thread::Builder::new()
            .name("carbide-ui-timing".into())
            .spawn(move || {
                while let Ok(sample) = receiver.recv() {
                    if writeln!(output, "{} {}us", sample.stage, sample.elapsed.as_micros())
                        .and_then(|()| output.flush())
                        .is_err()
                    {
                        break;
                    }
                }
            })?;
        Ok(Self(Some(sender)))
    }

    pub(super) fn record(&self, stage: &'static str, elapsed: Duration) {
        if let Some(sender) = &self.0 {
            submit(sender, stage, elapsed);
        }
    }

    pub(super) fn measure(&self, stage: &'static str) -> Measurement<'_> {
        Measurement(
            self.0
                .as_ref()
                .map(|sender| (sender, stage, Instant::now())),
        )
    }
}

pub(super) struct Measurement<'a>(
    Option<(
        &'a std::sync::mpsc::SyncSender<Sample>,
        &'static str,
        Instant,
    )>,
);

impl Drop for Measurement<'_> {
    fn drop(&mut self) {
        if let Some((sender, stage, started)) = self.0 {
            submit(sender, stage, started.elapsed());
        }
    }
}

fn submit(sender: &std::sync::mpsc::SyncSender<Sample>, stage: &'static str, elapsed: Duration) {
    let minimum = match stage {
        "input_dispatch" | "input_queue" => Duration::ZERO,
        "terminal_wait" => Duration::from_millis(80),
        _ => Duration::from_millis(20),
    };
    if elapsed >= minimum {
        let _ = sender.try_send(Sample { stage, elapsed });
    }
}
