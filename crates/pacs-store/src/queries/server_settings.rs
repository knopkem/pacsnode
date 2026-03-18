//! Server-settings SQL helpers.

use pacs_core::{PacsError, PacsResult, ServerSettings};
use sqlx::PgPool;

const SETTINGS_KEY: &str = "default";

#[derive(sqlx::FromRow)]
struct ServerSettingsRow {
    dicom_port: i32,
    ae_title: String,
    ae_whitelist_enabled: bool,
    accept_all_transfer_syntaxes: bool,
    accepted_transfer_syntaxes: Vec<String>,
    preferred_transfer_syntaxes: Vec<String>,
    max_associations: i64,
    dimse_timeout_secs: i64,
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
            accepted_transfer_syntaxes: row.accepted_transfer_syntaxes,
            preferred_transfer_syntaxes: row.preferred_transfer_syntaxes,
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

pub(crate) async fn get(pool: &PgPool) -> PacsResult<Option<ServerSettings>> {
    let row = sqlx::query_as::<_, ServerSettingsRow>(
        r#"
        SELECT
            dicom_port,
            ae_title,
            ae_whitelist_enabled,
            accept_all_transfer_syntaxes,
            accepted_transfer_syntaxes,
            preferred_transfer_syntaxes,
            max_associations,
            dimse_timeout_secs
        FROM server_settings
        WHERE settings_key = $1
        "#,
    )
    .bind(SETTINGS_KEY)
    .fetch_optional(pool)
    .await
    .map_err(|error| PacsError::Store(Box::new(error)))?;

    row.map(ServerSettings::try_from).transpose()
}

pub(crate) async fn upsert(pool: &PgPool, settings: &ServerSettings) -> PacsResult<()> {
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
            max_associations,
            dimse_timeout_secs
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9
        )
        ON CONFLICT (settings_key) DO UPDATE SET
            dicom_port = EXCLUDED.dicom_port,
            ae_title = EXCLUDED.ae_title,
            ae_whitelist_enabled = EXCLUDED.ae_whitelist_enabled,
            accept_all_transfer_syntaxes = EXCLUDED.accept_all_transfer_syntaxes,
            accepted_transfer_syntaxes = EXCLUDED.accepted_transfer_syntaxes,
            preferred_transfer_syntaxes = EXCLUDED.preferred_transfer_syntaxes,
            max_associations = EXCLUDED.max_associations,
            dimse_timeout_secs = EXCLUDED.dimse_timeout_secs,
            updated_at = NOW()
        "#,
    )
    .bind(SETTINGS_KEY)
    .bind(i32::from(settings.dicom_port))
    .bind(&settings.ae_title)
    .bind(settings.ae_whitelist_enabled)
    .bind(settings.accept_all_transfer_syntaxes)
    .bind(&settings.accepted_transfer_syntaxes)
    .bind(&settings.preferred_transfer_syntaxes)
    .bind(settings.max_associations as i64)
    .bind(settings.dimse_timeout_secs as i64)
    .execute(pool)
    .await
    .map_err(|error| PacsError::Store(Box::new(error)))?;

    Ok(())
}
