use std::net::SocketAddr;

use tokio::sync::broadcast;

use crate::capabilities::EventKind;

/// Indicates where a query originated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuerySource {
    /// Query came from DIMSE.
    Dimse {
        /// Calling AE title.
        calling_ae: String,
    },
    /// Query came from DICOMweb.
    Dicomweb,
}

/// DICOM resource hierarchy level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceLevel {
    /// Patient resource.
    Patient,
    /// Study resource.
    Study,
    /// Series resource.
    Series,
    /// Instance resource.
    Instance,
}

/// Event emitted by pacsnode runtime code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PacsEvent {
    /// A new instance was stored.
    InstanceStored {
        /// Study Instance UID.
        study_uid: String,
        /// Series Instance UID.
        series_uid: String,
        /// SOP Instance UID.
        sop_instance_uid: String,
        /// SOP Class UID.
        sop_class_uid: String,
        /// Calling AE title or transport source.
        source: String,
        /// Authenticated user ID for HTTP-originated requests, if available.
        user_id: Option<String>,
    },
    /// A study was completely received.
    StudyComplete {
        /// Study Instance UID.
        study_uid: String,
    },
    /// A resource was deleted.
    ResourceDeleted {
        /// Resource hierarchy level.
        level: ResourceLevel,
        /// Deleted resource UID.
        uid: String,
        /// Authenticated user ID, if available.
        user_id: Option<String>,
    },
    /// A DIMSE association was opened.
    AssociationOpened {
        /// Calling AE title.
        calling_ae: String,
        /// Peer socket address.
        peer_addr: SocketAddr,
    },
    /// A DIMSE association was closed.
    AssociationClosed {
        /// Calling AE title.
        calling_ae: String,
    },
    /// A query was executed.
    QueryPerformed {
        /// Query level, such as `STUDY`, `SERIES`, or `IMAGE`.
        level: String,
        /// Query source transport.
        source: QuerySource,
        /// Number of results returned.
        num_results: usize,
        /// Authenticated user ID for HTTP-originated requests, if available.
        user_id: Option<String>,
    },
}

impl PacsEvent {
    /// Returns the event kind used for subscriber routing.
    pub fn kind(&self) -> EventKind {
        match self {
            Self::InstanceStored { .. } => EventKind::InstanceStored,
            Self::StudyComplete { .. } => EventKind::StudyComplete,
            Self::ResourceDeleted { .. } => EventKind::ResourceDeleted,
            Self::AssociationOpened { .. } => EventKind::AssociationOpened,
            Self::AssociationClosed { .. } => EventKind::AssociationClosed,
            Self::QueryPerformed { .. } => EventKind::QueryPerformed,
        }
    }
}

/// Broadcast event bus shared across the process.
pub struct EventBus {
    tx: broadcast::Sender<PacsEvent>,
}

impl EventBus {
    /// Creates a new event bus with the given capacity.
    ///
    /// # Example
    ///
    /// ```rust
    /// use pacs_plugin::EventBus;
    ///
    /// let bus = EventBus::new(16);
    /// let _rx = bus.subscribe();
    /// ```
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Emits an event to all subscribers and returns the subscriber count.
    pub fn emit(&self, event: PacsEvent) -> usize {
        self.tx.send(event).unwrap_or_default()
    }

    /// Creates a new subscription receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<PacsEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn event_bus_round_trip() {
        let bus = EventBus::new(4);
        let mut rx = bus.subscribe();

        bus.emit(PacsEvent::StudyComplete {
            study_uid: "1.2.3".into(),
        });

        let event = rx.recv().await.unwrap();
        assert_eq!(
            event,
            PacsEvent::StudyComplete {
                study_uid: "1.2.3".into(),
            }
        );
    }
}
