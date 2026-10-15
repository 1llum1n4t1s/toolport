//! Bounded background indexing with a content-addressed, disposable vector cache.
use super::search_static;
use conduit_lib::tool_definitions::{SharedTools, ToolCatalog};
#[cfg(test)]
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct Vectors(pub Arc<OnceLock<Vec<Vec<f32>>>>, Mutex<Option<Arc<Job>>>);

#[cfg(test)]
fn key(tool: &Value) -> String {
    key_document(&search_static::document(tool))
}

fn key_document(text: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(search_static::asset_digest());
    hash.update(text.as_bytes());
    format!("{:x}", hash.finalize())
}

#[cfg(test)]
pub fn export_native_fixture(tools: &dyn ToolCatalog, directory: &Path, output: &Path) {
    let documents: Vec<(String, String)> = tools
        .iter()
        .map(|tool| {
            let text = search_static::document(tool);
            (key_document(&text), text)
        })
        .collect();
    let (vectors, encoded) = encode(&documents, Some(directory)).unwrap();
    eprintln!(
        "native fixture: {} documents, {encoded} cache misses encoded",
        documents.len()
    );
    let mut bytes = Vec::new();
    for ((key, _), vector) in documents.iter().zip(vectors) {
        for pair in key.as_bytes().as_chunks::<2>().0 {
            bytes.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
        }
        for value in vector {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
    }
    std::fs::write(output, bytes).unwrap();
}

#[cfg(test)]
pub fn seed_native_cache_for_test(tools: &dyn ToolCatalog, directory: &Path) {
    std::fs::create_dir_all(directory).unwrap();
    for tool in tools.iter() {
        write(
            &directory.join(key(tool)),
            &search_static::tool_vector(tool),
        )
        .unwrap();
    }
}

fn read(path: &Path, dimensions: usize) -> Option<Vec<f32>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take((dimensions * 4 + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() != dimensions * 4 {
        return None;
    }
    let vector: Vec<f32> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| f32::from_le_bytes(*v))
        .collect();
    let norm: f32 = vector.iter().map(|v| v * v).sum();
    (vector.iter().all(|v| v.is_finite()) && (norm == 0.0 || (norm - 1.0).abs() < 0.001))
        .then_some(vector)
}

fn write(path: &Path, vector: &[f32]) -> std::io::Result<()> {
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary)?;
        for value in vector {
            file.write_all(&value.to_le_bytes())?;
        }
        file.flush()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

fn encode(
    documents: &[(String, String)],
    directory: Option<&Path>,
) -> Result<(Vec<Vec<f32>>, usize), String> {
    let model = search_static::model();
    let cache = directory.filter(|dir| std::fs::create_dir_all(dir).is_ok());
    if let Some(directory) = cache {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
        }
        evict(
            directory,
            32 * 1024 * 1024,
            std::time::Duration::from_secs(14 * 86400),
        );
    }
    let mut embedded = 0;
    let mut vectors = vec![Vec::new(); documents.len()];
    let mut missing = Vec::new();
    for (n, (digest, _)) in documents.iter().enumerate() {
        let vector = cache.and_then(|dir| read(&dir.join(digest), model.dimensions()));
        if let Some(vector) = vector {
            vectors[n] = vector;
        } else {
            missing.push(n);
        }
    }
    for batch in missing.chunks(search_static::BATCH_SIZE) {
        let texts: Vec<_> = batch.iter().map(|n| documents[*n].1.as_str()).collect();
        let encoded = model.encode_documents(&texts)?;
        for (n, vector) in batch.iter().zip(encoded) {
            if let Some(directory) = cache {
                let _ = write(&directory.join(&documents[*n].0), &vector);
            }
            vectors[*n] = vector;
            embedded += 1;
        }
    }
    if let Some(directory) = cache {
        evict(
            directory,
            32 * 1024 * 1024,
            std::time::Duration::from_secs(14 * 86400),
        );
    }
    Ok((vectors, embedded))
}

#[derive(Debug, Default)]
struct RetryState {
    active: bool,
    attempts: u32,
    retry_at: Option<Instant>,
    logged: bool,
}

#[derive(Clone, Copy)]
struct RetryPolicy {
    delay: Duration,
    attempts: u32,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            delay: Duration::from_secs(60),
            attempts: 3,
        }
    }
}

impl RetryPolicy {
    fn retry_delay(self, attempts: u32) -> Duration {
        self.delay * (1 << (attempts - 1))
    }
}

#[derive(Debug)]
struct Job {
    tools: SharedTools,
    directory: Option<PathBuf>,
    ready: Arc<OnceLock<Vec<Vec<f32>>>>,
    completed: Option<std::sync::mpsc::Sender<()>>,
    retry: Mutex<RetryState>,
}
type Queue = Arc<(Mutex<Option<Arc<Job>>>, Condvar)>;

fn worker() -> Option<Queue> {
    worker_with(encode, RetryPolicy::default())
}

fn worker_with(
    encode: impl Fn(&[(String, String)], Option<&Path>) -> Result<(Vec<Vec<f32>>, usize), String>
        + Send
        + 'static,
    policy: RetryPolicy,
) -> Option<Queue> {
    let queue: Queue = Arc::new((Mutex::new(None), Condvar::new()));
    let worker = queue.clone();
    std::thread::Builder::new()
        .name("search-index".into())
        .spawn(move || loop {
            let (lock, wake) = &*worker;
            let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
            let job = loop {
                let delay = pending.as_ref().and_then(|job| {
                    job.retry.lock().unwrap_or_else(|e| e.into_inner()).retry_at
                }).map_or(Duration::from_secs(60), |at| at.saturating_duration_since(Instant::now()));
                if pending.is_some() && delay.is_zero() {
                    break pending.take().unwrap();
                }
                // A fresh job has no retry deadline and is immediately runnable.
                if pending.as_ref().is_some_and(|job| job.retry.lock().unwrap_or_else(|e| e.into_inner()).retry_at.is_none()) {
                    break pending.take().unwrap();
                }
                let (next, timeout) = wake.wait_timeout(pending, delay.min(Duration::from_secs(60)))
                    .unwrap_or_else(|e| e.into_inner());
                pending = next;
                if timeout.timed_out() && pending.is_none() {
                    drop(pending);
                    search_static::unload_idle();
                    pending = lock.lock().unwrap_or_else(|e| e.into_inner());
                }
            };
            drop(pending);
            if job.ready.get().is_some() { continue; }
            job.retry.lock().unwrap_or_else(|e| e.into_inner()).attempts += 1;
            // Only publish a complete successful catalog, even across retries.
            let documents: Vec<_> = job.tools.iter().map(|tool| {
                let text = search_static::document(tool);
                (key_document(&text), text)
            }).collect();
            let result = encode(&documents, job.directory.as_deref());
            drop(documents);
            search_static::release_allocator_memory();
            match result {
                Ok((vectors, _)) => {
                    let _ = job.ready.set(vectors);
                    job.retry.lock().unwrap_or_else(|e| e.into_inner()).active = false;
                    if let Some(completed) = &job.completed { let _ = completed.send(()); }
                }
                Err(error) => {
                    let mut retry = job.retry.lock().unwrap_or_else(|e| e.into_inner());
                    retry.retry_at = Some(Instant::now() + policy.retry_delay(retry.attempts));
                    let exhausted = retry.attempts >= policy.attempts;
                    if exhausted {
                        retry.active = false;
                        if !retry.logged {
                            retry.logged = true;
                            eprintln!("toolport: local search encoding failed after {} attempts; later searches can retry: {error}", retry.attempts);
                        }
                    }
                    drop(retry);
                    if exhausted {
                        if let Some(completed) = &job.completed { let _ = completed.send(()); }
                    } else {
                        // A newer catalog takes precedence. The displaced job stays
                        // available to its snapshot's next ensure_started call.
                        schedule(&worker, job, false);
                    }
                }
            }
        }).ok()?;
    Some(queue)
}

fn schedule(queue: &Queue, job: Arc<Job>, replace: bool) {
    let (lock, wake) = &**queue;
    let mut pending = lock.lock().unwrap_or_else(|e| e.into_inner());
    let displaced = if replace || pending.is_none() {
        pending.replace(job)
    } else {
        Some(job)
    };
    if let Some(displaced) = displaced {
        displaced
            .retry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active = false;
    }
    drop(pending);
    wake.notify_one();
}

#[cfg(test)]
fn submit(
    queue: &Queue,
    tools: SharedTools,
    directory: Option<PathBuf>,
    ready: Arc<OnceLock<Vec<Vec<f32>>>>,
    completed: Option<std::sync::mpsc::Sender<()>>,
) -> Arc<Job> {
    let job = Arc::new(Job {
        tools,
        directory,
        ready,
        completed,
        retry: Mutex::new(RetryState {
            active: true,
            ..RetryState::default()
        }),
    });
    schedule(queue, job.clone(), true);
    job
}

fn worker_queue() -> Option<&'static Queue> {
    static WORKER: OnceLock<Option<Queue>> = OnceLock::new();
    WORKER.get_or_init(worker).as_ref()
}

impl Vectors {
    /// Retain the catalog after failure; only one queued/running attempt per snapshot.
    pub fn ensure_started(&self) {
        if self.0.get().is_some() || self.1.lock().unwrap_or_else(|e| e.into_inner()).is_none() {
            return;
        }
        if let Some(queue) = worker_queue() {
            self.ensure_on(queue);
        }
    }

    fn ensure_on(&self, queue: &Queue) {
        let job = self.1.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(job) = job {
            let mut retry = job.retry.lock().unwrap_or_else(|e| e.into_inner());
            if job.ready.get().is_some()
                || retry.active
                || retry.retry_at.is_some_and(|at| Instant::now() < at)
            {
                return;
            }
            retry.active = true;
            retry.attempts = 0;
            retry.retry_at = None;
            drop(retry);
            schedule(queue, job, true);
        }
    }

    /// Test boundary using the shipping scheduler and atomic publication path.
    #[cfg(test)]
    pub fn start_background(
        &self,
        tools: SharedTools,
        directory: Option<PathBuf>,
        completed: Option<std::sync::mpsc::Sender<()>>,
    ) {
        if let Some(queue) = worker_queue() {
            *self.1.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(submit(queue, tools, directory, self.0.clone(), completed));
        }
    }

    pub fn build(tools: &dyn ToolCatalog) -> Self {
        let result = Self::default();
        if !search_static::enabled() {
            return result;
        }
        #[cfg(test)]
        result
            .0
            .set(tools.iter().map(search_static::tool_vector).collect())
            .unwrap();
        #[cfg(not(test))]
        {
            let directory =
                conduit_lib::registry::conduit_dir().map(|dir| dir.join("search-vectors"));
            *result.1.lock().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(Job {
                tools: tools.shared(),
                directory,
                ready: result.0.clone(),
                completed: None,
                retry: Mutex::new(RetryState::default()),
            }));
        }
        result
    }
}

// Cache files are disposable. Bound both age and total bytes, independently of
// catalog turnover. Run on the encoder thread, never during a search request.
fn evict(directory: &Path, maximum: u64, age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = std::time::SystemTime::now();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if now
            .duration_since(modified)
            .is_ok_and(|elapsed| elapsed > age)
        {
            let _ = std::fs::remove_file(entry.path());
        } else {
            files.push((modified, metadata.len(), entry.path()));
        }
    }
    files.sort_by_key(|(modified, _, _)| *modified);
    let mut size: u64 = files.iter().map(|(_, size, _)| *size).sum();
    for (_, bytes, path) in files {
        if size <= maximum {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            size = size.saturating_sub(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_catalog_retries_and_publishes_atomically() {
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = attempts.clone();
        let (failed, failure) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let queue = worker_with(
            move |documents, directory| {
                if count.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 0 {
                    failed.send(()).unwrap();
                    resume.recv_timeout(Duration::from_secs(30)).unwrap();
                    return Err("injected encoder resource failure".into());
                }
                encode(documents, directory)
            },
            RetryPolicy {
                delay: Duration::from_millis(10),
                attempts: 3,
            },
        )
        .unwrap();
        let tools =
            vec![serde_json::json!({"name":"sample__read_record","description":"Read record"})];
        let vectors = Vectors::default();
        let (completed, done) = std::sync::mpsc::channel();
        *vectors.1.lock().unwrap() = Some(submit(
            &queue,
            tools.clone().into(),
            None,
            vectors.0.clone(),
            Some(completed),
        ));
        failure.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(vectors.0.get().is_none(), "partial catalog was published");
        vectors.ensure_on(&queue); // A concurrent search must not duplicate the active job.
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(30)).unwrap();
        assert_eq!(attempts.load(std::sync::atomic::Ordering::Relaxed), 2);
        assert_eq!(
            vectors.0.get().unwrap(),
            &tools
                .iter()
                .map(search_static::tool_vector)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn exhausted_catalog_retries_resume_on_later_search() {
        let failing = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let fail = failing.clone();
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = attempts.clone();
        let queue = worker_with(
            move |documents, directory| {
                count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if fail.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err("injected persistent resource failure".into());
                }
                encode(documents, directory)
            },
            RetryPolicy {
                delay: Duration::from_millis(10),
                attempts: 3,
            },
        )
        .unwrap();
        let vectors = Vectors::default();
        let (completed, done) = std::sync::mpsc::channel();
        let job = submit(
            &queue,
            vec![serde_json::json!({"name":"sample__read_record"})].into(),
            None,
            vectors.0.clone(),
            Some(completed),
        );
        *vectors.1.lock().unwrap() = Some(job.clone());
        done.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(vectors.0.get().is_none());
        let mut retry = job.retry.lock().unwrap();
        assert_eq!(retry.attempts, 3);
        assert!(!retry.active);
        assert!(retry.logged);
        let policy = RetryPolicy {
            delay: Duration::from_millis(10),
            attempts: 3,
        };
        assert_eq!(
            (1..=3).map(|n| policy.retry_delay(n)).collect::<Vec<_>>(),
            [
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40)
            ]
        );
        // Control the deadline instead of relying on test-thread scheduling.
        retry.retry_at = Some(Instant::now() + Duration::from_secs(60));
        drop(retry);
        vectors.ensure_on(&queue);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::Relaxed), 3);
        failing.store(false, std::sync::atomic::Ordering::Relaxed);
        job.retry.lock().unwrap().retry_at = Some(Instant::now() - Duration::from_secs(1));
        vectors.ensure_on(&queue);
        done.recv_timeout(Duration::from_secs(30)).unwrap();
        assert!(vectors.0.get().is_some());
        assert_eq!(attempts.load(std::sync::atomic::Ordering::Relaxed), 4);
        assert!(job.retry.lock().unwrap().logged);
    }

    #[test]
    fn production_worker_publishes_and_cache_evicts() {
        let queue = worker().unwrap();
        let root =
            std::env::temp_dir().join(format!("toolport-search-worker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let tool = serde_json::json!({"name":"sample__read_record","description":"Read record","inputSchema":{}});
        let ready = Arc::new(OnceLock::new());
        let (send, receive) = std::sync::mpsc::channel();
        submit(
            &queue,
            vec![tool.clone()].into(),
            Some(root.clone()),
            ready.clone(),
            Some(send),
        );
        receive
            .recv_timeout(std::time::Duration::from_secs(30))
            .unwrap();
        assert_eq!(ready.get().unwrap()[0], search_static::tool_vector(&tool));
        assert!(root.join(key(&tool)).exists());
        evict(&root, 0, std::time::Duration::from_secs(86400));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::write(root.join("old"), b"old").unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(root.join("old"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(std::time::UNIX_EPOCH))
            .unwrap();
        evict(&root, u64::MAX, std::time::Duration::from_secs(86400));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cache_keys_follow_rendered_text_and_lazy_jobs_do_not_encode() {
        let first = serde_json::json!({"name":"sample__read","description":"Read","inputSchema":{"properties":{"id":{"type":"string"}}}});
        let mut changed = first.clone();
        changed["extraMetadata"] = serde_json::json!("ignored");
        assert_eq!(key(&first), key(&changed));
        changed["description"] = serde_json::json!("Read a different resource");
        assert_ne!(key(&first), key(&changed));
        assert_ne!(key_document("Read"), key_document("Write"));
        let vectors = Vectors::default();
        *vectors.1.lock().unwrap() = Some(Arc::new(Job {
            tools: vec![first].into(),
            directory: None,
            ready: vectors.0.clone(),
            completed: None,
            retry: Mutex::new(RetryState::default()),
        }));
        assert!(vectors.0.get().is_none());
        assert!(vectors.1.lock().unwrap().is_some());
    }
    #[test]
    fn cache_reuses_content_and_recovers_corruption() {
        let root =
            std::env::temp_dir().join(format!("toolport-search-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let first = serde_json::json!({"name":"example__read_record","description":"Read a record","inputSchema":{"properties":{"id":{"type":"string"}}}});
        let document = |tool: &Value| (key(tool), search_static::document(tool));
        let docs = vec![document(&first)];
        let (vectors, count) = encode(&docs, Some(&root)).unwrap();
        assert_eq!(count, 1);
        let (cached, count) = encode(&docs, Some(&root)).unwrap();
        assert_eq!(count, 0);
        assert_eq!(vectors, cached);
        let mut changed = first.clone();
        changed["inputSchema"]["properties"]["id"]["description"] =
            serde_json::json!("Resource identifier");
        assert_ne!(key(&first), key(&changed));
        let (_, count) = encode(&[document(&first), document(&changed)], Some(&root)).unwrap();
        assert_eq!(count, 1);
        std::fs::write(root.join(&docs[0].0), b"broken").unwrap();
        let (recovered, count) = encode(&docs, Some(&root)).unwrap();
        assert_eq!(count, 1);
        assert_eq!(vectors, recovered);
        let (uncached, count) = encode(&docs, None).unwrap();
        assert_eq!(count, 1);
        assert_eq!(vectors, uncached);
        std::fs::remove_dir_all(root).unwrap();
    }
}
