//! Error-mapping utilities converting [`sqlx::Error`] into [`pacs_core::PacsError`].

use pacs_core::PacsError;

/// Maps a [`sqlx::Error`] to a [`PacsError`].
///
/// [`sqlx::Error::RowNotFound`] becomes [`PacsError::NotFound`] with the supplied
/// `resource` label and `uid` string; every other variant becomes
/// [`PacsError::Store`].
pub(crate) fn map_db_err(e: sqlx::Error, resource: &'static str, uid: &str) -> PacsError {
    match e {
        sqlx::Error::RowNotFound => PacsError::NotFound {
            resource,
            uid: uid.to_string(),
        },
        other => PacsError::Store(Box::new(other)),
    }
}

/// Maps any [`sqlx::Error`] to [`PacsError::Store`].
pub(crate) fn map_store_err(e: sqlx::Error) -> PacsError {
    PacsError::Store(Box::new(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_not_found_becomes_not_found() {
        let err = map_db_err(sqlx::Error::RowNotFound, "study", "1.2.3.4");
        assert!(matches!(
            err,
            PacsError::NotFound { resource: "study", uid } if uid == "1.2.3.4"
        ));
    }

    #[test]
    fn pool_timed_out_becomes_store_err_via_map_db_err() {
        let err = map_db_err(sqlx::Error::PoolTimedOut, "series", "x.y.z");
        assert!(matches!(err, PacsError::Store(_)));
    }

    #[test]
    fn pool_timed_out_becomes_store_err_via_map_store_err() {
        let err = map_store_err(sqlx::Error::PoolTimedOut);
        assert!(matches!(err, PacsError::Store(_)));
    }
}
