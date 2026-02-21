-- LLM provider configuration (seeded from env vars on first boot)
CREATE TABLE IF NOT EXISTS llm_provider_config (
    id          UUID PRIMARY KEY,
    provider    TEXT NOT NULL,
    api_key     TEXT NOT NULL,
    base_url    TEXT,
    model       TEXT NOT NULL,
    priority    INT NOT NULL DEFAULT 0,
    is_active   BOOLEAN NOT NULL DEFAULT TRUE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
