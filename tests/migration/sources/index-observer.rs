// Appended observers expose state contracts without changing provider transitions.
fn current_utc_date() -> Option<String> {
    Some("2026-10-04".into())
}

pub fn observe_index_migration(request: &Value) -> Value {
    let raw = request["input"].as_str().unwrap();
    match request["kind"].as_str().unwrap() {
        "cnki_workflow" => observe_cnki_workflow(raw),
        "cnki_anchor" => match decode_domestic_anchor(raw) {
            Ok(state) => serde_json::json!({"encoded":encode_domestic_anchor(&state).unwrap()}),
            Err(error) => {
                serde_json::json!({"error":error.to_string(),"kind":format!("{:?}",error.kind())})
            }
        },
        "cnki_checkpoint" => match decode_domestic_checkpoint(raw) {
            Ok(state) => serde_json::json!({"encoded":encode_domestic_checkpoint(&state).unwrap()}),
            Err(error) => {
                serde_json::json!({"error":error.to_string(),"kind":format!("{:?}",error.kind())})
            }
        },
        "cnki_article" => {
            let input: Value = serde_json::from_str(raw).unwrap();
            let catalog = serde_json::from_value(input["catalog"].clone()).unwrap();
            let issue = cnki_issue_draft(&catalog, &input["issue"]).unwrap();
            serde_json::json!({"issue":issue,"article":cnki_article_draft(&catalog,&issue,&input["summary"],&input["detail"]),"skip":domestic_cnki_lacks_authors_and_doi(&input["summary"],&input["detail"])})
        }
        "workflow" => observe_index_workflow(request),
        "checkpoint" => match decode_scholarly_checkpoint(raw) {
            Ok(checkpoint) => {
                serde_json::json!({"encoded":encode_scholarly_checkpoint(&checkpoint).unwrap()})
            }
            Err(error) => {
                serde_json::json!({"error":error.to_string(),"kind":format!("{:?}",error.kind())})
            }
        },
        "window" => {
            let input: Value = serde_json::from_str(raw).unwrap();
            let window = serde_json::from_value(input["window"].clone()).unwrap();
            let anchors =
                serde_json::from_value::<Vec<Option<ScholarlyAnchor>>>(input["anchors"].clone())
                    .unwrap();
            match plan_scholarly_page_window(
                window,
                &anchors,
                input["unknown"].as_bool().unwrap(),
                input["next"].as_bool().unwrap(),
            ) {
                Ok(plan) => {
                    serde_json::json!({"selected":plan.selected_indices,"window":serde_json::to_string(&plan.window).unwrap(),"progress":match plan.progress{ScholarlyPageProgress::Continue=>"continue",ScholarlyPageProgress::Complete=>"complete",ScholarlyPageProgress::ReplayUnbounded=>"replay_unbounded"}})
                }
                Err(error) => {
                    serde_json::json!({"error":error.to_string(),"kind":format!("{:?}",error.kind())})
                }
            }
        }
        "context" => {
            let input: Value = serde_json::from_str(raw).unwrap();
            let context = IndexFetchContext {
                mode: serde_json::from_value(input["mode"].clone()).unwrap(),
                committed_anchor: input["anchor"].as_str(),
                traversal_checkpoint: input["checkpoint"].as_str(),
            };
            match scholarly_window_from_context(context) {
                Ok((window, source)) => {
                    serde_json::json!({"window":serde_json::to_string(&window).unwrap(),"source":source.as_ref().map(|source|serde_json::to_string(source).unwrap()),"filter":scholarly_window_filter(&window)})
                }
                Err(error) => {
                    serde_json::json!({"error":error.to_string(),"kind":format!("{:?}",error.kind())})
                }
            }
        }
        "article" => {
            let input: Value = serde_json::from_str(raw).unwrap();
            let catalog = serde_json::from_value(input["catalog"].clone()).unwrap();
            let result = if input["provider"] == "crossref" {
                scholarly_article_draft(
                    &catalog,
                    &input["work"],
                    input.get("openalex").filter(|value| !value.is_null()),
                    input
                        .get("semantic_scholar")
                        .filter(|value| !value.is_null()),
                )
            } else {
                openalex_article_draft(&catalog, &input["work"])
            };
            serde_json::json!({"article":result})
        }
        "scope" => {
            let input: Value = serde_json::from_str(raw).unwrap();
            serde_json::json!({"scope":workset_scope(&serde_json::from_value(input["catalog"].clone()).unwrap(),&serde_json::from_value(input["window"].clone()).unwrap()).unwrap()})
        }
        _ => panic!("unknown index observation"),
    }
}

fn observe_cnki_workflow(raw: &str) -> Value {
    let input: Value = serde_json::from_str(raw).unwrap();
    let catalog = serde_json::from_value(input["catalog"].clone()).unwrap();
    let fixture = crate::FixtureDomesticCnkiTransport::new(
        serde_json::from_value(input["fixture"].clone()).unwrap(),
    );
    let provider = DomesticCnkiIndexProvider::with_worker_count(
        fixture,
        input["workers"].as_u64().unwrap_or(1) as usize,
    )
    .unwrap();
    let mode = serde_json::from_value(input["mode"].clone()).unwrap();
    let mut checkpoint = input["checkpoint"].as_str().map(str::to_owned);
    let mut events = Vec::new();
    for step in 0..80 {
        let result = provider.fetch(
            &catalog,
            IndexFetchContext {
                mode,
                committed_anchor: input["anchor"].as_str(),
                traversal_checkpoint: checkpoint.as_deref(),
            },
        );
        match result {
            Err(error) => {
                events.push(serde_json::json!({"error":error.to_string(),"kind":format!("{:?}",error.kind())}));
                break;
            }
            Ok(batch) => {
                let is_complete = matches!(batch.progress, ProviderProgress::Complete { .. });
                if !input["replay_at"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().any(|value| value.as_u64() == Some(step)))
                {
                    checkpoint = match &batch.progress {
                        ProviderProgress::Continue { checkpoint } => Some(checkpoint.clone()),
                        _ => None,
                    };
                }
                events.push(serde_json::json!({"batch":batch}));
                if is_complete {
                    break;
                }
                assert!(step < 79, "workflow exceeded bound");
            }
        }
    }
    serde_json::json!({"events":events})
}

fn observe_index_workflow(request: &Value) -> Value {
    let input: Value = serde_json::from_str(request["input"].as_str().unwrap()).unwrap();
    let catalog = serde_json::from_value(input["catalog"].clone()).unwrap();
    let mode = serde_json::from_value(input["mode"].clone()).unwrap();
    let has_key = input["has_key"].as_bool().unwrap_or(false);
    let fixture = crate::FixtureScholarlyTransport::new(
        serde_json::from_value(input["fixture"].clone()).unwrap(),
    );
    let mut client = ScholarlyClient::new(fixture, has_key);
    let root = Path::new(request["root"].as_str().unwrap());
    let mut checkpoint = input["checkpoint"].as_str().map(str::to_owned);
    let mut events = Vec::new();
    let token_pattern = regex::Regex::new(r#""token":"[0-9a-f]{32}""#).unwrap();
    for step in 0..80 {
        let result = fetch_scholarly_batch_in_workset(
            &mut client,
            &catalog,
            IndexFetchContext {
                mode,
                committed_anchor: input["anchor"].as_str(),
                traversal_checkpoint: checkpoint.as_deref(),
            },
            has_key,
            root,
            &mut || Ok(1_791_072_000),
        );
        let attempts = client.drain_attempts();
        match result {
            Err(error) => {
                events.push(serde_json::json!({"error":error.to_string(),"kind":format!("{:?}",error.kind()),"attempts":attempts}));
                break;
            }
            Ok(batch) => {
                let is_complete = matches!(batch.progress, ProviderProgress::Complete { .. });
                if !input["replay_at"]
                    .as_array()
                    .is_some_and(|steps| steps.iter().any(|value| value.as_u64() == Some(step)))
                {
                    checkpoint = match &batch.progress {
                        ProviderProgress::Continue { checkpoint } => Some(checkpoint.clone()),
                        _ => None,
                    };
                }
                let mut encoded = serde_json::to_value(batch).unwrap();
                if let Some(raw) = encoded["progress"]["checkpoint"].as_str() {
                    encoded["progress"]["checkpoint"] = Value::String(
                        token_pattern
                            .replace_all(raw, r#""token":"00000000000000000000000000000001""#)
                            .into_owned(),
                    );
                }
                events.push(serde_json::json!({"batch":encoded,"attempts":attempts}));
                if is_complete {
                    break;
                }
                assert!(step < 79, "workflow exceeded bound");
            }
        }
    }
    serde_json::json!({"events":events})
}
