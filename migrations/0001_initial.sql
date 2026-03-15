-- Migration 0001: core DICOM schema
-- studies, series, instances + updated_at bookkeeping trigger

-- ── Helper: auto-refresh updated_at ─────────────────────────────────────────
CREATE OR REPLACE FUNCTION set_updated_at()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$;

-- ── studies ──────────────────────────────────────────────────────────────────
CREATE TABLE studies (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    study_uid           TEXT        NOT NULL UNIQUE,
    patient_id          TEXT,
    patient_name        TEXT,
    study_date          DATE,
    study_time          TEXT,
    accession_number    TEXT,
    modalities          TEXT[],
    referring_physician TEXT,
    description         TEXT,
    num_series          INTEGER     NOT NULL DEFAULT 0,
    num_instances       INTEGER     NOT NULL DEFAULT 0,
    metadata            JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_studies_patient_id   ON studies(patient_id);
CREATE INDEX idx_studies_patient_name ON studies(patient_name);
CREATE INDEX idx_studies_study_date   ON studies(study_date);
CREATE INDEX idx_studies_accession    ON studies(accession_number);
CREATE INDEX idx_studies_modalities   ON studies USING GIN(modalities);
CREATE INDEX idx_studies_metadata     ON studies USING GIN(metadata jsonb_path_ops);
CREATE INDEX idx_studies_updated_at   ON studies(updated_at);

-- Keeps updated_at current for worklist sync and change-tracking queries.
CREATE TRIGGER trg_studies_updated_at
    BEFORE UPDATE ON studies
    FOR EACH ROW EXECUTE FUNCTION set_updated_at();

-- ── series ───────────────────────────────────────────────────────────────────
CREATE TABLE series (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    series_uid    TEXT        NOT NULL UNIQUE,
    study_uid     TEXT        NOT NULL REFERENCES studies(study_uid) ON DELETE CASCADE,
    modality      TEXT,
    series_number INTEGER,
    description   TEXT,
    body_part     TEXT,
    num_instances INTEGER     NOT NULL DEFAULT 0,
    metadata      JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_series_study_uid ON series(study_uid);
CREATE INDEX idx_series_modality  ON series(modality);
CREATE INDEX idx_series_metadata  ON series USING GIN(metadata jsonb_path_ops);

-- ── instances ────────────────────────────────────────────────────────────────
CREATE TABLE instances (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    instance_uid    TEXT        NOT NULL UNIQUE,
    series_uid      TEXT        NOT NULL REFERENCES series(series_uid) ON DELETE CASCADE,
    study_uid       TEXT        NOT NULL,
    sop_class_uid   TEXT,
    instance_number INTEGER,
    transfer_syntax TEXT,
    rows            INTEGER,
    columns         INTEGER,
    blob_key        TEXT        NOT NULL,
    metadata        JSONB       NOT NULL DEFAULT '{}'::jsonb,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_instances_series_uid ON instances(series_uid);
CREATE INDEX idx_instances_study_uid  ON instances(study_uid);
CREATE INDEX idx_instances_sop_class  ON instances(sop_class_uid);
CREATE INDEX idx_instances_metadata   ON instances USING GIN(metadata jsonb_path_ops);
