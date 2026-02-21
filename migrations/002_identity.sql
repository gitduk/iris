-- core identity (immutable after creation)
CREATE TABLE IF NOT EXISTS iris_identity (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL,
    born_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    founding_values JSONB NOT NULL DEFAULT '{}'
);

-- self-model key-value store
CREATE TABLE IF NOT EXISTS self_model_kv (
    key         TEXT PRIMARY KEY,
    value       JSONB NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- narrative events (life milestones)
CREATE TABLE IF NOT EXISTS narrative_event (
    id              UUID PRIMARY KEY,
    occurred_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    event_type      TEXT NOT NULL,
    description     TEXT NOT NULL,
    significance    REAL NOT NULL DEFAULT 0.5
);

CREATE INDEX IF NOT EXISTS idx_narrative_event_time
    ON narrative_event (occurred_at DESC);
