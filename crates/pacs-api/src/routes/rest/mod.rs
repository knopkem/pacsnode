//! REST management API sub-modules.

use std::collections::BTreeSet;

use pacs_core::{InstanceQuery, PacsError, SeriesQuery, SeriesUid, SopInstanceUid, StudyUid};
use tracing::warn;

use crate::{error::ApiError, state::AppState};

pub mod instances;
pub mod nodes;
pub mod series;
pub mod studies;

pub(super) async fn collect_instance_blob_keys(
    state: &AppState,
    instance_uid: &SopInstanceUid,
) -> Result<BTreeSet<String>, ApiError> {
    let instance = state.store.get_instance(instance_uid).await?;
    Ok(BTreeSet::from([instance.blob_key]))
}

pub(super) async fn collect_series_blob_keys(
    state: &AppState,
    series_uid: &SeriesUid,
) -> Result<BTreeSet<String>, ApiError> {
    let instances = state
        .store
        .query_instances(&InstanceQuery {
            series_uid: series_uid.clone(),
            instance_uid: None,
            sop_class_uid: None,
            instance_number: None,
            limit: None,
            offset: None,
        })
        .await?;
    Ok(instances
        .into_iter()
        .map(|instance| instance.blob_key)
        .collect())
}

pub(super) async fn collect_study_blob_keys(
    state: &AppState,
    study_uid: &StudyUid,
) -> Result<BTreeSet<String>, ApiError> {
    let series = state
        .store
        .query_series(&SeriesQuery {
            study_uid: study_uid.clone(),
            series_uid: None,
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        })
        .await?;

    let mut blob_keys = BTreeSet::new();
    for series in series {
        blob_keys.extend(collect_series_blob_keys(state, &series.series_uid).await?);
    }
    Ok(blob_keys)
}

pub(super) async fn cleanup_blob_keys(state: &AppState, blob_keys: BTreeSet<String>) {
    for blob_key in blob_keys {
        match state.blobs.delete(&blob_key).await {
            Ok(()) | Err(PacsError::NotFound { .. }) => {}
            Err(error) => {
                warn!(blob_key = %blob_key, error = %error, "failed to delete blob after metadata deletion");
            }
        }
    }
}
