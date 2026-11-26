//! Project compiler diagnostics without exposing Rumoca types or losing labels.
use rumoca_core::{DiagnosticSeverity, Label, SourceMap};
use serde::Serialize;

#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilerSeverity {
    Error,
    Warning,
    Note,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct SourceLabel {
    pub file: Option<String>,
    pub byte_start: usize,
    /// Exclusive end offset in the original source's UTF-8 bytes.
    pub byte_end: usize,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub primary: bool,
    pub message: Option<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CompilerDiagnostic {
    pub code: Option<String>,
    pub severity: CompilerSeverity,
    pub model: Option<String>,
    pub phase: Option<String>,
    pub message: String,
    pub labels: Vec<SourceLabel>,
    pub notes: Vec<String>,
}

#[derive(Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CompilationFailure {
    pub summary: String,
    pub diagnostics: Vec<CompilerDiagnostic>,
}

fn location(label: &Label, sources: Option<&SourceMap>) -> SourceLabel {
    let source = sources.and_then(|map| map.get_source(label.span.source));
    let prefix = source.and_then(|(_, text)| text.get(..label.span.start.0));
    let line = prefix.map(|text| text.bytes().filter(|b| *b == b'\n').count() + 1);
    let column = prefix.map(|text| text.rsplit('\n').next().unwrap_or("").chars().count() + 1);
    SourceLabel {
        file: source.map(|(file, _)| file.to_owned()),
        byte_start: label.span.start.0,
        byte_end: label.span.end.0,
        line,
        column,
        primary: label.primary,
        message: label.message.clone(),
    }
}

pub(crate) fn compiler_error(error: rumoca::CompilerError) -> CompilationFailure {
    let summary = error.to_string();
    let diagnostics = match error {
        rumoca::CompilerError::CompileDiagnosticsError {
            failures,
            source_map,
            ..
        } => failures
            .into_iter()
            .map(|failure| CompilerDiagnostic {
                code: failure.error_code,
                severity: CompilerSeverity::Error,
                model: Some(failure.model_name),
                phase: failure.phase.map(|p| format!("{p:?}")),
                message: failure.error,
                labels: failure
                    .primary_label
                    .iter()
                    .chain(&failure.secondary_labels)
                    .map(|label| location(label, source_map.as_deref()))
                    .collect(),
                notes: failure.notes,
            })
            .collect(),
        rumoca::CompilerError::SourceDiagnosticsError {
            diagnostics,
            source_map,
            ..
        } => diagnostics
            .into_iter()
            .map(|diagnostic| CompilerDiagnostic {
                code: diagnostic.code,
                severity: match diagnostic.severity {
                    DiagnosticSeverity::Error => CompilerSeverity::Error,
                    DiagnosticSeverity::Warning => CompilerSeverity::Warning,
                    DiagnosticSeverity::Note => CompilerSeverity::Note,
                },
                model: None,
                phase: None,
                message: diagnostic.message,
                labels: diagnostic
                    .labels
                    .iter()
                    .map(|label| location(label, Some(&source_map)))
                    .collect(),
                notes: diagnostic.notes,
            })
            .collect(),
        _ => Vec::new(),
    };
    CompilationFailure {
        summary,
        diagnostics,
    }
}
