-- codegen history (tracks gap → codegen attempts)
CREATE TABLE IF NOT EXISTS codegen_history (
    id                  UUID PRIMARY KEY,
    gap_type            TEXT NOT NULL,
    approach_summary    TEXT,
    success             BOOLEAN NOT NULL DEFAULT FALSE,
    error_msg           TEXT,
    consolidated_flag   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- approved external crates for codegen
CREATE TABLE IF NOT EXISTS approved_crates (
    crate_name  TEXT PRIMARY KEY,
    approved_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    approved_by TEXT NOT NULL DEFAULT 'user'
);
