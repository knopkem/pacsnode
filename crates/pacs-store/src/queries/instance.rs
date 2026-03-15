//! Instance-level SQL helpers: upsert, get, query, delete, get_metadata.

use chrono::{DateTime, Utc};
use pacs_core::{
    DicomJson, Instance, InstanceQuery, PacsResult, SeriesUid, SopInstanceUid, StudyUid,
};
use sqlx::PgPool;

use crate::error::{map_db_err, map_store_err};

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

/// Raw database row returned by instance `SELECT` queries.
#[derive(sqlx::FromRow)]
struct InstanceRow {
    instance_uid: String,
    series_uid: String,
    study_uid: String,
    sop_class_uid: Option<String>,
    instance_number: Option<i32>,
    transfer_syntax: Option<String>,
    rows: Option<i32>,
    columns: Option<i32>,
    blob_key: String,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
}

impl From<InstanceRow> for Instance {
    fn from(row: InstanceRow) -> Self {
        Instance {
            instance_uid: SopInstanceUid::from(row.instance_uid),
            series_uid: SeriesUid::from(row.series_uid),
            study_uid: StudyUid::from(row.study_uid),
            sop_class_uid: row.sop_class_uid,
            instance_number: row.instance_number,
            transfer_syntax: row.transfer_syntax,
            rows: row.rows,
            columns: row.columns,
            blob_key: row.blob_key,
            metadata: DicomJson::from(row.metadata),
            created_at: Some(row.created_at),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared SELECT fragment
// ---------------------------------------------------------------------------

const SELECT_COLS: &str = r#"
    SELECT instance_uid, series_uid, study_uid, sop_class_uid, instance_number,
           transfer_syntax, rows, columns, blob_key, metadata, created_at
    FROM   instances
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Inserts or updates an [`Instance`] row (upsert on `instance_uid`).
pub(crate) async fn upsert(pool: &PgPool, instance: &Instance) -> PacsResult<()> {
    sqlx::query(
        r#"
        INSERT INTO instances (
            instance_uid, series_uid, study_uid, sop_class_uid, instance_number,
            transfer_syntax, rows, columns, blob_key, metadata, created_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW()
        )
        ON CONFLICT (instance_uid) DO UPDATE SET
            series_uid      = EXCLUDED.series_uid,
            study_uid       = EXCLUDED.study_uid,
            sop_class_uid   = EXCLUDED.sop_class_uid,
            instance_number = EXCLUDED.instance_number,
            transfer_syntax = EXCLUDED.transfer_syntax,
            rows            = EXCLUDED.rows,
            columns         = EXCLUDED.columns,
            blob_key        = EXCLUDED.blob_key,
            metadata        = EXCLUDED.metadata
        "#,
    )
    .bind(instance.instance_uid.as_ref())
    .bind(instance.series_uid.as_ref())
    .bind(instance.study_uid.as_ref())
    .bind(instance.sop_class_uid.as_deref())
    .bind(instance.instance_number)
    .bind(instance.transfer_syntax.as_deref())
    .bind(instance.rows)
    .bind(instance.columns)
    .bind(instance.blob_key.as_str())
    .bind(instance.metadata.as_value())
    .execute(pool)
    .await
    .map_err(map_store_err)?;

    Ok(())
}

/// Retrieves a single [`Instance`] by its UID.
///
/// Returns [`pacs_core::PacsError::NotFound`] when no matching row exists.
pub(crate) async fn get(pool: &PgPool, uid: &SopInstanceUid) -> PacsResult<Instance> {
    sqlx::query_as::<_, InstanceRow>(&format!("{SELECT_COLS} WHERE instance_uid = $1"))
        .bind(uid.as_ref())
        .fetch_one(pool)
        .await
        .map_err(|e| map_db_err(e, "instance", uid.as_ref()))
        .map(Instance::from)
}

/// Retrieves only the [`DicomJson`] metadata for an instance.
///
/// More efficient than [`get`] when only the DICOM JSON is needed.
/// Returns [`pacs_core::PacsError::NotFound`] when no matching row exists.
pub(crate) async fn get_metadata(pool: &PgPool, uid: &SopInstanceUid) -> PacsResult<DicomJson> {
    #[derive(sqlx::FromRow)]
    struct MetaRow {
        metadata: serde_json::Value,
    }

    sqlx::query_as::<_, MetaRow>("SELECT metadata FROM instances WHERE instance_uid = $1")
        .bind(uid.as_ref())
        .fetch_one(pool)
        .await
        .map_err(|e| map_db_err(e, "instance", uid.as_ref()))
        .map(|row| DicomJson::from(row.metadata))
}

/// Executes an [`InstanceQuery`], returning all matching instances for the parent series.
///
/// `series_uid` is always required; all other filters are optional.
pub(crate) async fn query(pool: &PgPool, q: &InstanceQuery) -> PacsResult<Vec<Instance>> {
    let mut qb =
        sqlx::QueryBuilder::<sqlx::Postgres>::new(format!("{SELECT_COLS} WHERE series_uid = "));
    qb.push_bind(q.series_uid.as_ref().to_owned());

    if let Some(ref uid) = q.instance_uid {
        qb.push(" AND instance_uid = ");
        qb.push_bind(uid.as_ref().to_owned());
    }

    if let Some(ref sop) = q.sop_class_uid {
        qb.push(" AND sop_class_uid = ");
        qb.push_bind(sop.clone());
    }

    if let Some(num) = q.instance_number {
        qb.push(" AND instance_number = ");
        qb.push_bind(num);
    }

    let limit = i64::from(q.limit.unwrap_or(100));
    let offset = i64::from(q.offset.unwrap_or(0));
    qb.push(" ORDER BY instance_number ASC NULLS LAST LIMIT ");
    qb.push_bind(limit);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    qb.build_query_as::<InstanceRow>()
        .fetch_all(pool)
        .await
        .map_err(map_store_err)
        .map(|rows| rows.into_iter().map(Instance::from).collect())
}

/// Deletes a single [`Instance`] by UID.
///
/// Returns [`pacs_core::PacsError::NotFound`] when no matching row exists.
pub(crate) async fn delete(pool: &PgPool, uid: &SopInstanceUid) -> PacsResult<()> {
    let result = sqlx::query("DELETE FROM instances WHERE instance_uid = $1")
        .bind(uid.as_ref())
        .execute(pool)
        .await
        .map_err(map_store_err)?;

    if result.rows_affected() == 0 {
        return Err(pacs_core::PacsError::NotFound {
            resource: "instance",
            uid: uid.to_string(),
        });
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    fn make_instance_row() -> InstanceRow {
        InstanceRow {
            instance_uid: "3.4.5.6".to_string(),
            series_uid: "2.3.4.5".to_string(),
            study_uid: "1.2.3.4".to_string(),
            sop_class_uid: Some("1.2.840.10008.5.1.4.1.1.2".to_string()),
            instance_number: Some(1),
            transfer_syntax: Some("1.2.840.10008.1.2.1".to_string()),
            rows: Some(512),
            columns: Some(512),
            blob_key: "1.2.3.4/2.3.4.5/3.4.5.6".to_string(),
            metadata: json!({"00280010": {"vr": "US", "Value": [512]}}),
            created_at: Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn instance_row_converts_to_instance() {
        let row = make_instance_row();
        let inst = Instance::from(row);

        assert_eq!(inst.instance_uid.as_ref(), "3.4.5.6");
        assert_eq!(inst.series_uid.as_ref(), "2.3.4.5");
        assert_eq!(inst.study_uid.as_ref(), "1.2.3.4");
        assert_eq!(
            inst.sop_class_uid.as_deref(),
            Some("1.2.840.10008.5.1.4.1.1.2")
        );
        assert_eq!(inst.instance_number, Some(1));
        assert_eq!(inst.rows, Some(512));
        assert_eq!(inst.columns, Some(512));
        assert_eq!(inst.blob_key, "1.2.3.4/2.3.4.5/3.4.5.6");
        assert!(inst.created_at.is_some());
    }

    #[test]
    fn optional_fields_round_trip_as_none() {
        let mut row = make_instance_row();
        row.sop_class_uid = None;
        row.instance_number = None;
        row.transfer_syntax = None;
        row.rows = None;
        row.columns = None;
        let inst = Instance::from(row);
        assert!(inst.sop_class_uid.is_none());
        assert!(inst.instance_number.is_none());
        assert!(inst.transfer_syntax.is_none());
        assert!(inst.rows.is_none());
        assert!(inst.columns.is_none());
    }

    #[test]
    fn metadata_round_trips_through_dicom_json() {
        let val = json!({"00280010": {"vr": "US"}});
        let mut row = make_instance_row();
        row.metadata = val.clone();
        let inst = Instance::from(row);
        assert_eq!(inst.metadata.as_value(), &val);
    }
}
