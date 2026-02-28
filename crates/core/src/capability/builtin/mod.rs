pub mod read_file;
pub mod write_file;
pub mod run_bash;

use std::collections::HashMap;
use uuid::Uuid;

use crate::types::{CapabilityRequest, CapabilityResponse, Permission};
use llm::provider::ToolDefinition;

/// Project-specific namespace for deterministic UUID v5 generation.
/// Generated via uuid5(NAMESPACE_URL, "iris-builtin-capability").
/// Builtin capabilities get stable IDs that survive restarts.
const BUILTIN_NS: Uuid = Uuid::from_bytes([
    0xc5, 0x8a, 0x0e, 0x72, 0x23, 0x9b, 0x5b, 0x5d,
    0x84, 0xf1, 0xfc, 0x91, 0xff, 0x98, 0x14, 0xfd,
]);

#[async_trait::async_trait]
pub trait BuiltinCapability: Send + Sync {
    fn name(&self) -> &str;
    fn keywords(&self) -> Vec<String>;
    fn permissions(&self) -> Vec<Permission>;
    fn tool_definition(&self) -> ToolDefinition;
    async fn execute(&self, request: CapabilityRequest) -> CapabilityResponse;
}

pub struct BuiltinRegistry {
    caps: HashMap<Uuid, Box<dyn BuiltinCapability>>,
}

impl Default for BuiltinRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BuiltinRegistry {
    pub fn new() -> Self {
        let mut reg = Self { caps: HashMap::new() };
        reg.register(Box::new(read_file::ReadFile));
        reg.register(Box::new(write_file::WriteFile));
        reg.register(Box::new(run_bash::RunBash));
        reg
    }

    fn register(&mut self, cap: Box<dyn BuiltinCapability>) {
        let id = Uuid::new_v5(&BUILTIN_NS, cap.name().as_bytes());
        self.caps.insert(id, cap);
    }

    /// Returns (id, keywords) pairs for FastPath registration.
    pub fn entries(&self) -> Vec<(Uuid, Vec<String>)> {
        self.caps.iter().map(|(id, cap)| (*id, cap.keywords())).collect()
    }

    /// Look up a builtin capability by UUID.
    pub fn get(&self, id: Uuid) -> Option<&dyn BuiltinCapability> {
        self.caps.get(&id).map(|b| b.as_ref())
    }

    /// Look up a builtin capability by name (e.g. "run_bash", "read_file").
    pub fn get_by_name(&self, name: &str) -> Option<&dyn BuiltinCapability> {
        self.caps.values().find(|cap| cap.name() == name).map(|b| b.as_ref())
    }

    /// Return the names of all registered builtin capabilities, sorted for determinism.
    pub fn list_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = self.caps.values().map(|cap| cap.name()).collect();
        names.sort();
        names
    }

    /// Human-readable description of all registered builtins, for LLM self-context injection.
    pub fn describe(&self) -> String {
        let mut lines: Vec<String> = self.caps.values().map(|cap| {
            let perms: Vec<&str> = cap.permissions().iter().map(|p| match p {
                Permission::FileRead => "FileRead",
                Permission::FileWrite => "FileWrite",
                Permission::ProcessSpawn => "ProcessSpawn",
                Permission::NetworkRead => "NetworkRead",
                Permission::NetworkWrite => "NetworkWrite",
                Permission::SystemInfo => "SystemInfo",
            }).collect();
            format!("- {} (permissions: {})", cap.name(), perms.join(", "))
        }).collect();
        lines.sort(); // deterministic order
        lines.join("\n")
    }

    /// Collect tool definitions from all registered builtins.
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let mut defs: Vec<ToolDefinition> = self.caps.values().map(|cap| cap.tool_definition()).collect();
        defs.sort_by(|a, b| a.name.cmp(&b.name));
        defs
    }
}
