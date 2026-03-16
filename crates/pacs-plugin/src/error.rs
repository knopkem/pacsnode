use thiserror::Error;

/// Errors returned by the pacsnode plugin system.
#[derive(Debug, Error)]
pub enum PluginError {
    /// A plugin configuration value was missing or invalid.
    #[error("plugin config error ({plugin_id}): {message}")]
    Config {
        /// Plugin identifier.
        plugin_id: String,
        /// Human-readable error message.
        message: String,
    },

    /// A plugin dependency was missing.
    #[error("missing dependency: plugin '{plugin_id}' requires '{dependency}'")]
    MissingDependency {
        /// Plugin identifier.
        plugin_id: String,
        /// Missing dependency plugin ID.
        dependency: String,
    },

    /// A circular dependency was detected between plugins.
    #[error("circular plugin dependency: {cycle}")]
    CircularDependency {
        /// Human-readable cycle description.
        cycle: String,
    },

    /// Two plugins tried to register the same capability singleton.
    #[error("duplicate {capability} provider: '{first}' and '{second}'")]
    DuplicateProvider {
        /// Capability name.
        capability: String,
        /// First plugin ID.
        first: String,
        /// Second plugin ID.
        second: String,
    },

    /// Two plugins used the same plugin identifier.
    #[error("duplicate plugin id: {id}")]
    DuplicatePluginId {
        /// Duplicate plugin ID.
        id: String,
    },

    /// A plugin capability was accessed before initialization.
    #[error("plugin '{plugin_id}' capability '{capability}' is not initialized")]
    NotInitialized {
        /// Plugin identifier.
        plugin_id: String,
        /// Capability name.
        capability: String,
    },

    /// Plugin initialization failed with an underlying source error.
    #[error("plugin init failed ({plugin_id}): {source}")]
    InitFailed {
        /// Plugin identifier.
        plugin_id: String,
        /// Underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// Generic runtime plugin error.
    #[error("plugin runtime error ({plugin_id}): {message}")]
    Runtime {
        /// Plugin identifier.
        plugin_id: String,
        /// Human-readable error message.
        message: String,
    },
}
