use tokio::sync::mpsc;

/// An outbound response to deliver to the user.
#[derive(Debug, Clone)]
pub struct OutputMessage {
    pub content: String,
    pub is_streaming: bool,
}

impl OutputMessage {
    pub fn complete(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_streaming: false,
        }
    }

    pub fn streaming_chunk(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_streaming: true,
        }
    }
}

/// Output channel sender — the runtime pushes responses here.
pub type OutputSender = mpsc::Sender<OutputMessage>;
/// Output channel receiver — external systems consume from here.
pub type OutputReceiver = mpsc::Receiver<OutputMessage>;

/// Create an output channel with the given buffer size.
pub fn channel(buffer: usize) -> (OutputSender, OutputReceiver) {
    mpsc::channel(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_message() {
        let msg = OutputMessage::complete("hello");
        assert_eq!(msg.content, "hello");
        assert!(!msg.is_streaming);
    }

    #[test]
    fn streaming_chunk_message() {
        let msg = OutputMessage::streaming_chunk("chunk");
        assert_eq!(msg.content, "chunk");
        assert!(msg.is_streaming);
    }

    #[tokio::test]
    async fn channel_send_recv() {
        let (tx, mut rx) = channel(4);
        tx.send(OutputMessage::complete("test")).await.unwrap();
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg.content, "test");
    }
}
