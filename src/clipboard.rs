//! Explicit-only clipboard writes. Never reads the local or remote clipboard.
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};

const MAX_BYTES: usize = 1_000_000;
pub const TEST_TEXT: &str = "Verus Explorer clipboard test";

pub struct PreparedCopy {
    sequence: Option<Vec<u8>>,
    route: &'static str,
}
impl std::fmt::Debug for PreparedCopy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedCopy")
            .field("route", &self.route)
            .finish_non_exhaustive()
    }
}
impl PreparedCopy {
    /// Direct terminal output belongs on the UI thread, between redraws.
    pub fn finish(self) -> Result<&'static str> {
        if let Some(sequence) = self.sequence {
            let mut output = io::stdout().lock();
            output.write_all(&sequence)?;
            output.flush()?;
        }
        Ok(self.route)
    }
}

fn osc52(value: &str) -> Vec<u8> {
    format!("\x1b]52;c;{}\x07", STANDARD.encode(value.as_bytes())).into_bytes()
}

/// Potential subprocess work must run outside the UI thread. Clipboard text is
/// passed on stdin, never through shell syntax, command arguments or temp files.
pub fn prepare(value: String) -> Result<PreparedCopy> {
    ensure!(
        value.len() <= MAX_BYTES,
        "Selected value exceeds the 1 MB clipboard limit; use s to save JSON"
    );
    let mode = std::env::var("VERUS_EXPLORER_CLIPBOARD").unwrap_or_else(|_| "auto".into());
    let in_tmux = std::env::var_os("TMUX").is_some_and(|value| !value.is_empty());
    let tmux = match mode.as_str() {
        "auto" => in_tmux,
        "osc52" => false,
        "tmux" => true,
        "off" => bail!("Clipboard is disabled by VERUS_EXPLORER_CLIPBOARD=off"),
        _ => bail!("VERUS_EXPLORER_CLIPBOARD must be auto, osc52, tmux or off"),
    };
    if !tmux {
        return Ok(PreparedCopy {
            sequence: Some(osc52(&value)),
            route: "OSC 52",
        });
    }
    let policy = tmux_command(&["show-options", "-sv", "set-clipboard"], None)?;
    ensure!(
        matches!(policy.trim(), "on" | "external"),
        "tmux clipboard forwarding is off; enable set-clipboard external before copying"
    );
    let pane = std::env::var("TMUX_PANE")
        .context("TMUX_PANE is missing; cannot target this pane's terminal clipboard")?;
    let clients = tmux_command(
        &[
            "list-clients",
            "-F",
            "#{client_tty}\t#{pane_id}\t#{client_termfeatures}",
        ],
        None,
    )?;
    let targets = clients
        .lines()
        .filter_map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            (fields.len() == 3 && fields[1] == pane).then_some(fields)
        })
        .collect::<Vec<_>>();
    ensure!(
        !targets.is_empty(),
        "No attached tmux client is displaying this pane; clipboard was not sent"
    );
    for target in &targets {
        ensure!(
            target[2].split(',').any(|feature| feature == "clipboard"),
            "tmux does not advertise clipboard support for this terminal; see --clipboard-test diagnostics"
        );
    }
    for target in targets {
        tmux_command(
            &[
                "load-buffer",
                "-w",
                "-b",
                "verus-explorer",
                "-t",
                target[0],
                "-",
            ],
            Some(value.as_bytes()),
        )?;
    }
    Ok(PreparedCopy {
        sequence: None,
        route: "tmux clipboard forwarding",
    })
}

fn tmux_command(args: &[&str], input: Option<&[u8]>) -> Result<String> {
    let mut child = Command::new("tmux")
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Cannot run tmux")?;
    let writer = input.map(|input| {
        let input = input.to_vec();
        let mut pipe = child.stdin.take().unwrap();
        std::thread::spawn(move || pipe.write_all(&input))
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let wrote = writer
        .map(|writer| {
            writer
                .join()
                .map_err(|_| anyhow::anyhow!("Clipboard writer stopped"))
        })
        .transpose()?;
    let status = status.context("tmux clipboard command timed out")?;
    let mut output = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .take(65536)
        .read_to_string(&mut output)?;
    if !status.success() {
        let mut error = String::new();
        child
            .stderr
            .take()
            .unwrap()
            .take(4096)
            .read_to_string(&mut error)?;
        bail!("tmux clipboard command failed: {}", error.trim());
    }
    if let Some(result) = wrote {
        result.context("Cannot send clipboard text to tmux")?;
    }
    Ok(output)
}

pub fn diagnose_and_test() -> Result<()> {
    println!("Clipboard test (writes a known test phrase; never reads clipboard contents)");
    println!("TERM: {:?}", std::env::var("TERM").unwrap_or_default());
    println!(
        "Mode: {}",
        std::env::var("VERUS_EXPLORER_CLIPBOARD").unwrap_or_else(|_| "auto".into())
    );
    if std::env::var_os("TMUX").is_some_and(|value| !value.is_empty()) {
        println!("tmux: {}", tmux_command(&["-V"], None)?.trim());
        println!(
            "set-clipboard: {}",
            tmux_command(&["show-options", "-sv", "set-clipboard"], None)?.trim()
        );
        let pane = std::env::var("TMUX_PANE").unwrap_or_default();
        let clients = tmux_command(
            &[
                "list-clients",
                "-F",
                "#{client_tty}\t#{pane_id}\t#{client_termfeatures}",
            ],
            None,
        )?;
        for line in clients.lines() {
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() == 3 && fields[1] == pane {
                println!("Attached client features: {}", fields[2]);
            }
        }
        println!("Auto mode uses tmux's own forwarding; allow-passthrough is not required.");
    } else {
        println!("No tmux detected; auto mode sends OSC 52 directly to the terminal.");
    }
    let result = prepare(TEST_TEXT.to_string()).and_then(PreparedCopy::finish);
    println!(
        "For Alacritty on your local machine: [terminal] osc52 = \"OnlyCopy\" permits clipboard writes."
    );
    println!(
        "If tmux lacks clipboard capability, configure terminal-features for its outside TERM (e.g. xterm-256color:clipboard)."
    );
    let route = result?;
    println!(
        "Sent test phrase via {route}. Paste into a local editor and verify exactly: {TEST_TEXT}"
    );
    println!("Writing to the terminal/server does not confirm the desktop clipboard received it.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_values_are_rejected_before_starting_a_transport() {
        assert!(
            prepare("x".repeat(MAX_BYTES + 1))
                .unwrap_err()
                .to_string()
                .contains("1 MB")
        );
    }

    #[test]
    fn osc52_encodes_utf8_without_injecting_control_sequences() {
        let value = "hello 🦉\n\x1b]52;c;untrusted\x07";
        let bytes = osc52(value);
        assert!(bytes.starts_with(b"\x1b]52;c;"));
        assert!(bytes.ends_with(b"\x07"));
        assert_eq!(
            STANDARD.decode(&bytes[7..bytes.len() - 1]).unwrap(),
            value.as_bytes()
        );
        assert_eq!(bytes.iter().filter(|byte| **byte == 0x1b).count(), 1);
    }
}
