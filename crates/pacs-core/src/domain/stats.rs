use serde::{Deserialize, Serialize};

/// System-wide PACS statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PacsStatistics {
    /// Total number of studies stored.
    pub num_studies: i64,
    /// Total number of series stored.
    pub num_series: i64,
    /// Total number of instances stored.
    pub num_instances: i64,
    /// Total raw disk usage across all blobs, in bytes.
    pub disk_usage_bytes: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_serde_roundtrip() {
        let stats = PacsStatistics {
            num_studies: 10,
            num_series: 50,
            num_instances: 500,
            disk_usage_bytes: 1_073_741_824,
        };
        let json = serde_json::to_string(&stats).unwrap();
        let back: PacsStatistics = serde_json::from_str(&json).unwrap();
        assert_eq!(back.num_studies, 10);
        assert_eq!(back.num_series, 50);
        assert_eq!(back.num_instances, 500);
        assert_eq!(back.disk_usage_bytes, 1_073_741_824);
    }

    #[test]
    fn test_clone() {
        let s = PacsStatistics {
            num_studies: 1,
            num_series: 2,
            num_instances: 3,
            disk_usage_bytes: 4,
        };
        let c = s.clone();
        assert_eq!(c.num_studies, 1);
        assert_eq!(c.disk_usage_bytes, 4);
    }
}
