//! [`SqliteMetadataStore`] — SQLite implementation of [`MetadataStore`].

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use pacs_core::{
    AuditLogEntry, AuditLogPage, AuditLogQuery, DicomJson, DicomNode, Instance, InstanceQuery,
    MetadataStore, NewAuditLogEntry, PacsError, PacsResult, PacsStatistics, PasswordPolicy,
    RefreshToken, RefreshTokenId, Series, SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid,
    Study, StudyQuery, StudyUid, User, UserId, UserQuery,
};
use sqlx::{types::Json, QueryBuilder, Sqlite, SqlitePool};
use tracing::instrument;
use uuid::Uuid;

/// SQLite-backed [`MetadataStore`] for pacsnode.
///
/// Wraps a `sqlx` [`SqlitePool`] and is cheaply cloneable. All trait methods are
/// fully `async` and safe to call from any tokio task.
pub struct SqliteMetadataStore {
    pool: SqlitePool,
}

impl SqliteMetadataStore {
    /// Creates a [`SqliteMetadataStore`] from an existing [`SqlitePool`].
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns a reference to the underlying connection pool.
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

#[derive(sqlx::FromRow)]
struct StudyRow {
    study_uid: String,
    patient_id: Option<String>,
    patient_name: Option<String>,
    study_date: Option<NaiveDate>,
    study_time: Option<String>,
    accession_number: Option<String>,
    modalities: Json<Vec<String>>,
    referring_physician: Option<String>,
    description: Option<String>,
    num_series: i32,
    num_instances: i32,
    metadata: Json<serde_json::Value>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl From<StudyRow> for Study {
    fn from(row: StudyRow) -> Self {
        Self {
            study_uid: StudyUid::from(row.study_uid),
            patient_id: row.patient_id,
            patient_name: row.patient_name,
            study_date: row.study_date,
            study_time: row.study_time,
            accession_number: row.accession_number,
            modalities: row.modalities.0,
            referring_physician: row.referring_physician,
            description: row.description,
            num_series: row.num_series,
            num_instances: row.num_instances,
            metadata: DicomJson::from(row.metadata.0),
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
        }
    }
}

#[derive(sqlx::FromRow)]
struct SeriesRow {
    series_uid: String,
    study_uid: String,
    modality: Option<String>,
    series_number: Option<i32>,
    description: Option<String>,
    body_part: Option<String>,
    num_instances: i32,
    metadata: Json<serde_json::Value>,
    created_at: DateTime<Utc>,
}

impl From<SeriesRow> for Series {
    fn from(row: SeriesRow) -> Self {
        Self {
            series_uid: SeriesUid::from(row.series_uid),
            study_uid: StudyUid::from(row.study_uid),
            modality: row.modality,
            series_number: row.series_number,
            description: row.description,
            body_part: row.body_part,
            num_instances: row.num_instances,
            metadata: DicomJson::from(row.metadata.0),
            created_at: Some(row.created_at),
        }
    }
}

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
    metadata: Json<serde_json::Value>,
    created_at: DateTime<Utc>,
}

impl From<InstanceRow> for Instance {
    fn from(row: InstanceRow) -> Self {
        Self {
            instance_uid: SopInstanceUid::from(row.instance_uid),
            series_uid: SeriesUid::from(row.series_uid),
            study_uid: StudyUid::from(row.study_uid),
            sop_class_uid: row.sop_class_uid,
            instance_number: row.instance_number,
            transfer_syntax: row.transfer_syntax,
            rows: row.rows,
            columns: row.columns,
            blob_key: row.blob_key,
            metadata: DicomJson::from(row.metadata.0),
            created_at: Some(row.created_at),
        }
    }
}

#[derive(sqlx::FromRow)]
struct MetadataRow {
    metadata: Json<serde_json::Value>,
}

#[derive(sqlx::FromRow)]
struct NodeRow {
    ae_title: String,
    host: String,
    port: i64,
    description: Option<String>,
    tls_enabled: bool,
}

#[derive(sqlx::FromRow)]
struct ServerSettingsRow {
    dicom_port: i64,
    ae_title: String,
    ae_whitelist_enabled: bool,
    accept_all_transfer_syntaxes: bool,
    accepted_transfer_syntaxes: String,
    preferred_transfer_syntaxes: String,
    storage_transfer_syntax: Option<String>,
    max_associations: i64,
    dimse_timeout_secs: i64,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    username: String,
    display_name: Option<String>,
    email: Option<String>,
    password_hash: String,
    role: String,
    attributes: Json<serde_json::Value>,
    is_active: bool,
    failed_login_attempts: i64,
    locked_until: Option<DateTime<Utc>>,
    password_changed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct RefreshTokenRow {
    id: String,
    user_id: String,
    token_hash: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

#[derive(sqlx::FromRow)]
struct PasswordPolicyRow {
    min_length: i64,
    require_uppercase: bool,
    require_digit: bool,
    require_special: bool,
    max_failed_attempts: i64,
    lockout_duration_secs: i64,
    max_age_days: Option<i64>,
}

impl TryFrom<ServerSettingsRow> for ServerSettings {
    type Error = PacsError;

    fn try_from(row: ServerSettingsRow) -> Result<Self, Self::Error> {
        Ok(Self {
            dicom_port: row
                .dicom_port
                .try_into()
                .map_err(|_| PacsError::Config("invalid persisted dicom_port".into()))?,
            ae_title: row.ae_title,
            ae_whitelist_enabled: row.ae_whitelist_enabled,
            accept_all_transfer_syntaxes: row.accept_all_transfer_syntaxes,
            accepted_transfer_syntaxes: serde_json::from_str(&row.accepted_transfer_syntaxes)
                .map_err(|error| {
                    PacsError::Config(format!("invalid accepted_transfer_syntaxes JSON: {error}"))
                })?,
            preferred_transfer_syntaxes: serde_json::from_str(&row.preferred_transfer_syntaxes)
                .map_err(|error| {
                    PacsError::Config(format!("invalid preferred_transfer_syntaxes JSON: {error}"))
                })?,
            storage_transfer_syntax: row.storage_transfer_syntax,
            max_associations: row
                .max_associations
                .try_into()
                .map_err(|_| PacsError::Config("invalid persisted max_associations".into()))?,
            dimse_timeout_secs: row
                .dimse_timeout_secs
                .try_into()
                .map_err(|_| PacsError::Config("invalid persisted dimse_timeout_secs".into()))?,
        })
    }
}

impl From<NodeRow> for DicomNode {
    fn from(row: NodeRow) -> Self {
        Self {
            ae_title: row.ae_title,
            host: row.host,
            port: row.port as u16,
            description: row.description,
            tls_enabled: row.tls_enabled,
        }
    }
}

impl TryFrom<UserRow> for User {
    type Error = PacsError;

    fn try_from(row: UserRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id.parse().map_err(|error| {
                PacsError::Config(format!("invalid persisted user id: {error}"))
            })?,
            username: row.username,
            display_name: row.display_name,
            email: row.email,
            password_hash: row.password_hash,
            role: row.role.parse().map_err(|_| {
                PacsError::Config(format!("invalid persisted user role: {}", row.role))
            })?,
            attributes: row.attributes.0,
            is_active: row.is_active,
            failed_login_attempts: row
                .failed_login_attempts
                .try_into()
                .map_err(|_| PacsError::Config("invalid persisted failed_login_attempts".into()))?,
            locked_until: row.locked_until,
            password_changed_at: row.password_changed_at,
            created_at: Some(row.created_at),
            updated_at: Some(row.updated_at),
        })
    }
}

impl TryFrom<PasswordPolicyRow> for PasswordPolicy {
    type Error = PacsError;

    fn try_from(row: PasswordPolicyRow) -> Result<Self, Self::Error> {
        Ok(Self {
            min_length: row
                .min_length
                .try_into()
                .map_err(|_| PacsError::Config("invalid persisted min_length".into()))?,
            require_uppercase: row.require_uppercase,
            require_digit: row.require_digit,
            require_special: row.require_special,
            max_failed_attempts: row
                .max_failed_attempts
                .try_into()
                .map_err(|_| PacsError::Config("invalid persisted max_failed_attempts".into()))?,
            lockout_duration_secs: row
                .lockout_duration_secs
                .try_into()
                .map_err(|_| PacsError::Config("invalid persisted lockout_duration_secs".into()))?,
            max_age_days: row
                .max_age_days
                .map(TryInto::try_into)
                .transpose()
                .map_err(|_| PacsError::Config("invalid persisted max_age_days".into()))?,
        })
    }
}

impl TryFrom<RefreshTokenRow> for RefreshToken {
    type Error = PacsError;

    fn try_from(row: RefreshTokenRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: RefreshTokenId(Uuid::parse_str(&row.id).map_err(|error| {
                PacsError::Config(format!("invalid refresh token id: {error}"))
            })?),
            user_id: row.user_id.parse().map_err(|error| {
                PacsError::Config(format!("invalid refresh token user id: {error}"))
            })?,
            token_hash: row.token_hash,
            expires_at: row.expires_at,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AuditLogRow {
    id: i64,
    occurred_at: DateTime<Utc>,
    user_id: Option<String>,
    action: String,
    resource: String,
    resource_uid: Option<String>,
    source_ip: Option<String>,
    status: String,
    details: Option<Json<serde_json::Value>>,
}

impl From<AuditLogRow> for AuditLogEntry {
    fn from(row: AuditLogRow) -> Self {
        Self {
            id: row.id,
            occurred_at: row.occurred_at,
            user_id: row.user_id,
            action: row.action,
            resource: row.resource,
            resource_uid: row.resource_uid,
            source_ip: row.source_ip,
            status: row.status,
            details: row
                .details
                .map(|details| details.0)
                .unwrap_or_else(|| serde_json::json!({})),
        }
    }
}

#[derive(sqlx::FromRow)]
struct CountRow {
    total: i64,
}

#[derive(sqlx::FromRow)]
struct StatsRow {
    num_studies: i64,
    num_series: i64,
    num_instances: i64,
    disk_usage_bytes: i64,
}

const STUDY_SELECT: &str = r#"
    SELECT studies.study_uid, patient_id, patient_name, study_date, study_time,
           accession_number, modalities, referring_physician, description,
           COALESCE((SELECT COUNT(*) FROM series WHERE series.study_uid = studies.study_uid), 0) AS num_series,
           COALESCE((SELECT COUNT(*) FROM instances WHERE instances.study_uid = studies.study_uid), 0) AS num_instances,
           metadata, created_at, updated_at
    FROM studies
"#;

const SERIES_SELECT: &str = r#"
    SELECT series.series_uid, study_uid, modality, series_number, description,
           body_part,
           COALESCE((SELECT COUNT(*) FROM instances WHERE instances.series_uid = series.series_uid), 0) AS num_instances,
           metadata, created_at
    FROM series
"#;

const INSTANCE_SELECT: &str = r#"
    SELECT instance_uid, series_uid, study_uid, sop_class_uid, instance_number,
           transfer_syntax, rows, columns, blob_key, metadata, created_at
    FROM instances
"#;

const AUDIT_SELECT: &str = r#"
    SELECT id, occurred_at, user_id, action, resource, resource_uid, source_ip, status, details
    FROM audit_log
"#;

const USER_SELECT: &str = r#"
    SELECT
        id,
        username,
        display_name,
        email,
        password_hash,
        role,
        attributes,
        is_active,
        failed_login_attempts,
        locked_until,
        password_changed_at,
        created_at,
        updated_at
    FROM users
"#;

const REFRESH_TOKEN_SELECT: &str = r#"
    SELECT id, user_id, token_hash, expires_at, created_at, revoked_at
    FROM refresh_tokens
"#;

fn map_db_err(error: sqlx::Error, resource: &'static str, uid: &str) -> PacsError {
    match error {
        sqlx::Error::RowNotFound => PacsError::NotFound {
            resource,
            uid: uid.to_string(),
        },
        other => PacsError::Store(Box::new(other)),
    }
}

fn map_store_err(error: sqlx::Error) -> PacsError {
    PacsError::Store(Box::new(error))
}

#[async_trait]
impl MetadataStore for SqliteMetadataStore {
    #[instrument(skip(self, study), fields(study_uid = %study.study_uid))]
    async fn store_study(&self, study: &Study) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO studies (
                study_uid, patient_id, patient_name, study_date, study_time,
                accession_number, modalities, referring_physician, description,
                num_series, num_instances, metadata, created_at, updated_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'),
                STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(study_uid) DO UPDATE SET
                patient_id = excluded.patient_id,
                patient_name = excluded.patient_name,
                study_date = excluded.study_date,
                study_time = excluded.study_time,
                accession_number = excluded.accession_number,
                modalities = excluded.modalities,
                referring_physician = excluded.referring_physician,
                description = excluded.description,
                metadata = excluded.metadata,
                updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            "#,
        )
        .bind(study.study_uid.as_ref())
        .bind(study.patient_id.as_deref())
        .bind(study.patient_name.as_deref())
        .bind(study.study_date)
        .bind(study.study_time.as_deref())
        .bind(study.accession_number.as_deref())
        .bind(Json(study.modalities.clone()))
        .bind(study.referring_physician.as_deref())
        .bind(study.description.as_deref())
        .bind(study.num_series)
        .bind(study.num_instances)
        .bind(Json(study.metadata.as_value().clone()))
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self, series), fields(series_uid = %series.series_uid))]
    async fn store_series(&self, series: &Series) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO series (
                series_uid, study_uid, modality, series_number, description,
                body_part, num_instances, metadata, created_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?, STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(series_uid) DO UPDATE SET
                study_uid = excluded.study_uid,
                modality = excluded.modality,
                series_number = excluded.series_number,
                description = excluded.description,
                body_part = excluded.body_part,
                metadata = excluded.metadata
            "#,
        )
        .bind(series.series_uid.as_ref())
        .bind(series.study_uid.as_ref())
        .bind(series.modality.as_deref())
        .bind(series.series_number)
        .bind(series.description.as_deref())
        .bind(series.body_part.as_deref())
        .bind(series.num_instances)
        .bind(Json(series.metadata.as_value().clone()))
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self, instance), fields(instance_uid = %instance.instance_uid))]
    async fn store_instance(&self, instance: &Instance) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO instances (
                instance_uid, series_uid, study_uid, sop_class_uid, instance_number,
                transfer_syntax, rows, columns, blob_key, metadata, created_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(instance_uid) DO UPDATE SET
                series_uid = excluded.series_uid,
                study_uid = excluded.study_uid,
                sop_class_uid = excluded.sop_class_uid,
                instance_number = excluded.instance_number,
                transfer_syntax = excluded.transfer_syntax,
                rows = excluded.rows,
                columns = excluded.columns,
                blob_key = excluded.blob_key,
                metadata = excluded.metadata
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
        .bind(Json(instance.metadata.as_value().clone()))
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self, query))]
    async fn query_studies(&self, query: &StudyQuery) -> PacsResult<Vec<Study>> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!("{STUDY_SELECT} WHERE 1=1"));

        if let Some(patient_id) = &query.patient_id {
            if patient_id.contains('*') || patient_id.contains('?') {
                qb.push(" AND patient_id LIKE ");
                qb.push_bind(patient_id.replace('*', "%").replace('?', "_"));
            } else {
                qb.push(" AND patient_id = ");
                qb.push_bind(patient_id.clone());
            }
        }

        if let Some(patient_name) = &query.patient_name {
            if query.fuzzy_matching || patient_name.contains('*') || patient_name.contains('?') {
                qb.push(" AND LOWER(patient_name) LIKE LOWER(");
                qb.push_bind(patient_name.replace('*', "%").replace('?', "_"));
                qb.push(")");
            } else {
                qb.push(" AND patient_name = ");
                qb.push_bind(patient_name.clone());
            }
        }

        if let Some(study_date_from) = query.study_date_from {
            qb.push(" AND study_date >= ");
            qb.push_bind(study_date_from);
        }

        if let Some(study_date_to) = query.study_date_to {
            qb.push(" AND study_date <= ");
            qb.push_bind(study_date_to);
        }

        if let Some(accession_number) = &query.accession_number {
            qb.push(" AND accession_number = ");
            qb.push_bind(accession_number.clone());
        }

        if let Some(study_uid) = &query.study_uid {
            qb.push(" AND study_uid = ");
            qb.push_bind(study_uid.as_ref().to_owned());
        }

        if let Some(modality) = &query.modality {
            qb.push(" AND EXISTS (SELECT 1 FROM json_each(modalities) WHERE value = ");
            qb.push_bind(modality.clone());
            qb.push(")");
        }

        qb.push(" ORDER BY created_at DESC LIMIT ");
        qb.push_bind(i64::from(query.limit.unwrap_or(100)));
        qb.push(" OFFSET ");
        qb.push_bind(i64::from(query.offset.unwrap_or(0)));

        qb.build_query_as::<StudyRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(map_store_err)
            .map(|rows| rows.into_iter().map(Study::from).collect())
    }

    #[instrument(skip(self, query), fields(study_uid = %query.study_uid))]
    async fn query_series(&self, query: &SeriesQuery) -> PacsResult<Vec<Series>> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!("{SERIES_SELECT} WHERE study_uid = "));
        qb.push_bind(query.study_uid.as_ref().to_owned());

        if let Some(series_uid) = &query.series_uid {
            qb.push(" AND series_uid = ");
            qb.push_bind(series_uid.as_ref().to_owned());
        }

        if let Some(modality) = &query.modality {
            qb.push(" AND modality = ");
            qb.push_bind(modality.clone());
        }

        if let Some(series_number) = query.series_number {
            qb.push(" AND series_number = ");
            qb.push_bind(series_number);
        }

        qb.push(" ORDER BY series_number IS NULL, series_number ASC LIMIT ");
        qb.push_bind(i64::from(query.limit.unwrap_or(100)));
        qb.push(" OFFSET ");
        qb.push_bind(i64::from(query.offset.unwrap_or(0)));

        qb.build_query_as::<SeriesRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(map_store_err)
            .map(|rows| rows.into_iter().map(Series::from).collect())
    }

    #[instrument(skip(self, query), fields(series_uid = %query.series_uid))]
    async fn query_instances(&self, query: &InstanceQuery) -> PacsResult<Vec<Instance>> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!("{INSTANCE_SELECT} WHERE series_uid = "));
        qb.push_bind(query.series_uid.as_ref().to_owned());

        if let Some(instance_uid) = &query.instance_uid {
            qb.push(" AND instance_uid = ");
            qb.push_bind(instance_uid.as_ref().to_owned());
        }

        if let Some(sop_class_uid) = &query.sop_class_uid {
            qb.push(" AND sop_class_uid = ");
            qb.push_bind(sop_class_uid.clone());
        }

        if let Some(instance_number) = query.instance_number {
            qb.push(" AND instance_number = ");
            qb.push_bind(instance_number);
        }

        qb.push(" ORDER BY instance_number IS NULL, instance_number ASC LIMIT ");
        qb.push_bind(i64::from(query.limit.unwrap_or(100)));
        qb.push(" OFFSET ");
        qb.push_bind(i64::from(query.offset.unwrap_or(0)));

        qb.build_query_as::<InstanceRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(map_store_err)
            .map(|rows| rows.into_iter().map(Instance::from).collect())
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study> {
        sqlx::query_as::<_, StudyRow>(&format!("{STUDY_SELECT} WHERE study_uid = ?"))
            .bind(uid.as_ref())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_db_err(error, "study", uid.as_ref()))
            .map(Study::from)
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series> {
        sqlx::query_as::<_, SeriesRow>(&format!("{SERIES_SELECT} WHERE series_uid = ?"))
            .bind(uid.as_ref())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_db_err(error, "series", uid.as_ref()))
            .map(Series::from)
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance> {
        sqlx::query_as::<_, InstanceRow>(&format!("{INSTANCE_SELECT} WHERE instance_uid = ?"))
            .bind(uid.as_ref())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_db_err(error, "instance", uid.as_ref()))
            .map(Instance::from)
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson> {
        sqlx::query_as::<_, MetadataRow>("SELECT metadata FROM instances WHERE instance_uid = ?")
            .bind(uid.as_ref())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_db_err(error, "instance", uid.as_ref()))
            .map(|row| DicomJson::from(row.metadata.0))
    }

    #[instrument(skip(self), fields(%uid))]
    async fn delete_study(&self, uid: &StudyUid) -> PacsResult<()> {
        let result = sqlx::query("DELETE FROM studies WHERE study_uid = ?")
            .bind(uid.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_store_err)?;

        if result.rows_affected() == 0 {
            return Err(PacsError::NotFound {
                resource: "study",
                uid: uid.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(%uid))]
    async fn delete_series(&self, uid: &SeriesUid) -> PacsResult<()> {
        let result = sqlx::query("DELETE FROM series WHERE series_uid = ?")
            .bind(uid.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_store_err)?;

        if result.rows_affected() == 0 {
            return Err(PacsError::NotFound {
                resource: "series",
                uid: uid.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self), fields(%uid))]
    async fn delete_instance(&self, uid: &SopInstanceUid) -> PacsResult<()> {
        let result = sqlx::query("DELETE FROM instances WHERE instance_uid = ?")
            .bind(uid.as_ref())
            .execute(&self.pool)
            .await
            .map_err(map_store_err)?;

        if result.rows_affected() == 0 {
            return Err(PacsError::NotFound {
                resource: "instance",
                uid: uid.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_statistics(&self) -> PacsResult<PacsStatistics> {
        let row = sqlx::query_as::<_, StatsRow>(
            r#"
            SELECT
                (SELECT COUNT(*) FROM studies) AS num_studies,
                (SELECT COUNT(*) FROM series) AS num_series,
                (SELECT COUNT(*) FROM instances) AS num_instances,
                COALESCE((SELECT SUM(LENGTH(metadata)) FROM instances), 0) AS disk_usage_bytes
            "#,
        )
        .fetch_one(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(PacsStatistics {
            num_studies: row.num_studies,
            num_series: row.num_series,
            num_instances: row.num_instances,
            disk_usage_bytes: row.disk_usage_bytes,
        })
    }

    #[instrument(skip(self, user), fields(user_id = %user.id, username = %user.username))]
    async fn store_user(&self, user: &User) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO users (
                id, username, display_name, email, password_hash, role,
                attributes, is_active, failed_login_attempts, locked_until,
                password_changed_at, created_at, updated_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'),
                STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(id) DO UPDATE SET
                username = excluded.username,
                display_name = excluded.display_name,
                email = excluded.email,
                password_hash = excluded.password_hash,
                role = excluded.role,
                attributes = excluded.attributes,
                is_active = excluded.is_active,
                failed_login_attempts = excluded.failed_login_attempts,
                locked_until = excluded.locked_until,
                password_changed_at = excluded.password_changed_at,
                updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            "#,
        )
        .bind(user.id.to_string())
        .bind(&user.username)
        .bind(user.display_name.as_deref())
        .bind(user.email.as_deref())
        .bind(&user.password_hash)
        .bind(user.role.as_str())
        .bind(Json(user.attributes.clone()))
        .bind(user.is_active)
        .bind(i64::from(user.failed_login_attempts))
        .bind(user.locked_until)
        .bind(user.password_changed_at)
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self), fields(user_id = %id))]
    async fn get_user(&self, id: &UserId) -> PacsResult<User> {
        sqlx::query_as::<_, UserRow>(&format!("{USER_SELECT} WHERE id = ?"))
            .bind(id.to_string())
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_db_err(error, "user", &id.to_string()))
            .and_then(User::try_from)
    }

    #[instrument(skip(self), fields(username = username))]
    async fn get_user_by_username(&self, username: &str) -> PacsResult<User> {
        sqlx::query_as::<_, UserRow>(&format!("{USER_SELECT} WHERE username = ?"))
            .bind(username)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_db_err(error, "user", username))
            .and_then(User::try_from)
    }

    #[instrument(skip(self, q))]
    async fn query_users(&self, q: &UserQuery) -> PacsResult<Vec<User>> {
        let mut qb = QueryBuilder::<Sqlite>::new(format!("{USER_SELECT} WHERE 1=1"));

        if let Some(search) = &q.search {
            let pattern = format!("%{search}%");
            qb.push(" AND (");
            qb.push("LOWER(username) LIKE LOWER(");
            qb.push_bind(pattern.clone());
            qb.push(") OR LOWER(COALESCE(display_name, '')) LIKE LOWER(");
            qb.push_bind(pattern.clone());
            qb.push(") OR LOWER(COALESCE(email, '')) LIKE LOWER(");
            qb.push_bind(pattern);
            qb.push("))");
        }

        if let Some(role) = q.role {
            qb.push(" AND role = ");
            qb.push_bind(role.as_str());
        }

        if let Some(is_active) = q.is_active {
            qb.push(" AND is_active = ");
            qb.push_bind(is_active);
        }

        qb.push(" ORDER BY username ASC LIMIT ");
        qb.push_bind(i64::from(q.limit.unwrap_or(100)));
        qb.push(" OFFSET ");
        qb.push_bind(i64::from(q.offset.unwrap_or(0)));

        qb.build_query_as::<UserRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(map_store_err)?
            .into_iter()
            .map(User::try_from)
            .collect()
    }

    #[instrument(skip(self), fields(user_id = %id))]
    async fn delete_user(&self, id: &UserId) -> PacsResult<()> {
        let result = sqlx::query("DELETE FROM users WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .map_err(map_store_err)?;

        if result.rows_affected() == 0 {
            return Err(PacsError::NotFound {
                resource: "user",
                uid: id.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self, token), fields(user_id = %token.user_id))]
    async fn store_refresh_token(&self, token: &RefreshToken) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO refresh_tokens (
                id, user_id, token_hash, expires_at, created_at, revoked_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?
            )
            ON CONFLICT(id) DO UPDATE SET
                user_id = excluded.user_id,
                token_hash = excluded.token_hash,
                expires_at = excluded.expires_at,
                revoked_at = excluded.revoked_at
            "#,
        )
        .bind(token.id.0.to_string())
        .bind(token.user_id.to_string())
        .bind(&token.token_hash)
        .bind(token.expires_at)
        .bind(token.created_at)
        .bind(token.revoked_at)
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self), fields(token_hash = token_hash))]
    async fn get_refresh_token(&self, token_hash: &str) -> PacsResult<RefreshToken> {
        sqlx::query_as::<_, RefreshTokenRow>(&format!(
            "{REFRESH_TOKEN_SELECT} WHERE token_hash = ?"
        ))
        .bind(token_hash)
        .fetch_one(&self.pool)
        .await
        .map_err(|error| map_db_err(error, "refresh_token", token_hash))
        .and_then(RefreshToken::try_from)
    }

    #[instrument(skip(self), fields(user_id = %user_id))]
    async fn revoke_refresh_tokens(&self, user_id: &UserId) -> PacsResult<()> {
        sqlx::query(
            r#"
            UPDATE refresh_tokens
            SET revoked_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            WHERE user_id = ?
              AND revoked_at IS NULL
            "#,
        )
        .bind(user_id.to_string())
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_password_policy(&self) -> PacsResult<PasswordPolicy> {
        let row = sqlx::query_as::<_, PasswordPolicyRow>(
            r#"
            SELECT
                min_length,
                require_uppercase,
                require_digit,
                require_special,
                max_failed_attempts,
                lockout_duration_secs,
                max_age_days
            FROM password_policy
            WHERE policy_key = ?
            "#,
        )
        .bind("default")
        .fetch_one(&self.pool)
        .await
        .map_err(map_store_err)?;

        PasswordPolicy::try_from(row)
    }

    #[instrument(skip(self, policy))]
    async fn upsert_password_policy(&self, policy: &PasswordPolicy) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO password_policy (
                policy_key,
                min_length,
                require_uppercase,
                require_digit,
                require_special,
                max_failed_attempts,
                lockout_duration_secs,
                max_age_days,
                created_at,
                updated_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?,
                STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'),
                STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(policy_key) DO UPDATE SET
                min_length = excluded.min_length,
                require_uppercase = excluded.require_uppercase,
                require_digit = excluded.require_digit,
                require_special = excluded.require_special,
                max_failed_attempts = excluded.max_failed_attempts,
                lockout_duration_secs = excluded.lockout_duration_secs,
                max_age_days = excluded.max_age_days,
                updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            "#,
        )
        .bind("default")
        .bind(i64::from(policy.min_length))
        .bind(policy.require_uppercase)
        .bind(policy.require_digit)
        .bind(policy.require_special)
        .bind(i64::from(policy.max_failed_attempts))
        .bind(i64::from(policy.lockout_duration_secs))
        .bind(policy.max_age_days.map(i64::from))
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self))]
    async fn list_nodes(&self) -> PacsResult<Vec<DicomNode>> {
        sqlx::query_as::<_, NodeRow>(
            "SELECT ae_title, host, port, description, tls_enabled FROM dicom_nodes ORDER BY ae_title",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(map_store_err)
        .map(|rows| rows.into_iter().map(DicomNode::from).collect())
    }

    #[instrument(skip(self, node), fields(ae_title = %node.ae_title))]
    async fn upsert_node(&self, node: &DicomNode) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO dicom_nodes (ae_title, host, port, description, tls_enabled, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'), STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT(ae_title) DO UPDATE SET
                host = excluded.host,
                port = excluded.port,
                description = excluded.description,
                tls_enabled = excluded.tls_enabled,
                updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            "#,
        )
        .bind(&node.ae_title)
        .bind(&node.host)
        .bind(i64::from(node.port))
        .bind(&node.description)
        .bind(node.tls_enabled)
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self), fields(ae_title = %ae_title))]
    async fn delete_node(&self, ae_title: &str) -> PacsResult<()> {
        let result = sqlx::query("DELETE FROM dicom_nodes WHERE ae_title = ?")
            .bind(ae_title)
            .execute(&self.pool)
            .await
            .map_err(map_store_err)?;

        if result.rows_affected() == 0 {
            return Err(PacsError::NotFound {
                resource: "node",
                uid: ae_title.to_string(),
            });
        }

        Ok(())
    }

    #[instrument(skip(self))]
    async fn get_server_settings(&self) -> PacsResult<Option<ServerSettings>> {
        let row = sqlx::query_as::<_, ServerSettingsRow>(
            r#"
            SELECT
                dicom_port,
                ae_title,
                ae_whitelist_enabled,
                accept_all_transfer_syntaxes,
                accepted_transfer_syntaxes,
                preferred_transfer_syntaxes,
                storage_transfer_syntax,
                max_associations,
                dimse_timeout_secs
            FROM server_settings
            WHERE settings_key = ?
            "#,
        )
        .bind("default")
        .fetch_optional(&self.pool)
        .await
        .map_err(map_store_err)?;

        row.map(ServerSettings::try_from).transpose()
    }

    #[instrument(skip(self, settings))]
    async fn upsert_server_settings(&self, settings: &ServerSettings) -> PacsResult<()> {
        let accepted_transfer_syntaxes =
            serde_json::to_string(&settings.accepted_transfer_syntaxes).map_err(|error| {
                PacsError::Config(format!(
                    "failed to serialize accepted transfer syntaxes: {error}"
                ))
            })?;
        let preferred_transfer_syntaxes =
            serde_json::to_string(&settings.preferred_transfer_syntaxes).map_err(|error| {
                PacsError::Config(format!(
                    "failed to serialize preferred transfer syntaxes: {error}"
                ))
            })?;

        sqlx::query(
            r#"
            INSERT INTO server_settings (
                settings_key,
                dicom_port,
                ae_title,
                ae_whitelist_enabled,
                accept_all_transfer_syntaxes,
                accepted_transfer_syntaxes,
                preferred_transfer_syntaxes,
                storage_transfer_syntax,
                max_associations,
                dimse_timeout_secs,
                created_at,
                updated_at
            ) VALUES (
                ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
                STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now'),
                STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            )
            ON CONFLICT(settings_key) DO UPDATE SET
                dicom_port = excluded.dicom_port,
                ae_title = excluded.ae_title,
                ae_whitelist_enabled = excluded.ae_whitelist_enabled,
                accept_all_transfer_syntaxes = excluded.accept_all_transfer_syntaxes,
                accepted_transfer_syntaxes = excluded.accepted_transfer_syntaxes,
                preferred_transfer_syntaxes = excluded.preferred_transfer_syntaxes,
                storage_transfer_syntax = excluded.storage_transfer_syntax,
                max_associations = excluded.max_associations,
                dimse_timeout_secs = excluded.dimse_timeout_secs,
                updated_at = STRFTIME('%Y-%m-%dT%H:%M:%fZ', 'now')
            "#,
        )
        .bind("default")
        .bind(i64::from(settings.dicom_port))
        .bind(&settings.ae_title)
        .bind(settings.ae_whitelist_enabled)
        .bind(settings.accept_all_transfer_syntaxes)
        .bind(accepted_transfer_syntaxes)
        .bind(preferred_transfer_syntaxes)
        .bind(&settings.storage_transfer_syntax)
        .bind(settings.max_associations as i64)
        .bind(settings.dimse_timeout_secs as i64)
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }

    #[instrument(skip(self, query))]
    async fn search_audit_logs(&self, query: &AuditLogQuery) -> PacsResult<AuditLogPage> {
        let limit = query.limit.unwrap_or(100);
        let offset = query.offset.unwrap_or(0);

        let mut count_qb = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) AS total FROM audit_log");
        push_audit_filters(&mut count_qb, query);
        let total = count_qb
            .build_query_as::<CountRow>()
            .fetch_one(&self.pool)
            .await
            .map_err(map_store_err)?
            .total;

        let mut qb = QueryBuilder::<Sqlite>::new(AUDIT_SELECT);
        push_audit_filters(&mut qb, query);
        qb.push(" ORDER BY occurred_at DESC, id DESC LIMIT ");
        qb.push_bind(i64::from(limit));
        qb.push(" OFFSET ");
        qb.push_bind(i64::from(offset));

        let entries = qb
            .build_query_as::<AuditLogRow>()
            .fetch_all(&self.pool)
            .await
            .map_err(map_store_err)?
            .into_iter()
            .map(AuditLogEntry::from)
            .collect();

        Ok(AuditLogPage {
            entries,
            total,
            limit,
            offset,
        })
    }

    #[instrument(skip(self), fields(audit_log_id = id))]
    async fn get_audit_log(&self, id: i64) -> PacsResult<AuditLogEntry> {
        sqlx::query_as::<_, AuditLogRow>(&format!("{AUDIT_SELECT} WHERE id = ?"))
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(|error| map_db_err(error, "audit_log", &id.to_string()))
            .map(AuditLogEntry::from)
    }

    #[instrument(skip(self, entry), fields(action = %entry.action, resource = %entry.resource))]
    async fn store_audit_log(&self, entry: &NewAuditLogEntry) -> PacsResult<()> {
        sqlx::query(
            r#"
            INSERT INTO audit_log (
                user_id, action, resource, resource_uid, source_ip, status, details
            ) VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(entry.user_id.as_deref())
        .bind(&entry.action)
        .bind(&entry.resource)
        .bind(entry.resource_uid.as_deref())
        .bind(entry.source_ip.as_deref())
        .bind(&entry.status)
        .bind(Json(entry.details.clone()))
        .execute(&self.pool)
        .await
        .map_err(map_store_err)?;

        Ok(())
    }
}

fn push_audit_filters(qb: &mut QueryBuilder<'_, Sqlite>, query: &AuditLogQuery) {
    qb.push(" WHERE 1=1");

    if let Some(user_id) = &query.user_id {
        qb.push(" AND user_id = ");
        qb.push_bind(user_id.clone());
    }

    if let Some(action) = &query.action {
        qb.push(" AND LOWER(action) = LOWER(");
        qb.push_bind(action.clone());
        qb.push(")");
    }

    if let Some(resource) = &query.resource {
        qb.push(" AND LOWER(resource) = LOWER(");
        qb.push_bind(resource.clone());
        qb.push(")");
    }

    if let Some(resource_uid) = &query.resource_uid {
        qb.push(" AND resource_uid = ");
        qb.push_bind(resource_uid.clone());
    }

    if let Some(source_ip) = &query.source_ip {
        qb.push(" AND source_ip = ");
        qb.push_bind(source_ip.clone());
    }

    if let Some(status) = &query.status {
        qb.push(" AND LOWER(status) = LOWER(");
        qb.push_bind(status.clone());
        qb.push(")");
    }

    if let Some(occurred_from) = query.occurred_from {
        qb.push(" AND occurred_at >= ");
        qb.push_bind(occurred_from);
    }

    if let Some(occurred_to) = query.occurred_to {
        qb.push(" AND occurred_at <= ");
        qb.push_bind(occurred_to);
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use chrono::{NaiveDate, TimeZone, Utc};
    use pacs_core::{blob_key_for, UserRole};
    use serde_json::json;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;

    async fn test_store() -> (TempDir, SqliteMetadataStore) {
        let tempdir = TempDir::new().expect("tempdir");
        let db_path = tempdir.path().join("pacsnode.db");
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
            .expect("sqlite connect options")
            .create_if_missing(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .expect("sqlite pool");
        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .expect("migrations");
        (tempdir, SqliteMetadataStore::new(pool))
    }

    fn sample_study() -> Study {
        Study {
            study_uid: StudyUid::from("1.2.3"),
            patient_id: Some("PID001".into()),
            patient_name: Some("Doe^Jane".into()),
            study_date: Some(NaiveDate::from_ymd_opt(2024, 6, 15).expect("date")),
            study_time: Some("120000".into()),
            accession_number: Some("ACC001".into()),
            modalities: vec!["CT".into()],
            referring_physician: Some("Dr. Smith".into()),
            description: Some("Chest CT".into()),
            num_series: 0,
            num_instances: 0,
            metadata: DicomJson::from(json!({"00080020": {"vr": "DA", "Value": ["20240615"]}})),
            created_at: None,
            updated_at: None,
        }
    }

    fn sample_series(study_uid: &StudyUid) -> Series {
        Series {
            series_uid: SeriesUid::from("1.2.3.1"),
            study_uid: study_uid.clone(),
            modality: Some("CT".into()),
            series_number: Some(1),
            description: Some("Axial".into()),
            body_part: Some("CHEST".into()),
            num_instances: 0,
            metadata: DicomJson::from(json!({"00080060": {"vr": "CS", "Value": ["CT"]}})),
            created_at: None,
        }
    }

    fn sample_instance(study_uid: &StudyUid, series_uid: &SeriesUid) -> Instance {
        let instance_uid = SopInstanceUid::from("1.2.3.1.1");
        Instance {
            blob_key: blob_key_for(study_uid, series_uid, &instance_uid),
            instance_uid,
            series_uid: series_uid.clone(),
            study_uid: study_uid.clone(),
            sop_class_uid: Some("1.2.840.10008.5.1.4.1.1.2".into()),
            instance_number: Some(1),
            transfer_syntax: Some("1.2.840.10008.1.2.1".into()),
            rows: Some(512),
            columns: Some(512),
            metadata: DicomJson::from(json!({"00080018": {"vr": "UI", "Value": ["1.2.3.1.1"]}})),
            created_at: None,
        }
    }

    fn sample_user() -> User {
        User {
            id: UserId::from(Uuid::from_u128(1)),
            username: "admin".into(),
            display_name: Some("Admin User".into()),
            email: Some("admin@example.test".into()),
            password_hash: "argon2-hash".into(),
            role: UserRole::Admin,
            attributes: json!({"department": "radiology", "modality_access": ["CT", "MR"]}),
            is_active: true,
            failed_login_attempts: 0,
            locked_until: None,
            password_changed_at: Some(Utc.with_ymd_and_hms(2026, 3, 18, 12, 0, 0).unwrap()),
            created_at: None,
            updated_at: None,
        }
    }

    fn sample_refresh_token(user_id: UserId) -> RefreshToken {
        RefreshToken {
            id: RefreshTokenId(Uuid::from_u128(2)),
            user_id,
            token_hash: "hashed-refresh-token".into(),
            expires_at: Utc.with_ymd_and_hms(2026, 3, 25, 12, 0, 0).unwrap(),
            created_at: Utc.with_ymd_and_hms(2026, 3, 18, 12, 0, 0).unwrap(),
            revoked_at: None,
        }
    }

    #[tokio::test]
    async fn round_trips_hierarchy_queries_and_counts() {
        let (_tempdir, store) = test_store().await;
        let study = sample_study();
        let series = sample_series(&study.study_uid);
        let instance = sample_instance(&study.study_uid, &series.series_uid);

        store.store_study(&study).await.expect("store study");
        store.store_series(&series).await.expect("store series");
        store
            .store_instance(&instance)
            .await
            .expect("store instance");

        let fetched_study = store.get_study(&study.study_uid).await.expect("get study");
        assert_eq!(fetched_study.patient_id.as_deref(), Some("PID001"));
        assert_eq!(fetched_study.num_series, 1);
        assert_eq!(fetched_study.num_instances, 1);

        let study_results = store
            .query_studies(&StudyQuery {
                patient_id: Some("PID001".into()),
                patient_name: Some("doe*".into()),
                modality: Some("CT".into()),
                limit: Some(10),
                offset: Some(0),
                include_fields: Vec::new(),
                fuzzy_matching: true,
                ..StudyQuery::default()
            })
            .await
            .expect("query studies");
        assert_eq!(study_results.len(), 1);

        let series_results = store
            .query_series(&SeriesQuery {
                study_uid: study.study_uid.clone(),
                series_uid: Some(series.series_uid.clone()),
                modality: Some("CT".into()),
                series_number: Some(1),
                limit: Some(10),
                offset: Some(0),
            })
            .await
            .expect("query series");
        assert_eq!(series_results.len(), 1);

        let instance_results = store
            .query_instances(&InstanceQuery {
                series_uid: series.series_uid.clone(),
                instance_uid: Some(instance.instance_uid.clone()),
                sop_class_uid: instance.sop_class_uid.clone(),
                instance_number: Some(1),
                limit: Some(10),
                offset: Some(0),
            })
            .await
            .expect("query instances");
        assert_eq!(instance_results.len(), 1);

        let metadata = store
            .get_instance_metadata(&instance.instance_uid)
            .await
            .expect("metadata");
        assert_eq!(metadata.as_value()["00080018"]["Value"][0], "1.2.3.1.1");

        let stats = store.get_statistics().await.expect("stats");
        assert_eq!(stats.num_studies, 1);
        assert_eq!(stats.num_series, 1);
        assert_eq!(stats.num_instances, 1);
        assert!(stats.disk_usage_bytes > 0);

        store
            .delete_series(&series.series_uid)
            .await
            .expect("delete series");
        let updated_study = store
            .get_study(&study.study_uid)
            .await
            .expect("study after delete");
        assert_eq!(updated_study.num_series, 0);
        assert_eq!(updated_study.num_instances, 0);
        assert!(matches!(
            store.get_instance(&instance.instance_uid).await,
            Err(PacsError::NotFound {
                resource: "instance",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn preserves_trigger_managed_counts_across_study_and_series_updates() {
        let (_tempdir, store) = test_store().await;
        let mut study = sample_study();
        let mut series = sample_series(&study.study_uid);
        let instance = sample_instance(&study.study_uid, &series.series_uid);

        store.store_study(&study).await.expect("store study");
        store.store_series(&series).await.expect("store series");
        store
            .store_instance(&instance)
            .await
            .expect("store instance");

        study.description = Some("Updated study".into());
        series.description = Some("Updated series".into());
        store.store_study(&study).await.expect("update study");
        store.store_series(&series).await.expect("update series");

        let study_counts: (i32, i32) =
            sqlx::query_as("SELECT num_series, num_instances FROM studies WHERE study_uid = ?")
                .bind(study.study_uid.as_ref())
                .fetch_one(store.pool())
                .await
                .expect("fetch study counts");
        assert_eq!(study_counts, (1, 1));

        let series_count: (i32,) =
            sqlx::query_as("SELECT num_instances FROM series WHERE series_uid = ?")
                .bind(series.series_uid.as_ref())
                .fetch_one(store.pool())
                .await
                .expect("fetch series count");
        assert_eq!(series_count.0, 1);
    }

    #[tokio::test]
    async fn round_trips_nodes() {
        let (_tempdir, store) = test_store().await;
        let node = DicomNode {
            ae_title: "REMOTE".into(),
            host: "pacs.example.test".into(),
            port: 4242,
            description: Some("Remote PACS".into()),
            tls_enabled: true,
        };

        store.upsert_node(&node).await.expect("upsert node");
        let listed = store.list_nodes().await.expect("list nodes");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].ae_title, "REMOTE");

        store.delete_node("REMOTE").await.expect("delete node");
        assert!(matches!(
            store.delete_node("REMOTE").await,
            Err(PacsError::NotFound {
                resource: "node",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn round_trips_server_settings() {
        let (_tempdir, store) = test_store().await;
        let settings = ServerSettings {
            dicom_port: 4242,
            ae_title: "PACSNODE_UI".into(),
            ae_whitelist_enabled: true,
            accept_all_transfer_syntaxes: false,
            accepted_transfer_syntaxes: vec![
                "1.2.840.10008.1.2.1".into(),
                "1.2.840.10008.1.2.4.50".into(),
            ],
            preferred_transfer_syntaxes: vec!["1.2.840.10008.1.2.4.50".into()],
            storage_transfer_syntax: Some("1.2.840.10008.1.2.4.90".into()),
            max_associations: 32,
            dimse_timeout_secs: 45,
        };

        assert_eq!(
            store.get_server_settings().await.expect("get settings"),
            None
        );

        store
            .upsert_server_settings(&settings)
            .await
            .expect("upsert settings");

        assert_eq!(
            store
                .get_server_settings()
                .await
                .expect("reloaded settings"),
            Some(settings)
        );
    }

    #[tokio::test]
    async fn round_trips_users_password_policy_and_refresh_tokens() {
        let (_tempdir, store) = test_store().await;
        let user = sample_user();
        let mut policy = store.get_password_policy().await.expect("default policy");
        let refresh_token = sample_refresh_token(user.id);

        store.store_user(&user).await.expect("store user");

        let fetched_user = store.get_user(&user.id).await.expect("get user");
        assert_eq!(fetched_user.username, user.username);
        assert_eq!(fetched_user.role, UserRole::Admin);

        let fetched_by_username = store
            .get_user_by_username(&user.username)
            .await
            .expect("get user by username");
        assert_eq!(fetched_by_username.id, user.id);

        let users = store
            .query_users(&UserQuery {
                search: Some("admin".into()),
                role: Some(UserRole::Admin),
                is_active: Some(true),
                limit: Some(10),
                offset: Some(0),
            })
            .await
            .expect("query users");
        assert_eq!(users.len(), 1);

        policy.min_length = 16;
        policy.require_special = true;
        store
            .upsert_password_policy(&policy)
            .await
            .expect("update policy");

        let fetched_policy = store.get_password_policy().await.expect("reloaded policy");
        assert_eq!(fetched_policy.min_length, 16);
        assert!(fetched_policy.require_special);

        store
            .store_refresh_token(&refresh_token)
            .await
            .expect("store refresh token");

        let fetched_token = store
            .get_refresh_token(&refresh_token.token_hash)
            .await
            .expect("get refresh token");
        assert!(fetched_token.revoked_at.is_none());

        store
            .revoke_refresh_tokens(&user.id)
            .await
            .expect("revoke refresh tokens");

        let revoked_token = store
            .get_refresh_token(&refresh_token.token_hash)
            .await
            .expect("get revoked token");
        assert!(revoked_token.revoked_at.is_some());
    }

    #[tokio::test]
    async fn round_trips_audit_logs() {
        let (_tempdir, store) = test_store().await;
        let entry = NewAuditLogEntry {
            user_id: Some("admin".into()),
            action: "QUERY".into(),
            resource: "query".into(),
            resource_uid: None,
            source_ip: Some("127.0.0.1".into()),
            status: "ok".into(),
            details: json!({
                "level": "STUDY",
                "num_results": 3,
            }),
        };

        store
            .store_audit_log(&entry)
            .await
            .expect("store audit entry");

        let page = store
            .search_audit_logs(&AuditLogQuery {
                action: Some("query".into()),
                status: Some("OK".into()),
                limit: Some(10),
                offset: Some(0),
                ..AuditLogQuery::default()
            })
            .await
            .expect("search audit logs");
        assert_eq!(page.total, 1);
        assert_eq!(page.entries.len(), 1);
        assert_eq!(page.entries[0].details["num_results"], 3);

        let fetched = store
            .get_audit_log(page.entries[0].id)
            .await
            .expect("get audit log");
        assert_eq!(fetched.action, "QUERY");
        assert_eq!(fetched.source_ip.as_deref(), Some("127.0.0.1"));
    }
}
