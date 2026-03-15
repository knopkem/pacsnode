//! Node-level SQL helpers: list, upsert, delete.

use pacs_core::{DicomNode, PacsError, PacsResult};
use sqlx::PgPool;

// ---------------------------------------------------------------------------
// Row type
// ---------------------------------------------------------------------------

/// Raw database row returned by `dicom_nodes` SELECT queries.
#[derive(sqlx::FromRow)]
struct NodeRow {
    ae_title: String,
    host: String,
    /// `INTEGER` in PostgreSQL; cast to `u16` on conversion.
    port: i32,
    description: Option<String>,
    tls_enabled: bool,
}

impl From<NodeRow> for DicomNode {
    fn from(r: NodeRow) -> Self {
        DicomNode {
            ae_title: r.ae_title,
            host: r.host,
            // PostgreSQL INTEGER is i32; DICOM port is u16. The CHECK constraint
            // (1–65535) in the migration guarantees the cast is lossless.
            port: r.port as u16,
            description: r.description,
            tls_enabled: r.tls_enabled,
        }
    }
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Returns all rows from `dicom_nodes` ordered by `ae_title`.
pub(crate) async fn list(pool: &PgPool) -> PacsResult<Vec<DicomNode>> {
    sqlx::query_as::<_, NodeRow>(
        "SELECT ae_title, host, port, description, tls_enabled \
         FROM dicom_nodes ORDER BY ae_title",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| PacsError::Store(Box::new(e)))
    .map(|rows| rows.into_iter().map(DicomNode::from).collect())
}

/// Inserts a new node or updates an existing one (keyed on `ae_title`).
pub(crate) async fn upsert(pool: &PgPool, node: &DicomNode) -> PacsResult<()> {
    sqlx::query(
        r#"
        INSERT INTO dicom_nodes (ae_title, host, port, description, tls_enabled)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (ae_title) DO UPDATE SET
            host        = EXCLUDED.host,
            port        = EXCLUDED.port,
            description = EXCLUDED.description,
            tls_enabled = EXCLUDED.tls_enabled,
            updated_at  = NOW()
        "#,
    )
    .bind(&node.ae_title)
    .bind(&node.host)
    .bind(node.port as i32)
    .bind(&node.description)
    .bind(node.tls_enabled)
    .execute(pool)
    .await
    .map_err(|e| PacsError::Store(Box::new(e)))?;

    Ok(())
}

/// Deletes a node by AE title.
///
/// # Errors
///
/// Returns [`PacsError::NotFound`] when no node with the given AE title exists.
pub(crate) async fn delete(pool: &PgPool, ae_title: &str) -> PacsResult<()> {
    let result = sqlx::query("DELETE FROM dicom_nodes WHERE ae_title = $1")
        .bind(ae_title)
        .execute(pool)
        .await
        .map_err(|e| PacsError::Store(Box::new(e)))?;

    if result.rows_affected() == 0 {
        return Err(PacsError::NotFound {
            resource: "node",
            uid: ae_title.to_string(),
        });
    }
    Ok(())
}
