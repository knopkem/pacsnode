use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use bytes::Bytes;
use pacs_dicom::{looks_like_dicom_part10, ParsedDicom};
use serde::Serialize;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use walkdir::WalkDir;

const MAX_TRACKED_ERRORS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ImportPhase {
    Idle,
    Scanning,
    ScanComplete,
    Importing,
    Complete,
    Failed,
}

#[derive(Debug, Clone)]
pub(crate) struct ScannedFile {
    pub(crate) path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct ScannedStudySummary {
    pub(crate) study_uid: String,
    pub(crate) patient_name: Option<String>,
    pub(crate) study_date: Option<String>,
    pub(crate) modalities: Vec<String>,
    pub(crate) num_series: usize,
    pub(crate) num_instances: usize,
    pub(crate) total_bytes: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportScanSummary {
    pub(crate) path: String,
    pub(crate) studies: Vec<ScannedStudySummary>,
    pub(crate) files: Vec<ScannedFile>,
    pub(crate) total_instances: usize,
    pub(crate) total_bytes: u64,
    pub(crate) skipped_non_dicom: usize,
    pub(crate) unreadable_dicom: usize,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportProgress {
    pub(crate) completed: usize,
    pub(crate) total: usize,
    pub(crate) imported: usize,
    pub(crate) skipped: usize,
    pub(crate) failed: usize,
    pub(crate) current_path: Option<String>,
    pub(crate) current_study_uid: Option<String>,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportResult {
    pub(crate) imported: usize,
    pub(crate) skipped: usize,
    pub(crate) failed: usize,
    pub(crate) cancelled: bool,
    pub(crate) errors: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ImportJobSnapshot {
    pub(crate) phase: ImportPhase,
    pub(crate) path_input: String,
    pub(crate) summary: Option<ImportScanSummary>,
    pub(crate) progress: Option<ImportProgress>,
    pub(crate) result: Option<ImportResult>,
    pub(crate) failure_message: Option<String>,
    pub(crate) cancellation_requested: bool,
}

impl Default for ImportJobSnapshot {
    fn default() -> Self {
        Self {
            phase: ImportPhase::Idle,
            path_input: String::new(),
            summary: None,
            progress: None,
            result: None,
            failure_message: None,
            cancellation_requested: false,
        }
    }
}

#[derive(Default)]
struct ImportJobState {
    snapshot: ImportJobSnapshot,
    cancel_token: Option<CancellationToken>,
}

pub(crate) struct ImportRuntime {
    state: RwLock<ImportJobState>,
}

impl ImportRuntime {
    pub(crate) fn new() -> Self {
        Self {
            state: RwLock::new(ImportJobState::default()),
        }
    }

    pub(crate) async fn snapshot(&self) -> ImportJobSnapshot {
        self.state.read().await.snapshot.clone()
    }

    pub(crate) async fn start_scan(&self, path_input: String) -> Result<CancellationToken, String> {
        let mut state = self.state.write().await;
        match state.snapshot.phase {
            ImportPhase::Scanning | ImportPhase::Importing => {
                Err("another import job is already running".into())
            }
            _ => {
                let cancel_token = CancellationToken::new();
                state.snapshot = ImportJobSnapshot {
                    phase: ImportPhase::Scanning,
                    path_input,
                    summary: None,
                    progress: None,
                    result: None,
                    failure_message: None,
                    cancellation_requested: false,
                };
                state.cancel_token = Some(cancel_token.clone());
                Ok(cancel_token)
            }
        }
    }

    pub(crate) async fn finish_scan(&self, summary: ImportScanSummary) {
        let mut state = self.state.write().await;
        state.snapshot.phase = ImportPhase::ScanComplete;
        state.snapshot.path_input = summary.path.clone();
        state.snapshot.summary = Some(summary);
        state.snapshot.progress = None;
        state.snapshot.result = None;
        state.snapshot.failure_message = None;
        state.snapshot.cancellation_requested = false;
        state.cancel_token = None;
    }

    pub(crate) async fn begin_import(
        &self,
    ) -> Result<(ImportScanSummary, CancellationToken), String> {
        let mut state = self.state.write().await;
        if state.snapshot.phase != ImportPhase::ScanComplete {
            return Err("scan a directory before starting an import".into());
        }
        let summary = state
            .snapshot
            .summary
            .clone()
            .ok_or_else(|| "scan results are no longer available".to_string())?;
        let cancel_token = CancellationToken::new();
        state.snapshot.phase = ImportPhase::Importing;
        state.snapshot.progress = Some(ImportProgress {
            completed: 0,
            total: summary.files.len(),
            imported: 0,
            skipped: 0,
            failed: 0,
            current_path: None,
            current_study_uid: None,
            errors: Vec::new(),
        });
        state.snapshot.result = None;
        state.snapshot.failure_message = None;
        state.snapshot.cancellation_requested = false;
        state.cancel_token = Some(cancel_token.clone());
        Ok((summary, cancel_token))
    }

    pub(crate) async fn update_progress(&self, progress: ImportProgress) {
        let mut state = self.state.write().await;
        state.snapshot.progress = Some(progress);
    }

    pub(crate) async fn finish_import(&self, result: ImportResult) {
        let mut state = self.state.write().await;
        state.snapshot.phase = ImportPhase::Complete;
        state.snapshot.result = Some(result);
        state.snapshot.progress = None;
        state.snapshot.failure_message = None;
        state.snapshot.cancellation_requested = false;
        state.cancel_token = None;
    }

    pub(crate) async fn fail(&self, path_input: String, message: String) {
        let mut state = self.state.write().await;
        state.snapshot.phase = ImportPhase::Failed;
        state.snapshot.path_input = path_input;
        state.snapshot.progress = None;
        state.snapshot.result = None;
        state.snapshot.failure_message = Some(message);
        state.snapshot.cancellation_requested = false;
        state.cancel_token = None;
    }

    pub(crate) async fn request_cancel(&self) -> bool {
        let mut state = self.state.write().await;
        let Some(cancel_token) = state.cancel_token.clone() else {
            return false;
        };
        state.snapshot.cancellation_requested = true;
        cancel_token.cancel();
        true
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DirectoryListing {
    pub(crate) current_path: String,
    pub(crate) parent_path: Option<String>,
    pub(crate) entries: Vec<DirectoryEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct DirectoryEntry {
    pub(crate) name: String,
    pub(crate) path: String,
}

fn default_browse_root() -> &'static str {
    #[cfg(target_os = "macos")]
    {
        "/Users"
    }

    #[cfg(not(target_os = "macos"))]
    {
        "/"
    }
}

fn display_directory_path(path: &Path) -> String {
    #[cfg(target_os = "macos")]
    {
        const DATA_PREFIX: &str = "/System/Volumes/Data";
        const PRIVATE_PREFIX: &str = "/private";

        if let Ok(stripped) = path.strip_prefix(DATA_PREFIX) {
            let logical = if stripped.as_os_str().is_empty() {
                PathBuf::from("/")
            } else {
                Path::new("/").join(stripped)
            };
            if logical.exists() {
                return logical.to_string_lossy().to_string();
            }
        }

        if let Ok(stripped) = path.strip_prefix(PRIVATE_PREFIX) {
            if !stripped.as_os_str().is_empty() {
                let logical = Path::new("/").join(stripped);
                if logical.exists() {
                    return logical.to_string_lossy().to_string();
                }
            }
        }
    }

    path.to_string_lossy().to_string()
}

fn include_browser_directory(entry_name: &str, _path: &Path) -> bool {
    if entry_name.is_empty() {
        return false;
    }

    #[cfg(target_os = "macos")]
    {
        if _path.parent() == Some(Path::new("/")) && entry_name.starts_with('.') {
            return false;
        }
    }

    true
}

pub(crate) fn canonicalize_directory(raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    let requested = if trimmed.is_empty() {
        default_browse_root()
    } else {
        trimmed
    };
    let canonical = fs::canonicalize(requested)
        .map_err(|error| format!("failed to access '{}': {error}", requested))?;
    if !canonical.is_dir() {
        return Err(format!("'{}' is not a directory", canonical.display()));
    }
    Ok(canonical)
}

pub(crate) fn list_directory_entries(raw: &str) -> Result<DirectoryListing, String> {
    let canonical = canonicalize_directory(raw)?;
    let mut entries = fs::read_dir(&canonical)
        .map_err(|error| format!("failed to read '{}': {error}", canonical.display()))?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .filter(|_| include_browser_directory(&name, &path))
                .map(|_| DirectoryEntry {
                    name,
                    path: display_directory_path(&path),
                })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.name.cmp(&right.name));

    Ok(DirectoryListing {
        current_path: display_directory_path(&canonical),
        parent_path: canonical.parent().map(display_directory_path),
        entries,
    })
}

pub(crate) fn scan_directory(
    path: &Path,
    cancel_token: &CancellationToken,
) -> Result<ImportScanSummary, String> {
    let mut studies = BTreeMap::<String, StudyAccumulator>::new();
    let mut files = Vec::new();
    let mut total_bytes = 0_u64;
    let mut skipped_non_dicom = 0_usize;
    let mut unreadable_dicom = 0_usize;
    let mut errors = Vec::new();

    for entry in WalkDir::new(path).follow_links(false) {
        if cancel_token.is_cancelled() {
            return Err("scan cancelled by operator".into());
        }

        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                push_error(&mut errors, format!("walk error: {error}"));
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }

        let file_path = entry.path().to_path_buf();
        let mut prefix = [0_u8; 132];
        let prefix_len =
            match fs::File::open(&file_path).and_then(|mut file| file.read(&mut prefix)) {
                Ok(read) => read,
                Err(error) => {
                    unreadable_dicom += 1;
                    push_error(&mut errors, format!("{}: {error}", file_path.display()));
                    continue;
                }
            };

        if !looks_like_dicom_part10(&prefix[..prefix_len]) {
            skipped_non_dicom += 1;
            continue;
        }

        let bytes = match fs::read(&file_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                unreadable_dicom += 1;
                push_error(&mut errors, format!("{}: {error}", file_path.display()));
                continue;
            }
        };
        let size_bytes = bytes.len() as u64;
        let parsed = match ParsedDicom::from_bytes(Bytes::from(bytes)) {
            Ok(parsed) => parsed,
            Err(error) => {
                unreadable_dicom += 1;
                push_error(&mut errors, format!("{}: {error}", file_path.display()));
                continue;
            }
        };

        total_bytes += size_bytes;
        files.push(ScannedFile {
            path: file_path.clone(),
        });
        studies
            .entry(parsed.study.study_uid.to_string())
            .and_modify(|study| study.add(&parsed, size_bytes))
            .or_insert_with(|| StudyAccumulator::new(&parsed, size_bytes));
    }

    let studies = studies
        .into_iter()
        .map(|(study_uid, study)| ScannedStudySummary {
            study_uid,
            patient_name: study.patient_name,
            study_date: study.study_date,
            modalities: study.modalities.into_iter().collect(),
            num_series: study.series_uids.len(),
            num_instances: study.num_instances,
            total_bytes: study.total_bytes,
        })
        .collect::<Vec<_>>();
    let total_instances = files.len();

    Ok(ImportScanSummary {
        path: path.to_string_lossy().to_string(),
        studies,
        files,
        total_instances,
        total_bytes,
        skipped_non_dicom,
        unreadable_dicom,
        errors,
    })
}

pub(crate) fn push_error(errors: &mut Vec<String>, message: String) {
    if errors.len() < MAX_TRACKED_ERRORS {
        errors.push(message);
    }
}

struct StudyAccumulator {
    patient_name: Option<String>,
    study_date: Option<String>,
    modalities: BTreeSet<String>,
    series_uids: HashSet<String>,
    num_instances: usize,
    total_bytes: u64,
}

impl StudyAccumulator {
    fn new(parsed: &ParsedDicom, size_bytes: u64) -> Self {
        let mut study = Self {
            patient_name: parsed.study.patient_name.clone(),
            study_date: parsed
                .study
                .study_date
                .map(|value| value.format("%Y-%m-%d").to_string()),
            modalities: BTreeSet::new(),
            series_uids: HashSet::new(),
            num_instances: 0,
            total_bytes: 0,
        };
        study.add(parsed, size_bytes);
        study
    }

    fn add(&mut self, parsed: &ParsedDicom, size_bytes: u64) {
        if let Some(modality) = parsed
            .series
            .modality
            .as_deref()
            .filter(|value| !value.is_empty())
        {
            self.modalities.insert(modality.to_string());
        }
        self.series_uids
            .insert(parsed.series.series_uid.to_string());
        self.num_instances += 1;
        self.total_bytes += size_bytes;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dicom_toolkit_data::{DataSet, DicomWriter, FileFormat};
    use dicom_toolkit_dict::{tags, Vr};
    use std::{env, fs, io::Write};
    use uuid::Uuid;

    fn make_test_dir() -> PathBuf {
        let dir = env::temp_dir().join(format!("pacs-admin-import-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_test_dicom(path: &Path, study_uid: &str, series_uid: &str, instance_uid: &str) {
        let mut ds = DataSet::new();
        ds.set_string(tags::PATIENT_NAME, Vr::PN, "Scan^Patient");
        ds.set_string(tags::STUDY_INSTANCE_UID, Vr::UI, study_uid);
        ds.set_string(tags::SERIES_INSTANCE_UID, Vr::UI, series_uid);
        ds.set_string(tags::SOP_INSTANCE_UID, Vr::UI, instance_uid);
        ds.set_string(tags::SOP_CLASS_UID, Vr::UI, "1.2.840.10008.5.1.4.1.1.2");
        ds.set_string(tags::MODALITY, Vr::CS, "CT");
        let ff = FileFormat::from_dataset("1.2.840.10008.5.1.4.1.1.2", instance_uid, ds);
        let mut buf = Vec::new();
        DicomWriter::new(std::io::Cursor::new(&mut buf))
            .write_file(&ff)
            .unwrap();
        fs::write(path, buf).unwrap();
    }

    #[test]
    fn scan_directory_groups_valid_part10_files() {
        let dir = make_test_dir();
        make_test_dicom(&dir.join("one.dcm"), "1.2.3", "1.2.3.1", "1.2.3.1.1");
        make_test_dicom(&dir.join("two.dcm"), "1.2.3", "1.2.3.2", "1.2.3.2.1");
        let mut not_dicom = fs::File::create(dir.join("notes.txt")).unwrap();
        writeln!(not_dicom, "hello").unwrap();

        let summary = scan_directory(&dir, &CancellationToken::new()).unwrap();

        assert_eq!(summary.total_instances, 2);
        assert_eq!(summary.skipped_non_dicom, 1);
        assert_eq!(summary.unreadable_dicom, 0);
        assert_eq!(summary.studies.len(), 1);
        assert_eq!(summary.studies[0].num_series, 2);
        assert_eq!(summary.studies[0].num_instances, 2);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn canonicalize_directory_rejects_files() {
        let dir = make_test_dir();
        let file_path = dir.join("file.txt");
        fs::write(&file_path, b"hello").unwrap();

        let error = canonicalize_directory(file_path.to_string_lossy().as_ref()).unwrap_err();
        assert!(error.contains("is not a directory"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn list_directory_entries_uses_logical_current_path() {
        let dir = make_test_dir();

        let listing = list_directory_entries(dir.to_string_lossy().as_ref()).unwrap();

        assert_eq!(listing.current_path, dir.to_string_lossy());

        let _ = fs::remove_dir_all(dir);
    }
}
