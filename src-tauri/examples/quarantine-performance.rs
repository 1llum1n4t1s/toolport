//! Offline quarantine benchmark. Always uses a private temporary data directory.
use conduit_lib::{integrity, registry::DataDirTestEnv};
use serde_json::json;
use std::{hint::black_box, time::Instant};

fn measure(mut work: impl FnMut(), samples: usize) -> serde_json::Value {
    work();
    let mut times = Vec::new();
    for _ in 0..samples {
        let start = Instant::now();
        work();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    json!({"median_ms":times[samples / 2], "p95_ms":times[(samples * 95).div_ceil(100) - 1]})
}

fn main() {
    let _data = DataDirTestEnv::new("quarantine-performance");
    let catalog: Vec<_> = (0..3000).map(|i| json!({
        "name":format!("srv__tool_{i}"),
        "description":"A captured definition with realistic schema help. ".repeat(40),
        "inputSchema":{"type":"object","properties":{"text":{"type":"string", "description":"Fixture parameter"}}},
        "annotations":{"readOnlyHint":true},
    })).collect();
    integrity::apply_quarantine(None, &catalog, &[json!({"type":"pins_load_failed", "change":"tamper", "severity":"high"})]).unwrap();
    let cold = Instant::now();
    let records = integrity::all_quarantined_summaries().unwrap();
    let cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(records.len(), 6000);
    let bytes = serde_json::to_vec(&records).unwrap().len();
    let list = measure(|| { black_box(integrity::all_quarantined_summaries().unwrap()); }, 9);
    let ipc = measure(|| { black_box(serde_json::to_vec(&records).unwrap()); }, 9);
    let definitions = measure(|| {
        for row in &records {
            if let Some(definition) = row.get("definition") {
                black_box(serde_json::to_string_pretty(definition).unwrap());
            }
        }
    }, 9);
    println!("{}", json!({"profile":"debug", "cold_list_ms":cold_ms, "tools":3000, "records":records.len(), "ipc_bytes":bytes, "list":list, "ipc_encode":ipc, "closed_rows_definition_format":definitions, "store_bytes":std::fs::metadata(_data.dir.join("quarantine.json")).unwrap().len()}));
}
