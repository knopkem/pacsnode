-- Migration 0002: known remote DICOM Application Entities

CREATE TABLE dicom_nodes (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    ae_title    TEXT        NOT NULL UNIQUE,
    host        TEXT        NOT NULL,
    port        INTEGER     NOT NULL CHECK (port BETWEEN 1 AND 65535),
    description TEXT,
    tls_enabled BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- set_updated_at() was created in 0001_initial.sql
CREATE TRIGGER trg_dicom_nodes_updated_at
    BEFORE UPDATE ON dicom_nodes
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();
