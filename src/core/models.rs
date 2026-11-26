//! Input and output contracts for every core tool.
//!
//! Every MCP tool parameter struct derives `JsonSchema`; the MCP schema and
//! the Rust API therefore cannot drift apart.

use serde::{Deserialize, Serialize};

/// Parameters for `slice_log`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SliceLogRequest {
    /// Log file to read. Mutually exclusive with `raw_text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Log text supplied directly (used for piped stdin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
    /// Root used to resolve crashing source files and classify app frames.
    #[serde(default = "default_workspace_root", skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default = "default_tail_lines", skip_serializing_if = "Option::is_none")]
    pub tail_lines: Option<usize>,
    #[serde(default = "default_context_lines", skip_serializing_if = "Option::is_none")]
    pub context_lines: Option<usize>,
    #[serde(default = "default_include_vendor_frames", skip_serializing_if = "Option::is_none")]
    pub include_vendor_frames: Option<bool>,
    #[serde(default = "default_max_chars", skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

/// Parameters for `slice_container`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SliceContainerRequest {
    pub container: String,
    #[serde(default = "default_container_tail_lines", skip_serializing_if = "Option::is_none")]
    pub tail_lines: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default = "default_workspace_root", skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default = "default_max_chars", skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

/// Parameters for `slice_service`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SliceServiceRequest {
    pub unit: String,
    #[serde(default = "default_service_lines", skip_serializing_if = "Option::is_none")]
    pub lines: Option<usize>,
    #[serde(default = "default_workspace_root", skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default = "default_max_chars", skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

fn default_workspace_root() -> Option<String> {
    Some(".".into())
}
fn default_tail_lines() -> Option<usize> {
    Some(5000)
}
fn default_container_tail_lines() -> Option<usize> {
    Some(2000)
}
fn default_service_lines() -> Option<usize> {
    Some(1000)
}
fn default_context_lines() -> Option<usize> {
    Some(3)
}
fn default_include_vendor_frames() -> Option<bool> {
    Some(false)
}
fn default_max_chars() -> Option<usize> {
    Some(12_000)
}

impl Default for SliceLogRequest {
    fn default() -> Self {
        Self {
            path: None,
            raw_text: None,
            workspace_root: default_workspace_root(),
            tail_lines: default_tail_lines(),
            context_lines: default_context_lines(),
            include_vendor_frames: default_include_vendor_frames(),
            max_chars: default_max_chars(),
        }
    }
}

impl Default for SliceContainerRequest {
    fn default() -> Self {
        Self {
            container: String::new(),
            tail_lines: default_container_tail_lines(),
            since: None,
            workspace_root: default_workspace_root(),
            max_chars: default_max_chars(),
        }
    }
}

impl Default for SliceServiceRequest {
    fn default() -> Self {
        Self {
            unit: String::new(),
            lines: default_service_lines(),
            workspace_root: default_workspace_root(),
            max_chars: default_max_chars(),
        }
    }
}

/// Language of a parsed stack frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StackLanguage {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Go,
    Java,
    C,
}

/// Whether a frame belongs to the user's workspace or to a framework/runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum FrameKind {
    App,
    Vendor,
}

/// One normalized stack frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StackFrame {
    pub language: StackLanguage,
    pub kind: FrameKind,
    pub path: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub function: String,
    /// Original line the frame was parsed from.
    pub raw: String,
}

/// A burst of repeated (normalized) log lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ErrorGroup {
    pub template: String,
    pub count: usize,
    pub first: Option<String>,
    pub last: Option<String>,
}

/// An inlined source excerpt around a crashing `path:line`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SourceSnippet {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub target_line: usize,
    pub text: String,
}

/// Size/token statistics for a slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SliceStats {
    pub raw_bytes: usize,
    pub sliced_bytes: usize,
    pub estimated_tokens_saved: usize,
    pub reduction_percent: f64,
}

/// Complete result of a slicing operation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SliceReport {
    pub text: String,
    pub stats: SliceStats,
    pub frames: Vec<StackFrame>,
    pub groups: Vec<ErrorGroup>,
    pub snippets: Vec<SourceSnippet>,
}

/// Compiler family that produced a diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CompilerKind {
    Rust,
    TypeScript,
    Go,
    C,
}

/// Parameters for `slice_build`.
#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SliceBuildArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_text: Option<String>,
    #[serde(default = "default_workspace_root", skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default = "default_max_errors", skip_serializing_if = "Option::is_none")]
    pub max_errors: Option<usize>,
    #[serde(default = "default_context_lines", skip_serializing_if = "Option::is_none")]
    pub context_lines: Option<usize>,
    #[serde(default = "default_max_chars", skip_serializing_if = "Option::is_none")]
    pub max_chars: Option<usize>,
}

fn default_max_errors() -> Option<usize> {
    Some(10)
}

impl Default for SliceBuildArgs {
    fn default() -> Self {
        Self {
            path: None,
            raw_text: None,
            workspace_root: default_workspace_root(),
            max_errors: default_max_errors(),
            context_lines: default_context_lines(),
            max_chars: default_max_chars(),
        }
    }
}

/// One root-cause compiler diagnostic after cascade deduplication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompilerError {
    pub kind: CompilerKind,
    pub code: Option<String>,
    pub message: String,
    pub path: String,
    pub line: usize,
    pub column: Option<usize>,
    pub count: usize,
    pub notes_collapsed: usize,
    pub primary_snippet: Option<String>,
}

/// Structured result of compiler error cascade slicing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SliceBuildReport {
    pub text: String,
    pub stats: SliceStats,
    pub errors: Vec<CompilerError>,
    pub snippets: Vec<SourceSnippet>,
    pub cascading_suppressed: usize,
}
