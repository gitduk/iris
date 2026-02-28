use llm::provider::{ChatMessage, CompletionRequest, LlmProvider, Role};
use std::io::Write;

/// Maximum repair iterations before giving up.
pub const MAX_REPAIR_ITERATIONS: u32 = 3;

/// Compile timeout in seconds.
pub const COMPILE_TIMEOUT_SECS: u64 = 120;

/// Result of a repair loop run.
#[derive(Debug)]
pub struct RepairResult {
    pub source_code: String,
    pub success: bool,
    pub iterations: u32,
    pub last_error: Option<String>,
}

/// Run the repair loop: LLM generates code → syntax check → compile.
/// Repeats up to MAX_REPAIR_ITERATIONS times on failure.
pub async fn run<P: LlmProvider + ?Sized>(
    llm: &P,
    initial_prompt: &str,
) -> Result<RepairResult, Box<dyn std::error::Error + Send + Sync>> {
    let mut source_code = String::new();
    let mut last_error: Option<String> = None;

    for iteration in 1..=MAX_REPAIR_ITERATIONS {
        // Build the prompt — include previous error if this is a retry
        let prompt_content = if let Some(ref err) = last_error {
            format!(
                "{}\n\n## Previous attempt failed with error:\n```\n{}\n```\n\nFix the error and regenerate the complete source code.",
                initial_prompt, err
            )
        } else {
            initial_prompt.to_string()
        };
        let request = CompletionRequest {
            messages: vec![
                ChatMessage {
                    role: Role::System,
                    content: "You are a Rust code generator. Output ONLY valid Rust source code, no markdown fences or explanations.".into(),
                    content_blocks: vec![],
                },
                ChatMessage {
                    role: Role::User,
                    content: prompt_content,
                    content_blocks: vec![],
                },
            ],
            max_tokens: 4096,
            temperature: 0.2,
            tools: vec![],
        };

        let response = llm.complete(request).await?;
        source_code = extract_code(&response.content);

        // Step 1: Syntax check via syn
        match syn::parse_file(&source_code) {
            Ok(_) => {
                tracing::debug!(iteration, "syntax check passed");
            }
            Err(e) => {
                let err_msg = format!("syntax error: {}", e);
                tracing::debug!(iteration, error = %err_msg, "syntax check failed");
                last_error = Some(err_msg);
                continue;
            }
        }

        // Step 2: Compilation check — cargo build in temp dir
        match compile_in_temp_dir(&source_code) {
            Ok(()) => {
                tracing::debug!(iteration, "compilation passed");
            }
            Err(e) => {
                let err_msg = format!("compilation error: {e}");
                tracing::debug!(iteration, error = %err_msg, "compilation failed");
                last_error = Some(err_msg);
                continue;
            }
        }

        return Ok(RepairResult {
            source_code,
            success: true,
            iterations: iteration,
            last_error: None,
        });
    }

    Ok(RepairResult {
        source_code,
        success: false,
        iterations: MAX_REPAIR_ITERATIONS,
        last_error,
    })
}

/// Extract Rust code from LLM response (strip markdown fences if present).
fn extract_code(response: &str) -> String {
    let trimmed = response.trim();
    if let Some(after_fence) = trimmed.strip_prefix("```rust")
        && let Some(end) = after_fence.rfind("```")
    {
        return after_fence[..end].trim().to_string();
    }
    if let Some(after_fence) = trimmed.strip_prefix("```")
        && let Some(end) = after_fence.rfind("```")
    {
        return after_fence[..end].trim().to_string();
    }
    trimmed.to_string()
}

/// Compile generated code in a temporary directory using `cargo build`.
/// Returns Ok(()) if compilation succeeds, Err with compiler output otherwise.
fn compile_in_temp_dir(source_code: &str) -> Result<(), String> {
    let tmp = tempfile::tempdir().map_err(|e| format!("failed to create temp dir: {e}"))?;
    let src_dir = tmp.path().join("src");
    std::fs::create_dir_all(&src_dir).map_err(|e| format!("failed to create src dir: {e}"))?;

    // Write Cargo.toml
    let cargo_toml = r#"[package]
name = "iris-codegen-check"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"
"#;
    let mut f = std::fs::File::create(tmp.path().join("Cargo.toml"))
        .map_err(|e| format!("failed to write Cargo.toml: {e}"))?;
    f.write_all(cargo_toml.as_bytes())
        .map_err(|e| format!("failed to write Cargo.toml: {e}"))?;

    // Write source
    std::fs::write(src_dir.join("lib.rs"), source_code)
        .map_err(|e| format!("failed to write lib.rs: {e}"))?;

    // Run cargo build with timeout
    let output = std::process::Command::new("cargo")
        .args(["build", "--lib"])
        .current_dir(tmp.path())
        .env("CARGO_TARGET_DIR", tmp.path().join("target"))
        .output()
        .map_err(|e| format!("failed to spawn cargo: {e}"))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Extract just the error lines, skip noise
        let errors: String = stderr
            .lines()
            .filter(|l| l.contains("error"))
            .take(20)
            .collect::<Vec<_>>()
            .join("\n");
        Err(if errors.is_empty() {
            stderr.chars().take(2000).collect()
        } else {
            errors
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_code_plain() {
        let code = "fn main() {}";
        assert_eq!(extract_code(code), "fn main() {}");
    }

    #[test]
    fn extract_code_rust_fence() {
        let input = "```rust\nfn main() {}\n```";
        assert_eq!(extract_code(input), "fn main() {}");
    }

    #[test]
    fn extract_code_generic_fence() {
        let input = "```\nfn main() {}\n```";
        assert_eq!(extract_code(input), "fn main() {}");
    }
}
