use crate::types::{CapabilityRequest, CapabilityResponse, Permission};
use iris_llm::provider::ToolDefinition;

pub struct ReadFile;

/// Extract a file path from user input.
/// Priority: quoted string > token containing `/` or `.`
fn extract_path(input: &str) -> Option<String> {
    // Try quoted strings first
    for delim in ['"', '\''] {
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if c == delim {
                let s: String = chars.by_ref().take_while(|&ch| ch != delim).collect();
                if !s.is_empty() && (s.contains('/') || s.contains('.')) {
                    return Some(s);
                }
            }
        }
    }
    // Fallback: token containing `/` or `.ext`
    input.split_whitespace()
        .find(|t| t.contains('/') || (t.contains('.') && !t.ends_with('.')))
        .map(|s| s.trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')').to_string())
}

#[async_trait::async_trait]
impl super::BuiltinCapability for ReadFile {
    fn name(&self) -> &str { "read_file" }

    fn keywords(&self) -> Vec<String> {
        ["read", "show", "contents", "view", "读", "查看", "打开"]
            .iter().map(|s| s.to_string()).collect()
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![Permission::FileRead]
    }

    fn tool_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".into(),
            description: "Read the contents of a file at the given path".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to read" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, request: CapabilityRequest) -> CapabilityResponse {
        // Try structured JSON params first, fall back to free-text extraction
        let path = request.params.get("path")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| extract_path(&request.method));

        let path = match path {
            Some(p) => p,
            None => {
                return CapabilityResponse {
                    id: request.id,
                    result: None,
                    error: Some("could not extract file path from input".into()),
                    metrics: None,
                    side_effects: vec![],
                };
            }
        };

        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let size = content.len();
                CapabilityResponse {
                    id: request.id,
                    result: Some(serde_json::json!({
                        "path": path,
                        "content": content,
                        "size_bytes": size,
                    })),
                    error: None,
                    metrics: None,
                    side_effects: vec![Permission::FileRead],
                }
            }
            Err(e) => CapabilityResponse {
                id: request.id,
                result: None,
                error: Some(format!("failed to read {path}: {e}")),
                metrics: None,
                side_effects: vec![],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_quoted_path() {
        assert_eq!(extract_path(r#"读一下 "Cargo.toml""#), Some("Cargo.toml".into()));
        assert_eq!(extract_path("read '/tmp/foo.txt'"), Some("/tmp/foo.txt".into()));
    }

    #[test]
    fn extracts_unquoted_path() {
        assert_eq!(extract_path("read Cargo.toml"), Some("Cargo.toml".into()));
        assert_eq!(extract_path("show /etc/hosts"), Some("/etc/hosts".into()));
    }

    #[test]
    fn no_path_found() {
        assert_eq!(extract_path("hello world"), None);
    }
}
