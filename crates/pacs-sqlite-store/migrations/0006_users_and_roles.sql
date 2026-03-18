CREATE TABLE users (
    id                    TEXT PRIMARY KEY,
    username              TEXT NOT NULL UNIQUE,
    display_name          TEXT,
    email                 TEXT,
    password_hash         TEXT NOT NULL,
    role                  TEXT NOT NULL CHECK (
        role IN ('admin', 'radiologist', 'technologist', 'viewer', 'uploader')
    ),
    attributes            TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(attributes)),
    is_active             INTEGER NOT NULL DEFAULT 1,
    failed_login_attempts INTEGER NOT NULL DEFAULT 0,
    locked_until          TEXT,
    password_changed_at   TEXT,
    created_at            TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at            TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_role ON users(role);
CREATE INDEX idx_users_is_active ON users(is_active);

CREATE TRIGGER trg_users_updated_at
AFTER UPDATE ON users
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE users
    SET updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = NEW.id;
END;

CREATE TABLE refresh_tokens (
    id          TEXT PRIMARY KEY,
    user_id     TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT NOT NULL UNIQUE,
    expires_at  TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
    revoked_at  TEXT
);

CREATE INDEX idx_refresh_tokens_user_id ON refresh_tokens(user_id);
CREATE INDEX idx_refresh_tokens_expires_at ON refresh_tokens(expires_at);

CREATE TABLE password_policy (
    policy_key            TEXT PRIMARY KEY CHECK (policy_key = 'default'),
    min_length            INTEGER NOT NULL DEFAULT 12 CHECK (min_length > 0),
    require_uppercase     INTEGER NOT NULL DEFAULT 1,
    require_digit         INTEGER NOT NULL DEFAULT 1,
    require_special       INTEGER NOT NULL DEFAULT 0,
    max_failed_attempts   INTEGER NOT NULL DEFAULT 5 CHECK (max_failed_attempts > 0),
    lockout_duration_secs INTEGER NOT NULL DEFAULT 900 CHECK (lockout_duration_secs > 0),
    max_age_days          INTEGER,
    created_at            TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at            TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TRIGGER trg_password_policy_updated_at
AFTER UPDATE ON password_policy
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE password_policy
    SET updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE policy_key = NEW.policy_key;
END;

INSERT INTO password_policy (
    policy_key,
    min_length,
    require_uppercase,
    require_digit,
    require_special,
    max_failed_attempts,
    lockout_duration_secs,
    max_age_days
) VALUES (
    'default', 12, 1, 1, 0, 5, 900, NULL
)
ON CONFLICT(policy_key) DO NOTHING;