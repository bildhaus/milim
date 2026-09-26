//! Retrieval regression benchmark over a checked-in labelled fixture.
//!
//! The fixture's memories are registered through the public API, then the
//! production `MemoryStore::benchmark` path scores the labelled queries. The
//! keyword-only run uses an embedder that is always unavailable, so ranking is
//! pure FTS5/BM25. The hybrid run uses the deterministic pseudo-embedding test
//! backend so rank fusion is exercised without a real model. Floors sit a
//! small margin below the measured values: a drop means retrieval regressed.

use std::collections::HashMap;
use std::sync::Arc;

use milim_inference::SharedService;
use milim_memory::{
    MemoryBenchmarkCase, MemoryBenchmarkReport, MemoryEventInput, MemoryNodeInput,
    MemoryScopeInput, MemoryScopeRef, MemoryStore,
};
use milim_storage::Database;
use serde_json::Value;

const FIXTURE: &str = include_str!("fixtures/retrieval.json");

struct Floors {
    recall_at_k: f32,
    mean_reciprocal_rank: f32,
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("fixture field {key} must be a string"))
}

fn scope_input(scopes: &Value, name: &str) -> MemoryScopeInput {
    let scope = scopes
        .get(name)
        .unwrap_or_else(|| panic!("fixture scope {name} is not defined"));
    MemoryScopeInput {
        kind: text(scope, "kind").to_string(),
        label: text(scope, "label").to_string(),
        locator: text(scope, "locator").to_string(),
    }
}

async fn run_fixture(embedder: SharedService) -> MemoryBenchmarkReport {
    let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture is valid JSON");
    let scopes = &fixture["scopes"];
    let store = MemoryStore::new(Database::open_in_memory().unwrap(), embedder).unwrap();

    let mut node_ids = HashMap::new();
    for memory in fixture["memories"].as_array().expect("memories array") {
        let registration = store
            .register(
                "fixture-embedding",
                scope_input(scopes, text(memory, "scope")),
                MemoryNodeInput {
                    kind: text(memory, "kind").to_string(),
                    title: text(memory, "title").to_string(),
                    body: text(memory, "body").to_string(),
                    confidence: 1.0,
                    source: "fixture".to_string(),
                },
                Vec::new(),
                MemoryEventInput::default(),
            )
            .await
            .unwrap();
        let previous = node_ids.insert(text(memory, "id").to_string(), registration.node.id);
        assert!(previous.is_none(), "duplicate fixture memory id");
    }

    let cases = fixture["queries"]
        .as_array()
        .expect("queries array")
        .iter()
        .map(|query| MemoryBenchmarkCase {
            name: text(query, "name").to_string(),
            query: text(query, "query").to_string(),
            relevant_node_ids: query["relevant"]
                .as_array()
                .expect("relevant array")
                .iter()
                .map(|id| {
                    let id = id.as_str().expect("relevant id is a string");
                    node_ids
                        .get(id)
                        .unwrap_or_else(|| panic!("unknown relevant memory {id}"))
                        .clone()
                })
                .collect(),
            scopes: query
                .get("scopes")
                .and_then(Value::as_array)
                .map(|names| {
                    names
                        .iter()
                        .map(|name| {
                            let scope = scope_input(scopes, name.as_str().unwrap());
                            MemoryScopeRef {
                                kind: scope.kind,
                                locator: scope.locator,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect::<Vec<_>>();
    assert!(node_ids.len() >= 30, "fixture needs at least 30 memories");
    assert!(cases.len() >= 20, "fixture needs at least 20 queries");

    let top_k = fixture["top_k"].as_u64().expect("top_k") as usize;
    store
        .benchmark("fixture-embedding", cases, top_k, false)
        .await
        .unwrap()
}

fn assert_floors(label: &str, report: &MemoryBenchmarkReport, floors: Floors) {
    let misses = report
        .cases
        .iter()
        .filter(|case| case.first_relevant_rank != Some(1))
        .map(|case| format!("{} (rank {:?})", case.name, case.first_relevant_rank))
        .collect::<Vec<_>>();
    println!(
        "{label}: {} cases, recall@{} = {:.3}, MRR = {:.3}; not ranked first: {misses:?}",
        report.case_count, report.top_k, report.recall_at_k, report.mean_reciprocal_rank
    );
    assert!(
        report.recall_at_k >= floors.recall_at_k,
        "{label} recall@{} {:.3} fell below the {:.3} floor",
        report.top_k,
        report.recall_at_k,
        floors.recall_at_k
    );
    assert!(
        report.mean_reciprocal_rank >= floors.mean_reciprocal_rank,
        "{label} MRR {:.3} fell below the {:.3} floor",
        report.mean_reciprocal_rank,
        floors.mean_reciprocal_rank
    );
}

#[tokio::test]
async fn keyword_only_retrieval_meets_recall_and_mrr_floors() {
    let embedder: SharedService = Arc::new(milim_inference::unavailable::UnavailableBackend::new());
    let report = run_fixture(embedder).await;
    assert_floors(
        "keyword-only",
        &report,
        // Measured: recall@5 0.976, MRR 0.946 (stable across runs).
        Floors {
            recall_at_k: 0.95,
            mean_reciprocal_rank: 0.90,
        },
    );
}

#[tokio::test]
async fn hybrid_retrieval_with_pseudo_embeddings_meets_recall_and_mrr_floors() {
    let embedder: SharedService = Arc::new(milim_inference::test_backend::TestBackend::new());
    let report = run_fixture(embedder).await;
    assert_floors(
        "hybrid (pseudo-embedding)",
        &report,
        // The pseudo-embedding is a byte histogram, so its semantic ranking is
        // mostly noise, and fusion ties break on random node ids. Measured
        // across runs: recall@5 0.714-0.738, MRR 0.466-0.493. The floor
        // catches fusion burying lexical matches, not embedding quality.
        Floors {
            recall_at_k: 0.65,
            mean_reciprocal_rank: 0.42,
        },
    );
}
