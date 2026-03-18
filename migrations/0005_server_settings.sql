-- Migration 0005: persisted DIMSE listener settings managed by the admin UI

CREATE TABLE server_settings (
    settings_key                  TEXT        PRIMARY KEY CHECK (settings_key = 'default'),
    dicom_port                    INTEGER     NOT NULL CHECK (dicom_port BETWEEN 1 AND 65535),
    ae_title                      TEXT        NOT NULL,
    ae_whitelist_enabled          BOOLEAN     NOT NULL DEFAULT FALSE,
    accept_all_transfer_syntaxes  BOOLEAN     NOT NULL DEFAULT TRUE,
    accepted_transfer_syntaxes    TEXT[]      NOT NULL DEFAULT ARRAY[]::TEXT[],
    preferred_transfer_syntaxes   TEXT[]      NOT NULL DEFAULT ARRAY[]::TEXT[],
    max_associations              BIGINT      NOT NULL CHECK (max_associations > 0),
    dimse_timeout_secs            BIGINT      NOT NULL CHECK (dimse_timeout_secs > 0),
    created_at                    TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TRIGGER trg_server_settings_updated_at
    BEFORE UPDATE ON server_settings
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
