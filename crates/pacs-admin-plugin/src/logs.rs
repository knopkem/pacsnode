//! In-memory log buffer service for admin web interface.
//!
//! Provides thread-safe circular buffer to capture recent application logs
//! from tracing events alongside existing stdout/stderr output.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, OnceLock, RwLock,
    },
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{Level, Subscriber};
use tracing_subscriber::{layer::Context, registry::LookupSpan, Layer};

/// Default buffer capacity (number of log entries to keep in memory).
const DEFAULT_BUFFER_SIZE: usize = 10_000;

/// Maximum buffer capacity to prevent excessive memory usage.
const MAX_BUFFER_SIZE: usize = 100_000;

/// Global log buffer instance.
static GLOBAL_LOG_BUFFER: OnceLock<Arc<LogBuffer>> = OnceLock::new();

/// A single log entry captured from tracing events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// Monotonic sequence ID for ordering and deduplication.
    pub id: u64,
    /// When the log event occurred.
    pub timestamp: DateTime<Utc>,
    /// Log level (ERROR, WARN, INFO, DEBUG, TRACE).
    pub level: String,
    /// Module/target path where the log originated.
    pub target: String,
    /// Formatted log message.
    pub message: String,
    /// Structured fields from the log event.
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    pub fields: HashMap<String, serde_json::Value>,
    /// Optional span name if logged within a tracing span.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub span_name: Option<String>,
}

/// Configuration for the log buffer service.
#[derive(Debug, Clone)]
pub struct LogBufferConfig {
    /// Maximum number of log entries to retain.
    pub capacity: usize,
    /// Whether to enable web log capture.
    pub enabled: bool,
}

impl Default for LogBufferConfig {
    fn default() -> Self {
        Self {
            capacity: DEFAULT_BUFFER_SIZE,
            enabled: true,
        }
    }
}

impl LogBufferConfig {
    /// Creates config with validated capacity.
    pub fn new(capacity: usize, enabled: bool) -> Self {
        let capacity = capacity.clamp(1, MAX_BUFFER_SIZE);
        Self { capacity, enabled }
    }
}

/// Thread-safe circular buffer for storing recent log entries.
#[derive(Debug)]
pub struct LogBuffer {
    entries: RwLock<VecDeque<LogEntry>>,
    capacity: usize,
    counter: AtomicU64,
}

impl LogBuffer {
    /// Creates a new log buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.clamp(1, MAX_BUFFER_SIZE);
        Self {
            entries: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
            counter: AtomicU64::new(0),
        }
    }

    /// Adds a log entry to the buffer, removing oldest if at capacity.
    pub fn push(&self, entry: LogEntry) {
        let mut entries = self.entries.write().unwrap();

        if entries.len() >= self.capacity {
            entries.pop_front();
        }

        entries.push_back(entry);
    }

    /// Returns all log entries, newest first.
    pub fn entries(&self) -> Vec<LogEntry> {
        let entries = self.entries.read().unwrap();
        let mut result: Vec<_> = entries.iter().cloned().collect();
        result.reverse(); // newest first
        result
    }

    /// Returns filtered log entries matching the criteria.
    pub fn entries_filtered(&self, filter: &LogFilter) -> Vec<LogEntry> {
        let entries = self.entries.read().unwrap();
        let mut result: Vec<_> = entries
            .iter()
            .filter(|entry| filter.matches(entry))
            .cloned()
            .collect();
        result.reverse(); // newest first
        result
    }

    /// Returns the current number of entries in the buffer.
    pub fn len(&self) -> usize {
        self.entries.read().unwrap().len()
    }

    /// Returns whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.read().unwrap().is_empty()
    }

    /// Clears all entries from the buffer.
    pub fn clear(&self) {
        self.entries.write().unwrap().clear();
    }

    /// Gets the next sequence ID for a new log entry.
    fn next_id(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::Relaxed)
    }
}

/// Criteria for filtering log entries.
#[derive(Debug, Clone, Default)]
pub struct LogFilter {
    /// Filter by log level (if specified).
    pub level: Option<String>,
    /// Filter by target/module (if specified).
    pub target: Option<String>,
    /// Filter by text content in message (if specified).
    pub search: Option<String>,
    /// Filter by minimum timestamp (if specified).
    pub since: Option<DateTime<Utc>>,
    /// Maximum number of entries to return.
    pub limit: Option<usize>,
}

impl LogFilter {
    /// Returns true if the log entry matches this filter.
    pub fn matches(&self, entry: &LogEntry) -> bool {
        if let Some(ref level) = self.level {
            if !entry.level.eq_ignore_ascii_case(level) {
                return false;
            }
        }

        if let Some(ref target) = self.target {
            if !entry.target.contains(target) {
                return false;
            }
        }

        if let Some(ref search) = self.search {
            let search_lower = search.to_lowercase();
            if !entry.message.to_lowercase().contains(&search_lower)
                && !entry.target.to_lowercase().contains(&search_lower)
            {
                return false;
            }
        }

        if let Some(since) = self.since {
            if entry.timestamp < since {
                return false;
            }
        }

        true
    }
}

/// Tracing subscriber layer that captures log events into a buffer.
pub struct LogBufferLayer {
    buffer: Arc<LogBuffer>,
}

impl LogBufferLayer {
    /// Creates a new log buffer layer.
    pub fn new(buffer: Arc<LogBuffer>) -> Self {
        Self { buffer }
    }
}

impl<S> Layer<S> for LogBufferLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = metadata.level();
        let target = metadata.target();

        // Skip debug/trace logs by default to avoid buffer spam
        if matches!(level, &Level::DEBUG | &Level::TRACE) {
            return;
        }

        let mut message = String::new();
        let mut fields = HashMap::new();

        // Visitor to extract message and structured fields
        struct FieldVisitor<'a> {
            message: &'a mut String,
            fields: &'a mut HashMap<String, serde_json::Value>,
        }

        impl tracing::field::Visit for FieldVisitor<'_> {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                let field_name = field.name();
                let field_value = format!("{:?}", value);

                if field_name == "message" {
                    *self.message = field_value;
                } else {
                    self.fields.insert(
                        field_name.to_string(),
                        serde_json::Value::String(field_value),
                    );
                }
            }
        }

        let mut visitor = FieldVisitor {
            message: &mut message,
            fields: &mut fields,
        };
        event.record(&mut visitor);

        // Get span name if we're in a span
        let span_name = _ctx
            .lookup_current()
            .map(|span| span.metadata().name().to_string());

        let entry = LogEntry {
            id: self.buffer.next_id(),
            timestamp: Utc::now(),
            level: level.to_string().to_uppercase(),
            target: target.to_string(),
            message,
            fields,
            span_name,
        };

        self.buffer.push(entry);
    }
}

/// Shared log buffer service that can be accessed from web routes.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LogBufferService {
    buffer: Arc<LogBuffer>,
    config: LogBufferConfig,
}

#[allow(dead_code)]
impl LogBufferService {
    /// Creates a new log buffer service.
    pub fn new(config: LogBufferConfig) -> Self {
        Self {
            buffer: Arc::new(LogBuffer::new(config.capacity)),
            config,
        }
    }

    /// Returns whether log capture is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Gets the underlying buffer.
    pub fn buffer(&self) -> Arc<LogBuffer> {
        Arc::clone(&self.buffer)
    }

    /// Gets all log entries, newest first.
    pub fn entries(&self) -> Vec<LogEntry> {
        self.buffer.entries()
    }

    /// Gets filtered log entries.
    pub fn entries_filtered(&self, filter: &LogFilter) -> Vec<LogEntry> {
        let mut entries = self.buffer.entries_filtered(filter);

        // Apply limit after filtering
        if let Some(limit) = filter.limit {
            entries.truncate(limit);
        }

        entries
    }

    /// Returns buffer statistics.
    pub fn stats(&self) -> LogBufferStats {
        LogBufferStats {
            total_entries: self.buffer.len(),
            capacity: self.config.capacity,
            enabled: self.config.enabled,
        }
    }

    /// Clears all entries from the buffer.
    pub fn clear(&self) {
        self.buffer.clear();
    }

    /// Creates a tracing layer that writes to this buffer.
    #[allow(dead_code)]
    pub fn layer(&self) -> LogBufferLayer {
        LogBufferLayer::new(Arc::clone(&self.buffer))
    }
}

/// Statistics about the log buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct LogBufferStats {
    /// Current number of entries in the buffer.
    pub total_entries: usize,
    /// Maximum capacity of the buffer.
    pub capacity: usize,
    /// Whether log capture is enabled.
    pub enabled: bool,
}

/// Initializes the global log buffer with the specified configuration.
/// This should be called early in the application startup, before tracing is initialized.
/// Returns true if the buffer was initialized, false if it was already initialized.
pub fn init_global_log_buffer(config: LogBufferConfig) -> bool {
    if !config.enabled {
        return false;
    }

    let buffer = Arc::new(LogBuffer::new(config.capacity));
    GLOBAL_LOG_BUFFER.set(buffer).is_ok()
}

/// Gets the global log buffer if it has been initialized.
pub fn global_log_buffer() -> Option<Arc<LogBuffer>> {
    GLOBAL_LOG_BUFFER.get().map(Arc::clone)
}

/// Creates a tracing layer that writes to the global log buffer, if initialized.
pub fn global_log_buffer_layer() -> Option<LogBufferLayer> {
    global_log_buffer().map(LogBufferLayer::new)
}

/// Gets all log entries from the global buffer, if available.
#[allow(dead_code)]
pub fn global_log_entries() -> Vec<LogEntry> {
    global_log_buffer()
        .map(|buffer| buffer.entries())
        .unwrap_or_default()
}

/// Gets filtered log entries from the global buffer, if available.
pub fn global_log_entries_filtered(filter: &LogFilter) -> Vec<LogEntry> {
    global_log_buffer()
        .map(|buffer| {
            let mut entries = buffer.entries_filtered(filter);
            if let Some(limit) = filter.limit {
                entries.truncate(limit);
            }
            entries
        })
        .unwrap_or_default()
}

/// Gets statistics about the global log buffer.
#[allow(dead_code)]
pub fn global_log_buffer_stats() -> Option<LogBufferStats> {
    global_log_buffer().map(|buffer| LogBufferStats {
        total_entries: buffer.len(),
        capacity: buffer.capacity,
        enabled: true,
    })
}

/// Clears all entries from the global log buffer, if available.
pub fn clear_global_log_buffer() {
    if let Some(buffer) = global_log_buffer() {
        buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_respects_capacity() {
        let buffer = LogBuffer::new(3);

        // Add entries up to capacity
        for i in 0..5 {
            buffer.push(LogEntry {
                id: i,
                timestamp: Utc::now(),
                level: "INFO".to_string(),
                target: "test".to_string(),
                message: format!("Message {}", i),
                fields: HashMap::new(),
                span_name: None,
            });
        }

        // Should only keep the last 3 entries
        assert_eq!(buffer.len(), 3);
        let entries = buffer.entries();
        assert_eq!(entries[0].id, 4); // newest first
        assert_eq!(entries[2].id, 2); // oldest kept
    }

    #[test]
    fn log_filter_matches_correctly() {
        let entry = LogEntry {
            id: 1,
            timestamp: Utc::now(),
            level: "ERROR".to_string(),
            target: "pacs_server::main".to_string(),
            message: "Connection failed".to_string(),
            fields: HashMap::new(),
            span_name: None,
        };

        // Level filter
        let filter = LogFilter {
            level: Some("ERROR".to_string()),
            ..Default::default()
        };
        assert!(filter.matches(&entry));

        let filter = LogFilter {
            level: Some("INFO".to_string()),
            ..Default::default()
        };
        assert!(!filter.matches(&entry));

        // Target filter
        let filter = LogFilter {
            target: Some("pacs_server".to_string()),
            ..Default::default()
        };
        assert!(filter.matches(&entry));

        // Search filter
        let filter = LogFilter {
            search: Some("Connection".to_string()),
            ..Default::default()
        };
        assert!(filter.matches(&entry));
    }

    #[test]
    fn log_buffer_config_validates_capacity() {
        let config = LogBufferConfig::new(MAX_BUFFER_SIZE + 1, true);
        assert_eq!(config.capacity, MAX_BUFFER_SIZE);

        let config = LogBufferConfig::new(0, true);
        assert_eq!(config.capacity, 1);
    }
}
