use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use crossterm::event::{self, Event, KeyEvent};
use tokio::sync::mpsc;

/// Events consumed by the TUI main loop.
#[derive(Debug)]
pub enum AppEvent {
    Key(KeyEvent),
}

/// Spawn crossterm event reader in a dedicated thread.
/// Returns a receiver of `AppEvent`. The thread exits when `stop` is set to true.
pub fn spawn(stop: Arc<AtomicBool>) -> mpsc::UnboundedReceiver<AppEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            // 50ms poll — fast enough for responsive input, low CPU
            if event::poll(Duration::from_millis(50)).unwrap_or(false)
                && let Ok(Event::Key(key)) = event::read()
                && tx.send(AppEvent::Key(key)).is_err()
            {
                break;
            }
        }
    });
    rx
}
