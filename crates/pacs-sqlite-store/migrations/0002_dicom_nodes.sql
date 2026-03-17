CREATE TABLE dicom_nodes (
    ae_title    TEXT PRIMARY KEY,
    host        TEXT NOT NULL,
    port        INTEGER NOT NULL CHECK (port BETWEEN 1 AND 65535),
    description TEXT,
    tls_enabled INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at  TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TRIGGER trg_dicom_nodes_updated_at
AFTER UPDATE ON dicom_nodes
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE dicom_nodes
    SET updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE ae_title = NEW.ae_title;
END;
