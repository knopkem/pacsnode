//! Series-level SQL helpers: upsert, get, query, delete.

use chrono::{DateTime, Utc};
use pacs_core::{DicomJson, PacsResult, Series, SeriesQuery, SeriesUid, StudyUid};
use sqlx::PgPool;

use crate::error::{map_db_err, map_store_err};

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

/// Raw database row returned by series `SELECT` queries.
#[derive(sqlx::FromRow)]
struct SeriesRow {
    series_uid: String,
    study_uid: String,
    modality: Option<String>,
    series_number: Option<i32>,
    description: Option<String>,
    body_part: Option<String>,
    num_instances: i32,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
}

impl From<SeriesRow> for Series {
    fn from(row: SeriesRow) -> Self {
        Series {
            series_uid: SeriesUid::from(row.series_uid),
            study_uid: StudyUid::from(row.study_uid),
            modality: row.modality,
            series_number: row.series_number,
            description: row.description,
            body_part: row.body_part,
            num_instances: row.num_instances,
            metadata: DicomJson::from(row.metadata),
            created_at: Some(row.created_at),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared SELECT fragment
// ---------------------------------------------------------------------------

const SELECT_COLS: &str = r#"
    SELECT series_uid, study_uid, modality, series_number, description,
           body_part, num_instances, metadata, created_at
    FROM   series
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Inserts or updates a [`Series`] row (upsert on `series_uid`).
pub(crate) async fn upsert(pool: &PgPool, series: &Series) -> PacsResult<()> {
    sqlx::query(
        r#"
        INSERT INTO series (
            series_uid, study_uid, modality, series_number, description,
            body_part, num_instances, metadata, created_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, NOW()
        )
        ON CONFLICT (series_uid) DO UPDATE SET
            study_uid     = EXCLUDED.study_uid,
            modality      = EXCLUDED.modality,
            series_number = EXCLUDED.series_number,
            description   = EXCLUDED.description,
            body_part     = EXCLUDED.body_part,
            num_instances = EXCLUDED.num_instances,
            metadata      = EXCLUDED.metadata
        "#,
    )
    .bind(series.series_uid.as_ref())
    .bind(series.study_uid.as_ref())
    .bind(series.modality.as_deref())
    .bind(series.series_number)
    .bind(series.description.as_deref())
    .bind(series.body_part.as_deref())
    .bind(series.num_instances)
    .bind(series.metadata.as_value())
    .execute(pool)
    .await
    .map_err(map_store_err)?;

    Ok(())
}

/// Retrieves a single [`Series`] by its UID.
///
/// Returns [`pacs_core::PacsError::NotFound`] when no matching row exists.
pub(crate) async fn get(pool: &PgPool, uid: &SeriesUid) -> PacsResult<Series> {
    sqlx::query_as::<_, SeriesRow>(&format!("{SELECT_COLS} WHERE series_uid = $1"))
        .bind(uid.as_ref())
        .fetch_one(pool)
        .await
        .map_err(|e| map_db_err(e, "series", uid.as_ref()))
        .map(Series::from)
}

/// Executes a [`SeriesQuery`], returning all matching series for the parent study.
///
/// `study_uid` is always required; all other filters are optional.
pub(crate) async fn query(pool: &PgPool, q: &SeriesQuery) -> PacsResult<Vec<Series>> {
    let mut qb =
        sqlx::QueryBuilder::<sqlx::Postgres>::new(format!("{SELECT_COLS} WHERE study_uid = "));
    qb.push_bind(q.study_uid.as_ref().to_owned());

    if let Some(ref uid) = q.series_uid {
        qb.push(" AND series_uid = ");
        qb.push_bind(uid.as_ref().to_owned());
    }

    if let Some(ref modality) = q.modality {
        qb.push(" AND modality = ");
        qb.push_bind(modality.clone());
    }

    if let Some(num) = q.series_number {
        qb.push(" AND series_number = ");
        qb.push_bind(num);
    }

    let limit = i64::from(q.limit.unwrap_or(100));
    let offset = i64::from(q.offset.unwrap_or(0));
    qb.push(" ORDER BY series_number ASC NULLS LAST LIMIT ");
    qb.push_bind(limit);
    qb.push(" OFFSET ");
    qb.push_bind(offset);

    qb.build_query_as::<SeriesRow>()
        .fetch_all(pool)
        .await
        .map_err(map_store_err)
        .map(|rows| rows.into_iter().map(Series::from).collect())
}

/// Deletes a [`Series`] by UID. Cascades to instances via FK.
///
/// Returns [`pacs_core::PacsError::NotFound`] when no matching row exists.
pub(crate) async fn delete(pool: &PgPool, uid: &SeriesUid) -> PacsResult<()> {
    let result = sqlx::query("DELETE FROM series WHERE series_uid = $1")
        .bind(uid.as_ref())
        .execute(pool)
        .await
        .map_err(map_store_err)?;

    if result.rows_affected() == 0 {
        return Err(pacs_core::PacsError::NotFound {
            resource: "series",
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

    fn make_series_row() -> SeriesRow {
        SeriesRow {
            series_uid: "2.3.4.5".to_string(),
            study_uid: "1.2.3.4".to_string(),
            modality: Some("CT".to_string()),
            series_number: Some(1),
            description: Some("Axial".to_string()),
            body_part: Some("CHEST".to_string()),
            num_instances: 10,
            metadata: json!({"00080060": {"vr": "CS", "Value": ["CT"]}}),
            created_at: Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn series_row_converts_to_series() {
        let row = make_series_row();
        let series = Series::from(row);

        assert_eq!(series.series_uid.as_ref(), "2.3.4.5");
        assert_eq!(series.study_uid.as_ref(), "1.2.3.4");
        assert_eq!(series.modality.as_deref(), Some("CT"));
        assert_eq!(series.series_number, Some(1));
        assert_eq!(series.description.as_deref(), Some("Axial"));
        assert_eq!(series.body_part.as_deref(), Some("CHEST"));
        assert_eq!(series.num_instances, 10);
        assert!(series.created_at.is_some());
    }

    #[test]
    fn optional_fields_round_trip_as_none() {
        let mut row = make_series_row();
        row.modality = None;
        row.series_number = None;
        row.description = None;
        row.body_part = None;
        let series = Series::from(row);
        assert!(series.modality.is_none());
        assert!(series.series_number.is_none());
        assert!(series.description.is_none());
        assert!(series.body_part.is_none());
    }

    #[test]
    fn metadata_round_trips_through_dicom_json() {
        let val = json!({"tag": "value"});
        let mut row = make_series_row();
        row.metadata = val.clone();
        let series = Series::from(row);
        assert_eq!(series.metadata.as_value(), &val);
    }
}
