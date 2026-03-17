CREATE TABLE audit_log (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    occurred_at  TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
    user_id      TEXT,
    action       TEXT NOT NULL,
    resource     TEXT NOT NULL,
    resource_uid TEXT,
    source_ip    TEXT,
    status       TEXT NOT NULL DEFAULT 'ok',
    details      TEXT CHECK (details IS NULL OR json_valid(details))
);

CREATE INDEX idx_audit_occurred_at ON audit_log(occurred_at);
CREATE INDEX idx_audit_user_id ON audit_log(user_id);
CREATE INDEX idx_audit_resource_uid ON audit_log(resource_uid);
