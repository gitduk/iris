use tokio::signal;
use tokio_util::sync::CancellationToken;

/// Manages graceful shutdown via CancellationToken.
/// Listens for SIGTERM and cancels the token.
#[derive(Debug)]
pub struct ShutdownGuard {
    token: CancellationToken,
}

impl ShutdownGuard {
    pub fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// The cancellation token that all tasks should monitor.
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Spawn a background task that listens for OS signals and triggers cancellation.
    pub fn spawn_signal_listener(&self) {
        let token = self.token.clone();
        tokio::spawn(async move {
            #[cfg(unix)]
            {
                match signal::unix::signal(signal::unix::SignalKind::terminate()) {
                    Ok(mut sigterm) => {
                        let _ = sigterm.recv().await;
                        tracing::info!("received SIGTERM, initiating shutdown");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to register SIGTERM handler");
                        return;
                    }
                }
            }
            #[cfg(not(unix))]
            {
                let _ = signal::ctrl_c().await;
                tracing::info!("received Ctrl+C, initiating shutdown");
            }
            token.cancel();
        });
    }
}

impl Default for ShutdownGuard {
    fn default() -> Self {
        Self::new()
    }
}
