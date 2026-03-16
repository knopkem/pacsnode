//! [`PgMetadataStore`] — PostgreSQL implementation of [`MetadataStore`].
//!
//! All public methods delegate to the internal SQL helper modules under
//! `crate::queries`.
//!
//! # Integration Tests
//!
//! ```bash
//! cargo test -p pacs-store
//! ```

use async_trait::async_trait;
use pacs_core::{
    DicomJson, DicomNode, Instance, InstanceQuery, MetadataStore, PacsError, PacsResult,
    PacsStatistics, Series, SeriesQuery, SeriesUid, SopInstanceUid, Study, StudyQuery, StudyUid,
};
use sqlx::PgPool;
use tracing::instrument;

use crate::queries::{instance, node, series, study};

/// PostgreSQL-backed [`MetadataStore`] for pacsnode.
///
/// Wraps a `sqlx` [`PgPool`] and is cheaply cloneable. All trait methods are
/// fully `async` and safe to call from any tokio task.
pub struct PgMetadataStore {
    pool: PgPool,
}

impl PgMetadataStore {
    /// Creates a [`PgMetadataStore`] from an existing [`PgPool`].
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns a reference to the underlying connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl MetadataStore for PgMetadataStore {
    #[instrument(skip(self, s), fields(study_uid = %s.study_uid))]
    async fn store_study(&self, s: &Study) -> PacsResult<()> {
        study::upsert(&self.pool, s).await
    }

    #[instrument(skip(self, s), fields(series_uid = %s.series_uid))]
    async fn store_series(&self, s: &Series) -> PacsResult<()> {
        series::upsert(&self.pool, s).await
    }

    #[instrument(skip(self, i), fields(instance_uid = %i.instance_uid))]
    async fn store_instance(&self, i: &Instance) -> PacsResult<()> {
        instance::upsert(&self.pool, i).await
    }

    #[instrument(skip(self, q))]
    async fn query_studies(&self, q: &StudyQuery) -> PacsResult<Vec<Study>> {
        study::query(&self.pool, q).await
    }

    #[instrument(skip(self, q), fields(study_uid = %q.study_uid))]
    async fn query_series(&self, q: &SeriesQuery) -> PacsResult<Vec<Series>> {
        series::query(&self.pool, q).await
    }

    #[instrument(skip(self, q), fields(series_uid = %q.series_uid))]
    async fn query_instances(&self, q: &InstanceQuery) -> PacsResult<Vec<Instance>> {
        instance::query(&self.pool, q).await
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_study(&self, uid: &StudyUid) -> PacsResult<Study> {
        study::get(&self.pool, uid).await
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_series(&self, uid: &SeriesUid) -> PacsResult<Series> {
        series::get(&self.pool, uid).await
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_instance(&self, uid: &SopInstanceUid) -> PacsResult<Instance> {
        instance::get(&self.pool, uid).await
    }

    #[instrument(skip(self), fields(%uid))]
    async fn get_instance_metadata(&self, uid: &SopInstanceUid) -> PacsResult<DicomJson> {
        instance::get_metadata(&self.pool, uid).await
    }

    #[instrument(skip(self), fields(%uid))]
    async fn delete_study(&self, uid: &StudyUid) -> PacsResult<()> {
        study::delete(&self.pool, uid).await
    }

    #[instrument(skip(self), fields(%uid))]
    async fn delete_series(&self, uid: &SeriesUid) -> PacsResult<()> {
        series::delete(&self.pool, uid).await
    }

    #[instrument(skip(self), fields(%uid))]
    async fn delete_instance(&self, uid: &SopInstanceUid) -> PacsResult<()> {
        instance::delete(&self.pool, uid).await
    }

    #[instrument(skip(self))]
    async fn get_statistics(&self) -> PacsResult<PacsStatistics> {
        get_stats(&self.pool).await
    }

    #[instrument(skip(self))]
    async fn list_nodes(&self) -> PacsResult<Vec<DicomNode>> {
        node::list(&self.pool).await
    }

    #[instrument(skip(self, n), fields(ae_title = %n.ae_title))]
    async fn upsert_node(&self, n: &DicomNode) -> PacsResult<()> {
        node::upsert(&self.pool, n).await
    }

    #[instrument(skip(self), fields(ae_title = %ae_title))]
    async fn delete_node(&self, ae_title: &str) -> PacsResult<()> {
        node::delete(&self.pool, ae_title).await
    }
}

async fn get_stats(pool: &PgPool) -> PacsResult<PacsStatistics> {
    #[derive(sqlx::FromRow)]
    struct StatsRow {
        num_studies: i64,
        num_series: i64,
        num_instances: i64,
        disk_usage_bytes: i64,
    }

    let row = sqlx::query_as::<_, StatsRow>(
        r#"
        SELECT
            (SELECT COUNT(*)::BIGINT FROM studies)   AS num_studies,
            (SELECT COUNT(*)::BIGINT FROM series)    AS num_series,
            (SELECT COUNT(*)::BIGINT FROM instances) AS num_instances,
            COALESCE(
                (SELECT SUM(length(metadata::text))::BIGINT FROM instances),
                0
            )                                        AS disk_usage_bytes
        "#,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| PacsError::Store(Box::new(e)))?;

    Ok(PacsStatistics {
        num_studies: row.num_studies,
        num_series: row.num_series,
        num_instances: row.num_instances,
        disk_usage_bytes: row.disk_usage_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify `PgMetadataStore` satisfies `Send + Sync` at compile time.
    #[test]
    fn pg_metadata_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PgMetadataStore>();
    }
}
