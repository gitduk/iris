use tokio_util::sync::CancellationToken;

/// Interrupt controller — cancels in-flight reasoning when new input arrives.
pub struct InterruptController {
    current_token: Option<CancellationToken>,
}

impl InterruptController {
    pub fn new() -> Self {
        Self {
            current_token: None,
        }
    }

    /// Issue a new cancellation token for the current reasoning task.
    /// Cancels any previous in-flight task.
    pub fn new_task(&mut self) -> CancellationToken {
        // Cancel previous task if any
        if let Some(old) = self.current_token.take() {
            old.cancel();
        }
        let token = CancellationToken::new();
        self.current_token = Some(token.clone());
        token
    }

    /// Cancel the current in-flight task (if any).
    pub fn cancel_current(&mut self) {
        if let Some(token) = self.current_token.take() {
            token.cancel();
        }
    }

    /// Check if there's an active task.
    pub fn has_active_task(&self) -> bool {
        self.current_token
            .as_ref()
            .is_some_and(|t| !t.is_cancelled())
    }
}

impl Default for InterruptController {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupt_cancels_previous_task() {
        let mut ctrl = InterruptController::new();
        let t1 = ctrl.new_task();
        assert!(!t1.is_cancelled());

        let _t2 = ctrl.new_task();
        assert!(t1.is_cancelled()); // t1 should be cancelled
    }

    #[test]
    fn interrupt_has_active_task() {
        let mut ctrl = InterruptController::new();
        assert!(!ctrl.has_active_task());

        let _t = ctrl.new_task();
        assert!(ctrl.has_active_task());

        ctrl.cancel_current();
        assert!(!ctrl.has_active_task());
    }
}
