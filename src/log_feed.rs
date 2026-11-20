use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use anyhow::Result;

const INITIAL_TAIL_BYTES: u64 = 128 * 1024;

#[derive(Debug)]
pub struct LogChunk {
    pub text: String,
    pub offset: u64,
    pub reset: bool,
}

pub fn daemon_log_path(config_file: Option<&Path>) -> Option<PathBuf> {
    config_file?
        .parent()
        .map(|directory| directory.join("debug.log"))
}

pub fn read_chunk(path: &Path, offset: Option<u64>) -> Result<LogChunk> {
    let length = fs::metadata(path)?.len();
    let (start, reset) = match offset {
        Some(offset) if offset <= length => (offset, false),
        Some(_) => (length.saturating_sub(INITIAL_TAIL_BYTES), true),
        None => (length.saturating_sub(INITIAL_TAIL_BYTES), true),
    };
    let mut file = File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)?;
    if reset
        && start > 0
        && let Some(first_newline) = contents.iter().position(|byte| *byte == b'\n')
    {
        contents.drain(..=first_newline);
    }
    Ok(LogChunk {
        text: String::from_utf8_lossy(&contents).into_owned(),
        offset: length,
        reset,
    })
}
