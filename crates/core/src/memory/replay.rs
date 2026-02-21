use sqlx::PgPool;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::memory::episodic;
use crate::types::{EventSource, SensoryEvent};

/// Scan for high-salience episodes eligible for replay.
/// Returns them as internal SensoryEvents to be re-injected into the tick loop.
pub async fn scan_for_replay(
    pool: &PgPool,
    min_salience: f32,
    limit: i64,
) -> Result<Vec<SensoryEvent>, sqlx::Error> {
    let episodes = episodic::fetch_for_replay(pool, min_salience, limit).await?;

    let events = episodes
        .into_iter()
        .map(|ep| SensoryEvent {
            id: uuid::Uuid::new_v4(),
            source: EventSource::Internal,
            content: format!("[replay] {}", ep.content),
            timestamp: chrono::Utc::now(),
        })
        .collect();

    Ok(events)
}

/// Spawn the replay background task.
/// Periodically scans for high-salience episodes and re-injects them as internal events.
pub fn spawn(
    pool: PgPool,
    event_tx: mpsc::Sender<SensoryEvent>,
    min_salience: f32,
    interval_secs: u64,
    cancel: CancellationToken,
) {
    tokio::spawn(async move {
        let interval = std::time::Duration::from_secs(interval_secs);

        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("replay task shutting down");
                    return;
                }
                _ = tokio::time::sleep(interval) => {}
            }

            if cancel.is_cancelled() {
                return;
            }

            match scan_for_replay(&pool, min_salience, 5).await {
                Ok(events) if !events.is_empty() => {
                    let count = events.len();
                    for event in events {
                        if event_tx.send(event).await.is_err() {
                            tracing::warn!("replay: event channel closed");
                            return;
                        }
                    }
                    tracing::info!(replayed = count, "replay cycle injected events");
                }
                Ok(_) => {} // no events to replay
                Err(e) => {
                    tracing::warn!(error = %e, "replay scan failed");
                }
            }
        }
    });
}
