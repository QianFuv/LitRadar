//! Observe scholarly client operations against the unchanged Rust fixture transport.
use litradar_sources::{CrossrefQuery, FixtureScholarlyTransport, ScholarlyClient, SourceError};
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn outcome(result: Result<Value, SourceError>) -> Value {
    match result {
        Ok(value) => json!({"ok":value}),
        Err(error) => {
            let detail = match &error {
                SourceError::HttpStatus {
                    service,
                    endpoint,
                    status_code,
                    body,
                } => {
                    json!({"kind":"HttpStatus","service":service,"endpoint":endpoint,"status_code":status_code,"body":body,"display":error.to_string()})
                }
                SourceError::InvalidFixture(message) => {
                    json!({"kind":"InvalidFixture","message":message,"display":error.to_string()})
                }
                SourceError::Configuration(message) => {
                    json!({"kind":"Configuration","message":message,"display":error.to_string()})
                }
                SourceError::Request {
                    service,
                    endpoint,
                    message,
                } => {
                    json!({"kind":"Request","service":service,"endpoint":endpoint,"message":message,"display":error.to_string()})
                }
            };
            json!({"error":detail})
        }
    }
}

fn strings(value: &Value) -> Vec<String> {
    serde_json::from_value(value.clone()).unwrap()
}

fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let data = serde_json::from_value(request["fixture"].clone()).unwrap();
        let mut client = ScholarlyClient::new(
            FixtureScholarlyTransport::new(data),
            request["has_semantic_scholar_key"]
                .as_bool()
                .unwrap_or(false),
        );
        let mut observations = Vec::new();
        for operation in request["operations"].as_array().unwrap() {
            let result = match operation["op"].as_str().unwrap() {
                "crossref" => {
                    let query = &operation["query"];
                    let query = if query["kind"] == "earliest_created" {
                        CrossrefQuery::EarliestCreated { until:query["until"].as_i64().unwrap() }
                    } else {
                        CrossrefQuery::Works { created_from:query["created_from"].as_i64().unwrap(),created_until:query["created_until"].as_i64().unwrap(),updated_from:query["updated_from"].as_str().map(str::to_string),updated_until:query["updated_until"].as_i64(),cursor:query["cursor"].as_str().map(str::to_string) }
                    };
                    client.fetch_crossref_page(operation["issn"].as_str().unwrap(),&query).map(|page|json!({"items":page.items,"total_results":page.total_results,"next_cursor":page.next_cursor}))
                },
                "issns" => client.fetch_openalex_source_by_issns(&strings(&operation["issns"])).map(|value|json!(value)),
                "title" => client.fetch_openalex_source_by_title(operation["title"].as_str().unwrap()).map(|value|json!(value)),
                "source_page" => client.fetch_openalex_works_by_source_page(operation["source_id"].as_str().unwrap(),operation["date"].as_str(),operation["cursor"].as_str()).map(|page|json!({"items":page.items,"next_cursor":page.next_cursor,"did_fallback_to_unfiltered":page.did_fallback_to_unfiltered})),
                "openalex_dois" => client.fetch_openalex_by_dois(&strings(&operation["dois"]),operation["batch_size"].as_u64().unwrap() as usize).map(|value|json!(value)),
                "s2_dois" => client.fetch_semantic_scholar_by_dois(&strings(&operation["dois"]),operation["batch_size"].as_u64().unwrap() as usize).map(|value|json!(value)),
                "drain" => Ok(json!(client.drain_attempts())),
                "normalize" => Ok(json!(litradar_sources::normalize_doi(operation.get("value")))),
                _ => panic!("unknown operation"),
            };
            let captured = client.clone().into_transport();
            observations.push(json!({"result":outcome(result),"attempts":client.attempts(),"captures":{
                "semantic_scholar_batches":captured.semantic_scholar_batches(),"openalex_doi_batches":captured.openalex_doi_batches(),"source_lookup_issns":captured.source_lookup_issns(),"source_lookup_titles":captured.source_lookup_titles(),"journal_work_requests":captured.journal_work_requests(),"source_work_requests":captured.source_work_requests()
            }}));
        }
        request["output"] = json!(observations);
        writeln!(output, "{request}")?;
    }
    output.flush()
}
