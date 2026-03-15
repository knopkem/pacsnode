-- Migration 0003: append-only HIPAA audit trail
--
-- IMPORTANT: application code must NEVER issue UPDATE or DELETE against this
-- table.  Row-level security or a dedicated DB role (audit_writer) with only
-- INSERT + SELECT can be added to enforce this at the database layer.

CREATE TABLE audit_log (
    id           BIGSERIAL   PRIMARY KEY,
    occurred_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    user_id      TEXT,
    action       TEXT        NOT NULL, -- STORE | RETRIEVE | QUERY | DELETE | …
    resource     TEXT        NOT NULL, -- study | series | instance
    resource_uid TEXT,                 -- UID of accessed resource (never PHI)
    source_ip    TEXT,
    status       TEXT        NOT NULL DEFAULT 'ok', -- ok | error
    details      JSONB
);

CREATE INDEX idx_audit_occurred_at  ON audit_log(occurred_at);
CREATE INDEX idx_audit_user_id      ON audit_log(user_id);
CREATE INDEX idx_audit_resource_uid ON audit_log(resource_uid);
