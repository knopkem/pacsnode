//! Password-policy SQL helpers.

use pacs_core::{PacsError, PacsResult, PasswordPolicy};
use sqlx::PgPool;

const POLICY_KEY: &str = "default";

#[derive(sqlx::FromRow)]
struct PasswordPolicyRow {
    min_length: i32,
    require_uppercase: bool,
    require_digit: bool,
    require_special: bool,
    max_failed_attempts: i32,
    lockout_duration_secs: i32,
    max_age_days: Option<i32>,
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

pub(crate) async fn get(pool: &PgPool) -> PacsResult<PasswordPolicy> {
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
        WHERE policy_key = $1
        "#,
    )
    .bind(POLICY_KEY)
    .fetch_one(pool)
    .await
    .map_err(|error| PacsError::Store(Box::new(error)))?;

    PasswordPolicy::try_from(row)
}

pub(crate) async fn upsert(pool: &PgPool, policy: &PasswordPolicy) -> PacsResult<()> {
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
            max_age_days
        ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8
        )
        ON CONFLICT (policy_key) DO UPDATE SET
            min_length = EXCLUDED.min_length,
            require_uppercase = EXCLUDED.require_uppercase,
            require_digit = EXCLUDED.require_digit,
            require_special = EXCLUDED.require_special,
            max_failed_attempts = EXCLUDED.max_failed_attempts,
            lockout_duration_secs = EXCLUDED.lockout_duration_secs,
            max_age_days = EXCLUDED.max_age_days,
            updated_at = NOW()
        "#,
    )
    .bind(POLICY_KEY)
    .bind(policy.min_length as i32)
    .bind(policy.require_uppercase)
    .bind(policy.require_digit)
    .bind(policy.require_special)
    .bind(policy.max_failed_attempts as i32)
    .bind(policy.lockout_duration_secs as i32)
    .bind(policy.max_age_days.map(|value| value as i32))
    .execute(pool)
    .await
    .map_err(|error| PacsError::Store(Box::new(error)))?;

    Ok(())
}
