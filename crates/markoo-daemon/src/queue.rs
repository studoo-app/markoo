//! The job queue: at most one sync per pair at a time, whatever asked for it.
//!
//! Requests received while a sync waits or runs are merged into a single
//! extra sync: the debounce that keeps a burst of triggers from piling up.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde::Serialize;
use tokio::sync::broadcast;

/// Finished jobs kept for `GET /jobs/{id}`.
const KEPT_JOBS: usize = 256;
/// Events a slow subscriber can fall behind by before missing some.
const EVENT_BUFFER: usize = 256;

/// What asked for a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Trigger {
    /// The daemon started.
    Startup,
    /// The interval elapsed.
    Interval,
    /// The configuration of the pair changed.
    Config,
    /// A request to the API.
    Api,
    /// A request from an MCP client.
    Mcp,
}

/// Progress of a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobStatus {
    /// Waiting for the pair's previous job.
    Queued,
    /// Being run.
    Running,
    /// Finished normally.
    Done,
    /// Finished with an error.
    Failed,
}

/// What a finished job did.
pub use markoo_core::service::SyncOutcome as JobOutcome;

/// A sync requested from the daemon.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Job {
    /// Identifier of the job.
    pub job: String,
    /// The pair.
    pub pair: String,
    /// Whether the job only computes the plan.
    pub dry_run: bool,
    /// What asked for it.
    pub trigger: Trigger,
    /// Progress.
    pub status: JobStatus,
    /// What it did, once done.
    pub outcome: Option<JobOutcome>,
    /// Why it failed.
    pub error: Option<String>,
    /// How long it ran, once finished.
    pub duration_ms: Option<u64>,
}

/// Something interfaces may want to know about.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "kebab-case")]
pub enum Event {
    /// A job started running.
    JobStarted {
        /// The job.
        job: String,
        /// The pair.
        pair: String,
    },
    /// A job finished, successfully or not.
    JobFinished {
        /// The finished job.
        job: Box<Job>,
    },
    /// More issues wait for the user than after the pair's previous job.
    NewConflicts {
        /// The pair.
        pair: String,
        /// Number of issues now waiting.
        count: usize,
    },
    /// The configuration file changed.
    ConfigReloaded,
    /// The daemon is stopping; event streams end here.
    Shutdown,
}

impl Event {
    /// Name of the event in the SSE stream.
    pub fn name(&self) -> &'static str {
        match self {
            Event::JobStarted { .. } => "job-started",
            Event::JobFinished { .. } => "job-finished",
            Event::NewConflicts { .. } => "new-conflicts",
            Event::ConfigReloaded => "config-reloaded",
            Event::Shutdown => "shutdown",
        }
    }
}

/// Runs one sync. It blocks, so the queue calls it outside the async runtime.
pub trait Runner: Send + Sync + 'static {
    /// Syncs `pair`; the error is a message for the user.
    fn run(&self, pair: &str, dry_run: bool) -> Result<JobOutcome, String>;
}

#[derive(Default)]
struct PairQueue {
    running: bool,
    waiting: VecDeque<u64>,
    last_conflicts: usize,
}

#[derive(Default)]
struct Inner {
    next_id: u64,
    jobs: BTreeMap<u64, Job>,
    pairs: HashMap<String, PairQueue>,
}

impl Inner {
    /// Forgets the oldest finished jobs.
    fn prune(&mut self) {
        let mut excess = self.jobs.len().saturating_sub(KEPT_JOBS);
        self.jobs.retain(|_, job| {
            let finished = matches!(job.status, JobStatus::Done | JobStatus::Failed);
            if excess > 0 && finished {
                excess -= 1;
                return false;
            }
            true
        });
    }
}

/// The job queue. Cloning gives another handle on the same queue.
#[derive(Clone)]
pub struct Queue {
    inner: Arc<Mutex<Inner>>,
    runner: Arc<dyn Runner>,
    events: broadcast::Sender<Event>,
}

impl std::fmt::Debug for Queue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Queue")
    }
}

fn job_id(number: u64) -> String {
    format!("j_{number}")
}

fn parse_job_id(id: &str) -> Option<u64> {
    id.strip_prefix("j_")?.parse().ok()
}

impl Queue {
    /// Creates an empty queue.
    pub fn new(runner: Arc<dyn Runner>) -> Self {
        Self {
            inner: Arc::default(),
            runner,
            events: broadcast::channel(EVENT_BUFFER).0,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Nothing panics while holding this lock; if something did, the job
        // table is still usable.
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Asks for a sync and returns the job that will do it: a new one, or the
    /// one already waiting for this pair. Must be called inside the runtime.
    pub fn submit(&self, pair: &str, dry_run: bool, trigger: Trigger) -> String {
        let mut inner = self.lock();
        let inner = &mut *inner;
        let waiting = &inner.pairs.entry(pair.to_owned()).or_default().waiting;
        let merged = waiting.iter().find(|number| {
            inner
                .jobs
                .get(number)
                .is_some_and(|job| job.dry_run == dry_run)
        });
        if let Some(number) = merged {
            return job_id(*number);
        }
        inner.next_id += 1;
        let number = inner.next_id;
        inner.jobs.insert(
            number,
            Job {
                job: job_id(number),
                pair: pair.to_owned(),
                dry_run,
                trigger,
                status: JobStatus::Queued,
                outcome: None,
                error: None,
                duration_ms: None,
            },
        );
        inner.prune();
        let slot = inner.pairs.entry(pair.to_owned()).or_default();
        slot.waiting.push_back(number);
        if !slot.running {
            slot.running = true;
            let queue = self.clone();
            let pair = pair.to_owned();
            tokio::spawn(async move { queue.work(pair).await });
        }
        job_id(number)
    }

    /// Runs the pair's jobs one after the other until none is left.
    async fn work(self, pair: String) {
        while let Some((number, dry_run)) = self.next_job(&pair) {
            self.publish(Event::JobStarted {
                job: job_id(number),
                pair: pair.clone(),
            });
            let started = Instant::now();
            let runner = Arc::clone(&self.runner);
            let name = pair.clone();
            let result = tokio::task::spawn_blocking(move || runner.run(&name, dry_run))
                .await
                .unwrap_or_else(|error| Err(format!("the sync stopped unexpectedly: {error}")));
            self.finish(&pair, number, result, started);
        }
    }

    /// Takes the pair's next job, or marks the pair idle if there is none.
    fn next_job(&self, pair: &str) -> Option<(u64, bool)> {
        let mut inner = self.lock();
        let inner = &mut *inner;
        let slot = inner.pairs.entry(pair.to_owned()).or_default();
        let Some(number) = slot.waiting.pop_front() else {
            slot.running = false;
            return None;
        };
        let job = inner.jobs.get_mut(&number)?;
        job.status = JobStatus::Running;
        Some((number, job.dry_run))
    }

    fn finish(
        &self,
        pair: &str,
        number: u64,
        result: Result<JobOutcome, String>,
        started: Instant,
    ) {
        let mut new_conflicts = None;
        let finished = {
            let mut inner = self.lock();
            if let Ok(outcome) = &result {
                let slot = inner.pairs.entry(pair.to_owned()).or_default();
                let count = outcome.conflict_count();
                if count > slot.last_conflicts {
                    new_conflicts = Some(count);
                }
                slot.last_conflicts = count;
            }
            inner.jobs.get_mut(&number).map(|job| {
                job.duration_ms =
                    Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
                match result {
                    Ok(outcome) => {
                        job.status = JobStatus::Done;
                        job.outcome = Some(outcome);
                    }
                    Err(error) => {
                        job.status = JobStatus::Failed;
                        job.error = Some(error);
                    }
                }
                job.clone()
            })
        };
        if let Some(job) = finished {
            if let Some(error) = &job.error {
                eprintln!("error: {pair}: {error}");
            }
            self.publish(Event::JobFinished { job: Box::new(job) });
        }
        if let Some(count) = new_conflicts {
            self.publish(Event::NewConflicts {
                pair: pair.to_owned(),
                count,
            });
        }
    }

    /// A job by identifier.
    pub fn job(&self, id: &str) -> Option<Job> {
        self.lock().jobs.get(&parse_job_id(id)?).cloned()
    }

    /// Pairs with a job running or waiting.
    pub fn active_pairs(&self) -> Vec<String> {
        let mut pairs: Vec<String> = self
            .lock()
            .pairs
            .iter()
            .filter(|(_, slot)| slot.running)
            .map(|(name, _)| name.clone())
            .collect();
        pairs.sort();
        pairs
    }

    /// Sends an event to the current subscribers.
    pub fn publish(&self, event: Event) {
        // An error only means that nobody is listening.
        let _ = self.events.send(event);
    }

    /// Subscribes to the events sent from now on.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::sync::Condvar;
    use std::time::Duration;

    /// Blocks every run until the test opens the gate, and counts the runs.
    #[derive(Default)]
    struct GatedRunner {
        open: Mutex<bool>,
        changed: Condvar,
        runs: Mutex<Vec<(String, bool)>>,
    }

    impl GatedRunner {
        fn open(&self) {
            *self.open.lock().unwrap() = true;
            self.changed.notify_all();
        }
    }

    impl Runner for GatedRunner {
        fn run(&self, pair: &str, dry_run: bool) -> Result<JobOutcome, String> {
            let open = self.open.lock().unwrap();
            drop(self.changed.wait_while(open, |open| !*open).unwrap());
            self.runs.lock().unwrap().push((pair.to_owned(), dry_run));
            if pair == "bad" {
                return Err("it failed".to_owned());
            }
            Ok(JobOutcome {
                kb_type: "markdown".to_owned(),
                direction_changed: false,
                summary: Value::Null,
                conflicts: if pair == "conflicted" {
                    vec![Value::Null]
                } else {
                    Vec::new()
                },
                report: Value::Null,
            })
        }
    }

    fn gated() -> (Arc<GatedRunner>, Queue) {
        let runner = Arc::new(GatedRunner::default());
        let queue = Queue::new(Arc::clone(&runner) as Arc<dyn Runner>);
        (runner, queue)
    }

    async fn wait_for(queue: &Queue, id: &str, status: JobStatus) -> Job {
        for _ in 0..500 {
            if let Some(job) = queue.job(id).filter(|job| job.status == status) {
                return job;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("{id} never reached {status:?}");
    }

    #[tokio::test]
    async fn requests_during_a_sync_merge_into_one_extra_sync() {
        let (runner, queue) = gated();
        let first = queue.submit("p", false, Trigger::Api);
        wait_for(&queue, &first, JobStatus::Running).await;
        let second = queue.submit("p", false, Trigger::Interval);
        let third = queue.submit("p", false, Trigger::Api);
        assert_eq!(second, third);
        runner.open();
        wait_for(&queue, &third, JobStatus::Done).await;
        assert_eq!(runner.runs.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn dry_run_is_not_merged_into_a_real_sync() {
        let (runner, queue) = gated();
        let first = queue.submit("p", false, Trigger::Api);
        wait_for(&queue, &first, JobStatus::Running).await;
        let real = queue.submit("p", false, Trigger::Api);
        let dry = queue.submit("p", true, Trigger::Api);
        assert_ne!(real, dry);
        runner.open();
        wait_for(&queue, &dry, JobStatus::Done).await;
    }

    #[tokio::test]
    async fn a_pair_never_runs_two_jobs_at_once() {
        let (runner, queue) = gated();
        let first = queue.submit("p", false, Trigger::Api);
        wait_for(&queue, &first, JobStatus::Running).await;
        let second = queue.submit("p", false, Trigger::Api);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(queue.job(&second).unwrap().status, JobStatus::Queued);
        runner.open();
        wait_for(&queue, &second, JobStatus::Done).await;
    }

    #[tokio::test]
    async fn other_pairs_run_in_parallel() {
        let (runner, queue) = gated();
        let a = queue.submit("a", false, Trigger::Api);
        let b = queue.submit("b", false, Trigger::Api);
        wait_for(&queue, &a, JobStatus::Running).await;
        wait_for(&queue, &b, JobStatus::Running).await;
        runner.open();
    }

    #[tokio::test]
    async fn a_failed_job_keeps_its_error_and_the_queue_goes_on() {
        let (runner, queue) = gated();
        runner.open();
        let failed = queue.submit("bad", false, Trigger::Api);
        let job = wait_for(&queue, &failed, JobStatus::Failed).await;
        assert_eq!(job.error.as_deref(), Some("it failed"));
        let next = queue.submit("bad", false, Trigger::Api);
        wait_for(&queue, &next, JobStatus::Failed).await;
    }

    #[tokio::test]
    async fn the_pair_becomes_idle_after_its_last_job() {
        let (runner, queue) = gated();
        runner.open();
        let id = queue.submit("p", false, Trigger::Api);
        wait_for(&queue, &id, JobStatus::Done).await;
        for _ in 0..500 {
            if queue.active_pairs().is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the pair stayed active");
    }

    #[tokio::test]
    async fn new_conflicts_are_announced_once() {
        let (runner, queue) = gated();
        runner.open();
        let mut events = queue.subscribe();
        for _ in 0..2 {
            let id = queue.submit("conflicted", false, Trigger::Api);
            wait_for(&queue, &id, JobStatus::Done).await;
        }
        let mut announced = 0;
        while let Ok(event) = events.try_recv() {
            announced += usize::from(matches!(event, Event::NewConflicts { .. }));
        }
        assert_eq!(announced, 1);
    }

    #[test]
    fn unknown_job_ids_are_not_found() {
        let queue = Queue::new(Arc::new(GatedRunner::default()));
        assert!(queue.job("nope").is_none());
    }
}
