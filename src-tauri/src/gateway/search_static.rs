//! Offline BGE CLS embeddings, with a lazy, idle-unloaded CPU session.
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
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

pub const DOCUMENT_REPRESENTATION: &str = "bge-round2-cls-v1";
pub const ENCODE_TOKEN_LIMIT: usize = 256;
pub const DIMENSIONS: usize = 384;
pub const BATCH_SIZE: usize = 8;
const QUERY_INSTRUCTION: &str = "Represent this sentence for searching relevant passages: ";
const IDLE: Duration = Duration::from_secs(600);

struct Session {
    model: BertModel,
    last_used: Instant,
}

impl Session {
    fn load() -> candle_core::Result<Self> {
        let config: Config = serde_json::from_slice(CONFIG).map_err(candle_core::Error::wrap)?;
        let vb = VarBuilder::from_slice_safetensors(WEIGHTS, DType::F32, &Device::Cpu)?;
        Ok(Self {
            model: BertModel::load(vb, &config)?,
            last_used: Instant::now(),
        })
    }
}

pub struct Model {
    tokenizer: Tokenizer,
    session: Mutex<Option<Session>>,
    pool: rayon::ThreadPool,
}

impl Model {
    fn load() -> Result<Self, String> {
        let mut tokenizer = Tokenizer::from_bytes(TOKENIZER).map_err(|e| e.to_string())?;
        tokenizer.with_padding(None);
        tokenizer.with_truncation(None).map_err(|e| e.to_string())?;
        Ok(Self {
            tokenizer,
            session: Mutex::new(None),
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
        self.pool
            .install(|| -> candle_core::Result<()> {
                if state.is_none() {
                    *state = Some(Session::load()?);
                }
                Ok(())
            })
            .map_err(|e| e.to_string())
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

    pub fn encode(&self, text: &str) -> Vec<f32> {
        self.encode_batch(&[text])
            .ok()
            .and_then(|mut vectors| vectors.pop())
            .unwrap_or_else(|| vec![0.0; DIMENSIONS])
    }

    fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
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
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        let state = &mut *session;
        self.pool
            .install(|| -> candle_core::Result<Vec<Vec<f32>>> {
                if state.is_none() {
                    *state = Some(Session::load()?);
                }
                let current = state.as_mut().unwrap();
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
                current.last_used = Instant::now();
                Ok(vectors)
            })
            .map_err(|e| e.to_string())
    }

    fn unload_idle(&self, now: Instant) {
        let mut session = self.session.lock().unwrap_or_else(|e| e.into_inner());
        if session
            .as_ref()
            .is_some_and(|s| now.saturating_duration_since(s.last_used) >= IDLE)
        {
            *session = None;
        }
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
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(output)
        .unwrap();
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
            file.write_all(&hash.finalize()).unwrap();
            for value in vector {
                file.write_all(&value.to_le_bytes()).unwrap();
            }
        }
    }
}

pub fn query_vector(query: &str) -> Vec<f32> {
    #[cfg(test)]
    if let Some(vectors) = benchmark_vectors() {
        return serde_json::from_value(vectors["queries_vectors"][query].clone())
            .expect("benchmark query vector");
    }
    #[cfg(test)]
    {
        if let Some(vector) = fixture_vector(&format!("{QUERY_INSTRUCTION}{query}")) {
            return vector;
        }
        // Each distinct query is still encoded by the native model. Quality and
        // handler tests reuse that exact result rather than repeating inference.
        static QUERIES: OnceLock<Mutex<std::collections::HashMap<String, Vec<f32>>>> =
            OnceLock::new();
        QUERIES
            .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(query.to_owned())
            .or_insert_with(|| model().encode(&format!("{QUERY_INSTRUCTION}{query}")))
            .clone()
    }
    #[cfg(not(test))]
    model().encode(&format!("{QUERY_INSTRUCTION}{query}"))
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
    fn bge_rendering_preserves_prose_and_schema_order() {
        let tool:Value=serde_json::from_str(r#"{"name":"github__readIssue2Comments","description":"First sentence. Later constraint.","inputSchema":{"properties":{"zFlag":{"description":"last"},"a_id":{"description":"first"}}}}"#).unwrap();
        assert_eq!(document(&tool), "Provider: GitHub\nTool: read Issue2 Comments\nDescription: First sentence. Later constraint.\nParameters: z Flag: last; a id: first");
        let tool = serde_json::json!({"name":"aws_lambda__do_job","inputSchema":{"properties":{"long":{"description":"é".repeat(200)}}}});
        assert!(document(&tool).ends_with(&"é".repeat(160)));
        assert_eq!(provider_name("clerk_full_api"), "Clerk Full Api");
    }
    #[test]
    fn bge_session_is_lazy_and_unloads_after_idle() {
        let m = Model::load().unwrap();
        assert!(m.session.lock().unwrap().is_none());
        assert_eq!(m.dimensions(), DIMENSIONS);
        let v = m.encode("record working hours");
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-5);
        assert!(m.session.lock().unwrap().is_some());
        m.unload_idle(Instant::now() + IDLE + Duration::from_secs(1));
        assert!(m.session.lock().unwrap().is_none());
        assert_eq!(v, m.encode("record working hours"));
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
