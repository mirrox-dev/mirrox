use crate::error::AppError;
use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header::CONTENT_TYPE, Response, StatusCode};
use std::path::PathBuf;
use std::sync::Arc;

/// Shared state for the script static file server.
#[derive(Clone)]
pub struct ScriptServerState {
    pub dir: PathBuf,
}

impl ScriptServerState {
    pub fn new(dir: &str) -> Self {
        Self {
            dir: PathBuf::from(dir),
        }
    }
}

/// Serves a local JS file from the configured scripts directory.
pub async fn static_file_handler(
    State(state): State<Arc<ScriptServerState>>,
    Path(filename): Path<String>,
) -> Result<Response<Body>, AppError> {
    // Prevent path traversal: only allow simple filenames.
    if filename.contains('/') || filename.contains('\\') || filename.contains("..") {
        return Err(AppError::Config("invalid script filename".into()));
    }
    let path = state.dir.join(&filename);
    let content = std::fs::read(&path).map_err(|_| {
        AppError::Config(format!("script file not found: {}", path.display()))
    })?;
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/javascript; charset=utf-8")
        .header("cache-control", "public, max-age=3600")
        .body(Body::from(content))
        .map_err(|err| AppError::Config(format!("failed to build script response: {err}")))
}

/// Returns true if the script reference is a remote URL (http:// or https://).
pub fn is_remote_url(script: &str) -> bool {
    script.starts_with("http://") || script.starts_with("https://")
}

/// Builds the `<script>` tags HTML string for the given list of scripts.
/// `prefix` is the URL prefix for serving local scripts (e.g. "/mirrox-scripts").
pub fn build_script_tags(scripts: &[String], prefix: &str) -> String {
    if scripts.is_empty() {
        return String::new();
    }
    let mut tags = String::new();
    for script in scripts {
        let src = if is_remote_url(script) {
            script.clone()
        } else {
            format!("{}/{}", prefix.trim_end_matches('/'), script)
        };
        tags.push_str(&format!("<script src=\"{}\"></script>\n", html_escape(&src)));
    }
    tags
}

/// Injects `<script>` tags into an HTML string just before `</head>`.
/// If `</head>` is not found, appends the tags at the end of the document.
pub fn inject_scripts(html: &str, scripts: &[String], prefix: &str) -> String {
    if scripts.is_empty() {
        return html.to_string();
    }
    let tags = build_script_tags(scripts, prefix);
    // Try case-insensitive injection before </head>.
    if let Some(pos) = find_closing_head(html) {
        let mut result = String::with_capacity(html.len() + tags.len());
        result.push_str(&html[..pos]);
        result.push_str(&tags);
        result.push_str(&html[pos..]);
        result
    } else {
        // No </head> found — append at the end.
        let mut result = String::with_capacity(html.len() + tags.len());
        result.push_str(html);
        result.push_str(&tags);
        result
    }
}

/// Finds the position of `</head>` in a case-insensitive manner.
/// Returns the byte offset of the `<` character.
fn find_closing_head(html: &str) -> Option<usize> {
    let lower = html.to_ascii_lowercase();
    lower.rfind("</head>")
}

/// Minimal HTML escaping for attribute values.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_script_tags_local() {
        let scripts = vec!["theme.js".to_string(), "analytics.js".to_string()];
        let tags = build_script_tags(&scripts, "/mirrox-scripts");
        assert!(tags.contains("<script src=\"/mirrox-scripts/theme.js\"></script>"));
        assert!(tags.contains("<script src=\"/mirrox-scripts/analytics.js\"></script>"));
    }

    #[test]
    fn test_build_script_tags_remote() {
        let scripts = vec!["https://cdn.example.com/ext.js".to_string()];
        let tags = build_script_tags(&scripts, "/mirrox-scripts");
        assert!(tags.contains("<script src=\"https://cdn.example.com/ext.js\"></script>"));
    }

    #[test]
    fn test_build_script_tags_mixed() {
        let scripts = vec![
            "local.js".to_string(),
            "https://cdn.example.com/remote.js".to_string(),
        ];
        let tags = build_script_tags(&scripts, "/mirrox-scripts");
        assert!(tags.contains("<script src=\"/mirrox-scripts/local.js\"></script>"));
        assert!(tags.contains("<script src=\"https://cdn.example.com/remote.js\"></script>"));
    }

    #[test]
    fn test_build_script_tags_empty() {
        let tags = build_script_tags(&[], "/mirrox-scripts");
        assert!(tags.is_empty());
    }

    #[test]
    fn test_inject_scripts_before_head() {
        let html = "<html><head><title>Test</title></head><body></body></html>";
        let scripts = vec!["my.js".to_string()];
        let result = inject_scripts(html, &scripts, "/mirrox-scripts");
        assert!(result.contains("<script src=\"/mirrox-scripts/my.js\"></script>\n</head>"));
    }

    #[test]
    fn test_inject_scripts_case_insensitive() {
        let html = "<HTML><HEAD><TITLE>Test</TITLE></HEAD><body></body></HTML>";
        let scripts = vec!["my.js".to_string()];
        let result = inject_scripts(html, &scripts, "/mirrox-scripts");
        assert!(result.contains("<script src=\"/mirrox-scripts/my.js\"></script>\n</HEAD>"));
    }

    #[test]
    fn test_inject_scripts_no_head() {
        let html = "<body><p>Hello</p></body>";
        let scripts = vec!["my.js".to_string()];
        let result = inject_scripts(html, &scripts, "/mirrox-scripts");
        assert!(result.ends_with("<script src=\"/mirrox-scripts/my.js\"></script>\n"));
    }

    #[test]
    fn test_inject_scripts_empty() {
        let html = "<html><head></head><body></body></html>";
        let result = inject_scripts(html, &[], "/mirrox-scripts");
        assert_eq!(result, html);
    }

    #[test]
    fn test_is_remote_url() {
        assert!(is_remote_url("https://cdn.example.com/a.js"));
        assert!(is_remote_url("http://example.com/a.js"));
        assert!(!is_remote_url("local.js"));
        assert!(!is_remote_url("/path/to/file.js"));
    }

    #[test]
    fn test_html_escape() {
        assert_eq!(html_escape("a&b\"c<d>e"), "a&amp;b&quot;c&lt;d&gt;e");
    }
}
