use crate::types::GapDescriptor;

/// Build an LLM prompt for generating a capability from a gap descriptor.
///
/// The prompt includes:
/// - Gap type and trigger description
/// - Suggested crates (pre-approved only)
/// - Constraints (no unsafe, resource limits, IPC protocol)
/// - Past failure summaries (if any)
pub fn build_codegen_prompt(
    gap: &GapDescriptor,
    approved_crates: &[String],
    failure_summaries: &[String],
) -> String {
    let mut prompt = String::with_capacity(2048);

    prompt.push_str("You are a Rust code generator for the iris digital life system.\n");
    prompt.push_str("Generate a standalone capability binary that communicates via stdin/stdout NDJSON.\n\n");

    prompt.push_str(&format!("## Gap Type: {}\n", gap.gap_type.as_str()));
    prompt.push_str(&format!("## Trigger: {}\n\n", gap.trigger_description));

    // Approved crates
    if !approved_crates.is_empty() {
        prompt.push_str("## Approved crates (you may use these):\n");
        for c in approved_crates {
            prompt.push_str(&format!("- {}\n", c));
        }
        prompt.push('\n');
    }

    // Suggested crates from the gap descriptor
    if !gap.suggested_crates.is_empty() {
        prompt.push_str("## Suggested crates (need approval if not in approved list):\n");
        for c in &gap.suggested_crates {
            prompt.push_str(&format!("- {}\n", c));
        }
        prompt.push('\n');
    }

    // Constraints
    prompt.push_str("## Constraints:\n");
    prompt.push_str("- No `unsafe` code\n");
    prompt.push_str("- Must read CapabilityRequest from stdin (NDJSON) and write CapabilityResponse to stdout\n");
    prompt.push_str("- Compile timeout: 120s, memory budget: 512MB\n");
    prompt.push_str("- Handle errors gracefully, return error in CapabilityResponse\n\n");

    // IPC Protocol Types
    prompt.push_str("## IPC Protocol Types (use these exact definitions):\n");
    prompt.push_str("```rust\n");
    prompt.push_str("#[derive(serde::Deserialize)]\n");
    prompt.push_str("struct CapabilityRequest {\n    id: uuid::Uuid,\n    method: String,\n    params: serde_json::Value,\n    version: u8,\n}\n\n");
    prompt.push_str("#[derive(serde::Serialize)]\n");
    prompt.push_str("struct CapabilityResponse {\n    id: uuid::Uuid,\n    result: Option<serde_json::Value>,\n    error: Option<String>,\n    metrics: Option<serde_json::Value>,\n    side_effects: Vec<String>,\n}\n");
    prompt.push_str("```\n\n");

    // Past failures
    if !failure_summaries.is_empty() {
        prompt.push_str("## Previous failed attempts (avoid these mistakes):\n");
        for (i, summary) in failure_summaries.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", i + 1, summary));
        }
        prompt.push('\n');
    }

    prompt.push_str("Generate the complete Rust source code for this capability.\n");
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{EventSource, GapType};
    use uuid::Uuid;

    #[test]
    fn prompt_includes_gap_info() {
        let gap = GapDescriptor {
            id: Uuid::new_v4(),
            gap_type: GapType::FileSystem,
            trigger_description: "read a CSV file".into(),
            source: EventSource::External,
            suggested_crates: vec!["csv".into()],
            created_at: chrono::Utc::now(),
        };
        let prompt = build_codegen_prompt(&gap, &["serde".into()], &[]);
        assert!(prompt.contains("file_system"));
        assert!(prompt.contains("read a CSV file"));
        assert!(prompt.contains("serde"));
        assert!(prompt.contains("csv"));
    }

    #[test]
    fn prompt_includes_failure_summaries() {
        let gap = GapDescriptor {
            id: Uuid::new_v4(),
            gap_type: GapType::Network,
            trigger_description: "fetch URL".into(),
            source: EventSource::External,
            suggested_crates: vec![],
            created_at: chrono::Utc::now(),
        };
        let failures = vec!["missing error handling".into()];
        let prompt = build_codegen_prompt(&gap, &[], &failures);
        assert!(prompt.contains("missing error handling"));
        assert!(prompt.contains("Previous failed attempts"));
    }
}
