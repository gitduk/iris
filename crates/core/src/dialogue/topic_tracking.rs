use std::collections::HashMap;
use uuid::Uuid;

/// A tracked conversation topic.
#[derive(Debug, Clone)]
pub struct Topic {
    pub id: Uuid,
    pub label: String,
    pub message_count: u32,
    pub context_version: u64,
}

/// Topic tracker — maintains active conversation topics.
#[derive(Debug)]
pub struct TopicTracker {
    topics: HashMap<Uuid, Topic>,
    current: Option<Uuid>,
    version_counter: u64,
    max_active: usize,
}

impl TopicTracker {
    pub fn new() -> Self {
        Self::with_max(8)
    }

    pub fn with_max(max_active: usize) -> Self {
        Self {
            topics: HashMap::new(),
            current: None,
            version_counter: 0,
            max_active,
        }
    }

    /// Start or switch to a topic. Returns the context version.
    pub fn activate(&mut self, id: Uuid, label: impl Into<String>) -> u64 {
        self.version_counter += 1;
        let topic = self.topics.entry(id).or_insert_with(|| Topic {
            id,
            label: label.into(),
            message_count: 0,
            context_version: self.version_counter,
        });
        topic.message_count += 1;
        topic.context_version = self.version_counter;
        self.current = Some(id);

        // Evict oldest if over limit
        if self.topics.len() > self.max_active {
            self.evict_oldest();
        }

        self.version_counter
    }

    /// Get the current active topic ID.
    pub fn current_topic(&self) -> Option<Uuid> {
        self.current
    }

    /// Get a topic by ID.
    pub fn get(&self, id: &Uuid) -> Option<&Topic> {
        self.topics.get(id)
    }

    /// Number of active topics.
    pub fn active_count(&self) -> usize {
        self.topics.len()
    }

    fn evict_oldest(&mut self) {
        if let Some((&oldest_id, _)) = self
            .topics
            .iter()
            .filter(|(id, _)| Some(**id) != self.current)
            .min_by_key(|(_, t)| t.context_version)
        {
            self.topics.remove(&oldest_id);
        }
    }
}

impl Default for TopicTracker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_tracker_basic() {
        let mut tracker = TopicTracker::new();
        let id = Uuid::new_v4();
        let v = tracker.activate(id, "test topic");
        assert_eq!(v, 1);
        assert_eq!(tracker.current_topic(), Some(id));
        assert_eq!(tracker.active_count(), 1);
    }

    #[test]
    fn topic_tracker_evicts_oldest() {
        let mut tracker = TopicTracker::new();
        let max = 8;
        let mut ids = Vec::new();
        for i in 0..=max {
            let id = Uuid::new_v4();
            ids.push(id);
            tracker.activate(id, format!("topic {}", i));
        }
        // Should have evicted the first topic
        assert_eq!(tracker.active_count(), max);
        assert!(tracker.get(&ids[0]).is_none());
        assert!(tracker.get(ids.last().unwrap()).is_some());
    }

    #[test]
    fn topic_tracker_increments_message_count() {
        let mut tracker = TopicTracker::new();
        let id = Uuid::new_v4();
        tracker.activate(id, "topic");
        tracker.activate(id, "topic");
        tracker.activate(id, "topic");
        assert_eq!(tracker.get(&id).unwrap().message_count, 3);
    }
}
