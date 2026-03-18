-- Migration 0006: local users, refresh tokens, and password policy

CREATE TABLE users (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    username              TEXT        NOT NULL UNIQUE,
    display_name          TEXT,
    email                 TEXT,
    password_hash         TEXT        NOT NULL,
    role                  TEXT        NOT NULL,
    attributes            JSONB       NOT NULL DEFAULT '{}'::jsonb,
    is_active             BOOLEAN     NOT NULL DEFAULT TRUE,
    failed_login_attempts INTEGER     NOT NULL DEFAULT 0,
    locked_until          TIMESTAMPTZ,
    password_changed_at   TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT chk_users_role CHECK (
        role IN ('admin', 'radiologist', 'technologist', 'viewer', 'uploader')
    )
);

CREATE INDEX idx_users_username ON users(username);
CREATE INDEX idx_users_email ON users(email);
CREATE INDEX idx_users_role ON users(role);
CREATE INDEX idx_users_is_active ON users(is_active);

CREATE TRIGGER trg_users_updated_at
    BEFORE UPDATE ON users
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

CREATE TABLE refresh_tokens (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash  TEXT        NOT NULL UNIQUE,
    expires_at  TIMESTAMPTZ NOT NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at  TIMESTAMPTZ
);

CREATE INDEX idx_refresh_tokens_user_id ON refresh_tokens(user_id);
CREATE INDEX idx_refresh_tokens_expires_at ON refresh_tokens(expires_at);

CREATE TABLE password_policy (
    policy_key            TEXT        PRIMARY KEY CHECK (policy_key = 'default'),
    min_length            INTEGER     NOT NULL DEFAULT 12 CHECK (min_length > 0),
    require_uppercase     BOOLEAN     NOT NULL DEFAULT TRUE,
    require_digit         BOOLEAN     NOT NULL DEFAULT TRUE,
    require_special       BOOLEAN     NOT NULL DEFAULT FALSE,
    max_failed_attempts   INTEGER     NOT NULL DEFAULT 5 CHECK (max_failed_attempts > 0),
    lockout_duration_secs INTEGER     NOT NULL DEFAULT 900 CHECK (lockout_duration_secs > 0),
    max_age_days          INTEGER,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER trg_password_policy_updated_at
    BEFORE UPDATE ON password_policy
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

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
    'default', 12, TRUE, TRUE, FALSE, 5, 900, NULL
)
ON CONFLICT (policy_key) DO NOTHING;