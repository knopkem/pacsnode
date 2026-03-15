-- Migration 0004: triggers to keep denormalized counts consistent
--
-- Maintained counters
--   studies.num_series    — incremented/decremented by series INSERT/DELETE
--   studies.num_instances — incremented/decremented by instances INSERT/DELETE
--   series.num_instances  — incremented/decremented by instances INSERT/DELETE
--
-- Using AFTER … FOR EACH ROW so the referencing row already exists when the
-- trigger fires (important for FK-constrained INSERTs).

-- ── series → studies.num_series ──────────────────────────────────────────────

CREATE OR REPLACE FUNCTION _trg_series_inc_study_counts()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    UPDATE studies SET num_series = num_series + 1 WHERE study_uid = NEW.study_uid;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION _trg_series_dec_study_counts()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    UPDATE studies SET num_series = num_series - 1 WHERE study_uid = OLD.study_uid;
    RETURN NULL;
END;
$$;

CREATE TRIGGER trg_series_after_insert
    AFTER INSERT ON series
    FOR EACH ROW EXECUTE FUNCTION _trg_series_inc_study_counts();

CREATE TRIGGER trg_series_after_delete
    AFTER DELETE ON series
    FOR EACH ROW EXECUTE FUNCTION _trg_series_dec_study_counts();

-- ── instances → series.num_instances + studies.num_instances ─────────────────

CREATE OR REPLACE FUNCTION _trg_instances_inc_counts()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    UPDATE series  SET num_instances = num_instances + 1 WHERE series_uid = NEW.series_uid;
    UPDATE studies SET num_instances = num_instances + 1 WHERE study_uid  = NEW.study_uid;
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION _trg_instances_dec_counts()
RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    UPDATE series  SET num_instances = num_instances - 1 WHERE series_uid = OLD.series_uid;
    UPDATE studies SET num_instances = num_instances - 1 WHERE study_uid  = OLD.study_uid;
    RETURN NULL;
END;
$$;

CREATE TRIGGER trg_instances_after_insert
    AFTER INSERT ON instances
    FOR EACH ROW EXECUTE FUNCTION _trg_instances_inc_counts();

CREATE TRIGGER trg_instances_after_delete
    AFTER DELETE ON instances
    FOR EACH ROW EXECUTE FUNCTION _trg_instances_dec_counts();
