-- iris_config: all system parameters, loaded at startup
CREATE TABLE IF NOT EXISTS iris_config (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    description TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- capability registry
CREATE TABLE IF NOT EXISTS capability (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL UNIQUE,
    binary_path     TEXT NOT NULL,
    manifest        JSONB NOT NULL,
    state           TEXT NOT NULL DEFAULT 'staged'
                    CHECK (state IN ('staged','active_candidate','confirmed','quarantined','retired')),
    lkg_version     UUID,
    quarantine_count INT NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- capability scoring
CREATE TABLE IF NOT EXISTS capability_score (
    capability_id   UUID PRIMARY KEY REFERENCES capability(id),
    usage_count     BIGINT NOT NULL DEFAULT 0,
    success_count   BIGINT NOT NULL DEFAULT 0,
    fail_count      BIGINT NOT NULL DEFAULT 0,
    quarantine_count INT NOT NULL DEFAULT 0,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- episodic memory
CREATE TABLE IF NOT EXISTS episodes (
    id              UUID PRIMARY KEY,
    topic_id        UUID,
    content         TEXT NOT NULL,
    embedding       BYTEA,
    salience        REAL NOT NULL DEFAULT 0.0,
    is_consolidated BOOLEAN NOT NULL DEFAULT FALSE,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_episodes_not_consolidated
    ON episodes (created_at) WHERE NOT is_consolidated;
CREATE INDEX IF NOT EXISTS idx_episodes_salience
    ON episodes (salience DESC);

-- semantic memory (consolidated knowledge)
CREATE TABLE IF NOT EXISTS knowledge (
    id                  UUID PRIMARY KEY,
    summary             TEXT NOT NULL,
    embedding           BYTEA,
    source_episode_ids  UUID[] NOT NULL DEFAULT '{}',
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);
