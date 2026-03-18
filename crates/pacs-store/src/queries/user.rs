//! User SQL helpers.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use pacs_core::{PacsError, PacsResult, User, UserId, UserQuery, UserRole};
use sqlx::{PgPool, Postgres, QueryBuilder};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    username: String,
    display_name: Option<String>,
    email: Option<String>,
    password_hash: String,
    role: String,
    attributes: serde_json::Value,
    is_active: bool,
    failed_login_attempts: i32,
    locked_until: Option<DateTime<Utc>>,
    password_changed_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl TryFrom<UserRow> for User {
    type Error = PacsError;

    fn try_from(row: UserRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: UserId::from(row.id),
            username: row.username,
            display_name: row.display_name,
            email: row.email,
            password_hash: row.password_hash,
            role: UserRole::from_str(&row.role)
                .map_err(|_| PacsError::Config(format!("invalid persisted role: {}", row.role)))?,
            attributes: row.attributes,
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

pub(crate) async fn upsert(pool: &PgPool, user: &User) -> PacsResult<()> {
    sqlx::query(
        r#"
        INSERT INTO users (
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
            password_changed_at
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11
        )
        ON CONFLICT (id) DO UPDATE SET
            username = EXCLUDED.username,
            display_name = EXCLUDED.display_name,
            email = EXCLUDED.email,
            password_hash = EXCLUDED.password_hash,
            role = EXCLUDED.role,
            attributes = EXCLUDED.attributes,
            is_active = EXCLUDED.is_active,
            failed_login_attempts = EXCLUDED.failed_login_attempts,
            locked_until = EXCLUDED.locked_until,
            password_changed_at = EXCLUDED.password_changed_at,
            updated_at = NOW()
        "#,
    )
    .bind(*user.id.as_uuid())
    .bind(&user.username)
    .bind(user.display_name.as_deref())
    .bind(user.email.as_deref())
    .bind(&user.password_hash)
    .bind(user.role.as_str())
    .bind(&user.attributes)
    .bind(user.is_active)
    .bind(user.failed_login_attempts as i32)
    .bind(user.locked_until)
    .bind(user.password_changed_at)
    .execute(pool)
    .await
    .map_err(|error| PacsError::Store(Box::new(error)))?;

    Ok(())
}

pub(crate) async fn get(pool: &PgPool, id: &UserId) -> PacsResult<User> {
    sqlx::query_as::<_, UserRow>(&format!("{USER_SELECT} WHERE id = $1"))
        .bind(*id.as_uuid())
        .fetch_one(pool)
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => PacsError::NotFound {
                resource: "user",
                uid: id.to_string(),
            },
            other => PacsError::Store(Box::new(other)),
        })
        .and_then(User::try_from)
}

pub(crate) async fn get_by_username(pool: &PgPool, username: &str) -> PacsResult<User> {
    sqlx::query_as::<_, UserRow>(&format!("{USER_SELECT} WHERE username = $1"))
        .bind(username)
        .fetch_one(pool)
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => PacsError::NotFound {
                resource: "user",
                uid: username.to_string(),
            },
            other => PacsError::Store(Box::new(other)),
        })
        .and_then(User::try_from)
}

pub(crate) async fn query(pool: &PgPool, query: &UserQuery) -> PacsResult<Vec<User>> {
    let mut qb = QueryBuilder::<Postgres>::new(format!("{USER_SELECT} WHERE 1=1"));

    if let Some(search) = &query.search {
        let pattern = format!("%{search}%");
        qb.push(" AND (");
        qb.push("username ILIKE ");
        qb.push_bind(pattern.clone());
        qb.push(" OR COALESCE(display_name, '') ILIKE ");
        qb.push_bind(pattern.clone());
        qb.push(" OR COALESCE(email, '') ILIKE ");
        qb.push_bind(pattern);
        qb.push(")");
    }

    if let Some(role) = query.role {
        qb.push(" AND role = ");
        qb.push_bind(role.as_str());
    }

    if let Some(is_active) = query.is_active {
        qb.push(" AND is_active = ");
        qb.push_bind(is_active);
    }

    qb.push(" ORDER BY username ASC LIMIT ");
    qb.push_bind(i64::from(query.limit.unwrap_or(100)));
    qb.push(" OFFSET ");
    qb.push_bind(i64::from(query.offset.unwrap_or(0)));

    qb.build_query_as::<UserRow>()
        .fetch_all(pool)
        .await
        .map_err(|error| PacsError::Store(Box::new(error)))?
        .into_iter()
        .map(User::try_from)
        .collect()
}

pub(crate) async fn delete(pool: &PgPool, id: &UserId) -> PacsResult<()> {
    let result = sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(*id.as_uuid())
        .execute(pool)
        .await
        .map_err(|error| PacsError::Store(Box::new(error)))?;

    if result.rows_affected() == 0 {
        return Err(PacsError::NotFound {
            resource: "user",
            uid: id.to_string(),
        });
    }

    Ok(())
}
