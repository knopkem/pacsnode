use pacs_core::{
    PacsError, PolicyAction, PolicyEngine, PolicyResource, PolicyUser, Series, SeriesQuery, Study,
    StudyQuery, UserRole,
};
use pacs_plugin::AuthenticatedUser;

fn parse_policy_user(user: &AuthenticatedUser) -> Result<PolicyUser<'_>, PacsError> {
    let role = user
        .role
        .parse::<UserRole>()
        .map_err(|_| PacsError::Internal("authenticated user has invalid role state".into()))?;
    Ok(PolicyUser::new(role, &user.attributes))
}

fn role_label(user: &AuthenticatedUser) -> &str {
    user.role.as_str()
}

fn action_label(action: PolicyAction) -> &'static str {
    match action {
        PolicyAction::Query => "query PACS resources",
        PolicyAction::Read => "read PACS resources",
        PolicyAction::Upload => "upload studies",
        PolicyAction::Delete => "delete PACS resources",
        PolicyAction::Admin => "perform admin actions",
    }
}

fn forbidden_for_action(user: &AuthenticatedUser, action: PolicyAction) -> PacsError {
    PacsError::Forbidden(format!(
        "role '{}' cannot {}",
        role_label(user),
        action_label(action)
    ))
}

fn forbidden_for_resource(
    user: &AuthenticatedUser,
    action: PolicyAction,
    resource: &'static str,
) -> PacsError {
    PacsError::Forbidden(format!(
        "role '{}' cannot {} for this {}",
        role_label(user),
        action_label(action),
        resource
    ))
}

pub(crate) fn authorize_action(
    user: Option<&AuthenticatedUser>,
    action: PolicyAction,
) -> Result<(), PacsError> {
    let Some(user) = user else {
        return Ok(());
    };
    let subject = parse_policy_user(user)?;
    if PolicyEngine::new().check_permission(&subject, action, PolicyResource::System) {
        Ok(())
    } else {
        Err(forbidden_for_action(user, action))
    }
}

pub(crate) fn apply_study_query_filters(
    user: Option<&AuthenticatedUser>,
    query: &mut StudyQuery,
) -> Result<(), PacsError> {
    if let Some(user) = user {
        let subject = parse_policy_user(user)?;
        PolicyEngine::new().apply_query_filters(&subject, query);
    }
    Ok(())
}

pub(crate) fn filter_studies(
    user: Option<&AuthenticatedUser>,
    studies: Vec<Study>,
    action: PolicyAction,
) -> Result<Vec<Study>, PacsError> {
    let Some(user) = user else {
        return Ok(studies);
    };
    let subject = parse_policy_user(user)?;
    Ok(studies
        .into_iter()
        .filter(|study| PolicyEngine::new().can_access_study(&subject, study, action))
        .collect())
}

pub(crate) fn apply_series_query_filters(
    user: Option<&AuthenticatedUser>,
    query: &mut SeriesQuery,
) -> Result<(), PacsError> {
    if let Some(user) = user {
        let subject = parse_policy_user(user)?;
        PolicyEngine::new().apply_series_query_filters(&subject, query);
    }
    Ok(())
}

pub(crate) fn filter_series(
    user: Option<&AuthenticatedUser>,
    series: Vec<Series>,
    action: PolicyAction,
) -> Result<Vec<Series>, PacsError> {
    let Some(user) = user else {
        return Ok(series);
    };
    let subject = parse_policy_user(user)?;
    Ok(series
        .into_iter()
        .filter(|series| PolicyEngine::new().can_access_series(&subject, series, action))
        .collect())
}

pub(crate) fn authorize_study(
    user: Option<&AuthenticatedUser>,
    study: &Study,
    action: PolicyAction,
) -> Result<(), PacsError> {
    let Some(user) = user else {
        return Ok(());
    };
    let subject = parse_policy_user(user)?;
    if PolicyEngine::new().can_access_study(&subject, study, action) {
        Ok(())
    } else {
        Err(forbidden_for_resource(user, action, "study"))
    }
}

pub(crate) fn authorize_series(
    user: Option<&AuthenticatedUser>,
    series: &Series,
    action: PolicyAction,
) -> Result<(), PacsError> {
    let Some(user) = user else {
        return Ok(());
    };
    let subject = parse_policy_user(user)?;
    if PolicyEngine::new().can_access_series(&subject, series, action) {
        Ok(())
    } else {
        Err(forbidden_for_resource(user, action, "series"))
    }
}

#[cfg(test)]
mod tests {
    use pacs_core::{StudyUid, UserRole};
    use serde_json::json;

    use super::*;

    fn auth_user(role: UserRole, attributes: serde_json::Value) -> AuthenticatedUser {
        AuthenticatedUser::new("1", "alice", role.as_str(), attributes)
    }

    #[test]
    fn uploader_cannot_query() {
        let user = auth_user(UserRole::Uploader, json!({}));
        let error = authorize_action(Some(&user), PolicyAction::Query).unwrap_err();

        assert!(matches!(error, PacsError::Forbidden(_)));
    }

    #[test]
    fn study_filter_removes_disallowed_modalities() {
        let user = auth_user(UserRole::Viewer, json!({"modality_access": ["CT"]}));
        let studies = vec![
            Study {
                study_uid: StudyUid::from("1.2.3"),
                patient_id: None,
                patient_name: None,
                study_date: None,
                study_time: None,
                accession_number: None,
                modalities: vec!["CT".into()],
                referring_physician: None,
                description: None,
                num_series: 1,
                num_instances: 1,
                metadata: pacs_core::DicomJson::empty(),
                created_at: None,
                updated_at: None,
            },
            Study {
                study_uid: StudyUid::from("9.9.9"),
                modalities: vec!["US".into()],
                patient_id: None,
                patient_name: None,
                study_date: None,
                study_time: None,
                accession_number: None,
                referring_physician: None,
                description: None,
                num_series: 1,
                num_instances: 1,
                metadata: pacs_core::DicomJson::empty(),
                created_at: None,
                updated_at: None,
            },
        ];

        let filtered = filter_studies(Some(&user), studies, PolicyAction::Read).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].study_uid.as_ref(), "1.2.3");
    }
}
