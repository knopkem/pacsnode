use serde_json::Value;

use crate::{Series, SeriesQuery, Study, StudyQuery, UserRole};

/// Authorization action evaluated by the built-in policy engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAction {
    /// Query/search access to indexed metadata.
    Query,
    /// Read or retrieve previously stored resources.
    Read,
    /// Upload or ingest new instances.
    Upload,
    /// Destructive deletion of resources.
    Delete,
    /// Administrative-only configuration or user-management actions.
    Admin,
}

/// Authenticated subject data used by the built-in policy engine.
///
/// # Example
///
/// ```
/// use pacs_core::{PolicyUser, UserRole};
/// use serde_json::json;
///
/// let attrs = json!({"modality_access": ["CT", "MR"]});
/// let user = PolicyUser::new(UserRole::Radiologist, &attrs);
/// assert_eq!(user.role(), UserRole::Radiologist);
/// ```
pub struct PolicyUser<'a> {
    role: UserRole,
    attributes: &'a Value,
}

impl<'a> PolicyUser<'a> {
    /// Creates a new policy subject from a role and attribute document.
    pub fn new(role: UserRole, attributes: &'a Value) -> Self {
        Self { role, attributes }
    }

    /// Returns the effective application role.
    pub fn role(&self) -> UserRole {
        self.role
    }

    /// Returns the raw authorization attributes.
    pub fn attributes(&self) -> &'a Value {
        self.attributes
    }
}

/// Resource context used for authorization checks.
pub enum PolicyResource<'a> {
    /// Non-DICOM system or admin resource.
    System,
    /// Study-scoped resource carrying one or more modalities.
    Study { modalities: &'a [String] },
    /// Series-scoped resource carrying a single modality when known.
    Series { modality: Option<&'a str> },
    /// Instance-scoped resource carrying a single modality when known.
    Instance { modality: Option<&'a str> },
}

/// Built-in role and attribute-based authorization engine.
///
/// This engine currently enforces the default role matrix and the
/// `modality_access` attribute. Additional dimensions such as department or
/// assignment-based access can be layered on later without changing handler
/// call sites.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolicyEngine;

impl PolicyEngine {
    /// Creates the default built-in policy engine.
    pub fn new() -> Self {
        Self
    }

    /// Returns `true` when the subject is allowed to perform the action on the
    /// supplied resource context.
    pub fn check_permission(
        &self,
        user: &PolicyUser<'_>,
        action: PolicyAction,
        resource: PolicyResource<'_>,
    ) -> bool {
        if !role_allows(user.role(), action) {
            return false;
        }

        match self.allowed_modalities(user) {
            None => true,
            Some(allowed) => match resource {
                PolicyResource::System => true,
                PolicyResource::Study { modalities } => {
                    !modalities.is_empty()
                        && modalities
                            .iter()
                            .map(String::as_str)
                            .all(|modality| modality_is_allowed(&allowed, Some(modality)))
                }
                PolicyResource::Series { modality } | PolicyResource::Instance { modality } => {
                    modality_is_allowed(&allowed, modality)
                }
            },
        }
    }

    /// Applies subject-derived restrictions to a study query before it reaches
    /// the metadata store.
    pub fn apply_query_filters(&self, user: &PolicyUser<'_>, query: &mut StudyQuery) {
        if let Some(allowed) = self.allowed_modalities(user) {
            if allowed.len() == 1 && query.modality.is_none() {
                query.modality = allowed.first().cloned();
            }
        }
    }

    /// Applies subject-derived restrictions to a series query before it reaches
    /// the metadata store.
    pub fn apply_series_query_filters(&self, user: &PolicyUser<'_>, query: &mut SeriesQuery) {
        if let Some(allowed) = self.allowed_modalities(user) {
            if allowed.len() == 1 && query.modality.is_none() {
                query.modality = allowed.first().cloned();
            }
        }
    }

    /// Returns `true` when the subject may access the given study.
    pub fn can_access_study(
        &self,
        user: &PolicyUser<'_>,
        study: &Study,
        action: PolicyAction,
    ) -> bool {
        self.check_permission(
            user,
            action,
            PolicyResource::Study {
                modalities: &study.modalities,
            },
        )
    }

    /// Returns `true` when the subject may access the given series.
    pub fn can_access_series(
        &self,
        user: &PolicyUser<'_>,
        series: &Series,
        action: PolicyAction,
    ) -> bool {
        self.check_permission(
            user,
            action,
            PolicyResource::Series {
                modality: series.modality.as_deref(),
            },
        )
    }

    /// Returns the normalized `modality_access` restriction list, or `None`
    /// when no modality restriction applies.
    pub fn allowed_modalities(&self, user: &PolicyUser<'_>) -> Option<Vec<String>> {
        let values = user
            .attributes()
            .get("modality_access")
            .and_then(Value::as_array)?;

        let allowed = values
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| value.to_ascii_uppercase())
            .collect::<Vec<_>>();

        if allowed.is_empty() || allowed.iter().any(|value| value == "ALL") {
            None
        } else {
            Some(allowed)
        }
    }
}

fn role_allows(role: UserRole, action: PolicyAction) -> bool {
    match role {
        UserRole::Admin => true,
        UserRole::Radiologist => matches!(action, PolicyAction::Query | PolicyAction::Read),
        UserRole::Technologist => {
            matches!(
                action,
                PolicyAction::Query | PolicyAction::Read | PolicyAction::Upload
            )
        }
        UserRole::Viewer => matches!(action, PolicyAction::Query | PolicyAction::Read),
        UserRole::Uploader => matches!(action, PolicyAction::Upload),
    }
}

fn modality_is_allowed(allowed: &[String], modality: Option<&str>) -> bool {
    let Some(modality) = modality.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    allowed
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(modality))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::{SeriesUid, StudyUid};

    #[test]
    fn radiologist_cannot_upload() {
        let attrs = json!({});
        let engine = PolicyEngine::new();
        let user = PolicyUser::new(UserRole::Radiologist, &attrs);

        assert!(!engine.check_permission(&user, PolicyAction::Upload, PolicyResource::System));
    }

    #[test]
    fn viewer_with_modality_access_is_restricted_to_matching_series() {
        let attrs = json!({"modality_access": ["CT", "MR"]});
        let engine = PolicyEngine::new();
        let user = PolicyUser::new(UserRole::Viewer, &attrs);
        let allowed_series = Series {
            series_uid: SeriesUid::from("1.2.3"),
            study_uid: StudyUid::from("1.2"),
            modality: Some("CT".into()),
            series_number: None,
            description: None,
            body_part: None,
            num_instances: 0,
            metadata: crate::DicomJson::empty(),
            created_at: None,
        };
        let denied_series = Series {
            modality: Some("US".into()),
            ..allowed_series.clone()
        };

        assert!(engine.can_access_series(&user, &allowed_series, PolicyAction::Read));
        assert!(!engine.can_access_series(&user, &denied_series, PolicyAction::Read));
    }

    #[test]
    fn mixed_modality_study_is_denied_when_any_modality_is_outside_scope() {
        let attrs = json!({"modality_access": ["CT"]});
        let engine = PolicyEngine::new();
        let user = PolicyUser::new(UserRole::Viewer, &attrs);
        let study = Study {
            study_uid: StudyUid::from("1.2.3"),
            patient_id: None,
            patient_name: None,
            study_date: None,
            study_time: None,
            accession_number: None,
            modalities: vec!["CT".into(), "US".into()],
            referring_physician: None,
            description: None,
            num_series: 2,
            num_instances: 2,
            metadata: crate::DicomJson::empty(),
            created_at: None,
            updated_at: None,
        };

        assert!(!engine.can_access_study(&user, &study, PolicyAction::Read));
    }

    #[test]
    fn single_modality_access_is_injected_into_queries() {
        let attrs = json!({"modality_access": ["ct"]});
        let engine = PolicyEngine::new();
        let user = PolicyUser::new(UserRole::Radiologist, &attrs);
        let mut query = StudyQuery::default();

        engine.apply_query_filters(&user, &mut query);

        assert_eq!(query.modality.as_deref(), Some("CT"));
    }

    #[test]
    fn all_modality_access_disables_restriction() {
        let attrs = json!({"modality_access": ["all"]});
        let engine = PolicyEngine::new();
        let user = PolicyUser::new(UserRole::Viewer, &attrs);

        assert!(engine.allowed_modalities(&user).is_none());
    }
}
