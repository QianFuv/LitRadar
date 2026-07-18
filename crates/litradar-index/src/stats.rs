//! In-memory aggregate metrics for structured index terminal events.

use serde::{Deserialize, Serialize};

use crate::schema::ContentWriteOutcome;

/// Safe aggregate counters carried between index workers and emitted once per run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct IndexRunMetrics {
    /// Catalog journals selected for this worker or run.
    pub journals_total: usize,
    /// Journals that completed successfully.
    pub journals_succeeded: usize,
    /// Journals that were skipped from a completed checkpoint.
    pub journals_resumed: usize,
    /// Journals that failed.
    pub journals_failed: usize,
    /// Canonical provider pages committed.
    pub pages_committed: usize,
    /// Canonical article drafts examined.
    pub articles_seen: usize,
    /// New or changed canonical article rows.
    pub articles_changed: usize,
    /// New canonical identity aliases attached.
    pub identity_aliases_added: usize,
    /// Provider-neutral outbox events emitted.
    pub change_events_emitted: usize,
}

impl IndexRunMetrics {
    /// Record one committed canonical content page.
    ///
    /// # Arguments
    ///
    /// * `outcome` - Common writer result.
    pub fn record_write(&mut self, outcome: ContentWriteOutcome) {
        self.pages_committed += 1;
        self.articles_seen += outcome.articles_seen;
        self.articles_changed += outcome.articles_changed;
        self.identity_aliases_added += outcome.identity_aliases_added;
        self.change_events_emitted += outcome.change_events_emitted;
    }

    /// Merge one worker's safe aggregate counters.
    ///
    /// # Arguments
    ///
    /// * `worker` - Completed worker metrics.
    pub fn merge(&mut self, worker: &Self) {
        self.journals_total += worker.journals_total;
        self.journals_succeeded += worker.journals_succeeded;
        self.journals_resumed += worker.journals_resumed;
        self.journals_failed += worker.journals_failed;
        self.pages_committed += worker.pages_committed;
        self.articles_seen += worker.articles_seen;
        self.articles_changed += worker.articles_changed;
        self.identity_aliases_added += worker.identity_aliases_added;
        self.change_events_emitted += worker.change_events_emitted;
    }

    /// Emit one terminal structured event without persisting observability rows.
    ///
    /// # Arguments
    ///
    /// * `run_id` - Core-owned correlation identifier.
    /// * `catalog_name` - Stable catalog stem.
    /// * `provider_name` - Runtime indexing route.
    /// * `worker_id` - Worker identifier or `all` for the parent aggregate.
    /// * `outcome` - Terminal outcome label.
    pub fn emit_terminal(
        &self,
        run_id: &str,
        catalog_name: &str,
        provider_name: &str,
        worker_id: &str,
        outcome: &str,
    ) {
        tracing::info!(
            event = "index.run.completed",
            component = "index",
            run_id,
            catalog = catalog_name,
            provider = provider_name,
            worker_id,
            outcome,
            journals_total = self.journals_total,
            journals_succeeded = self.journals_succeeded,
            journals_resumed = self.journals_resumed,
            journals_failed = self.journals_failed,
            pages_committed = self.pages_committed,
            articles_seen = self.articles_seen,
            articles_changed = self.articles_changed,
            identity_aliases_added = self.identity_aliases_added,
            change_events_emitted = self.change_events_emitted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::IndexRunMetrics;
    use crate::schema::ContentWriteOutcome;

    #[test]
    fn worker_metrics_merge_without_persistent_or_request_fields() {
        let mut worker = IndexRunMetrics {
            journals_total: 1,
            journals_succeeded: 1,
            ..IndexRunMetrics::default()
        };
        worker.record_write(ContentWriteOutcome {
            articles_seen: 2,
            articles_changed: 1,
            identity_aliases_added: 1,
            change_events_emitted: 1,
        });
        let mut parent = IndexRunMetrics::default();
        parent.merge(&worker);
        assert_eq!(parent.journals_total, 1);
        assert_eq!(parent.pages_committed, 1);
        assert_eq!(parent.articles_seen, 2);
    }
}
