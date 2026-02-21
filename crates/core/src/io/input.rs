use tokio::sync::mpsc;

use crate::types::SensoryEvent;

/// Input channel sender — external systems push events here.
pub type InputSender = mpsc::Sender<SensoryEvent>;
/// Input channel receiver — the runtime consumes from here.
pub type InputReceiver = mpsc::Receiver<SensoryEvent>;

/// Create an input channel with the given buffer size.
pub fn channel(buffer: usize) -> (InputSender, InputReceiver) {
    mpsc::channel(buffer)
}

/// Submit user text as an external sensory event.
pub async fn submit_text(
    tx: &InputSender,
    text: impl Into<String>,
) -> Result<(), mpsc::error::SendError<SensoryEvent>> {
    tx.send(SensoryEvent::external(text)).await
}

/// Submit an internal thought as a sensory event.
pub async fn submit_internal(
    tx: &InputSender,
    text: impl Into<String>,
) -> Result<(), mpsc::error::SendError<SensoryEvent>> {
    tx.send(SensoryEvent::internal(text)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EventSource;

    #[tokio::test]
    async fn submit_text_creates_external_event() {
        let (tx, mut rx) = channel(4);
        submit_text(&tx, "hello").await.unwrap();
        let event = rx.recv().await.unwrap();
        assert_eq!(event.content, "hello");
        assert_eq!(event.source, EventSource::External);
    }

    #[tokio::test]
    async fn submit_internal_creates_internal_event() {
        let (tx, mut rx) = channel(4);
        submit_internal(&tx, "thought").await.unwrap();
        let event = rx.recv().await.unwrap();
        assert_eq!(event.content, "thought");
        assert_eq!(event.source, EventSource::Internal);
    }

    #[tokio::test]
    async fn channel_respects_buffer() {
        let (tx, _rx) = channel(2);
        // Fill buffer
        tx.send(SensoryEvent::external("a")).await.unwrap();
        tx.send(SensoryEvent::external("b")).await.unwrap();
        // Third send would block — use try_send to verify
        assert!(tx.try_send(SensoryEvent::external("c")).is_err());
    }
}
