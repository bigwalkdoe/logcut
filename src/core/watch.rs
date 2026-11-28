//! Live child-process watch with a bounded ring buffer and crash reports.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use regex::Regex;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc;

use crate::core::models::{CrashReport, SliceLogRequest, WatchRequest};

/// Severity-style tokens and traceback headers that indicate a crash even
/// when the child exits successfully. Bare substrings such as `error` are not
/// enough: `0 errors` and `error-free` are routine successful output.
static CRASH_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(^|[\s\[({])(panic|fatal|error|assertionerror|segmentation fault)([\]\s:,.;!?]|$)|traceback \(most recent call last\)|"level"\s*:\s*"(error|fatal)""#,
    )
    .expect("crash line regex")
});

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn last_crash_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".logcut/last-crash.md")
}

pub fn read_last_crash(workspace_root: &Path) -> Result<String> {
    let path = last_crash_path(workspace_root);
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => bail!(
            "no saved crash report at {}; run `logcut watch` to capture one",
            path.display()
        ),
        Err(error) => {
            Err(error).with_context(|| format!("failed to read crash report at {}", path.display()))
        }
    }
}

/// Which of the child's streams produced a line.
#[derive(Debug, Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

/// Callback used by the CLI to echo streamed child lines.
pub type LineSink<'a> = &'a mut dyn FnMut(bool, &str);

type LineMessage = (Stream, String);
type LineChannel = (mpsc::Sender<LineMessage>, mpsc::Receiver<LineMessage>);

/// Backpressured queue between the child's reader tasks and the drain loop.
fn line_channel(capacity: usize) -> LineChannel {
    mpsc::channel(capacity.max(1))
}

/// Run `request.command`, buffering its output and optionally forwarding each
/// line to the caller's sink. Printing stays in the CLI transport; the core
/// never writes to stdout or stderr itself.
pub async fn watch(
    request: WatchRequest,
    mut on_line: Option<LineSink<'_>>,
) -> Result<CrashReport> {
    let (program, args) = request
        .command
        .split_first()
        .context("empty watch command: provide a command and its arguments")?;
    let ring_size = if request.ring_lines == 0 {
        500
    } else {
        request.ring_lines
    };

    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to spawn `{}`", program))?;

    let stdout = child.stdout.take().context("child stdout was not piped")?;
    let stderr = child.stderr.take().context("child stderr was not piped")?;

    let (tx, mut rx) = line_channel(ring_size);

    let stdout_task = tokio::spawn({
        let tx = tx.clone();
        async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Some(line) = lines.next_line().await? {
                if tx.send((Stream::Stdout, line)).await.is_err() {
                    break;
                }
            }
            Ok::<_, std::io::Error>(())
        }
    });
    let stderr_task = tokio::spawn({
        let tx = tx.clone();
        async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Some(line) = lines.next_line().await? {
                if tx.send((Stream::Stderr, line)).await.is_err() {
                    break;
                }
            }
            Ok::<_, std::io::Error>(())
        }
    });
    // Both stream tasks hold a sender; dropping ours lets the drain loop end
    // as soon as they have forwarded every line.
    drop(tx);

    let mut ring: VecDeque<String> = VecDeque::with_capacity(ring_size);
    let mut suspicious_line = false;

    while let Some((stream, line)) = rx.recv().await {
        if let Some(sink) = on_line.as_mut() {
            sink(matches!(stream, Stream::Stderr), &line);
        }
        if CRASH_LINE.is_match(&line) {
            suspicious_line = true;
        }
        ring.push_back(line);
        while ring.len() > ring_size {
            ring.pop_front();
        }
    }

    stdout_task.await??;
    stderr_task.await??;
    let status = child.wait().await?;
    let exit_code = status.code();

    let reason = if suspicious_line {
        Some("crash line".to_owned())
    } else if !status.success() {
        Some("non-zero exit".to_owned())
    } else {
        None
    };

    if reason.is_none() {
        return Ok(CrashReport {
            text: String::new(),
            path: String::new(),
            exit_code,
            detected: false,
            reason: None,
        });
    }

    let joined = ring.into_iter().collect::<Vec<_>>().join("\n");
    let sliced = crate::core::slice_log(SliceLogRequest {
        raw_text: Some(joined),
        workspace_root: Some(request.workspace_root.clone()),
        max_chars: Some(request.max_chars),
        ..Default::default()
    })?;
    let path = last_crash_path(Path::new(&request.workspace_root));
    write_atomically(&request.workspace_root, &path, &sliced.text)?;

    Ok(CrashReport {
        text: sliced.text,
        path: path.display().to_string(),
        exit_code,
        detected: true,
        reason,
    })
}

/// Persist a crash report without ever leaving a partial file behind.
fn write_atomically(workspace_root: &str, destination: &Path, text: &str) -> Result<()> {
    let dir = Path::new(workspace_root).join(".logcut");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(".gitignore"), "*")?;

    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let temp = dir.join(format!(
        ".last-crash.md.tmp-{}-{sequence}",
        std::process::id()
    ));

    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{last_crash_path, line_channel, read_last_crash, watch};
    use crate::core::models::WatchRequest;

    #[tokio::test]
    async fn non_zero_exit_and_error_line_writes_report() {
        let dir = tempfile::tempdir().unwrap();
        let workspace_root = dir.path().to_str().unwrap().to_owned();
        let report = watch(
            WatchRequest {
                command: vec!["sh".into(), "-c".into(), "echo 'ERROR boom'; exit 3".into()],
                workspace_root,
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

        assert!(report.detected);
        assert_eq!(report.exit_code, Some(3));
        assert_eq!(report.reason.as_deref(), Some("crash line"));

        let saved = std::fs::read_to_string(last_crash_path(dir.path())).unwrap();
        assert!(saved.contains("ERROR boom"), "saved report: {saved}");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".logcut/.gitignore")).unwrap(),
            "*"
        );
        assert_eq!(read_last_crash(dir.path()).unwrap(), saved);
        assert_eq!(report.text, saved);
    }

    #[tokio::test]
    async fn successful_command_leaves_no_crash_file() {
        let dir = tempfile::tempdir().unwrap();
        let report = watch(
            WatchRequest {
                command: vec!["sh".into(), "-c".into(), "echo ok".into()],
                workspace_root: dir.path().to_str().unwrap().to_owned(),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

        assert!(!report.detected);
        assert!(report.text.is_empty());
        assert!(report.reason.is_none());
        assert_eq!(report.exit_code, Some(0));
        assert!(!last_crash_path(dir.path()).exists());
    }

    #[tokio::test]
    async fn benign_error_word_with_zero_exit_leaves_no_crash_file() {
        let dir = tempfile::tempdir().unwrap();
        let report = watch(
            WatchRequest {
                command: vec![
                    "sh".into(),
                    "-c".into(),
                    "echo 'found 0 errors; all error-free'; exit 0".into(),
                ],
                workspace_root: dir.path().to_str().unwrap().to_owned(),
                ..Default::default()
            },
            None,
        )
        .await
        .unwrap();

        assert!(!report.detected, "{report:?}");
        assert!(report.reason.is_none());
        assert_eq!(report.exit_code, Some(0));
        assert!(!last_crash_path(dir.path()).exists());
    }

    #[test]
    fn line_channel_is_bounded() {
        let (tx, _rx) = line_channel(500);
        assert_eq!(tx.capacity(), 500);
    }

    #[tokio::test]
    async fn empty_command_bails() {
        let error = watch(WatchRequest::default(), None).await.unwrap_err();
        assert!(
            error.to_string().contains("empty watch command"),
            "unexpected error: {error:#}"
        );
    }
}
