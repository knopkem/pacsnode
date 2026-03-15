use serde::{Deserialize, Serialize};

use crate::error::{PacsError, PacsResult};

/// DICOM JSON representation (PS3.18) — a thin wrapper around [`serde_json::Value`].
///
/// # Examples
///
/// ```
/// use pacs_core::DicomJson;
/// use serde_json::json;
///
/// let dj = DicomJson::from(json!({"00080060": {"vr": "CS", "Value": ["CT"]}}));
/// assert!(!dj.to_json_string().is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DicomJson(pub serde_json::Value);

impl DicomJson {
    /// Creates an empty DICOM JSON object (`{}`).
    ///
    /// # Examples
    ///
    /// ```
    /// use pacs_core::DicomJson;
    ///
    /// let dj = DicomJson::empty();
    /// assert_eq!(dj.to_json_string(), "{}");
    /// ```
    pub fn empty() -> Self {
        Self(serde_json::Value::Object(Default::default()))
    }

    /// Returns a reference to the underlying [`serde_json::Value`].
    pub fn as_value(&self) -> &serde_json::Value {
        &self.0
    }

    /// Returns the inner JSON string representation.
    pub fn to_json_string(&self) -> String {
        self.0.to_string()
    }
}

impl From<serde_json::Value> for DicomJson {
    fn from(value: serde_json::Value) -> Self {
        Self(value)
    }
}

impl TryFrom<&str> for DicomJson {
    type Error = PacsError;

    /// Parses a DICOM JSON string into a [`DicomJson`].
    ///
    /// # Errors
    ///
    /// Returns [`PacsError::DicomParse`] if the string is not valid JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// use pacs_core::DicomJson;
    ///
    /// let dj = DicomJson::try_from(r#"{"00080060":{"vr":"CS"}}"#).unwrap();
    /// assert!(!dj.to_json_string().is_empty());
    /// ```
    fn try_from(s: &str) -> PacsResult<Self> {
        serde_json::from_str(s)
            .map(DicomJson)
            .map_err(|e| PacsError::DicomParse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_empty_is_empty_object() {
        let dj = DicomJson::empty();
        assert_eq!(dj.to_json_string(), "{}");
    }

    #[test]
    fn test_from_value_roundtrip() {
        let val = json!({"00080060": {"vr": "CS", "Value": ["CT"]}});
        let dj = DicomJson::from(val.clone());
        assert_eq!(dj.as_value(), &val);
    }

    #[test]
    fn test_try_from_str_valid() {
        let s = r#"{"value":42}"#;
        let dj = DicomJson::try_from(s).unwrap();
        assert_eq!(dj.as_value(), &json!({"value": 42}));
    }

    #[test]
    fn test_try_from_str_invalid() {
        let result = DicomJson::try_from("not valid json at all");
        assert!(matches!(result, Err(PacsError::DicomParse(_))));
    }

    #[test]
    fn test_serde_roundtrip() {
        let dj = DicomJson::from(json!({
            "00100010": {"vr": "PN", "Value": [{"Alphabetic": "DOE^JOHN"}]}
        }));
        let serialized = serde_json::to_string(&dj).unwrap();
        let deserialized: DicomJson = serde_json::from_str(&serialized).unwrap();
        assert_eq!(dj, deserialized);
    }

    #[test]
    fn test_to_json_string_contains_key() {
        let dj = DicomJson::from(json!({"mykey": "myvalue"}));
        let s = dj.to_json_string();
        assert!(s.contains("\"mykey\""));
        assert!(s.contains("\"myvalue\""));
    }

    #[test]
    fn test_clone_equality() {
        let dj = DicomJson::from(json!({"a": 1}));
        assert_eq!(dj, dj.clone());
    }
}
