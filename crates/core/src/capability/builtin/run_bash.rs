use crate::types::{CapabilityRequest, CapabilityResponse, Permission};
use iris_llm::provider::ToolDefinition;
use std::time::Duration;

pub struct RunBash;

/// Extract command string from user input.
/// Priority: fenced code block > backtick > quoted string > text after trigger word.
fn extract_command(input: &str) -> Option<String> {
    // Fenced code block
    if let Some(start) = input.find("```") {
        let after = &input[start + 3..];
        let body_start = after.find('\n').map(|i| i + 1).unwrap_or(0);
        let body = &after[body_start..];
        if let Some(end) = body.find("```") {
            let cmd = body[..end].trim();
            if !cmd.is_empty() {
                return Some(cmd.to_string());
            }
        }
    }

    // Inline backtick
    let parts: Vec<&str> = input.split('`').collect();
    if parts.len() >= 3 {
        let cmd = parts[1].trim();
        if !cmd.is_empty() {
            return Some(cmd.to_string());
        }
    }

    // Quoted string
    for delim in ['"', '\''] {
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            if c == delim {
                let s: String = chars.by_ref().take_while(|&ch| ch != delim).collect();
                if !s.is_empty() {
                    return Some(s);
                }
            }
        }
    }

    // Text after trigger words — search char-by-char to avoid byte offset mismatch
    // between lowercased and original strings (non-ASCII can change byte lengths).
    for trigger in ["run ", "execute ", "运行 ", "执行 "] {
        let trig_chars: Vec<char> = trigger.chars().collect();
        let input_chars: Vec<char> = input.chars().collect();
        for i in 0..input_chars.len() {
            let matches = trig_chars.iter().enumerate().all(|(j, &tc)| {
                i + j < input_chars.len()
                    && input_chars[i + j].to_lowercase().eq(tc.to_lowercase())
            });
            if matches {
                let after_idx = i + trig_chars.len();
                let after: String = input_chars[after_idx..].iter().collect();
                let after = after.trim();
                if !after.is_empty() {
                    return Some(after.to_string());
                }
            }
        }
    }

    None
}

const TIMEOUT_SECS: u64 = 30;

#[async_trait::async_trait]
impl super::BuiltinCapability for RunBash {
    fn name(&self) -> &str { "run_bash" }

    fn keywords(&self) -> Vec<String> {
        ["run", "bash", "command", "execute", "shell", "terminal", "运行", "执行", "命令"]
            .iter().map(|s| s.to_string()).collect()
    }

    fn permissions(&self) -> Vec<Permission> {
        vec![Permission::ProcessSpawn]
    }

    fn tool_definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "run_bash".into(),
            description: "Execute a bash command and return stdout, stderr, and exit code".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The bash command to execute" }
                },
                "required": ["command"]
            }),
        }
    }

    async fn execute(&self, request: CapabilityRequest) -> CapabilityResponse {
        // Try structured JSON params first, fall back to free-text extraction
        let cmd = request.params.get("command")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| extract_command(&request.method));

        let cmd = match cmd {
            Some(c) => c,
            None => {
                return CapabilityResponse {
                    id: request.id,
                    result: None,
                    error: Some("could not extract command from input".into()),
                    metrics: None,
                    side_effects: vec![],
                };
            }
        };

        let result = tokio::time::timeout(
            Duration::from_secs(TIMEOUT_SECS),
            tokio::process::Command::new("bash")
                .arg("-c")
                .arg(&cmd)
                .output(),
        ).await;

        match result {
            Ok(Ok(output)) => {
                let code = output.status.code().unwrap_or(-1);
                const MAX_OUTPUT: usize = 64 * 1024;
                let stdout_raw = String::from_utf8_lossy(&output.stdout);
                let stderr_raw = String::from_utf8_lossy(&output.stderr);
                let stdout = if stdout_raw.len() > MAX_OUTPUT {
                    format!("{}... [truncated, {} bytes total]", &stdout_raw[..MAX_OUTPUT], stdout_raw.len())
                } else {
                    stdout_raw.to_string()
                };
                let stderr = if stderr_raw.len() > MAX_OUTPUT {
                    format!("{}... [truncated, {} bytes total]", &stderr_raw[..MAX_OUTPUT], stderr_raw.len())
                } else {
                    stderr_raw.to_string()
                };
                let error = if code == 0 {
                    None
                } else {
                    let preview = stderr
                        .lines()
                        .next()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .or_else(|| stdout.lines().next().map(str::trim).filter(|s| !s.is_empty()))
                        .unwrap_or("no output");
                    Some(format!("command exited with code {code}: {preview}"))
                };
                CapabilityResponse {
                    id: request.id,
                    result: Some(serde_json::json!({
                        "command": cmd,
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": code,
                    })),
                    error,
                    metrics: None,
                    side_effects: vec![Permission::ProcessSpawn],
                }
            }
            Ok(Err(e)) => CapabilityResponse {
                id: request.id,
                result: None,
                error: Some(format!("failed to execute command: {e}")),
                metrics: None,
                side_effects: vec![],
            },
            Err(_) => CapabilityResponse {
                id: request.id,
                result: None,
                error: Some(format!("command timed out after {TIMEOUT_SECS}s")),
                metrics: None,
                side_effects: vec![],
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::builtin::BuiltinCapability;
    use crate::types::CapabilityRequest;

    #[test]
    fn extracts_backtick_command() {
        assert_eq!(extract_command("run `ls -la`"), Some("ls -la".into()));
    }

    #[test]
    fn extracts_quoted_command() {
        assert_eq!(extract_command(r#"execute "echo hello""#), Some("echo hello".into()));
    }

    #[test]
    fn extracts_after_trigger() {
        assert_eq!(extract_command("run ls -la"), Some("ls -la".into()));
        assert_eq!(extract_command("运行 pwd"), Some("pwd".into()));
    }

    #[test]
    fn no_command_found() {
        assert_eq!(extract_command("hello"), None);
    }

    #[tokio::test]
    async fn non_zero_exit_sets_error() {
        let cap = RunBash;
        let req = CapabilityRequest {
            id: uuid::Uuid::new_v4(),
            method: "run false".into(),
            params: serde_json::json!({"command":"false"}),
            version: 1,
        };
        let resp = cap.execute(req).await;
        assert!(resp.error.is_some());
        assert!(resp.result.is_some());
    }
}
