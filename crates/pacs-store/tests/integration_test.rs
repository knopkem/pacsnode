//! Integration tests for [`PgMetadataStore`].
//!
//! Each test spins up a throwaway PostgreSQL container via
//! [testcontainers](https://crates.io/crates/testcontainers), runs the project
//! migrations, and executes against a real database.
//!
//! # Running
//!
//! ```bash
//! cargo test -p pacs-store
//! ```
//!
//! Docker (or a compatible container runtime) must be available and running.

use chrono::{NaiveDate, TimeZone, Utc};
use pacs_core::{
    DicomJson, Instance, InstanceQuery, MetadataStore, PacsError, PasswordPolicy, RefreshToken,
    RefreshTokenId, Series, SeriesQuery, SeriesUid, ServerSettings, SopInstanceUid, Study,
    StudyQuery, StudyUid, User, UserId, UserQuery, UserRole,
};
use pacs_store::PgMetadataStore;
use rstest::rstest;
use sqlx::PgPool;
use testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};
use testcontainers_modules::postgres::Postgres;
use uuid::Uuid;

const DEFAULT_POSTGRES_IMAGE_TAG: &str = "16-alpine";

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn postgres_image_tag() -> String {
    std::env::var("PACSNODE_TEST_POSTGRES_TAG")
        .unwrap_or_else(|_| DEFAULT_POSTGRES_IMAGE_TAG.to_string())
}

/// Starts a PostgreSQL container, connects a pool, and runs migrations.
///
/// The returned `ContainerAsync` must be kept alive for the duration of the
/// test (dropping it stops the container).
async fn setup_pool() -> (PgPool, ContainerAsync<Postgres>) {
    let container = Postgres::default()
        .with_tag(postgres_image_tag())
        .start()
        .await
        .expect("failed to start Postgres container");

    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get Postgres host port");

    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let pool = PgPool::connect(&url)
        .await
        .expect("failed to connect to Postgres");

    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("failed to run migrations");

    (pool, container)
}

fn study_uid(n: u32) -> StudyUid {
    StudyUid::from(format!("1.2.3.{n}"))
}

fn series_uid(n: u32) -> SeriesUid {
    SeriesUid::from(format!("2.3.4.{n}"))
}

fn instance_uid(n: u32) -> SopInstanceUid {
    SopInstanceUid::from(format!("3.4.5.{n}"))
}

fn make_study(uid: StudyUid) -> Study {
    Study {
        study_uid: uid,
        patient_id: Some("PID001".to_string()),
        patient_name: Some("Doe^John".to_string()),
        study_date: NaiveDate::from_ymd_opt(2024, 6, 15),
        study_time: Some("120000".to_string()),
        accession_number: Some("ACC001".to_string()),
        modalities: vec!["CT".to_string()],
        referring_physician: Some("Dr. Smith".to_string()),
        description: Some("Chest CT".to_string()),
        num_series: 0,
        num_instances: 0,
        metadata: DicomJson::empty(),
        created_at: None,
        updated_at: None,
    }
}

fn make_series(series_uid: SeriesUid, study: &Study) -> Series {
    Series {
        series_uid,
        study_uid: study.study_uid.clone(),
        modality: Some("CT".to_string()),
        series_number: Some(1),
        description: Some("Axial".to_string()),
        body_part: Some("CHEST".to_string()),
        num_instances: 0,
        metadata: DicomJson::empty(),
        created_at: None,
    }
}

fn make_instance(inst_uid: SopInstanceUid, series: &Series, study: &Study) -> Instance {
    Instance {
        instance_uid: inst_uid,
        series_uid: series.series_uid.clone(),
        study_uid: study.study_uid.clone(),
        sop_class_uid: Some("1.2.840.10008.5.1.4.1.1.2".to_string()),
        instance_number: Some(1),
        transfer_syntax: Some("1.2.840.10008.1.2.1".to_string()),
        rows: Some(512),
        columns: Some(512),
        blob_key: format!("{}/{}/{}", study.study_uid, series.series_uid, "3.4.5.1"),
        metadata: DicomJson::empty(),
        created_at: None,
    }
}

fn make_server_settings() -> ServerSettings {
    ServerSettings {
        dicom_port: 11112,
        ae_title: "PACSNODE_UI".into(),
        ae_whitelist_enabled: true,
        accept_all_transfer_syntaxes: false,
        accepted_transfer_syntaxes: vec![
            "1.2.840.10008.1.2.1".into(),
            "1.2.840.10008.1.2.4.50".into(),
        ],
        preferred_transfer_syntaxes: vec!["1.2.840.10008.1.2.4.50".into()],
        max_associations: 24,
        dimse_timeout_secs: 40,
    }
}

fn make_user(id_suffix: u128) -> User {
    User {
        id: UserId::from(Uuid::from_u128(id_suffix)),
        username: format!("user{id_suffix}"),
        display_name: Some("Admin User".into()),
        email: Some(format!("user{id_suffix}@example.test")),
        password_hash: "argon2-hash".into(),
        role: UserRole::Admin,
        attributes: serde_json::json!({"department": "radiology"}),
        is_active: true,
        failed_login_attempts: 0,
        locked_until: None,
        password_changed_at: Some(Utc.with_ymd_and_hms(2026, 3, 18, 12, 0, 0).unwrap()),
        created_at: None,
        updated_at: None,
    }
}

fn make_refresh_token(user_id: UserId, id_suffix: u128) -> RefreshToken {
    RefreshToken {
        id: RefreshTokenId(Uuid::from_u128(id_suffix)),
        user_id,
        token_hash: format!("refresh-hash-{id_suffix}"),
        expires_at: Utc.with_ymd_and_hms(2026, 3, 25, 12, 0, 0).unwrap(),
        created_at: Utc.with_ymd_and_hms(2026, 3, 18, 12, 0, 0).unwrap(),
        revoked_at: None,
    }
}

// ---------------------------------------------------------------------------
// Study tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_store_and_retrieve_study() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);
    let study = make_study(study_uid(1));

    store.store_study(&study).await.expect("store_study failed");

    let fetched = store
        .get_study(&study.study_uid)
        .await
        .expect("get_study failed");
    assert_eq!(fetched.study_uid, study.study_uid);
    assert_eq!(fetched.patient_id, study.patient_id);
    assert_eq!(fetched.modalities, study.modalities);
    assert_eq!(fetched.num_series, study.num_series);
    assert_eq!(fetched.num_instances, study.num_instances);
    assert!(fetched.created_at.is_some());
    assert!(fetched.updated_at.is_some());
}

#[tokio::test]
async fn test_study_upsert_updates_fields() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);
    let mut study = make_study(study_uid(10));
    store.store_study(&study).await.expect("first store failed");

    // Mutate and re-store
    study.patient_id = Some("PID_UPDATED".to_string());
    study.modalities = vec!["CT".to_string(), "PT".to_string()];
    store
        .store_study(&study)
        .await
        .expect("second store failed");

    let fetched = store.get_study(&study.study_uid).await.expect("get failed");
    assert_eq!(fetched.patient_id.as_deref(), Some("PID_UPDATED"));
    assert_eq!(fetched.modalities, vec!["CT", "PT"]);
    assert_eq!(fetched.num_series, 0);
    assert_eq!(fetched.num_instances, 0);
}

#[tokio::test]
async fn test_study_counts_are_derived_from_related_rows() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(11));
    let series = make_series(series_uid(11), &study);
    let instance = make_instance(instance_uid(11), &series, &study);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store
        .store_instance(&instance)
        .await
        .expect("store instance");

    let fetched_study = store.get_study(&study.study_uid).await.expect("get study");
    assert_eq!(fetched_study.num_series, 1);
    assert_eq!(fetched_study.num_instances, 1);

    let fetched_series = store
        .get_series(&series.series_uid)
        .await
        .expect("get series");
    assert_eq!(fetched_series.num_instances, 1);
}

#[tokio::test]
async fn test_user_policy_and_refresh_token_round_trip() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);
    let user = make_user(101);
    let refresh_token = make_refresh_token(user.id, 201);

    store.store_user(&user).await.expect("store user");

    let fetched = store.get_user(&user.id).await.expect("get user");
    assert_eq!(fetched.username, user.username);
    assert_eq!(fetched.role, UserRole::Admin);

    let fetched_by_username = store
        .get_user_by_username(&user.username)
        .await
        .expect("get user by username");
    assert_eq!(fetched_by_username.id, user.id);

    let users = store
        .query_users(&UserQuery {
            search: Some("user101".into()),
            role: Some(UserRole::Admin),
            is_active: Some(true),
            limit: Some(10),
            offset: Some(0),
        })
        .await
        .expect("query users");
    assert_eq!(users.len(), 1);

    let mut policy = store.get_password_policy().await.expect("default policy");
    assert_eq!(policy, PasswordPolicy::default());

    policy.min_length = 16;
    policy.require_special = true;
    store
        .upsert_password_policy(&policy)
        .await
        .expect("update policy");

    let reloaded_policy = store.get_password_policy().await.expect("reloaded policy");
    assert_eq!(reloaded_policy.min_length, 16);
    assert!(reloaded_policy.require_special);

    store
        .store_refresh_token(&refresh_token)
        .await
        .expect("store refresh token");

    let fetched_token = store
        .get_refresh_token(&refresh_token.token_hash)
        .await
        .expect("get refresh token");
    assert!(fetched_token.revoked_at.is_none());

    store
        .revoke_refresh_tokens(&user.id)
        .await
        .expect("revoke refresh tokens");

    let revoked_token = store
        .get_refresh_token(&refresh_token.token_hash)
        .await
        .expect("get revoked refresh token");
    assert!(revoked_token.revoked_at.is_some());
}

#[tokio::test]
async fn test_get_nonexistent_study_returns_not_found() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let err = store
        .get_study(&StudyUid::from("9.9.9.9.9"))
        .await
        .expect_err("should return NotFound");

    assert!(matches!(
        err,
        PacsError::NotFound {
            resource: "study",
            ..
        }
    ));
}

#[tokio::test]
async fn test_delete_study_removes_row() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);
    let study = make_study(study_uid(20));
    store.store_study(&study).await.expect("store failed");

    store
        .delete_study(&study.study_uid)
        .await
        .expect("delete failed");

    let err = store
        .get_study(&study.study_uid)
        .await
        .expect_err("should be gone");
    assert!(matches!(err, PacsError::NotFound { .. }));
}

#[tokio::test]
async fn test_delete_study_cascades_to_series_and_instances() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(30));
    let series = make_series(series_uid(30), &study);
    let inst = make_instance(instance_uid(30), &series, &study);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&inst).await.expect("store instance");

    // Deleting the study must cascade
    store
        .delete_study(&study.study_uid)
        .await
        .expect("delete study");

    let err = store
        .get_series(&series.series_uid)
        .await
        .expect_err("series should be gone");
    assert!(matches!(
        err,
        PacsError::NotFound {
            resource: "series",
            ..
        }
    ));

    let err = store
        .get_instance(&inst.instance_uid)
        .await
        .expect_err("instance should be gone");
    assert!(matches!(
        err,
        PacsError::NotFound {
            resource: "instance",
            ..
        }
    ));
}

#[tokio::test]
async fn test_delete_nonexistent_study_returns_not_found() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let err = store
        .delete_study(&StudyUid::from("0.0.0.0"))
        .await
        .expect_err("should be NotFound");
    assert!(matches!(err, PacsError::NotFound { .. }));
}

#[tokio::test]
async fn test_server_settings_round_trip() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    assert_eq!(
        store.get_server_settings().await.expect("get settings"),
        None
    );

    let settings = make_server_settings();
    store
        .upsert_server_settings(&settings)
        .await
        .expect("upsert settings");

    assert_eq!(
        store.get_server_settings().await.expect("reload settings"),
        Some(settings)
    );
}

// ---------------------------------------------------------------------------
// Query tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_query_studies_by_patient_id() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let mut s1 = make_study(study_uid(40));
    s1.patient_id = Some("PID_A".to_string());
    let mut s2 = make_study(study_uid(41));
    s2.patient_id = Some("PID_B".to_string());

    store.store_study(&s1).await.expect("store s1");
    store.store_study(&s2).await.expect("store s2");

    let results = store
        .query_studies(&StudyQuery {
            patient_id: Some("PID_A".to_string()),
            ..Default::default()
        })
        .await
        .expect("query failed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].study_uid, s1.study_uid);
}

#[tokio::test]
async fn test_query_studies_by_date_range() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let mut s1 = make_study(study_uid(50));
    s1.study_date = NaiveDate::from_ymd_opt(2024, 1, 10);
    let mut s2 = make_study(study_uid(51));
    s2.study_date = NaiveDate::from_ymd_opt(2024, 6, 20);
    let mut s3 = make_study(study_uid(52));
    s3.study_date = NaiveDate::from_ymd_opt(2024, 12, 31);

    for s in [&s1, &s2, &s3] {
        store.store_study(s).await.expect("store failed");
    }

    let results = store
        .query_studies(&StudyQuery {
            study_date_from: NaiveDate::from_ymd_opt(2024, 1, 1),
            study_date_to: NaiveDate::from_ymd_opt(2024, 6, 30),
            ..Default::default()
        })
        .await
        .expect("query failed");

    let uids: Vec<_> = results.iter().map(|s| s.study_uid.as_ref()).collect();
    assert!(uids.contains(&s1.study_uid.as_ref()));
    assert!(uids.contains(&s2.study_uid.as_ref()));
    assert!(!uids.contains(&s3.study_uid.as_ref()));
}

#[tokio::test]
async fn test_query_studies_fuzzy_patient_name() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let mut s1 = make_study(study_uid(60));
    s1.patient_name = Some("Anderson^Alice".to_string());
    let mut s2 = make_study(study_uid(61));
    s2.patient_name = Some("Brown^Bob".to_string());

    store.store_study(&s1).await.expect("store s1");
    store.store_study(&s2).await.expect("store s2");

    // Wildcard prefix search
    let results = store
        .query_studies(&StudyQuery {
            patient_name: Some("Anderson*".to_string()),
            fuzzy_matching: true,
            ..Default::default()
        })
        .await
        .expect("query failed");

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].study_uid, s1.study_uid);
}

#[tokio::test]
async fn test_query_studies_by_modality() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let mut ct_study = make_study(study_uid(70));
    ct_study.modalities = vec!["CT".to_string()];
    let mut mr_study = make_study(study_uid(71));
    mr_study.modalities = vec!["MR".to_string()];

    store.store_study(&ct_study).await.expect("store ct");
    store.store_study(&mr_study).await.expect("store mr");

    let results = store
        .query_studies(&StudyQuery {
            modality: Some("MR".to_string()),
            ..Default::default()
        })
        .await
        .expect("query failed");

    let uids: Vec<_> = results.iter().map(|s| s.study_uid.as_ref()).collect();
    assert!(uids.contains(&mr_study.study_uid.as_ref()));
    assert!(!uids.contains(&ct_study.study_uid.as_ref()));
}

#[rstest]
#[case(Some(1), 1)]
#[case(Some(2), 2)]
#[case(None, 3)]
#[tokio::test]
async fn test_query_studies_limit(#[case] limit: Option<u32>, #[case] expected_len: usize) {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    for n in [80u32, 81, 82] {
        let mut s = make_study(study_uid(n));
        s.patient_id = Some("PID_LIMIT".to_string());
        store.store_study(&s).await.expect("store failed");
    }

    let results = store
        .query_studies(&StudyQuery {
            patient_id: Some("PID_LIMIT".to_string()),
            limit,
            ..Default::default()
        })
        .await
        .expect("query failed");

    assert_eq!(results.len(), expected_len);
}

// ---------------------------------------------------------------------------
// Series tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_store_and_retrieve_series() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(100));
    let series = make_series(series_uid(100), &study);
    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");

    let fetched = store
        .get_series(&series.series_uid)
        .await
        .expect("get_series failed");

    assert_eq!(fetched.series_uid, series.series_uid);
    assert_eq!(fetched.study_uid, series.study_uid);
    assert_eq!(fetched.modality.as_deref(), Some("CT"));
    assert_eq!(fetched.series_number, Some(1));
}

#[tokio::test]
async fn test_query_series_by_study_uid() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(110));
    let s1 = make_series(series_uid(110), &study);
    let mut s2 = make_series(series_uid(111), &study);
    s2.series_number = Some(2);

    store.store_study(&study).await.expect("store study");
    store.store_series(&s1).await.expect("store s1");
    store.store_series(&s2).await.expect("store s2");

    let results = store
        .query_series(&SeriesQuery {
            study_uid: study.study_uid.clone(),
            series_uid: None,
            modality: None,
            series_number: None,
            limit: None,
            offset: None,
        })
        .await
        .expect("query_series failed");

    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn test_get_nonexistent_series_returns_not_found() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let err = store
        .get_series(&SeriesUid::from("9.9.9.9"))
        .await
        .expect_err("should be NotFound");
    assert!(matches!(
        err,
        PacsError::NotFound {
            resource: "series",
            ..
        }
    ));
}

// ---------------------------------------------------------------------------
// Instance tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_store_and_retrieve_instance() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(200));
    let series = make_series(series_uid(200), &study);
    let inst = make_instance(instance_uid(200), &series, &study);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&inst).await.expect("store instance");

    let fetched = store
        .get_instance(&inst.instance_uid)
        .await
        .expect("get_instance failed");

    assert_eq!(fetched.instance_uid, inst.instance_uid);
    assert_eq!(fetched.series_uid, inst.series_uid);
    assert_eq!(fetched.study_uid, inst.study_uid);
    assert_eq!(fetched.rows, Some(512));
    assert_eq!(fetched.columns, Some(512));
    assert_eq!(fetched.blob_key, inst.blob_key);
}

#[tokio::test]
async fn test_get_instance_metadata() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(210));
    let series = make_series(series_uid(210), &study);
    let meta = serde_json::json!({"00080060": {"vr": "CS", "Value": ["CT"]}});
    let mut inst = make_instance(instance_uid(210), &series, &study);
    inst.metadata = DicomJson::from(meta.clone());

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&inst).await.expect("store instance");

    let fetched_meta = store
        .get_instance_metadata(&inst.instance_uid)
        .await
        .expect("get_instance_metadata failed");

    assert_eq!(fetched_meta.as_value(), &meta);
}

#[tokio::test]
async fn test_instance_upsert_updates_fields() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(220));
    let series = make_series(series_uid(220), &study);
    let mut inst = make_instance(instance_uid(220), &series, &study);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&inst).await.expect("first store");

    inst.rows = Some(1024);
    inst.columns = Some(1024);
    store.store_instance(&inst).await.expect("second store");

    let fetched = store.get_instance(&inst.instance_uid).await.expect("get");
    assert_eq!(fetched.rows, Some(1024));
    assert_eq!(fetched.columns, Some(1024));
}

#[tokio::test]
async fn test_query_instances_by_series_uid() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(230));
    let series = make_series(series_uid(230), &study);
    let i1 = make_instance(instance_uid(230), &series, &study);
    let mut i2 = make_instance(instance_uid(231), &series, &study);
    i2.instance_number = Some(2);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&i1).await.expect("store i1");
    store.store_instance(&i2).await.expect("store i2");

    let results = store
        .query_instances(&InstanceQuery {
            series_uid: series.series_uid.clone(),
            instance_uid: None,
            sop_class_uid: None,
            instance_number: None,
            limit: None,
            offset: None,
        })
        .await
        .expect("query_instances failed");

    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn test_get_nonexistent_instance_returns_not_found() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let err = store
        .get_instance(&SopInstanceUid::from("9.9.9.9.9"))
        .await
        .expect_err("should be NotFound");
    assert!(matches!(
        err,
        PacsError::NotFound {
            resource: "instance",
            ..
        }
    ));
}

#[tokio::test]
async fn test_delete_series_cascades_to_instances() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(240));
    let series = make_series(series_uid(240), &study);
    let inst = make_instance(instance_uid(240), &series, &study);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&inst).await.expect("store instance");

    store
        .delete_series(&series.series_uid)
        .await
        .expect("delete series");

    let err = store
        .get_instance(&inst.instance_uid)
        .await
        .expect_err("instance should be gone");
    assert!(matches!(err, PacsError::NotFound { .. }));
}

// ---------------------------------------------------------------------------
// Statistics tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_statistics_counts_correctly() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let initial = store.get_statistics().await.expect("initial stats");

    let study = make_study(study_uid(300));
    let series = make_series(series_uid(300), &study);
    let inst = make_instance(instance_uid(300), &series, &study);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&inst).await.expect("store instance");

    let after = store.get_statistics().await.expect("after stats");

    assert_eq!(after.num_studies, initial.num_studies + 1);
    assert_eq!(after.num_series, initial.num_series + 1);
    assert_eq!(after.num_instances, initial.num_instances + 1);
    // Metadata is `{}` → at least 2 bytes
    assert!(after.disk_usage_bytes > initial.disk_usage_bytes);
}

#[tokio::test]
async fn test_statistics_decrements_after_delete() {
    let (pool, _c) = setup_pool().await;
    let store = PgMetadataStore::new(pool);

    let study = make_study(study_uid(310));
    let series = make_series(series_uid(310), &study);
    let inst = make_instance(instance_uid(310), &series, &study);

    store.store_study(&study).await.expect("store study");
    store.store_series(&series).await.expect("store series");
    store.store_instance(&inst).await.expect("store instance");

    let before = store.get_statistics().await.expect("before stats");
    store.delete_study(&study.study_uid).await.expect("delete");
    let after = store.get_statistics().await.expect("after stats");

    assert_eq!(after.num_studies, before.num_studies - 1);
    assert_eq!(after.num_series, before.num_series - 1);
    assert_eq!(after.num_instances, before.num_instances - 1);
}
