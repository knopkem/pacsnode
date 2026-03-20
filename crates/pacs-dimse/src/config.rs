//! DIMSE server and client configuration.

const DEFAULT_STORAGE_TRANSFER_SYNTAX_UID: &str = "1.2.840.10008.1.2.4.201";

/// Configuration for the DICOM SCP listener and SCU operations.
#[derive(Debug, Clone)]
pub struct DimseConfig {
    /// AE title for this PACS node.
    pub ae_title: String,
    /// TCP port on which the SCP listens.
    pub port: u16,
    /// Whether inbound calling AE titles must exist in the registered node list.
    pub ae_whitelist_enabled: bool,
    /// Whether the DIMSE SCP accepts any offered transfer syntax by default.
    pub accept_all_transfer_syntaxes: bool,
    /// Explicit DIMSE SCP transfer syntax allow-list.
    pub accepted_transfer_syntaxes: Vec<String>,
    /// Preferred DIMSE SCP transfer syntax order, highest priority first.
    pub preferred_transfer_syntaxes: Vec<String>,
    /// Optional transfer syntax to use when archiving newly ingested objects.
    pub storage_transfer_syntax: Option<String>,
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
            ae_whitelist_enabled: false,
            accept_all_transfer_syntaxes: true,
            accepted_transfer_syntaxes: Vec::new(),
            preferred_transfer_syntaxes: Vec::new(),
            storage_transfer_syntax: Some(DEFAULT_STORAGE_TRANSFER_SYNTAX_UID.into()),
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
    fn default_ae_whitelist_disabled() {
        assert!(!DimseConfig::default().ae_whitelist_enabled);
    }

    #[test]
    fn default_transfer_syntax_policy_accepts_all() {
        let cfg = DimseConfig::default();
        assert!(cfg.accept_all_transfer_syntaxes);
        assert!(cfg.accepted_transfer_syntaxes.is_empty());
        assert!(cfg.preferred_transfer_syntaxes.is_empty());
    }

    #[test]
    fn default_timeout() {
        assert_eq!(DimseConfig::default().timeout_secs, 30);
    }

    #[test]
    fn default_storage_transfer_syntax_is_htj2k_lossless() {
        assert_eq!(
            DimseConfig::default().storage_transfer_syntax.as_deref(),
            Some(DEFAULT_STORAGE_TRANSFER_SYNTAX_UID)
        );
    }

    #[test]
    fn clone_is_equal() {
        let cfg = DimseConfig {
            ae_title: "TEST".into(),
            port: 104,
            ae_whitelist_enabled: true,
            accept_all_transfer_syntaxes: false,
            accepted_transfer_syntaxes: vec!["1.2.840.10008.1.2.1".into()],
            preferred_transfer_syntaxes: vec!["1.2.840.10008.1.2.4.50".into()],
            storage_transfer_syntax: Some("1.2.840.10008.1.2.4.90".into()),
            max_associations: 10,
            timeout_secs: 60,
        };
        let cloned = cfg.clone();
        assert_eq!(cfg.ae_title, cloned.ae_title);
        assert_eq!(cfg.port, cloned.port);
        assert_eq!(cfg.ae_whitelist_enabled, cloned.ae_whitelist_enabled);
        assert_eq!(
            cfg.accept_all_transfer_syntaxes,
            cloned.accept_all_transfer_syntaxes
        );
        assert_eq!(
            cfg.accepted_transfer_syntaxes,
            cloned.accepted_transfer_syntaxes
        );
        assert_eq!(cfg.storage_transfer_syntax, cloned.storage_transfer_syntax);
        assert_eq!(
            cfg.preferred_transfer_syntaxes,
            cloned.preferred_transfer_syntaxes
        );
        assert_eq!(cfg.max_associations, cloned.max_associations);
        assert_eq!(cfg.timeout_secs, cloned.timeout_secs);
    }
}
