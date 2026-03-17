CREATE TABLE studies (
    study_uid           TEXT PRIMARY KEY,
    patient_id          TEXT,
    patient_name        TEXT,
    study_date          TEXT,
    study_time          TEXT,
    accession_number    TEXT,
    modalities          TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(modalities)),
    referring_physician TEXT,
    description         TEXT,
    num_series          INTEGER NOT NULL DEFAULT 0,
    num_instances       INTEGER NOT NULL DEFAULT 0,
    metadata            TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    created_at          TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at          TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_studies_patient_id ON studies(patient_id);
CREATE INDEX idx_studies_patient_name ON studies(patient_name);
CREATE INDEX idx_studies_study_date ON studies(study_date);
CREATE INDEX idx_studies_accession ON studies(accession_number);
CREATE INDEX idx_studies_updated_at ON studies(updated_at);

CREATE TRIGGER trg_studies_updated_at
AFTER UPDATE ON studies
FOR EACH ROW
WHEN NEW.updated_at = OLD.updated_at
BEGIN
    UPDATE studies
    SET updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE study_uid = NEW.study_uid;
END;

CREATE TABLE series (
    series_uid    TEXT PRIMARY KEY,
    study_uid     TEXT NOT NULL REFERENCES studies(study_uid) ON DELETE CASCADE,
    modality      TEXT,
    series_number INTEGER,
    description   TEXT,
    body_part     TEXT,
    num_instances INTEGER NOT NULL DEFAULT 0,
    metadata      TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    created_at    TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_series_study_uid ON series(study_uid);
CREATE INDEX idx_series_modality ON series(modality);

CREATE TABLE instances (
    instance_uid    TEXT PRIMARY KEY,
    series_uid      TEXT NOT NULL REFERENCES series(series_uid) ON DELETE CASCADE,
    study_uid       TEXT NOT NULL REFERENCES studies(study_uid) ON DELETE CASCADE,
    sop_class_uid   TEXT,
    instance_number INTEGER,
    transfer_syntax TEXT,
    rows            INTEGER,
    columns         INTEGER,
    blob_key        TEXT NOT NULL,
    metadata        TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(metadata)),
    created_at      TEXT NOT NULL DEFAULT (STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_instances_series_uid ON instances(series_uid);
CREATE INDEX idx_instances_study_uid ON instances(study_uid);
CREATE INDEX idx_instances_sop_class ON instances(sop_class_uid);
