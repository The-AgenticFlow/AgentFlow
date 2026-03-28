/// Event-Driven File System Watcher for AgentFlow Pair Harness
///
/// Monitors .sprintless/pairs/pair-N/shared/ for file changes that trigger
/// SENTINEL spawning and pair termination.
///
/// # Validation: C3-01 No Polling
/// Uses notify crate (inotify/FSEvents) - ZERO polling loops
use anyhow::{Context, Result};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;
use tracing::{debug, error, info, warn};

/// Events emitted by the file watcher
#[derive(Debug, Clone)]
pub enum WatchEvent {
    /// WORKLOG.md was modified - trigger SENTINEL evaluation
    WorklogModified,
    /// STATUS.json was created or modified - check for termination
    StatusChanged,
    /// PLAN.md was modified - initial plan ready for review
    PlanModified,
    /// HANDOFF.md was created - context reset requested
    HandoffCreated,
}

/// File system watcher for pair artifacts
pub struct PairWatcher {
    /// Path to the shared directory being watched
    shared_path: PathBuf,
    /// Pair identifier for logging
    pair_id: String,
    /// Channel for sending watch events
    event_tx: mpsc::UnboundedSender<WatchEvent>,
}

impl PairWatcher {
    /// Creates a new watcher for the given shared directory
    pub fn new(
        shared_path: PathBuf,
        pair_id: String,
    ) -> (Self, mpsc::UnboundedReceiver<WatchEvent>) {
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        let watcher = Self {
            shared_path,
            pair_id,
            event_tx,
        };

        (watcher, event_rx)
    }

    /// Starts watching the shared directory
    ///
    /// # Validation: C3-01 No Polling
    /// This function uses the notify crate's event-driven mechanism.
    /// ZERO sleep or polling loops.
    ///
    /// # Validation: C3-02 inotify Integration
    /// Uses notify crate to watch directory
    ///
    /// # Validation: C3-03 Reaction Latency
    /// Events trigger immediately within the same async tick
    pub async fn watch(self) -> Result<()> {
        self.watch_with_ready(None).await
    }

    async fn watch_with_ready(
        self,
        ready_tx: Option<tokio::sync::oneshot::Sender<()>>,
    ) -> Result<()> {
        info!(
            pair_id = %self.pair_id,
            shared_path = %self.shared_path.display(),
            "Starting file system watcher"
        );

        // Ensure directory exists
        if !self.shared_path.exists() {
            std::fs::create_dir_all(&self.shared_path)
                .context("Failed to create shared directory")?;
        }

        let (notify_tx, mut notify_rx) = mpsc::unbounded_channel();
        let shared_path_clone = self.shared_path.clone();

        // Validation: C3-02 - Uses notify crate for inotify/FSEvents integration
        let mut watcher = RecommendedWatcher::new(
            move |result: Result<Event, notify::Error>| match result {
                Ok(event) => {
                    if notify_tx.send(event).is_err() {
                        error!("Failed to send notify event - receiver dropped");
                    }
                }
                Err(e) => {
                    error!(error = %e, "File watcher error");
                }
            },
            Config::default(),
        )
        .context("Failed to create file watcher")?;

        // Watch the shared directory
        watcher
            .watch(&self.shared_path, RecursiveMode::NonRecursive)
            .context("Failed to start watching directory")?;

        if let Some(ready_tx) = ready_tx {
            let _ = ready_tx.send(());
        }

        info!(
            pair_id = %self.pair_id,
            "File watcher started successfully"
        );

        // Validation: C3-01 No Polling - Event-driven loop, no sleep() calls
        // Validation: C3-03 Reaction Latency - Immediate processing within same async tick
        while let Some(event) = notify_rx.recv().await {
            debug!(
                pair_id = %self.pair_id,
                event = ?event,
                "Received file system event"
            );

            // Process the event
            if let Err(e) = self.process_event(&event, &shared_path_clone) {
                warn!(
                    pair_id = %self.pair_id,
                    error = %e,
                    "Failed to process file event"
                );
            }
        }

        info!(
            pair_id = %self.pair_id,
            "File watcher stopped"
        );

        Ok(())
    }

    /// Processes a file system event and emits appropriate WatchEvent
    fn process_event(&self, event: &Event, _shared_path: &Path) -> Result<()> {
        // Only process modify and create events
        match event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                for path in &event.paths {
                    if let Some(file_name) = path.file_name() {
                        let file_name_str = file_name.to_string_lossy();

                        // Validation: C3-03 - WORKLOG.md modification triggers spawn_sentinel
                        // within the same async tick (no delay)
                        let watch_event = match file_name_str.as_ref() {
                            "WORKLOG.md" => {
                                info!(
                                    pair_id = %self.pair_id,
                                    "WORKLOG.md modified - triggering SENTINEL evaluation"
                                );
                                Some(WatchEvent::WorklogModified)
                            }
                            "STATUS.json" => {
                                info!(
                                    pair_id = %self.pair_id,
                                    "STATUS.json changed - checking for termination"
                                );
                                Some(WatchEvent::StatusChanged)
                            }
                            "PLAN.md" => {
                                info!(
                                    pair_id = %self.pair_id,
                                    "PLAN.md modified - plan ready for review"
                                );
                                Some(WatchEvent::PlanModified)
                            }
                            "HANDOFF.md" => {
                                info!(
                                    pair_id = %self.pair_id,
                                    "HANDOFF.md created - context reset requested"
                                );
                                Some(WatchEvent::HandoffCreated)
                            }
                            _ => {
                                debug!(
                                    pair_id = %self.pair_id,
                                    file_name = %file_name_str,
                                    "Ignoring unmonitored file"
                                );
                                None
                            }
                        };

                        if let Some(watch_event) = watch_event {
                            self.event_tx
                                .send(watch_event)
                                .context("Failed to send watch event")?;
                        }
                    }
                }
            }
            _ => {
                // Ignore other event types (remove, access, etc.)
                debug!(
                    pair_id = %self.pair_id,
                    event_kind = ?event.kind,
                    "Ignoring non-modify event"
                );
            }
        }

        Ok(())
    }
}

/// Watches multiple pairs concurrently
pub struct MultiPairWatcher {
    watchers: Vec<(String, PairWatcher, mpsc::UnboundedReceiver<WatchEvent>)>,
}

impl MultiPairWatcher {
    pub fn new() -> Self {
        Self {
            watchers: Vec::new(),
        }
    }

    /// Adds a pair to watch
    pub fn add_pair(&mut self, pair_id: String, shared_path: PathBuf) {
        let (watcher, event_rx) = PairWatcher::new(shared_path, pair_id.clone());
        self.watchers.push((pair_id, watcher, event_rx));
    }

    /// Starts watching all pairs
    /// Returns a receiver that receives (pair_id, WatchEvent) tuples
    pub async fn watch_all(self) -> Result<mpsc::UnboundedReceiver<(String, WatchEvent)>> {
        let (combined_tx, combined_rx) = mpsc::unbounded_channel();

        for (pair_id, watcher, mut event_rx) in self.watchers {
            let combined_tx_clone = combined_tx.clone();
            let pair_id_clone = pair_id.clone();

            // Spawn watcher task
            tokio::spawn(async move {
                if let Err(e) = watcher.watch().await {
                    error!(
                        pair_id = %pair_id,
                        error = %e,
                        "Watcher task failed"
                    );
                }
            });

            // Spawn event forwarding task
            tokio::spawn(async move {
                while let Some(event) = event_rx.recv().await {
                    if combined_tx_clone
                        .send((pair_id_clone.clone(), event))
                        .is_err()
                    {
                        break;
                    }
                }
            });
        }

        Ok(combined_rx)
    }
}

impl Default for MultiPairWatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_watcher_creation() {
        let temp_dir = std::env::temp_dir().join("agentflow-test-watcher");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (watcher, _rx) = PairWatcher::new(temp_dir.clone(), "pair-1".to_string());

        assert_eq!(watcher.pair_id, "pair-1");
        assert_eq!(watcher.shared_path, temp_dir);

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[tokio::test]
    async fn test_worklog_detection() {
        let temp_dir = std::env::temp_dir().join("agentflow-test-worklog");
        std::fs::create_dir_all(&temp_dir).unwrap();

        let (watcher, mut rx) = PairWatcher::new(temp_dir.clone(), "pair-1".to_string());
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();

        // Validation: C3-01 No Polling - zero sleep() in test code
        // Spawn watcher in background
        tokio::spawn(async move {
            watcher.watch_with_ready(Some(ready_tx)).await.ok();
        });

        ready_rx.await.expect("Watcher should start successfully");

        // Create WORKLOG.md immediately - watcher will react
        let worklog_path = temp_dir.join("WORKLOG.md");
        std::fs::write(&worklog_path, "# Worklog\n").unwrap();

        // Wait for event (event-driven, no sleep)
        tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(event) = rx.recv().await {
                if matches!(event, WatchEvent::WorklogModified) {
                    return;
                }
            }
        })
        .await
        .expect("Should receive WorklogModified event");

        // Cleanup
        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
