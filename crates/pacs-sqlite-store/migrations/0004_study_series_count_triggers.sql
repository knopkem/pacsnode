CREATE TRIGGER trg_series_after_insert
AFTER INSERT ON series
FOR EACH ROW
BEGIN
    UPDATE studies
    SET num_series = num_series + 1
    WHERE study_uid = NEW.study_uid;
END;

CREATE TRIGGER trg_series_after_delete
AFTER DELETE ON series
FOR EACH ROW
BEGIN
    UPDATE studies
    SET num_series = num_series - 1
    WHERE study_uid = OLD.study_uid;
END;

CREATE TRIGGER trg_instances_after_insert
AFTER INSERT ON instances
FOR EACH ROW
BEGIN
    UPDATE series
    SET num_instances = num_instances + 1
    WHERE series_uid = NEW.series_uid;

    UPDATE studies
    SET num_instances = num_instances + 1
    WHERE study_uid = NEW.study_uid;
END;

CREATE TRIGGER trg_instances_after_delete
AFTER DELETE ON instances
FOR EACH ROW
BEGIN
    UPDATE series
    SET num_instances = num_instances - 1
    WHERE series_uid = OLD.series_uid;

    UPDATE studies
    SET num_instances = num_instances - 1
    WHERE study_uid = OLD.study_uid;
END;
