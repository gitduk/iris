use std::collections::HashMap;
use std::io;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::types::{CapabilityRecord, CapabilityRequest, CapabilityResponse, CapabilityState};

// ── Error type ─────────────────────────────────────────────────

#[derive(Debug)]
pub enum ProcessError {
    SpawnFailed(String),
    NotRunning(Uuid),
    Timeout(Duration),
    InvalidResponse(String),
    StdinClosed(Uuid),
    Io(io::Error),
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(msg) => write!(f, "spawn failed: {msg}"),
            Self::NotRunning(id) => write!(f, "capability {id} not running"),
            Self::Timeout(d) => write!(f, "timeout after {d:?}"),
            Self::InvalidResponse(msg) => write!(f, "invalid response: {msg}"),
            Self::StdinClosed(id) => write!(f, "stdin closed for {id}"),
            Self::Io(e) => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for ProcessError {}

impl From<io::Error> for ProcessError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

// ── Health events ──────────────────────────────────────────────

pub enum HealthEvent {
    Crashed { cap_id: Uuid, exit_code: Option<i32> },
    ReadyToConfirm { cap_id: Uuid },
}

// ── ChildHandle ────────────────────────────────────────────────

struct ChildHandle {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    record: CapabilityRecord,
    spawned_at: Instant,
}

// ── ProcessManager ─────────────────────────────────────────────

pub struct ProcessManager {
    children: HashMap<Uuid, ChildHandle>,
    #[allow(dead_code)]
    shutdown: CancellationToken,
}

impl ProcessManager {
    pub fn new(shutdown: CancellationToken) -> Self {
        Self {
            children: HashMap::new(),
            shutdown,
        }
    }

    /// Spawn a capability subprocess. Captures stdin/stdout for NDJSON IPC.
    pub fn spawn(&mut self, record: &CapabilityRecord) -> Result<(), ProcessError> {
        if self.children.contains_key(&record.id) {
            return Ok(()); // already running
        }

        let memory_mb = record
            .manifest
            .resource_limits
            .get("memory_mb")
            .and_then(|v| v.as_u64())
            .unwrap_or(256);

        let mut cmd = tokio::process::Command::new(&record.binary_path);
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);

        // Unix: set RLIMIT_AS to cap virtual memory
        #[cfg(unix)]
        {
            let limit_bytes = memory_mb * 1024 * 1024;
            unsafe {
                cmd.pre_exec(move || {
                    let rlim = libc::rlimit {
                        rlim_cur: limit_bytes,
                        rlim_max: limit_bytes,
                    };
                    if libc::setrlimit(libc::RLIMIT_AS, &rlim) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| ProcessError::SpawnFailed(format!("{}: {e}", record.binary_path)))?;

        let stdin = child.stdin.take().ok_or_else(|| {
            ProcessError::SpawnFailed("failed to capture stdin".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ProcessError::SpawnFailed("failed to capture stdout".into())
        })?;

        let handle = ChildHandle {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout).lines(),
            record: record.clone(),
            spawned_at: Instant::now(),
        };

        tracing::info!(
            capability = %record.name,
            id = %record.id,
            binary = %record.binary_path,
            "capability process spawned"
        );

        self.children.insert(record.id, handle);
        Ok(())
    }

    /// Send an NDJSON request and read one NDJSON response line.
    pub async fn invoke(
        &mut self,
        cap_id: Uuid,
        request: CapabilityRequest,
        timeout: Duration,
    ) -> Result<CapabilityResponse, ProcessError> {
        let handle = self
            .children
            .get_mut(&cap_id)
            .ok_or(ProcessError::NotRunning(cap_id))?;

        // Serialize request as a single JSON line
        let mut line = serde_json::to_string(&request)
            .map_err(|e| ProcessError::InvalidResponse(e.to_string()))?;
        line.push('\n');

        handle
            .stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|_| ProcessError::StdinClosed(cap_id))?;
        handle
            .stdin
            .flush()
            .await
            .map_err(|_| ProcessError::StdinClosed(cap_id))?;

        // Read one response line with timeout
        let resp_line = tokio::time::timeout(timeout, handle.stdout.next_line())
            .await
            .map_err(|_| ProcessError::Timeout(timeout))?
            .map_err(ProcessError::Io)?
            .ok_or(ProcessError::StdinClosed(cap_id))?;

        serde_json::from_str::<CapabilityResponse>(&resp_line)
            .map_err(|e| ProcessError::InvalidResponse(e.to_string()))
    }

    /// Kill a specific capability process.
    pub fn kill(&mut self, cap_id: Uuid) {
        if let Some(mut handle) = self.children.remove(&cap_id) {
            // Child has kill_on_drop, but explicit kill is cleaner
            let _ = handle.child.start_kill();
            tracing::info!(capability = %handle.record.name, "capability process killed");
        }
    }

    /// Graceful shutdown: SIGTERM all children, wait up to `timeout`, then SIGKILL.
    pub async fn shutdown_all(&mut self, timeout: Duration) {
        if self.children.is_empty() {
            return;
        }

        tracing::info!(count = self.children.len(), "shutting down capability processes");

        // Send SIGTERM to all
        #[cfg(unix)]
        for handle in self.children.values() {
            if let Some(pid) = handle.child.id() {
                unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            }
        }

        // Wait for graceful exit up to timeout
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);

        loop {
            if self.children.is_empty() {
                break;
            }

            // Try to reap any exited children
            let mut exited = Vec::new();
            for (id, handle) in &mut self.children {
                if let Ok(Some(_status)) = handle.child.try_wait() {
                    exited.push(*id);
                }
            }
            for id in exited {
                if let Some(h) = self.children.remove(&id) {
                    tracing::debug!(capability = %h.record.name, "capability exited gracefully");
                }
            }

            tokio::select! {
                _ = &mut deadline => break,
                _ = tokio::time::sleep(Duration::from_millis(50)) => {}
            }
        }

        // Force-kill any remaining
        for (_, mut handle) in self.children.drain() {
            let _ = handle.child.start_kill();
            tracing::warn!(capability = %handle.record.name, "capability force-killed after timeout");
        }
    }

    /// Non-blocking health check: detect crashed processes and candidates ready to confirm.
    pub fn health_check(&mut self) -> Vec<HealthEvent> {
        let mut events = Vec::new();
        let mut crashed = Vec::new();

        for (id, handle) in &mut self.children {
            match handle.child.try_wait() {
                Ok(Some(status)) => {
                    crashed.push(*id);
                    events.push(HealthEvent::Crashed {
                        cap_id: *id,
                        exit_code: status.code(),
                    });
                }
                Ok(None) => {
                    // Still running — check if ActiveCandidate is ready to confirm
                    if handle.record.state == CapabilityState::ActiveCandidate {
                        events.push(HealthEvent::ReadyToConfirm { cap_id: *id });
                    }
                }
                Err(e) => {
                    tracing::warn!(capability = %handle.record.name, error = %e, "try_wait failed");
                }
            }
        }

        for id in crashed {
            self.children.remove(&id);
        }

        events
    }

    /// Check if a capability process is currently running.
    pub fn is_running(&self, cap_id: Uuid) -> bool {
        self.children.contains_key(&cap_id)
    }

    /// Check if a capability has been running for at least `duration`.
    pub fn has_been_running_for(&self, cap_id: Uuid, duration: Duration) -> bool {
        self.children
            .get(&cap_id)
            .is_some_and(|h| h.spawned_at.elapsed() >= duration)
    }

    /// Number of active child processes.
    pub fn active_count(&self) -> usize {
        self.children.len()
    }
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(name: &str, binary: &str) -> CapabilityRecord {
        use crate::types::{CapabilityManifest, CapabilityState};
        CapabilityRecord {
            id: Uuid::new_v4(),
            name: name.into(),
            binary_path: binary.into(),
            manifest: CapabilityManifest {
                name: name.into(),
                binary_path: binary.into(),
                permissions: vec![],
                resource_limits: serde_json::json!({"memory_mb": 128}),
                keywords: vec![],
            },
            state: CapabilityState::Confirmed,
            lkg_version: None,
            quarantine_count: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn new_manager_is_empty() {
        let token = CancellationToken::new();
        let mgr = ProcessManager::new(token);
        assert_eq!(mgr.active_count(), 0);
    }

    #[tokio::test]
    async fn spawn_and_kill_cat() {
        let token = CancellationToken::new();
        let mut mgr = ProcessManager::new(token);
        let rec = make_record("test-cat", "cat");

        mgr.spawn(&rec).unwrap();
        assert!(mgr.is_running(rec.id));
        assert_eq!(mgr.active_count(), 1);

        mgr.kill(rec.id);
        assert!(!mgr.is_running(rec.id));
        assert_eq!(mgr.active_count(), 0);
    }

    #[test]
    fn spawn_nonexistent_binary_fails() {
        let token = CancellationToken::new();
        let mut mgr = ProcessManager::new(token);
        let rec = make_record("bad", "/nonexistent/binary/path");

        let result = mgr.spawn(&rec);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn health_check_detects_exit() {
        let token = CancellationToken::new();
        let mut mgr = ProcessManager::new(token);
        // `true` exits immediately with code 0
        let rec = make_record("test-true", "true");

        mgr.spawn(&rec).unwrap();
        // Give the process a moment to exit
        tokio::time::sleep(Duration::from_millis(50)).await;

        let events = mgr.health_check();
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], HealthEvent::Crashed { cap_id, .. } if *cap_id == rec.id));
        assert_eq!(mgr.active_count(), 0);
    }

    #[tokio::test]
    async fn has_been_running_for_works() {
        let token = CancellationToken::new();
        let mut mgr = ProcessManager::new(token);
        let rec = make_record("test-sleep", "cat");

        mgr.spawn(&rec).unwrap();
        assert!(!mgr.has_been_running_for(rec.id, Duration::from_secs(60)));
        assert!(mgr.has_been_running_for(rec.id, Duration::ZERO));

        mgr.kill(rec.id);
    }

    #[tokio::test]
    async fn shutdown_all_cleans_up() {
        let token = CancellationToken::new();
        let mut mgr = ProcessManager::new(token);
        let rec = make_record("test-cat2", "cat");

        mgr.spawn(&rec).unwrap();
        assert_eq!(mgr.active_count(), 1);

        mgr.shutdown_all(Duration::from_secs(2)).await;
        assert_eq!(mgr.active_count(), 0);
    }
}
