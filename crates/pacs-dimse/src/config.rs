//! DIMSE server and client configuration.

/// Configuration for the DICOM SCP listener and SCU operations.
#[derive(Debug, Clone)]
pub struct DimseConfig {
    /// AE title for this PACS node.
    pub ae_title: String,
    /// TCP port on which the SCP listens.
    pub port: u16,
    /// Maximum number of concurrent associations.
    pub max_associations: usize,
    /// Timeout in seconds for DIMSE operations and association negotiation.
    pub timeout_secs: u64,
}

impl Default for DimseConfig {
    fn default() -> Self {
        Self {
            ae_title: "PACSNODE".into(),
            port: 4242,
            max_associations: 64,
            timeout_secs: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ae_title() {
        assert_eq!(DimseConfig::default().ae_title, "PACSNODE");
    }

    #[test]
    fn default_port() {
        assert_eq!(DimseConfig::default().port, 4242);
    }

    #[test]
    fn default_max_associations() {
        assert_eq!(DimseConfig::default().max_associations, 64);
    }

    #[test]
    fn default_timeout() {
        assert_eq!(DimseConfig::default().timeout_secs, 30);
    }

    #[test]
    fn clone_is_equal() {
        let cfg = DimseConfig {
            ae_title: "TEST".into(),
            port: 104,
            max_associations: 10,
            timeout_secs: 60,
        };
        let cloned = cfg.clone();
        assert_eq!(cfg.ae_title, cloned.ae_title);
        assert_eq!(cfg.port, cloned.port);
        assert_eq!(cfg.max_associations, cloned.max_associations);
        assert_eq!(cfg.timeout_secs, cloned.timeout_secs);
    }
}
