use std::fs;
use std::io::{BufRead, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde_json::Value;

use logcut::core::dedup::{format_group, group_lines, normalize_line};
use logcut::core::models::{
    CompilerKind, FrameKind, SliceBuildArgs, SliceContainerRequest, SliceLogRequest,
    SliceServiceRequest, StackFrame, StackLanguage,
};
use logcut::core::redactor::redact;
use logcut::core::source_ctx::{render_snippet, resolve_frame};
use logcut::core::sources::{docker_args, journal_args, read_log_tail, tail_text};
use logcut::core::stacktrace::{collapse_vendor_frames, parse_stacktrace, render_stacktrace};
use logcut::core::{slice_build, slice_log};

fn fixture(name: &str) -> String {
    fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("fixture {name}: {error}"))
}

// ---------------------------------------------------------------- Phase 1

#[test]
fn redactor_masks_bearer_tokens() {
    let line = "GET /api HTTP/1.1 Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.abcdefghijkl";

    let redacted = redact(line);

    assert!(!redacted.contains("eyJhbGciOiJIUzI1NiJ9"), "{redacted}");
    assert!(redacted.contains("Bearer <REDACTED>"), "{redacted}");
}

#[test]
fn redactor_masks_api_keys_and_aws_keys() {
    let line = "keys sk-abcdefghijklmnopqrstuvwxyz123456 ghp_abcdefghijklmnopqrstuvwxyz0123456789 AKIAIOSFODNN7EXAMPLE";

    let redacted = redact(line);

    for secret in [
        "sk-abcdefghijklmnopqrstuvwxyz123456",
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        "AKIAIOSFODNN7EXAMPLE",
    ] {
        assert!(!redacted.contains(secret), "leaked {secret}: {redacted}");
    }
    assert!(redacted.contains("<REDACTED>"), "{redacted}");
}

#[test]
fn redactor_masks_database_urls_but_keeps_host() {
    let line =
        "dsn=postgres://deploy:s3cr3t@db.internal:5432/app mysql://root:pw@127.0.0.1:3306/db";

    let redacted = redact(line);

    assert!(!redacted.contains("s3cr3t"), "{redacted}");
    assert!(!redacted.contains(":pw@"), "{redacted}");
    assert!(
        redacted.contains("postgres://<REDACTED>@db.internal:5432/app"),
        "{redacted}"
    );
    assert!(
        redacted.contains("mysql://<REDACTED>@127.0.0.1:3306/db"),
        "{redacted}"
    );
}

#[test]
fn redactor_leaves_ordinary_log_text_alone() {
    let line = "2026-11-27T02:14:01Z INFO server listening on 0.0.0.0:8080";

    assert_eq!(redact(line), line);
}

// ---------------------------------------------------------------- Phase 2

#[test]
fn tool_requests_share_json_schema_with_mcp() {
    let schema = serde_json::to_value(schemars::schema_for!(SliceLogRequest)).unwrap();

    assert!(schema["properties"]["tail_lines"].is_object());
    assert!(schema["properties"]["workspace_root"].is_object());

    let build_schema = serde_json::to_value(schemars::schema_for!(SliceBuildArgs)).unwrap();
    assert!(build_schema["properties"]["max_errors"].is_object());
    assert!(build_schema["properties"]["raw_text"].is_object());
}

fn assert_trace(name: &str, root: &Path, expected_vendor: usize, app_marker: &str) {
    let text = fixture(name);
    let frames = parse_stacktrace(&text, root, false);
    let vendor = frames
        .iter()
        .filter(|frame| frame.kind == FrameKind::Vendor)
        .count();
    let app = frames
        .iter()
        .filter(|frame| frame.kind == FrameKind::App)
        .count();

    assert_eq!(vendor, expected_vendor, "{name}: {frames:#?}");
    assert!(app > 0, "{name}: no app frames: {frames:#?}");

    let collapsed = collapse_vendor_frames(&frames);
    assert!(
        collapsed.contains(&format!(
            "... [{expected_vendor} framework/runtime frames hidden] ..."
        )),
        "{name}: {collapsed}"
    );
    assert!(collapsed.contains(app_marker), "{name}: {collapsed}");
}

#[test]
fn rust_trace_collapses_three_vendor_frames() {
    assert_trace("rust_panic.txt", Path::new("/app"), 3, "src/handler.rs:42");
}

#[test]
fn python_trace_collapses_site_packages_and_stdlib_frames() {
    assert_trace(
        "python_traceback.txt",
        Path::new("/app"),
        2,
        "src/worker.py:88",
    );
}

#[test]
fn node_trace_collapses_node_modules_and_internal_frames() {
    assert_trace(
        "node_traceback.txt",
        Path::new("/app"),
        2,
        "src/users.ts:24",
    );
}

#[test]
fn go_panic_collapses_runtime_and_http_frames() {
    assert_trace("go_panic.txt", Path::new("/app"), 2, "src/server.go:57");
}

#[test]
fn java_trace_collapses_jdk_and_spring_frames() {
    assert_trace(
        "java_traceback.txt",
        Path::new("/app"),
        2,
        "UserController.java:42",
    );
}

#[test]
fn include_vendor_frames_keeps_every_frame() {
    let text = fixture("rust_panic.txt");
    let collapsed = render_stacktrace(&text, Path::new("/app"), false);
    let rendered = render_stacktrace(&text, Path::new("/app"), true);

    assert!(!collapsed.contains("/rustc/a1b2c3/library/std/src/panicking.rs:690"));
    assert!(collapsed.contains("frames hidden"));
    assert!(rendered.contains("/rustc/a1b2c3/library/std/src/panicking.rs:690"));
    assert!(rendered.contains("src/handler.rs:42"));
    assert!(!rendered.contains("frames hidden"));
}

#[test]
fn dedup_normalizes_volatile_tokens() {
    let normalized =
        normalize_line("2026-11-27T02:14:02Z ERROR 0xDEADBEEF req_abc123 done in 812ms");

    assert_eq!(normalized, "<TS> ERROR <HEX> <REQID> done in <DUR>");
}

#[test]
fn dedup_groups_error_burst_with_first_and_last_timestamps() {
    let lines: Vec<String> = fixture("noisy_json.log")
        .lines()
        .map(str::to_owned)
        .collect();

    let groups = group_lines(&lines);
    let connection = groups
        .iter()
        .find(|group| group.template.contains("Connection refused"))
        .expect("connection burst grouped");

    assert_eq!(connection.count, 3);
    assert_eq!(connection.first.as_deref(), Some("02:14:02"));
    assert_eq!(connection.last.as_deref(), Some("02:14:58"));
    assert_eq!(
        format_group(connection),
        "[x3 | first: 02:14:02 | last: 02:14:58] ERROR Connection refused to <UUID>"
    );
}

#[test]
fn source_context_inlines_lines_around_crash_site() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("app.rs"),
        (1..=10)
            .map(|line| format!("line {line}"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();

    let frame = StackFrame {
        language: StackLanguage::Rust,
        kind: FrameKind::App,
        path: "./src/app.rs".into(),
        line: Some(5),
        column: Some(1),
        function: "app::crash".into(),
        raw: "at ./src/app.rs:5:1".into(),
    };

    let snippet = resolve_frame(dir.path(), &frame, 2).expect("snippet");
    assert_eq!(
        (snippet.start_line, snippet.end_line, snippet.target_line),
        (3, 7, 5)
    );
    assert_eq!(snippet.text.lines().count(), 5);
    assert!(render_snippet(&snippet).contains("> 5 | line 5"));

    let absolute = StackFrame {
        path: src.join("app.rs").to_string_lossy().into_owned(),
        ..frame
    };
    assert!(resolve_frame(dir.path(), &absolute, 1).is_some());
}

// ---------------------------------------------------------------- Phase 3

#[test]
fn tail_text_keeps_only_the_last_lines() {
    assert_eq!(tail_text("a\nb\nc\nd\n", 2), "c\nd\n");
}

#[test]
fn file_tail_read_never_loads_the_whole_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("big.log");
    fs::write(
        &path,
        (1..=5000)
            .map(|line| format!("line {line}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let tail = read_log_tail(&path, 10).unwrap();

    assert_eq!(tail.lines().count(), 10);
    assert!(tail.contains("line 5000"));
    assert!(!tail.contains("line 1\n"));
}

#[test]
fn small_file_tail_still_honors_tail_lines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("small.log");

    fs::write(&path, "line 1\nline 2\nline 3\nline 4\n").unwrap();
    assert_eq!(read_log_tail(&path, 2).unwrap(), "line 3\nline 4\n");

    fs::write(&path, "line 1\nline 2\nline 3").unwrap();
    assert_eq!(read_log_tail(&path, 2).unwrap(), "line 2\nline 3");
}

#[test]
fn external_commands_are_built_as_argument_arrays() {
    let container = SliceContainerRequest {
        container: "web; rm -rf /".into(),
        tail_lines: Some(50),
        since: Some("1h".into()),
        ..Default::default()
    };
    assert_eq!(
        docker_args(&container),
        ["logs", "--tail", "50", "--since", "1h", "web; rm -rf /"]
    );

    let service = SliceServiceRequest {
        unit: "api.service".into(),
        lines: Some(20),
        ..Default::default()
    };
    assert_eq!(
        journal_args(&service),
        ["-u", "api.service", "-n", "20", "--no-pager"]
    );
}

#[test]
fn slice_log_redacts_secrets_and_groups_error_bursts() {
    let report = slice_log(SliceLogRequest {
        raw_text: Some(fixture("noisy_json.log")),
        workspace_root: Some("/app".into()),
        ..Default::default()
    })
    .unwrap();

    assert!(!report.text.contains("s3cr3t"), "{}", report.text);
    assert!(
        !report
            .text
            .contains("sk-test-abcdefghijklmnopqrstuvwxyz123456"),
        "{}",
        report.text
    );
    assert!(report.text.contains("[x3"), "{}", report.text);
    assert!(report.text.contains("<UUID>"), "{}", report.text);
    assert_eq!(report.stats.raw_bytes, fixture("noisy_json.log").len());
    assert!(report.stats.sliced_bytes < report.stats.raw_bytes);
    assert!(report.stats.reduction_percent > 0.0);
    assert!(report.stats.estimated_tokens_saved > 0);
}

#[test]
fn slice_log_collapses_vendor_frames_and_inlines_source() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("handler.rs"),
        (1..=60)
            .map(|line| {
                if line == 42 {
                    "let conn = connect();".to_owned()
                } else {
                    format!("// line {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();

    let report = slice_log(SliceLogRequest {
        raw_text: Some(fixture("rust_panic.txt")),
        workspace_root: Some(dir.path().to_string_lossy().into_owned()),
        ..Default::default()
    })
    .unwrap();

    assert!(
        report.text.contains("framework/runtime frames hidden"),
        "{}",
        report.text
    );
    assert!(report.text.contains("src/handler.rs:42"), "{}", report.text);
    assert!(
        report.text.contains("> 42 | let conn = connect();"),
        "{}",
        report.text
    );
    assert_eq!(report.frames.len(), 5);
    assert!(!report.snippets.is_empty());
}

#[test]
fn slice_log_requires_an_input_source() {
    let error = slice_log(SliceLogRequest::default()).unwrap_err();
    assert!(error.to_string().contains("path"), "{error}");
}

#[test]
fn slice_log_respects_tail_and_max_chars_limits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("many.log");
    fs::write(
        &path,
        (1..=5000)
            .map(|line| format!("ERROR failure number {line}\n"))
            .collect::<String>(),
    )
    .unwrap();

    let report = slice_log(SliceLogRequest {
        path: Some(path.to_string_lossy().into_owned()),
        tail_lines: Some(5),
        max_chars: Some(80),
        ..Default::default()
    })
    .unwrap();

    assert!(report.stats.raw_bytes < 300, "{}", report.stats.raw_bytes);
    assert!(report.text.len() <= 80, "{}", report.text);
}

#[test]
fn mcp_stdio_handshake_initializes_and_lists_tools() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_logcut"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn logcut mcp");

    let mut stdin = child.stdin.take().expect("stdin");
    let reader = std::io::BufReader::new(child.stdout.take().expect("stdout"));
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });

    let next_message = || -> Value {
        let line = receiver
            .recv_timeout(Duration::from_secs(15))
            .expect("MCP response timed out");
        serde_json::from_str(line.trim()).expect("stdout must contain only JSON-RPC messages")
    };

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":"2025-06-18","capabilities":{{}},"clientInfo":{{"name":"logcut-test","version":"0.1.0"}}}}}}"#
    )
    .unwrap();
    let initialized = next_message();
    assert_eq!(initialized["jsonrpc"], "2.0");
    assert_eq!(initialized["id"], 1);
    assert!(
        initialized["result"]["serverInfo"]["name"].is_string(),
        "{initialized}"
    );

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{{}}}}"#
    )
    .unwrap();

    let listed = loop {
        let message = next_message();
        if message["id"] == 2 {
            break message;
        }
    };
    let names: Vec<&str> = listed["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert!(names.contains(&"slice_log"), "{names:?}");
    assert!(names.contains(&"slice_build"), "{names:?}");
    assert!(names.contains(&"slice_container"), "{names:?}");
    assert!(names.contains(&"slice_service"), "{names:?}");

    drop(stdin);
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn cli_slice_prints_report_and_stats() {
    let output = Command::new(env!("CARGO_BIN_EXE_logcut"))
        .args(["slice", "--raw-text", "ERROR boom\nERROR boom\n", "--stats"])
        .output()
        .expect("run logcut slice");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("[x2"), "{stdout}");
    assert!(stderr.contains("reduction_percent"), "{stderr}");
}

#[test]
fn cli_build_slices_piped_compiler_output() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_logcut"))
        .args(["build", "--max-errors", "1", "--stats"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run logcut build");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(fixture("rustc_errors.txt").as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stdout.contains("E0277"), "{stdout}");
    assert!(stdout.contains("trait/note lines collapsed"), "{stdout}");
    assert!(stderr.contains("reduction_percent"), "{stderr}");
}

#[test]
fn cli_init_generates_all_host_configs() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_logcut"))
        .args(["init", "--all"])
        .current_dir(dir.path())
        .output()
        .expect("run logcut init");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for relative in [
        "opencode.json",
        ".mcp.json",
        ".cursor/mcp.json",
        ".opencode/commands/debug-log.md",
        ".claude/commands/debug-log.md",
        ".cursor/rules/debug-log.mdc",
    ] {
        let path = dir.path().join(relative);
        let contents =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{relative}: {error}"));
        if relative.ends_with(".json") {
            assert!(contents.contains("\"logcut\""), "{relative}: {contents}");
            assert!(contents.contains("\"mcp\""), "{relative}: {contents}");
        } else {
            assert!(contents.contains("logcut mcp"), "{relative}: {contents}");
        }
    }

    let opencode: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("opencode.json")).unwrap())
            .unwrap();
    let server = &opencode["mcp"]["servers"]["logcut"];
    assert_eq!(server["type"], "local");
    assert_eq!(server["command"], serde_json::json!(["logcut", "mcp"]));
}

#[cfg(feature = "cli")]
#[test]
fn agent_advertises_the_core_tools() {
    let definitions = logcut::agent::tool_definitions();
    let names: Vec<&str> = definitions
        .iter()
        .map(|tool| tool["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "slice_log",
            "slice_build",
            "slice_container",
            "slice_service"
        ]
    );
}

// ---------------------------------------------------------------- v0.2.0

#[test]
fn slice_build_parses_rustc_trait_bound_wall() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    fs::create_dir_all(&src).unwrap();
    fs::write(
        src.join("api.rs"),
        (1..=60)
            .map(|line| {
                if line == 42 {
                    "    serde_json::to_string(user)\n".to_owned()
                } else {
                    format!("// line {line}\n")
                }
            })
            .collect::<String>(),
    )
    .unwrap();

    let report = slice_build(SliceBuildArgs {
        raw_text: Some(fixture("rustc_errors.txt")),
        workspace_root: Some(dir.path().to_string_lossy().into_owned()),
        ..Default::default()
    })
    .unwrap();

    assert_eq!(report.errors.len(), 2, "{:?}", report.errors);
    let first = &report.errors[0];
    assert_eq!(first.kind, CompilerKind::Rust);
    assert_eq!(first.code.as_deref(), Some("E0277"));
    assert_eq!(first.path, "src/api.rs");
    assert_eq!(first.line, 42);
    assert_eq!(first.column, Some(18));
    assert!(first.notes_collapsed >= 2, "{first:?}");
    assert!(
        first
            .primary_snippet
            .as_deref()
            .unwrap()
            .contains("serde_json::to_string(user)")
    );
    assert!(
        report.text.contains("trait/note lines collapsed"),
        "{}",
        report.text
    );
    assert!(report.text.contains("> 42 |"), "{}", report.text);
    assert!(!report.snippets.is_empty());
}

#[test]
fn slice_build_deduplicates_tsc_cascades() {
    let report = slice_build(SliceBuildArgs {
        raw_text: Some(fixture("tsc_errors.txt")),
        ..Default::default()
    })
    .unwrap();

    assert_eq!(report.errors.len(), 2, "{:?}", report.errors);
    let first = &report.errors[0];
    assert_eq!(first.kind, CompilerKind::TypeScript);
    assert_eq!(first.code.as_deref(), Some("TS2345"));
    assert_eq!(first.count, 3);
    assert_eq!(first.line, 12);
    assert_eq!(first.column, Some(7));
    assert_eq!(report.cascading_suppressed, 2);
    assert!(
        report.text.contains("[x3 cascading occurrences"),
        "{}",
        report.text
    );
}

#[test]
fn slice_build_parses_go_and_clang_diagnostics() {
    let combined = format!(
        "{}\n{}",
        fixture("go_build_errors.txt"),
        fixture("clang_errors.txt")
    );
    let report = slice_build(SliceBuildArgs {
        raw_text: Some(combined),
        ..Default::default()
    })
    .unwrap();

    assert_eq!(report.errors.len(), 4, "{:?}", report.errors);
    assert_eq!(report.errors[0].kind, CompilerKind::Go);
    assert_eq!(report.errors[0].path, "api/handler.go");
    assert_eq!(report.errors[0].line, 18);
    assert!(
        !report.text.contains("# example.com/app/api"),
        "{}",
        report.text
    );

    let clang = report
        .errors
        .iter()
        .find(|error| error.kind == CompilerKind::C)
        .expect("clang error");
    assert_eq!(clang.path, "src/main.c");
    assert_eq!(clang.line, 10);
    assert!(clang.notes_collapsed >= 2, "{clang:?}");
}

#[test]
fn slice_build_respects_max_errors_and_redacts_secrets() {
    let raw = format!(
        "{}\nerror: Authorization: Bearer sk-test-abcdefghijklmnopqrstuvwxyz123456\n",
        fixture("rustc_errors.txt")
    );
    let report = slice_build(SliceBuildArgs {
        raw_text: Some(raw.clone()),
        max_errors: Some(1),
        ..Default::default()
    })
    .unwrap();

    assert_eq!(report.errors.len(), 1);
    assert!(
        !report
            .text
            .contains("sk-test-abcdefghijklmnopqrstuvwxyz123456"),
        "{}",
        report.text
    );
    assert!(report.stats.raw_bytes == raw.len());
}

#[test]
fn slice_build_requires_an_input_source() {
    let error = slice_build(SliceBuildArgs::default()).unwrap_err();
    assert!(error.to_string().contains("path"), "{error}");
}
