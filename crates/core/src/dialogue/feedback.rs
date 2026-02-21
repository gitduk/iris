use uuid::Uuid;

use crate::types::FeedbackType;

/// Keyword patterns for explicit positive feedback.
const POSITIVE_KEYWORDS: &[&str] = &["thanks", "great", "perfect", "good", "nice", "correct"];
/// Keyword patterns for explicit negative feedback.
const NEGATIVE_KEYWORDS: &[&str] = &["wrong", "bad", "incorrect", "no", "fix", "error"];

/// Detect feedback from user text (layer 1: explicit keywords).
pub fn detect_keyword_feedback(text: &str) -> FeedbackType {
    let lower = text.to_lowercase();
    for kw in POSITIVE_KEYWORDS {
        if lower.contains(kw) {
            return FeedbackType::Positive;
        }
    }
    for kw in NEGATIVE_KEYWORDS {
        if lower.contains(kw) {
            return FeedbackType::Negative;
        }
    }
    FeedbackType::Neutral
}

/// Record feedback to user_preference table.
pub async fn record_preference(
    pool: &sqlx::PgPool,
    request_type: &str,
    feedback: FeedbackType,
) -> Result<(), sqlx::Error> {
    // Upsert: increment frequency if same type+feedback exists
    sqlx::query(
        "INSERT INTO user_preference (id, request_type, feedback, frequency_30d, updated_at)
         VALUES ($1, $2, $3, 1, now())
         ON CONFLICT (request_type, feedback)
         DO UPDATE SET frequency_30d = user_preference.frequency_30d + 1, updated_at = now()",
    )
    .bind(Uuid::new_v4())
    .bind(request_type)
    .bind(feedback.as_str())
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_positive_feedback() {
        assert_eq!(detect_keyword_feedback("thanks!"), FeedbackType::Positive);
        assert_eq!(detect_keyword_feedback("That's great"), FeedbackType::Positive);
    }

    #[test]
    fn detect_negative_feedback() {
        assert_eq!(detect_keyword_feedback("that's wrong"), FeedbackType::Negative);
        assert_eq!(detect_keyword_feedback("please fix this"), FeedbackType::Negative);
    }

    #[test]
    fn detect_neutral_feedback() {
        assert_eq!(detect_keyword_feedback("tell me more"), FeedbackType::Neutral);
    }
}
