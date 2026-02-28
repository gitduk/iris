use crate::types::{CapabilityRequest, CapabilityResponse, Permission};
use llm::provider::ToolDefinition;

pub struct WriteFile;

/// Extract file path from input (same heuristic as read_file).
fn extract_path(input: &str) -> Option<String> {
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
    input.split_whitespace()
        .find(|t| t.contains('/') || (t.contains('.') && !t.ends_with('.')))
        .map(|s| s.trim_matches(|c: char| c == ',' || c == ';' || c == '(' || c == ')').to_string())
}

/// Extract content to write: code block > second quoted string > text after path.
fn extract_content(input: &str, path: &str) -> Option<String> {
    // Try fenced code block: ```...```
    if let Some(start) = input.find("```") {
        let after_fence = &input[start + 3..];
        // Skip optional language tag on the same line
        let body_start = after_fence.find('\n').map(|i| i + 1).unwrap_or(0);
        let body = &after_fence[body_start..];
        if let Some(end) = body.find("```") {
            let content = &body[..end];
            if !content.trim().is_empty() {
                return Some(content.to_string());
            }
        } else if !body.trim().is_empty() {
            // Unclosed code block — use everything after the language tag line
            return Some(body.to_string());
        }
    }

    // Try inline backtick content (single `)
    let backtick_sections: Vec<&str> = input.split('`').collect();
    if backtick_sections.len() >= 3 {
        // Find a backtick section that isn't the path
        for chunk in backtick_sections.iter().skip(1).step_by(2) {
            let trimmed = chunk.trim();
            if !trimmed.is_empty() && trimmed != path {
                return Some(trimmed.to_string());
            }
        }
    }

    // Try second quoted string across all delimiter types in a single pass
    let mut quoted_strings: Vec<String> = Vec::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' || c == '\'' {
            let s: String = chars.by_ref().take_while(|&ch| ch != c).collect();
            if !s.is_empty() {
                quoted_strings.push(s);
            }
        }
    }
    if quoted_strings.len() >= 2 {
        return Some(quoted_strings.remove(1));
    }

    // Fallback: everything after the path token
    if let Some(idx) = input.find(path) {
        let after = input[idx + path.len()..].trim();
        if !after.is_empty() {
            return Some(after.to_string());
        }
    }

    None
}

#[async_trait::async_trait]
impl super::BuiltinCapability for WriteFile {
    fn name(&self) -> &str { "write_file" }

    fn keywords(&self) -> Vec<String> {
        ["write", "save", "create", "写", "保存", "创建"]
            .iter().map(|s| s.to_string()).collect()
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![Permission::FileWrite]
    }

    fn tool_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".into(),
            description: "Write content to a file at the given path, creating or overwriting it".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The file path to write to" },
                    "content": { "type": "string", "description": "The content to write" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, request: CapabilityRequest) -> CapabilityResponse {
        // Try structured JSON params first, fall back to free-text extraction
        let structured_path = request.params.get("path").and_then(|v| v.as_str()).map(String::from);
        let structured_content = request.params.get("content").and_then(|v| v.as_str()).map(String::from);

        let (path, content) = if let (Some(p), Some(c)) = (structured_path, structured_content) {
            (p, c)
        } else {
            let input = &request.method;
            let p = match extract_path(input) {
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
            let c = match extract_content(input, &p) {
                Some(c) => c,
                None => {
                    return CapabilityResponse {
                        id: request.id,
                        result: None,
                        error: Some("could not extract content to write".into()),
                        metrics: None,
                        side_effects: vec![],
                    };
                }
            };
            (p, c)
        };

        let bytes = content.len();
        match tokio::fs::write(&path, &content).await {
            Ok(()) => CapabilityResponse {
                id: request.id,
                result: Some(serde_json::json!({
                    "path": path,
                    "bytes_written": bytes,
                })),
                error: None,
                metrics: None,
                side_effects: vec![Permission::FileWrite],
            },
            Err(e) => CapabilityResponse {
                id: request.id,
                result: None,
                error: Some(format!("failed to write {path}: {e}")),
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
    fn extracts_code_block_content() {
        let input = "write to test.txt ```\nhello world\n```";
        assert_eq!(extract_content(input, "test.txt"), Some("hello world\n".into()));
    }

    #[test]
    fn extracts_second_quoted_string() {
        let input = r#"write "test.txt" "hello world""#;
        assert_eq!(extract_content(input, "test.txt"), Some("hello world".into()));
    }

    #[test]
    fn extracts_path_from_input() {
        assert_eq!(extract_path("写入 /tmp/test.txt"), Some("/tmp/test.txt".into()));
    }
}
