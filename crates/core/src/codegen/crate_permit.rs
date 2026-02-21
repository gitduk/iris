use sqlx::PgPool;

/// Crates that are always approved (standard library ecosystem).
const AUTO_APPROVED: &[&str] = &["std", "core", "alloc"];

/// Check if a crate is auto-approved (std/core/alloc).
pub fn is_auto_approved(crate_name: &str) -> bool {
    AUTO_APPROVED.contains(&crate_name)
}

/// Check if a crate is approved (auto-approved or in the DB).
pub async fn is_approved(pool: &PgPool, crate_name: &str) -> Result<bool, sqlx::Error> {
    if is_auto_approved(crate_name) {
        return Ok(true);
    }
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT crate_name FROM approved_crates WHERE crate_name = $1",
    )
    .bind(crate_name)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Approve a crate (insert into approved_crates table).
pub async fn approve(pool: &PgPool, crate_name: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO approved_crates (crate_name) VALUES ($1) ON CONFLICT DO NOTHING",
    )
    .bind(crate_name)
    .execute(pool)
    .await?;
    Ok(())
}

/// Filter a list of crate names, returning only those not yet approved.
pub async fn unapproved(pool: &PgPool, crates: &[String]) -> Result<Vec<String>, sqlx::Error> {
    let mut result = Vec::new();
    for c in crates {
        if !is_approved(pool, c).await? {
            result.push(c.clone());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_approved_crates() {
        assert!(is_auto_approved("std"));
        assert!(is_auto_approved("core"));
        assert!(is_auto_approved("alloc"));
        assert!(!is_auto_approved("tokio"));
        assert!(!is_auto_approved("serde"));
    }
}
