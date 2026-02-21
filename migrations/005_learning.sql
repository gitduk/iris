-- user preference tracking
CREATE TABLE IF NOT EXISTS user_preference (
    id              UUID PRIMARY KEY,
    request_type    TEXT NOT NULL,
    feedback        TEXT NOT NULL CHECK (feedback IN ('positive','negative','neutral')),
    frequency_30d   INT NOT NULL DEFAULT 1,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- boot health records
CREATE TABLE IF NOT EXISTS boot_health_record (
    id          UUID PRIMARY KEY,
    booted_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    success     BOOLEAN NOT NULL,
    error_msg   TEXT,
    duration_ms BIGINT
);
