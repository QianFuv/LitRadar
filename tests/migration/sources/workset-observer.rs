// Appended observations call the original implementation without changing its state transitions.
pub fn observe_migration(request: &Value) -> Value {
    let input = request["input"].as_str().unwrap_or("null");
    match request["kind"].as_str().unwrap() {
        "workflow" => observe_workset_workflow(request),
        "checkpoint" => match serde_json::from_str::<CrossrefCheckpoint>(input) {
            Err(_) => serde_json::json!({"decoded":false}),
            Ok(state) => {
                let query=state.query().map(|query|match query{CrossrefQuery::EarliestCreated{until}=>serde_json::json!({"kind":"discover","until":until}),CrossrefQuery::Works{created_from,created_until,updated_from,updated_until,cursor}=>serde_json::json!({"kind":"collect","from":created_from,"until":created_until,"updated_from":updated_from,"updated_until":updated_until,"cursor":cursor})});
                serde_json::json!({"decoded":true,"encoded":encode(&state).unwrap(),"validation":state.validate().err().map(|error|error.to_string()),"query":query.ok()})
            }
        },
        "anchor" => match serde_json::from_str::<ScholarlyAnchor>(input) {
            Err(_) => serde_json::json!({"decoded":false}),
            Ok(anchor) => {
                serde_json::json!({"decoded":true,"encoded":encode(&anchor).unwrap(),"valid":crate::providers::is_valid_scholarly_anchor(&anchor)})
            }
        },
        "work" => {
            let work: Value = serde_json::from_str(input).unwrap();
            let created = created_second(&work);
            let date = crossref_date(&work);
            let anchor = crossref_work_issue_anchor(&work);
            let payload = consumed_payload(work);
            let serialized = encode(&payload).unwrap();
            let key = work_key(&payload, &serialized);
            let order = crossref_order(&payload, &key).unwrap();
            serde_json::json!({"created":created.as_ref().ok().map(|value|value.to_string()),"created_error":created.err().map(|error|error.to_string()),"date":date,"anchor":anchor.as_ref().map(|anchor|encode(anchor).unwrap()),"payload":serialized,"key":key,"order":{"date":order.date,"fingerprint":order.fingerprint,"anchor":order.anchor,"year":order.year,"volume":order.volume,"issue":order.issue}})
        }
        _ => panic!("unknown observation"),
    }
}

fn workset_snapshot(cache: &CrossrefWorkset) -> Value {
    let counts: Vec<i64> = ["partitions", "works", "groups"]
        .iter()
        .map(|table| {
            cache
                .connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        })
        .collect();
    let previous: Option<String> = cache
        .connection
        .query_row("SELECT previous FROM metadata WHERE id=1", [], |row| {
            row.get(0)
        })
        .unwrap();
    serde_json::json!({"state":encode(&cache.state).unwrap(),"counts":counts,"previous":previous})
}

fn observe_workset_workflow(request: &Value) -> Value {
    let input: Value = serde_json::from_str(request["input"].as_str().unwrap()).unwrap();
    let initial: CrossrefCheckpoint = serde_json::from_value(input["initial"].clone()).unwrap();
    let root = Path::new(request["root"].as_str().unwrap());
    let (mut cache, replay) = if input["open"].as_bool() == Some(true) {
        CrossrefWorkset::open(root, "catalog", &initial).unwrap()
    } else {
        (
            CrossrefWorkset::create(root, "catalog", initial.clone()).unwrap(),
            false,
        )
    };
    let mut states = vec![initial];
    let mut events = vec![serde_json::json!({"snapshot":workset_snapshot(&cache),"replay":replay})];
    for action in input["actions"].as_array().unwrap() {
        let mut event = serde_json::json!({});
        match action["kind"].as_str().unwrap() {
            "accept" => {
                let page = CrossrefWorksPage {
                    items: action["page"]["items"].as_array().unwrap().clone(),
                    total_results: action["page"]["total_results"].as_u64().unwrap(),
                    next_cursor: action["page"]["next_cursor"].as_str().map(str::to_owned),
                };
                event["error"] =
                    serde_json::json!(cache.accept(page).err().map(|error| error.to_string()));
            }
            "seal" => {
                let lower = action["lower"].as_str().map(str::to_owned);
                let candidate = action
                    .get("candidate")
                    .filter(|value| !value.is_null())
                    .map(|value| serde_json::from_value(value.clone()).unwrap());
                event["error"] = serde_json::json!(
                    cache
                        .seal_selection(
                            action["upper"].as_str().unwrap().to_owned(),
                            lower,
                            candidate
                        )
                        .err()
                        .map(|error| error.to_string())
                );
            }
            "emit" => {
                let after: Option<EmissionKey> = action
                    .get("after")
                    .filter(|value| !value.is_null())
                    .map(|value| serde_json::from_value(value.clone()).unwrap());
                match cache.emit(
                    action["upper"].as_str().unwrap(),
                    action["lower"].as_str(),
                    after.as_ref(),
                ) {
                    Ok(page) => {
                        event["page"] = serde_json::json!({"works":page.works.iter().map(|work|encode(work).unwrap()).collect::<Vec<_>>(),"after":page.after,"has_more":page.has_more});
                    }
                    Err(error) => {
                        event["error"] = serde_json::json!(error.to_string());
                    }
                }
            }
            "groups" => {
                let group = cache.first_group().unwrap();
                event["first"] = group.map(|group|serde_json::json!({"anchor":encode(&group.anchor).unwrap(),"date":group.date})).unwrap_or(Value::Null);
                event["unknown"] = serde_json::json!(cache.has_unknown_groups().unwrap());
            }
            "reopen" => {
                let checkpoint = &states[action["index"].as_u64().unwrap() as usize];
                drop(cache);
                let (restored, replay) =
                    CrossrefWorkset::open(root, "catalog", checkpoint).unwrap();
                cache = restored;
                event["replay"] = serde_json::json!(replay);
            }
            _ => panic!("unknown workflow operation"),
        }
        states.push(cache.checkpoint());
        event["snapshot"] = workset_snapshot(&cache);
        events.push(event);
    }
    serde_json::json!({"events":events})
}
