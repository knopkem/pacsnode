//! Study-level SQL helpers: upsert, get, query, delete.

use chrono::{DateTime, NaiveDate, Utc};
use pacs_core::{DicomJson, PacsResult, Study, StudyQuery, StudyUid};
use sqlx::PgPool;

use crate::error::{map_db_err, map_store_err};

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

/// Raw database row returned by study `SELECT` queries.
#[derive(sqlx::FromRow)]
struct StudyRow {
    study_uid: String,
    patient_id: Option<String>,
    patient_name: Option<String>,
    study_date: Option<NaiveDate>,
    study_time: Option<String>,
    accession_number: Option<String>,
    /// `TEXT[]` — nullable in schema, decoded as `Option<Vec<String>>`.
    modalities: Option<Vec<String>>,
    referring_physician: Option<String>,
    description: Option<String>,
    num_series: i32,
    num_instances: i32,
    metadata: serde_json::Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<StudyRow> for Study {
    fn from(row: StudyRow) -> Self {
        Study {
            study_uid: StudyUid::from(row.study_uid),
            patient_id: row.patient_id,
            patient_name: row.patient_name,
            study_date: row.study_date,
            study_time: row.study_time,
            accession_number: row.accession_number,
            modalities: row.modalities.unwrap_or_default(),
            referring_physician: row.referring_physician,
            description: row.description,
            num_series: row.num_series,
            num_instances: row.num_instances,
            metadata: DicomJson::from(row.metadata),
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared SELECT fragment
// ---------------------------------------------------------------------------

const SELECT_COLS: &str = r#"
    SELECT studies.study_uid, patient_id, patient_name, study_date, study_time,
           accession_number, modalities, referring_physician, description,
           COALESCE((SELECT COUNT(*)::int FROM series WHERE series.study_uid = studies.study_uid), 0) AS num_series,
           COALESCE((SELECT COUNT(*)::int FROM instances WHERE instances.study_uid = studies.study_uid), 0) AS num_instances,
           metadata, created_at, updated_at
    FROM   studies
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Inserts or updates a [`Study`] row (upsert on `study_uid`).
pub(crate) async fn upsert(pool: &PgPool, study: &Study) -> PacsResult<()> {
    sqlx::query(
        r#"
        INSERT INTO studies (
            study_uid, patient_id, patient_name, study_date, study_time,
            accession_number, modalities, referring_physician, description,
            num_series, num_instances, metadata, created_at, updated_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NOW(), NOW()
        )
        ON CONFLICT (study_uid) DO UPDATE SET
            patient_id          = EXCLUDED.patient_id,
            patient_name        = EXCLUDED.patient_name,
            study_date          = EXCLUDED.study_date,
            study_time          = EXCLUDED.study_time,
            accession_number    = EXCLUDED.accession_number,
            modalities          = EXCLUDED.modalities,
            referring_physician = EXCLUDED.referring_physician,
            description         = EXCLUDED.description,
            metadata            = EXCLUDED.metadata,
            updated_at          = NOW()
        "#,
    )
    .bind(study.study_uid.as_ref())
    .bind(study.patient_id.as_deref())
    .bind(study.patient_name.as_deref())
    .bind(study.study_date)
    .bind(study.study_time.as_deref())
    .bind(study.accession_number.as_deref())
    .bind(&study.modalities)
    .bind(study.referring_physician.as_deref())
    .bind(study.description.as_deref())
    .bind(study.num_series)
    .bind(study.num_instances)
    .bind(study.metadata.as_value())
    .execute(pool)
    .await
    .map_err(map_store_err)?;

    Ok(())
}

/// Retrieves a single [`Study`] by its UID.
///
/// Returns [`pacs_core::PacsError::NotFound`] when no matching row exists.
pub(crate) async fn get(pool: &PgPool, uid: &StudyUid) -> PacsResult<Study> {
    sqlx::query_as::<_, StudyRow>(&format!("{SELECT_COLS} WHERE study_uid = $1"))
        .bind(uid.as_ref())
        .fetch_one(pool)
        .await
        .map_err(|e| map_db_err(e, "study", uid.as_ref()))
        .map(Study::from)
}

/// Executes a dynamic [`StudyQuery`], returning all matching studies.
///
/// Applies filters in order: patient_id, patient_name (with optional fuzzy
/// matching), date range, accession_number, study_uid, modality.
/// Applies ordering and optional pagination.
pub(crate) async fn query(pool: &PgPool, q: &StudyQuery) -> PacsResult<Vec<Study>> {
    let mut qb = sqlx::QueryBuilder::<sqlx::Postgres>::new(format!("{SELECT_COLS} WHERE 1=1"));

    if let Some(ref pid) = q.patient_id {
        if pid.contains('*') || pid.contains('?') {
            qb.push(" AND patient_id LIKE ");
            qb.push_bind(pid.replace('*', "%").replace('?', "_"));
        } else {
            qb.push(" AND patient_id = ");
            qb.push_bind(pid.clone());
        }
    }

    if let Some(ref pname) = q.patient_name {
        if q.fuzzy_matching || pname.contains('*') || pname.contains('?') {
            qb.push(" AND patient_name ILIKE ");
            qb.push_bind(pname.replace('*', "%").replace('?', "_"));
        } else {
            qb.push(" AND patient_name = ");
            qb.push_bind(pname.clone());
        }
    }

    if let Some(from) = q.study_date_from {
        qb.push(" AND study_date >= ");
        qb.push_bind(from);
    }

    if let Some(to) = q.study_date_to {
        qb.push(" AND study_date <= ");
        qb.push_bind(to);
    }

    if let Some(ref acc) = q.accession_number {
        qb.push(" AND accession_number = ");
        qb.push_bind(acc.clone());
    }

    if let Some(ref uid) = q.study_uid {
        qb.push(" AND study_uid = ");
        qb.push_bind(uid.as_ref().to_owned());
    }

    if let Some(ref modality) = q.modality {
        qb.push(" AND ");
        qb.push_bind(modality.clone());
        qb.push(" = ANY(modalities)");
    }

    qb.push(" ORDER BY created_at DESC");
    if let Some(limit) = q.limit {
        qb.push(" LIMIT ");
        qb.push_bind(i64::from(limit));
    }
    if let Some(offset) = q.offset {
        qb.push(" OFFSET ");
        qb.push_bind(i64::from(offset));
    }

    qb.build_query_as::<StudyRow>()
        .fetch_all(pool)
        .await
        .map_err(map_store_err)
        .map(|rows| rows.into_iter().map(Study::from).collect())
}

/// Deletes a [`Study`] by UID. Cascades to series and instances via FK.
///
/// Returns [`pacs_core::PacsError::NotFound`] when no matching row exists.
pub(crate) async fn delete(pool: &PgPool, uid: &StudyUid) -> PacsResult<()> {
    let result = sqlx::query("DELETE FROM studies WHERE study_uid = $1")
        .bind(uid.as_ref())
        .execute(pool)
        .await
        .map_err(map_store_err)?;

    if result.rows_affected() == 0 {
        return Err(pacs_core::PacsError::NotFound {
            resource: "study",
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

    fn make_study_row() -> StudyRow {
        StudyRow {
            study_uid: "1.2.3.4".to_string(),
            patient_id: Some("PID001".to_string()),
            patient_name: Some("Doe^John".to_string()),
            study_date: Some(NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()),
            study_time: Some("120000".to_string()),
            accession_number: Some("ACC001".to_string()),
            modalities: Some(vec!["CT".to_string(), "PT".to_string()]),
            referring_physician: Some("Dr. Smith".to_string()),
            description: Some("Chest CT".to_string()),
            num_series: 2,
            num_instances: 20,
            metadata: json!({"00080060": {"vr": "CS"}}),
            created_at: Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
            updated_at: Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        }
    }

    #[test]
    fn study_row_converts_to_study() {
        let row = make_study_row();
        let study = Study::from(row);

        assert_eq!(study.study_uid.as_ref(), "1.2.3.4");
        assert_eq!(study.patient_id.as_deref(), Some("PID001"));
        assert_eq!(study.modalities, vec!["CT", "PT"]);
        assert_eq!(study.num_series, 2);
        assert_eq!(study.num_instances, 20);
        assert!(study.created_at.is_some());
        assert!(study.updated_at.is_some());
    }

    #[test]
    fn null_modalities_become_empty_vec() {
        let mut row = make_study_row();
        row.modalities = None;
        let study = Study::from(row);
        assert!(study.modalities.is_empty());
    }

    #[test]
    fn metadata_round_trips_through_dicom_json() {
        let val = json!({"key": "value"});
        let mut row = make_study_row();
        row.metadata = val.clone();
        let study = Study::from(row);
        assert_eq!(study.metadata.as_value(), &val);
    }
}
