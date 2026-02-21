use std::time::{Duration, Instant};
use uuid::Uuid;

/// Commit window — buffers same-topic input before committing to processing.
/// If new input arrives within the window, the timer resets.
#[derive(Debug)]
pub struct CommitWindow {
    topic_id: Option<Uuid>,
    buffer: Vec<String>,
    deadline: Option<Instant>,
    window: Duration,
}

impl CommitWindow {
    pub fn new() -> Self {
        Self::with_window_ms(600)
    }

    pub fn with_window_ms(ms: u64) -> Self {
        Self {
            topic_id: None,
            buffer: Vec::new(),
            deadline: None,
            window: Duration::from_millis(ms),
        }
    }

    /// Push input into the window. Resets the timer.
    /// Returns true if this is a topic change (previous buffer should be committed first).
    pub fn push(&mut self, topic_id: Option<Uuid>, text: String) -> bool {
        let topic_changed = self.topic_id.is_some()
            && topic_id.is_some()
            && self.topic_id != topic_id;

        if topic_changed {
            // Caller should commit the old buffer first
            return true;
        }

        self.topic_id = topic_id;
        self.buffer.push(text);
        self.deadline = Some(Instant::now() + self.window);
        false
    }

    /// Check if the commit window has expired.
    pub fn is_ready(&self) -> bool {
        self.deadline
            .is_some_and(|d| Instant::now() >= d)
    }

    /// Take the buffered content and reset.
    pub fn commit(&mut self) -> Option<(Option<Uuid>, String)> {
        if self.buffer.is_empty() {
            return None;
        }
        let content = self.buffer.join("\n");
        let topic = self.topic_id.take();
        self.buffer.clear();
        self.deadline = None;
        Some((topic, content))
    }

    /// Time remaining until commit, if any.
    pub fn remaining(&self) -> Option<Duration> {
        self.deadline.map(|d| d.saturating_duration_since(Instant::now()))
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }
}

impl Default for CommitWindow {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_window_basic_flow() {
        let mut w = CommitWindow::new();
        assert!(w.is_empty());

        let changed = w.push(None, "hello".into());
        assert!(!changed);
        assert!(!w.is_empty());

        let (topic, content) = w.commit().unwrap();
        assert!(topic.is_none());
        assert_eq!(content, "hello");
        assert!(w.is_empty());
    }

    #[test]
    fn commit_window_merges_same_topic() {
        let mut w = CommitWindow::new();
        let tid = Some(Uuid::new_v4());
        w.push(tid, "line 1".into());
        w.push(tid, "line 2".into());

        let (_, content) = w.commit().unwrap();
        assert_eq!(content, "line 1\nline 2");
    }

    #[test]
    fn commit_window_detects_topic_change() {
        let mut w = CommitWindow::new();
        let t1 = Some(Uuid::new_v4());
        let t2 = Some(Uuid::new_v4());
        w.push(t1, "topic 1".into());
        let changed = w.push(t2, "topic 2".into());
        assert!(changed);
    }
}
