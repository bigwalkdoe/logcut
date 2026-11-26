//! Bounded readers for files, raw text, Docker and journald.
//!
//! No shell is involved: external commands are spawned with explicit argument
//! arrays, so container and unit names can never be interpreted as shell code.

use std::cmp::min;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use crate::core::models::{SliceContainerRequest, SliceServiceRequest};

const CHUNK: usize = 8192;

/// Keep only the last `tail_lines` lines of in-memory text.
pub fn tail_text(text: &str, tail_lines: usize) -> String {
    if tail_lines == 0 {
        return String::new();
    }
    let trailing_newline = text.ends_with('\n');
    let mut kept: Vec<&str> = text.lines().rev().take(tail_lines).collect();
    kept.reverse();
    let mut kept_text = kept.join("\n");
    if trailing_newline && !kept.is_empty() {
        kept_text.push('\n');
    }
    kept_text
}

/// Read a file backwards in bounded chunks, returning at most `tail_lines`.
///
/// The reader never seeks from the beginning and never buffers the whole file.
pub fn read_log_tail(path: &Path, tail_lines: usize) -> Result<String> {
    if tail_lines == 0 {
        return Ok(String::new());
    }

    let mut file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let length = file.seek(SeekFrom::End(0))? as usize;
    if length == 0 {
        return Ok(String::new());
    }

    file.seek(SeekFrom::End(-1))?;
    let mut last_byte = [0u8; 1];
    file.read_exact(&mut last_byte)?;
    let ends_with_newline = last_byte[0] == b'\n';
    let needed = tail_lines + usize::from(ends_with_newline);

    let mut collected = Vec::new();
    let mut position = length;
    let mut newlines = 0;
    while position > 0 && newlines < needed {
        let take = min(CHUNK, position);
        position -= take;
        file.seek(SeekFrom::Start(position as u64))?;
        let mut chunk = vec![0u8; take];
        file.read_exact(&mut chunk)?;
        for byte in chunk.into_iter().rev() {
            collected.push(byte);
            if byte == b'\n' {
                newlines += 1;
                if newlines >= needed {
                    break;
                }
            }
        }
    }

    collected.reverse();
    let text = String::from_utf8_lossy(&collected).into_owned();
    if newlines < needed {
        // The whole file is shorter than the requested tail.
        return Ok(text);
    }
    // The window ends at a boundary newline; discard the partial line before it.
    Ok(text.find('\n').map_or(text.clone(), |index| text[index + 1..].to_owned()))
}

/// Argument array for `docker logs` — never a shell command string.
pub fn docker_args(request: &SliceContainerRequest) -> Vec<String> {
    let mut args = vec![
        "logs".to_owned(),
        "--tail".to_owned(),
        request.tail_lines.unwrap_or(2000).to_string(),
    ];
    if let Some(since) = &request.since {
        args.push("--since".to_owned());
        args.push(since.clone());
    }
    args.push(request.container.clone());
    args
}

/// Argument array for `journalctl` — never a shell command string.
pub fn journal_args(request: &SliceServiceRequest) -> Vec<String> {
    vec![
        "-u".to_owned(),
        request.unit.clone(),
        "-n".to_owned(),
        request.lines.unwrap_or(1000).to_string(),
        "--no-pager".to_owned(),
    ]
}

/// Run `docker logs` and combine its stdout and stderr into one log stream.
pub async fn docker_logs(request: &SliceContainerRequest) -> Result<String> {
    let output = Command::new("docker")
        .args(docker_args(request))
        .output()
        .await
        .context("spawn docker logs")?;
    if !output.status.success() {
        bail!(
            "docker logs failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

/// Run `journalctl` and return the requested service log stream.
pub async fn journal_logs(request: &SliceServiceRequest) -> Result<String> {
    let output = Command::new("journalctl")
        .args(journal_args(request))
        .output()
        .await
        .context("spawn journalctl")?;
    if !output.status.success() {
        bail!(
            "journalctl failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::tail_text;

    #[test]
    fn zero_tail_is_empty() {
        assert_eq!(tail_text("a\nb\n", 0), "");
    }
}
