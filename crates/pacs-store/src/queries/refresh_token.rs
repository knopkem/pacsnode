//! Refresh-token SQL helpers.

use chrono::{DateTime, Utc};
use pacs_core::{PacsError, PacsResult, RefreshToken, RefreshTokenId, UserId};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct RefreshTokenRow {
    id: Uuid,
    user_id: Uuid,
    token_hash: String,
    expires_at: DateTime<Utc>,
    created_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

impl From<RefreshTokenRow> for RefreshToken {
    fn from(row: RefreshTokenRow) -> Self {
        Self {
            id: RefreshTokenId(row.id),
            user_id: UserId::from(row.user_id),
            token_hash: row.token_hash,
            expires_at: row.expires_at,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        }
    }
}

pub(crate) async fn upsert(pool: &PgPool, token: &RefreshToken) -> PacsResult<()> {
    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (
            id,
            user_id,
            token_hash,
            expires_at,
            created_at,
            revoked_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6
        )
        ON CONFLICT (id) DO UPDATE SET
            user_id = EXCLUDED.user_id,
            token_hash = EXCLUDED.token_hash,
            expires_at = EXCLUDED.expires_at,
            revoked_at = EXCLUDED.revoked_at
        "#,
    )
    .bind(token.id.0)
    .bind(*token.user_id.as_uuid())
    .bind(&token.token_hash)
    .bind(token.expires_at)
    .bind(token.created_at)
    .bind(token.revoked_at)
    .execute(pool)
    .await
    .map_err(|error| PacsError::Store(Box::new(error)))?;

    Ok(())
}

pub(crate) async fn get(pool: &PgPool, token_hash: &str) -> PacsResult<RefreshToken> {
    sqlx::query_as::<_, RefreshTokenRow>(
        r#"
        SELECT id, user_id, token_hash, expires_at, created_at, revoked_at
        FROM refresh_tokens
        WHERE token_hash = $1
        "#,
    )
    .bind(token_hash)
    .fetch_one(pool)
    .await
    .map_err(|error| match error {
        sqlx::Error::RowNotFound => PacsError::NotFound {
            resource: "refresh_token",
            uid: token_hash.to_string(),
        },
        other => PacsError::Store(Box::new(other)),
    })
    .map(RefreshToken::from)
}

pub(crate) async fn revoke_all(pool: &PgPool, user_id: &UserId) -> PacsResult<()> {
    sqlx::query(
        r#"
        UPDATE refresh_tokens
        SET revoked_at = NOW()
        WHERE user_id = $1
          AND revoked_at IS NULL
        "#,
    )
    .bind(*user_id.as_uuid())
    .execute(pool)
    .await
    .map_err(|error| PacsError::Store(Box::new(error)))?;

    Ok(())
}
