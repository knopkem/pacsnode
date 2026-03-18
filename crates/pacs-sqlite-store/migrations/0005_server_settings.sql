CREATE TABLE server_settings (
    settings_key                 TEXT PRIMARY KEY CHECK (settings_key = 'default'),
    dicom_port                   INTEGER NOT NULL CHECK (dicom_port BETWEEN 1 AND 65535),
    ae_title                     TEXT NOT NULL,
    ae_whitelist_enabled         INTEGER NOT NULL DEFAULT 0,
    accept_all_transfer_syntaxes INTEGER NOT NULL DEFAULT 1,
    accepted_transfer_syntaxes   TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(accepted_transfer_syntaxes)),
    preferred_transfer_syntaxes  TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(preferred_transfer_syntaxes)),
    max_associations             INTEGER NOT NULL CHECK (max_associations > 0),
    dimse_timeout_secs           INTEGER NOT NULL CHECK (dimse_timeout_secs > 0),
    created_at                   TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at                   TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TRIGGER trg_server_settings_updated_at
AFTER UPDATE ON server_settings
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE server_settings
    SET updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE settings_key = NEW.settings_key;
END;