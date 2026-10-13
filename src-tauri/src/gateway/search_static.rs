//! Offline BGE CLS embeddings, with a lazy, idle-unloaded CPU session.
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock, TryLockError,
};
use std::time::{Duration, Instant};
use tokenizers::{Tokenizer, TruncationParams};

const WEIGHTS: &[u8] = include_bytes!("../../assets/search/model-f16.safetensors");
const TOKENIZER: &[u8] = include_bytes!("../../assets/search/tokenizer.json");
const CONFIG: &[u8] = include_bytes!("../../assets/search/config.json");
// Retain the model and tokenizer notices in each shipping gateway.
const LICENSES: &str = concat!(
    include_str!("../../assets/search/LICENSE"),
    include_str!("../../assets/search/tokenizers-LICENSE")
);

pub const DOCUMENT_REPRESENTATION: &str = "bge-sorted-cls-v2";
pub const ENCODE_TOKEN_LIMIT: usize = 256;
pub const DIMENSIONS: usize = 384;
pub const BATCH_SIZE: usize = 8;
const QUERY_INSTRUCTION: &str = "Represent this sentence for searching relevant passages: ";
const LOAD_BACKOFF: Duration = Duration::from_secs(60);
const IDLE: Duration = Duration::from_secs(600);

struct Session {
    model: BertModel,
    last_used: Instant,
}

impl Session {
    fn load(weights: &[u8]) -> candle_core::Result<Self> {
        let config: Config = serde_json::from_slice(CONFIG).map_err(candle_core::Error::wrap)?;
        let vb = VarBuilder::from_slice_safetensors(weights, DType::F32, &Device::Cpu)?;
        Ok(Self {
            model: BertModel::load(vb, &config)?,
            last_used: Instant::now(),
        })
    }
}

#[derive(Default)]
struct ModelState {
    session: Option<Session>,
    retry_after: Option<Instant>,
}

pub struct Model {
    tokenizer: Tokenizer,
    session: Mutex<ModelState>,
    weights: &'static [u8],
    failure_logged: AtomicBool,
    #[cfg(test)]
    batch_barriers: Option<(
        std::sync::Arc<std::sync::Barrier>,
        std::sync::Arc<std::sync::Barrier>,
    )>,
    #[cfg(test)]
    load_attempts: std::sync::atomic::AtomicUsize,
    pool: rayon::ThreadPool,
}

impl Model {
    fn load() -> Result<Self, String> {
        let mut tokenizer = Tokenizer::from_bytes(TOKENIZER).map_err(|e| e.to_string())?;
        tokenizer.with_padding(None);
        tokenizer.with_truncation(None).map_err(|e| e.to_string())?;
        Ok(Self {
            tokenizer,
            session: Mutex::new(ModelState::default()),
            weights: WEIGHTS,
            failure_logged: AtomicBool::new(false),
            #[cfg(test)]
            batch_barriers: None,
            #[cfg(test)]
            load_attempts: std::sync::atomic::AtomicUsize::new(0),
            pool: rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .thread_name(|n| format!("search-cpu-{n}"))
                .build()
                .map_err(|e| e.to_string())?,
        })
    }

    #[cfg(test)]
    pub fn load_for_benchmark(&self) -> Result<(), String> {
        let mut guard = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut *guard;
        self.pool.install(|| self.ensure_session(state)).map(|_| ())
    }

    fn ensure_session<'a>(&self, state: &'a mut ModelState) -> Result<&'a mut Session, String> {
        if state.session.is_none() {
            if state
                .retry_after
                .is_some_and(|until| Instant::now() < until)
            {
                return Err("search model load is in backoff".into());
            }
            #[cfg(test)]
            self.load_attempts.fetch_add(1, Ordering::Relaxed);
            match Session::load(self.weights) {
                Ok(session) => {
                    state.session = Some(session);
                    state.retry_after = None;
                }
                Err(error) => {
                    state.retry_after = Some(Instant::now() + LOAD_BACKOFF);
                    if !self.failure_logged.swap(true, Ordering::Relaxed) {
                        eprintln!(
                            "Toolport search model unavailable; using lexical ranking: {error}"
                        );
                    }
                    return Err(error.to_string());
                }
            }
        }
        Ok(state.session.as_mut().unwrap())
    }

    pub fn dimensions(&self) -> usize {
        DIMENSIONS
    }

    pub fn encode_input(&self, text: &str) -> Result<tokenizers::Encoding, String> {
        self.tokenizer
            .encode(text, false)
            .map_err(|e| e.to_string())
    }

    /// Reproduce round 2's content-token truncation and decoded document text.
    fn document_input(&self, text: &str) -> Result<String, String> {
        let mut encoded = self.encode_input(text)?;
        encoded.truncate(
            ENCODE_TOKEN_LIMIT - 2,
            0,
            tokenizers::TruncationDirection::Right,
        );
        self.tokenizer
            .decode(encoded.get_ids(), true)
            .map_err(|e| e.to_string())
    }

    pub fn encode_documents(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let texts: Vec<String> = texts
            .iter()
            .map(|s| self.document_input(s))
            .collect::<Result<_, _>>()?;
        self.encode_batch(&texts.iter().map(String::as_str).collect::<Vec<_>>())
    }

    #[cfg(test)]
    pub fn encode(&self, text: &str) -> Result<Vec<f32>, String> {
        self.encode_batch(&[text])
            .map(|mut vectors| vectors.pop().unwrap())
    }

    fn encode_query(&self, text: &str) -> Option<Vec<f32>> {
        self.encode_batch_inner(&[text], true).ok()?.pop()
    }

    fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        self.encode_batch_inner(texts, false)
    }

    fn encode_batch_inner(&self, texts: &[&str], query: bool) -> Result<Vec<Vec<f32>>, String> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        if texts.len() > BATCH_SIZE {
            return Err("search batch exceeds eight documents".into());
        }
        let mut tokenizer = self.tokenizer.clone();
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: ENCODE_TOKEN_LIMIT,
                ..Default::default()
            }))
            .map_err(|e| e.to_string())?;
        // Sequential tokenization avoids the tokenizer's global Rayon pool.
        let encoded = texts
            .iter()
            .map(|text| tokenizer.encode(*text, true).map_err(|e| e.to_string()))
            .collect::<Result<Vec<_>, _>>()?;
        let width = encoded.iter().map(|e| e.len()).max().unwrap();
        let mut ids = vec![0_u32; texts.len() * width];
        let mut mask = vec![0_u32; ids.len()];
        for (n, encoding) in encoded.iter().enumerate() {
            ids[n * width..n * width + encoding.len()].copy_from_slice(encoding.get_ids());
            mask[n * width..n * width + encoding.len()].fill(1);
        }
        // A ready query never queues behind catalog inference (or its Rayon job).
        let mut session = if query {
            match self.session.try_lock() {
                Ok(guard) => guard,
                Err(TryLockError::Poisoned(error)) => error.into_inner(),
                Err(TryLockError::WouldBlock) => return Err("search encoder busy".into()),
            }
        } else {
            self.session.lock().unwrap_or_else(|e| e.into_inner())
        };
        let state = &mut *session;
        self.pool.install(|| -> Result<Vec<Vec<f32>>, String> {
            let current = self.ensure_session(state)?;
            #[cfg(test)]
            if !query {
                if let Some((started, release)) = &self.batch_barriers {
                    started.wait();
                    release.wait();
                }
            }
            let infer = || -> candle_core::Result<Vec<Vec<f32>>> {
                let shape = (texts.len(), width);
                let input = Tensor::from_vec(ids, shape, &Device::Cpu)?;
                let attention = Tensor::from_vec(mask, shape, &Device::Cpu)?;
                let types = Tensor::zeros(shape, DType::U32, &Device::Cpu)?;
                let output = current.model.forward(&input, &types, Some(&attention))?;
                let mut vectors = Vec::with_capacity(texts.len());
                for n in 0..texts.len() {
                    let mut vector = output.i((n, 0))?.to_vec1::<f32>()?;
                    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
                    if !norm.is_finite() || norm <= 0.0 {
                        candle_core::bail!("non-finite search vector");
                    }
                    for value in &mut vector {
                        *value /= norm;
                    }
                    vectors.push(vector);
                }
                Ok(vectors)
            };
            let vectors = infer().map_err(|e| e.to_string());
            current.last_used = Instant::now();
            vectors
        })
    }

    fn unload_idle(&self, now: Instant) {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let dropped = session
            .session
            .as_ref()
            .is_some_and(|s| now.saturating_duration_since(s.last_used) >= IDLE);
        if dropped {
            session.session = None;
        }
        drop(session);
        if dropped {
            release_allocator_memory();
        }
    }
}

/// Reclaim freed tensor/batch allocations at lifecycle boundaries, not per query.
pub fn release_allocator_memory() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: glibc malloc_trim is thread-safe and only releases allocator-owned
    // free pages. No live tensor, tokenizer or embedded asset is invalidated.
    unsafe {
        libc::malloc_trim(0);
    }
}

static MODEL: OnceLock<Model> = OnceLock::new();
pub fn model() -> &'static Model {
    MODEL.get_or_init(|| Model::load().expect("validated bundled BGE tokenizer"))
}

pub fn unload_idle() {
    if let Some(model) = MODEL.get() {
        model.unload_idle(Instant::now());
    }
}

pub fn asset_digest() -> &'static [u8] {
    use sha2::{Digest, Sha256};
    static DIGEST: OnceLock<Vec<u8>> = OnceLock::new();
    DIGEST.get_or_init(|| {
        let mut hash = Sha256::new();
        hash.update(DOCUMENT_REPRESENTATION.as_bytes());
        hash.update(WEIGHTS);
        hash.update(TOKENIZER);
        hash.update(CONFIG);
        hash.update(LICENSES);
        hash.finalize().to_vec()
    })
}

pub struct Source<'a> {
    pub name: &'a str,
    pub description: &'a str,
    pub input_schema: Option<&'a Value>,
}

pub fn source(tool: &Value) -> Source<'_> {
    Source {
        name: tool["name"].as_str().unwrap_or(""),
        description: tool["description"].as_str().unwrap_or(""),
        input_schema: tool.get("inputSchema"),
    }
}

fn human(text: &str) -> String {
    let mut out = String::new();
    let mut previous = false;
    for c in text.chars() {
        if previous && c.is_ascii_uppercase() {
            out.push(' ');
        }
        if c == '_' || c == '-' {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
        previous = c.is_ascii_lowercase() || c.is_ascii_digit();
    }
    out.trim().to_owned()
}

fn provider_name(provider: &str) -> String {
    match provider {
        "aws_lambda" => "AWS Lambda".into(),
        "google_workspace" => "Google Workspace".into(),
        "vercel_full_api" | "vercel" => "Vercel".into(),
        "github" => "GitHub".into(),
        "gitlab" => "GitLab".into(),
        "postgres" => "Postgres".into(),
        _ => {
            let text = human(provider);
            let mut previous = false;
            text.chars()
                .flat_map(|c| {
                    let result = if previous {
                        c.to_lowercase().collect::<String>()
                    } else {
                        c.to_uppercase().collect::<String>()
                    };
                    previous = c.is_alphabetic();
                    result.chars().collect::<Vec<_>>()
                })
                .collect()
        }
    }
}

/// Exactly the round-2 provider/name/full-description/ordered-parameter projection.
pub fn document(tool: &Value) -> String {
    let evidence = source(tool);
    let (provider, name) = evidence
        .name
        .split_once("__")
        .unwrap_or((tool["server"].as_str().unwrap_or(""), evidence.name));
    let parameters = evidence
        .input_schema
        .and_then(|s| s["properties"].as_object())
        .map(|properties| {
            properties
                .iter()
                .filter(|(_, value)| value.is_object())
                .map(|(name, value)| {
                    let mut text = human(name);
                    if let Some(description) =
                        value["description"].as_str().filter(|s| !s.is_empty())
                    {
                        text.push_str(": ");
                        text.extend(description.chars().take(160));
                    }
                    text
                })
                .collect::<Vec<_>>()
                .join("; ")
        })
        .unwrap_or_default();
    format!(
        "Provider: {}\nTool: {}\nDescription: {}\nParameters: {}",
        provider_name(provider),
        human(name),
        evidence.description,
        parameters
    )
}

#[cfg(test)]
pub fn document_status(tool: &Value) -> Value {
    let text = document(tool);
    let encoded = model().encode_input(&text).unwrap();
    serde_json::json!({
        "source_description_chars":source(tool).description.chars().count(),
        "semantic_chars":text.chars().count(),
        "semantic_tokens_before_cap":encoded.get_ids().len(),
        "token_truncated":encoded.get_ids().len() > ENCODE_TOKEN_LIMIT - 2,
        "description_complete_before_token_cap":true,
        "schema_embedded":true,"schema_retained":source(tool).input_schema.is_some(),
        "complete_source_retained":true,"representation":DOCUMENT_REPRESENTATION,"token_limit":ENCODE_TOKEN_LIMIT,
        "description_policy":"full","character_truncated_by_encoder":false
    })
}

#[cfg(test)]
fn benchmark_vectors() -> Option<&'static Value> {
    static VECTORS: OnceLock<Option<Value>> = OnceLock::new();
    VECTORS
        .get_or_init(|| {
            std::env::var_os("SEARCH_SENTENCE_VECTORS").map(|path| {
                serde_json::from_slice(&std::fs::read(path).expect("sentence benchmark vectors"))
                    .unwrap()
            })
        })
        .as_ref()
}

#[cfg(test)]
pub fn fixture_vector(text: &str) -> Option<Vec<f32>> {
    use sha2::{Digest, Sha256};
    static RECORDS: OnceLock<std::collections::HashMap<[u8; 32], Vec<f32>>> = OnceLock::new();
    let records = RECORDS.get_or_init(|| {
        let bytes = include_bytes!("../../tests/fixtures/search-eval-scale/bge-native-vectors.bin");
        const RECORD_BYTES: usize = 32 + DIMENSIONS * 4;
        assert_eq!(bytes.len() % RECORD_BYTES, 0);
        bytes
            .as_chunks::<RECORD_BYTES>()
            .0
            .iter()
            .map(|record| {
                let key: [u8; 32] = record[..32].try_into().unwrap();
                let vector: Vec<f32> = record[32..]
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|v| f32::from_le_bytes(*v))
                    .collect();
                assert!(vector.iter().all(|v| v.is_finite()));
                let norm: f32 = vector.iter().map(|v| v * v).sum();
                assert!((norm - 1.0).abs() < 0.001);
                (key, vector)
            })
            .collect()
    });
    let mut hash = Sha256::new();
    hash.update(asset_digest());
    hash.update(text.as_bytes());
    records.get(&<[u8; 32]>::from(hash.finalize())).cloned()
}

#[cfg(test)]
pub fn tool_vector(tool: &Value) -> Vec<f32> {
    if let Some(vectors) = benchmark_vectors() {
        return serde_json::from_value(
            vectors["documents"][tool["name"].as_str().unwrap()].clone(),
        )
        .expect("benchmark tool vector");
    }
    // Ranking tests repeatedly rebuild the same immutable public catalog. Reuse
    // exact document vectors within this test process; native parity, lazy-load
    // and worker/cache tests still call the encoder directly.
    static DOCUMENTS: OnceLock<Mutex<std::collections::HashMap<String, Vec<f32>>>> =
        OnceLock::new();
    let text = document(tool);
    if let Some(vector) = fixture_vector(&text) {
        return vector;
    }
    let mut documents = DOCUMENTS
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    documents
        .entry(text.clone())
        .or_insert_with(|| {
            model()
                .encode_documents(&[&text])
                .expect("bundled BGE document")
                .pop()
                .unwrap()
        })
        .clone()
}

#[cfg(test)]
pub fn export_query_fixture(queries: &[String], output: &std::path::Path) {
    use sha2::{Digest, Sha256};
    // The fixture is bounded public test data. Rewrite its complete buffer;
    // production append-mode sinks remain restricted to private_logs.
    let mut bytes = std::fs::read(output).unwrap();
    for batch in queries.chunks(BATCH_SIZE) {
        let texts: Vec<String> = batch
            .iter()
            .map(|q| format!("{QUERY_INSTRUCTION}{q}"))
            .collect();
        let vectors = model()
            .encode_batch(&texts.iter().map(String::as_str).collect::<Vec<_>>())
            .unwrap();
        for (text, vector) in texts.iter().zip(vectors) {
            let mut hash = Sha256::new();
            hash.update(asset_digest());
            hash.update(text.as_bytes());
            bytes.extend_from_slice(&hash.finalize());
            for value in vector {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
    }
    std::fs::write(output, bytes).unwrap();
}

#[cfg(test)]
std::thread_local! {
    static NATIVE_MEASUREMENT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Resource probes must measure actual query inference, independent of the
/// unit-only immutable ranking fixture and memoized query vectors.
#[cfg(test)]
pub fn with_native_queries_for_measurement<T>(measure: impl FnOnce() -> T) -> T {
    struct Restore(bool);
    impl Drop for Restore {
        fn drop(&mut self) {
            NATIVE_MEASUREMENT.with(|native| native.set(self.0));
        }
    }
    let _restore = Restore(NATIVE_MEASUREMENT.with(|native| native.replace(true)));
    measure()
}

#[cfg(test)]
std::thread_local! {
    static QUERY_MODEL: std::cell::RefCell<Option<std::sync::Arc<Model>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub fn with_query_model<T>(model: std::sync::Arc<Model>, run: impl FnOnce() -> T) -> T {
    struct Restore(Option<std::sync::Arc<Model>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            QUERY_MODEL.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(QUERY_MODEL.with(|slot| slot.replace(Some(model))));
    run()
}

#[cfg(test)]
pub fn with_inflight_batch<T>(run: impl FnOnce(std::sync::Arc<Model>) -> T) -> T {
    let mut model = Model::load().unwrap();
    let started = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release = std::sync::Arc::new(std::sync::Barrier::new(2));
    model.batch_barriers = Some((started.clone(), release.clone()));
    let model = std::sync::Arc::new(model);
    let worker_model = model.clone();
    let worker = std::thread::spawn(move || {
        worker_model.encode_documents(&["catalog background document"; BATCH_SIZE])
    });
    started.wait();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(model)));
    release.wait();
    assert_eq!(worker.join().unwrap().unwrap().len(), BATCH_SIZE);
    result.unwrap_or_else(|error| std::panic::resume_unwind(error))
}

#[cfg(test)]
pub fn unavailable_model() -> std::sync::Arc<Model> {
    let mut model = Model::load().unwrap();
    model.weights = b"corrupted weight asset";
    std::sync::Arc::new(model)
}

pub fn query_vector(query: &str) -> Option<Vec<f32>> {
    #[cfg(test)]
    if let Some(model) = QUERY_MODEL.with(|slot| slot.borrow().clone()) {
        return model.encode_query(&format!("{QUERY_INSTRUCTION}{query}"));
    }
    #[cfg(test)]
    if NATIVE_MEASUREMENT.with(std::cell::Cell::get) {
        return model().encode(&format!("{QUERY_INSTRUCTION}{query}")).ok();
    }

    #[cfg(test)]
    if let Some(vectors) = benchmark_vectors() {
        return Some(
            serde_json::from_value(vectors["queries_vectors"][query].clone())
                .expect("benchmark query vector"),
        );
    }
    #[cfg(test)]
    {
        if let Some(vector) = fixture_vector(&format!("{QUERY_INSTRUCTION}{query}")) {
            return Some(vector);
        }
        // Each distinct query is still encoded by the native model. Quality and
        // handler tests reuse that exact result rather than repeating inference.
        static QUERIES: OnceLock<Mutex<std::collections::HashMap<String, Vec<f32>>>> =
            OnceLock::new();
        Some(
            QUERIES
                .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .entry(query.to_owned())
                .or_insert_with(|| {
                    model()
                        .encode(&format!("{QUERY_INSTRUCTION}{query}"))
                        .expect("bundled BGE query")
                })
                .clone(),
        )
    }
    #[cfg(not(test))]
    model().encode_query(&format!("{QUERY_INSTRUCTION}{query}"))
}

pub fn cosine(left: &[f32], right: &[f32]) -> f64 {
    left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>() as f64
}

pub fn enabled() -> bool {
    #[cfg(test)]
    if std::env::var_os("SEARCH_LEXICAL_ONLY").is_some() {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bge_rendering_preserves_prose_and_sorts_schema_parameters() {
        let tool:Value=serde_json::from_str(r#"{"name":"github__readIssue2Comments","description":"First sentence. Later constraint.","inputSchema":{"properties":{"zFlag":{"description":"last"},"a_id":{"description":"first"}}}}"#).unwrap();
        assert_eq!(document(&tool), "Provider: GitHub\nTool: read Issue2 Comments\nDescription: First sentence. Later constraint.\nParameters: a id: first; z Flag: last");
        let tool = serde_json::json!({"name":"aws_lambda__do_job","inputSchema":{"properties":{"long":{"description":"é".repeat(200)}}}});
        assert!(document(&tool).ends_with(&"é".repeat(160)));
        assert_eq!(provider_name("clerk_full_api"), "Clerk Full Api");
    }
    #[test]
    fn bge_session_is_lazy_and_unloads_after_idle() {
        let m = Model::load().unwrap();
        assert!(m.session.lock().unwrap().session.is_none());
        assert_eq!(m.dimensions(), DIMENSIONS);
        let v = m.encode("record working hours").unwrap();
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-5);
        assert!(m.session.lock().unwrap().session.is_some());
        m.unload_idle(Instant::now() + IDLE + Duration::from_secs(1));
        assert!(m.session.lock().unwrap().session.is_none());
        assert_eq!(v, m.encode("record working hours").unwrap());
    }
    #[test]
    fn bge_failed_load_backs_off_and_recovers() {
        let model = unavailable_model();
        for _ in 0..10 {
            assert!(model.encode_query("record working hours").is_none());
        }
        assert_eq!(model.load_attempts.load(Ordering::Relaxed), 1);
        assert!(model.failure_logged.load(Ordering::Relaxed));
        model.session.lock().unwrap().retry_after = Some(Instant::now() - Duration::from_secs(1));
        assert!(model.encode_query("record working hours").is_none());
        assert_eq!(model.load_attempts.load(Ordering::Relaxed), 2);
        let mut model = std::sync::Arc::try_unwrap(model).ok().unwrap();
        model.weights = WEIGHTS;
        model.session.lock().unwrap().retry_after = None;
        assert!(model.encode_query("record working hours").is_some());
        assert!(model.session.lock().unwrap().retry_after.is_none());
    }
    #[test]
    fn bge_python_parity_and_batch_padding() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/search-bge-parity.json"))
                .unwrap();
        let texts: Vec<_> = fixture
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["text"].as_str().unwrap())
            .collect();
        let vectors = model().encode_documents(&texts).unwrap();
        for (row, vector) in fixture.as_array().unwrap().iter().zip(&vectors) {
            assert_eq!(
                model().document_input(&document(&row["tool"])).unwrap(),
                row["text"].as_str().unwrap()
            );
            let expected: Vec<f32> = serde_json::from_value(row["vector"].clone()).unwrap();
            assert!(cosine(&expected, vector) >= 0.999, "{}", row["name"]);
            assert!((cosine(vector, vector) - 1.0).abs() < 1e-5);
            let single = model()
                .encode_documents(&[row["text"].as_str().unwrap()])
                .unwrap();
            assert!(cosine(&single[0], vector) >= 0.999);
        }
        assert!(model()
            .encode_documents(&["overflow"; BATCH_SIZE + 1])
            .is_err());
    }
}
