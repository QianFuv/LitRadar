//! Private acknowledged protocol between live index workers and the parent writer.

use std::error::Error;
use std::fmt;
use std::io::{Read, Write};

use litradar_domain::{JournalCatalogEntry, ProviderBatch};
use litradar_sources::LiveScholarlyConfig;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Current private worker protocol version.
pub(crate) const PROTOCOL_VERSION: u32 = 1;

/// One journal and optional resume cursor assigned to a fetch worker.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerJournalAssignment {
    /// Stable ordinal in the selected catalog.
    pub(crate) journal_ordinal: usize,
    /// Provider-free maintained journal contract.
    pub(crate) entry: JournalCatalogEntry,
    /// Opaque provider cursor read by the parent before spawning workers.
    pub(crate) initial_checkpoint: Option<String>,
}

/// Versioned fetch-only worker request persisted by the parent.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerRequest {
    /// Private protocol version expected by both processes.
    pub(crate) protocol_version: u32,
    /// Stable catalog stem used only for safe terminal correlation.
    pub(crate) catalog_name: String,
    /// Registered indexing provider name.
    pub(crate) provider_name: String,
    /// Core-owned run identifier.
    pub(crate) run_id: String,
    /// Zero-based journal worker identifier.
    pub(crate) worker_id: usize,
    /// Actual number of journal worker processes.
    pub(crate) process_count: usize,
    /// Number of bounded provider-side source workers.
    pub(crate) source_worker_count: usize,
    /// Common provider scheduling epoch.
    pub(crate) schedule_epoch_unix_millis: u64,
    /// Provider request timeout in seconds.
    pub(crate) timeout_seconds: u64,
    /// Scholarly provider runtime configuration.
    pub(crate) scholarly_config: LiveScholarlyConfig,
    /// Ordered journal assignments owned by this worker.
    pub(crate) assignments: Vec<WorkerJournalAssignment>,
}

impl fmt::Debug for WorkerRequest {
    /// Format a worker request without exposing provider credentials or cursors.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WorkerRequest")
            .field("protocol_version", &self.protocol_version)
            .field("catalog_name", &self.catalog_name)
            .field("provider_name", &self.provider_name)
            .field("run_id", &self.run_id)
            .field("worker_id", &self.worker_id)
            .field("process_count", &self.process_count)
            .field("source_worker_count", &self.source_worker_count)
            .field(
                "schedule_epoch_unix_millis",
                &self.schedule_epoch_unix_millis,
            )
            .field("timeout_seconds", &self.timeout_seconds)
            .field("assignment_count", &self.assignments.len())
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

/// Safe worker failure class sent across the process boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerFailureClass {
    /// Filesystem operation failed.
    Io,
    /// JSON or protocol serialization failed.
    Json,
    /// Catalog parsing failed.
    Catalog,
    /// SQLite returned a typed error.
    Sqlite,
    /// Canonical content validation or persistence failed.
    Content,
    /// Disposable control persistence failed.
    Control,
    /// Provider registry setup failed.
    Registry,
    /// Provider transport construction failed.
    ProviderSetup,
    /// Provider request or parsing failed.
    Provider,
    /// Runtime configuration or protocol data was invalid.
    InvalidConfig,
    /// Worker process or pipe supervision failed.
    Worker,
    /// Notification handoff failed.
    Notify,
    /// Provider lease heartbeat failed.
    Heartbeat,
}

impl WorkerFailureClass {
    /// Return the fixed event value for this failure class.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Io => "io",
            Self::Json => "json",
            Self::Catalog => "catalog",
            Self::Sqlite => "sqlite",
            Self::Content => "content",
            Self::Control => "control",
            Self::Registry => "registry",
            Self::ProviderSetup => "provider_setup",
            Self::Provider => "provider",
            Self::InvalidConfig => "invalid_config",
            Self::Worker => "worker",
            Self::Notify => "notify",
            Self::Heartbeat => "heartbeat",
        }
    }
}

/// Safe worker operation sent across the process boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkerOperation {
    /// Worker request file access.
    FileSystem,
    /// Worker JSON or protocol serialization.
    WorkerJson,
    /// Catalog parsing.
    CatalogRead,
    /// Content database initialization or opening.
    ContentDatabaseOpen,
    /// Canonical content transaction.
    ContentCommit,
    /// Disposable checkpoint transaction.
    CheckpointCommit,
    /// Disposable control database operation.
    ControlDatabase,
    /// Provider lease heartbeat.
    Heartbeat,
    /// Provider registry construction.
    ProviderRegistry,
    /// Provider transport setup.
    ProviderSetup,
    /// Provider request, parsing, or enrichment.
    ProviderRequest,
    /// Runtime or protocol configuration.
    Configuration,
    /// Worker process lifecycle.
    WorkerProcess,
    /// Worker bidirectional protocol I/O.
    WorkerProtocol,
    /// Notification handoff.
    Notification,
}

impl WorkerOperation {
    /// Return the fixed event value for this worker operation.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::FileSystem => "file_system",
            Self::WorkerJson => "worker_json",
            Self::CatalogRead => "catalog_read",
            Self::ContentDatabaseOpen => "content_database_open",
            Self::ContentCommit => "content_commit",
            Self::CheckpointCommit => "checkpoint_commit",
            Self::ControlDatabase => "control_database",
            Self::Heartbeat => "heartbeat",
            Self::ProviderRegistry => "provider_registry",
            Self::ProviderSetup => "provider_setup",
            Self::ProviderRequest => "provider_request",
            Self::Configuration => "configuration",
            Self::WorkerProcess => "worker_process",
            Self::WorkerProtocol => "worker_protocol",
            Self::Notification => "notification",
        }
    }
}

/// Redacted structured failure retained across the worker boundary.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkerFailure {
    /// Stable failure class.
    pub(crate) class: WorkerFailureClass,
    /// Stable operation boundary.
    pub(crate) operation: WorkerOperation,
    /// Optional typed SQLite result code.
    pub(crate) sqlite_code: Option<String>,
    /// Optional SQLite extended result code.
    pub(crate) sqlite_extended_code: Option<i32>,
    /// Whether SQLite classified the failure as busy or locked.
    pub(crate) is_busy_or_locked: bool,
}

impl WorkerFailure {
    /// Build one non-SQLite fixed failure classification.
    pub(crate) fn fixed(class: WorkerFailureClass, operation: WorkerOperation) -> Self {
        Self {
            class,
            operation,
            sqlite_code: None,
            sqlite_extended_code: None,
            is_busy_or_locked: false,
        }
    }
}

/// One child-to-parent streamed protocol message.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WorkerMessage {
    /// One canonical provider page awaiting durable parent acknowledgement.
    Batch {
        /// Private protocol version.
        protocol_version: u32,
        /// Worker that fetched this page.
        worker_id: usize,
        /// Monotonic sequence within the worker stream.
        sequence: u64,
        /// Catalog ordinal assigned by the parent.
        journal_ordinal: usize,
        /// Zero-based provider page index for this journal.
        page_index: usize,
        /// Canonical provider page.
        batch: ProviderBatch,
    },
    /// Worker completed every assigned journal after final acknowledgements.
    Succeeded {
        /// Private protocol version.
        protocol_version: u32,
        /// Completed worker identifier.
        worker_id: usize,
        /// Next expected monotonic sequence.
        sequence: u64,
    },
    /// Worker stopped after a redacted provider or protocol failure.
    Failed {
        /// Private protocol version.
        protocol_version: u32,
        /// Failed worker identifier.
        worker_id: usize,
        /// Next expected monotonic sequence.
        sequence: u64,
        /// Redacted structured failure.
        failure: WorkerFailure,
    },
}

/// One parent-to-child durable commit acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum ParentMessage {
    /// The exact batch committed content before its checkpoint.
    Committed {
        /// Private protocol version.
        protocol_version: u32,
        /// Worker receiving this acknowledgement.
        worker_id: usize,
        /// Acknowledged worker sequence.
        sequence: u64,
        /// Acknowledged catalog ordinal.
        journal_ordinal: usize,
        /// Acknowledged provider page index.
        page_index: usize,
        /// Whether the acknowledged page completed its journal.
        is_complete: bool,
    },
}

/// Internal worker protocol framing failure.
#[derive(Debug)]
pub(crate) enum ProtocolError {
    /// Underlying pipe I/O failed.
    Io(std::io::Error),
    /// One JSON value was invalid.
    Json(serde_json::Error),
    /// The peer closed its stream before another complete value.
    EndOfStream,
}

impl fmt::Display for ProtocolError {
    /// Format a fixed protocol failure without payload content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(_) => formatter.write_str("worker protocol I/O failed"),
            Self::Json(_) => formatter.write_str("worker protocol JSON failed"),
            Self::EndOfStream => formatter.write_str("worker protocol stream ended"),
        }
    }
}

impl Error for ProtocolError {
    /// Return the typed framing failure.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::EndOfStream => None,
        }
    }
}

impl From<std::io::Error> for ProtocolError {
    /// Convert pipe I/O failures.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Serialize and flush one self-delimiting JSON protocol value.
///
/// # Arguments
///
/// * `writer` - Buffered child stdin or stdout writer.
/// * `message` - Versioned protocol value.
///
/// # Returns
///
/// Success after the value and its separator are flushed.
pub(crate) fn write_message<Message: Serialize>(
    writer: &mut impl Write,
    message: &Message,
) -> Result<(), ProtocolError> {
    serde_json::to_writer(&mut *writer, message).map_err(ProtocolError::Json)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    Ok(())
}

/// Deserialize one self-delimiting JSON protocol value from a stream.
///
/// # Arguments
///
/// * `reader` - Buffered parent or child pipe reader positioned before one value.
///
/// # Returns
///
/// The next complete value, or an explicit end-of-stream failure.
pub(crate) fn read_message<Message: DeserializeOwned>(
    reader: &mut impl Read,
) -> Result<Message, ProtocolError> {
    let mut deserializer = serde_json::Deserializer::from_reader(reader);
    Message::deserialize(&mut deserializer).map_err(|error| {
        if error.is_eof() {
            ProtocolError::EndOfStream
        } else {
            ProtocolError::Json(error)
        }
    })
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};

    use super::{read_message, write_message, ParentMessage, PROTOCOL_VERSION};

    #[test]
    fn worker_protocol_round_trips_one_acknowledgement() {
        let message = ParentMessage::Committed {
            protocol_version: PROTOCOL_VERSION,
            worker_id: 2,
            sequence: 7,
            journal_ordinal: 3,
            page_index: 11,
            is_complete: false,
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).expect("acknowledgement should serialize");
        let mut reader = BufReader::new(Cursor::new(bytes));
        let decoded: ParentMessage =
            read_message(&mut reader).expect("acknowledgement should deserialize");

        assert_eq!(decoded, message);
    }

    #[test]
    fn worker_protocol_rejects_unknown_acknowledgement_fields() {
        let payload = br#"{"type":"committed","protocol_version":1,"worker_id":0,"sequence":0,"journal_ordinal":0,"page_index":0,"is_complete":false,"unexpected":true}"#;
        let mut reader = Cursor::new(payload);
        let error = read_message::<ParentMessage>(&mut reader)
            .expect_err("unknown protocol fields should fail closed");

        assert!(matches!(error, super::ProtocolError::Json(_)));
    }

    #[test]
    fn worker_protocol_reads_consecutive_json_values_without_losing_bytes() {
        let messages = [
            ParentMessage::Committed {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence: 0,
                journal_ordinal: 1,
                page_index: 0,
                is_complete: false,
            },
            ParentMessage::Committed {
                protocol_version: PROTOCOL_VERSION,
                worker_id: 0,
                sequence: 1,
                journal_ordinal: 1,
                page_index: 1,
                is_complete: true,
            },
        ];
        let mut bytes = Vec::new();
        for message in &messages {
            write_message(&mut bytes, message).expect("message should serialize");
        }
        let mut reader = BufReader::new(Cursor::new(bytes));

        for expected in messages {
            let actual: ParentMessage =
                read_message(&mut reader).expect("message should deserialize");
            assert_eq!(actual, expected);
        }
    }
}
