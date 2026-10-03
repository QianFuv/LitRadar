//! Observe public domestic CNKI parsers without changing their original implementation.
use litradar_sources::*;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn outcome(result: Result<Value, DomesticCnkiSourceError>) -> Value {
    match result {
        Ok(value) => json!({"ok": value}),
        Err(error) => {
            let kind = match &error {
                DomesticCnkiSourceError::Request(_) => "Request",
                DomesticCnkiSourceError::Parse(_) => "Parse",
                DomesticCnkiSourceError::MissingFixture(_) => "MissingFixture",
                DomesticCnkiSourceError::PermanentArticleMissing => "PermanentArticleMissing",
                DomesticCnkiSourceError::Source(_) => "Source",
            };
            json!({"error":{"kind":kind,"display":error.to_string()}})
        }
    }
}

fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let text = request["text"].as_str().unwrap_or("");
        let result = match request["kind"].as_str().unwrap() {
            "search" => parse_domestic_journal_search_results(text).map(|value| json!(value)),
            "detail" => parse_domestic_journal_detail(text),
            "years" => parse_domestic_year_issues(text).map(|value| json!(value)),
            "papers" => parse_domestic_issue_articles(text, &request["issue"], request["page"].as_u64().unwrap_or(0) as usize).map(|value| json!({"articles":value.articles,"page_index":value.page_index,"article_count":value.article_count,"has_next_page":value.has_next_page})),
            "article" => parse_domestic_article_detail(text, request["url"].as_str().unwrap()),
            "form" => Ok(json!(domestic_journal_search_form(text, request["field"].as_str().unwrap()))),
            "challenge" => extract_challenge_url(text, request["url"].as_str().unwrap()).map(|value|json!({"detected":looks_like_captcha_challenge(text,request["url"].as_str().unwrap()),"url":value})),
            "puzzle" => parse_captcha_puzzle(request["url"].as_str().unwrap(), &request["body"]).map(|puzzle|json!({"challenge_url":puzzle.challenge_url,"captcha_type":puzzle.captcha_type,"ident":puzzle.ident,"captcha_id":puzzle.captcha_id,"return_url":puzzle.return_url,"secret_key":puzzle.secret_key,"token":puzzle.token,"original":puzzle.original_image_b64,"jigsaw":puzzle.jigsaw_image_b64,"check":captcha_check_request_body(&puzzle,"ciphertext")})),
            "success" => Ok(json!(captcha_check_succeeded(&request["body"]))),
            "fixture_decode" => Ok(json!(serde_json::from_str::<DomesticCnkiFixtureData>(text).is_ok())),
            "checkpoint" => Ok(match serde_json::from_str::<DomesticCnkiCheckpoint>(text){Ok(value)=>json!({"encoded":serde_json::to_string(&value).unwrap()}),Err(_)=>Value::Null}),
            "anchor" => Ok(match serde_json::from_str::<litradar_sources::cnki_domestic::DomesticCnkiAnchor>(text){Ok(value)=>json!({"encoded":serde_json::to_string(&value).unwrap()}),Err(_)=>Value::Null}),
            "fixture" => Ok(fixture_sequence(&request)),
            "solve" => Ok(solve_sequence(&request)),
            "locator" => {
                let locator = DomesticJournalLocator::new(serde_json::from_value(request["titles"].clone()).unwrap(), serde_json::from_value(request["issns"].clone()).unwrap());
                Ok(json!({"titles":locator.titles(),"issns":locator.issns()}))
            }
            _ => panic!("unknown CNKI operation"),
        };
        request["output"] = outcome(result);
        writeln!(output, "{request}")?;
    }
    output.flush()
}

fn fixture_sequence(request: &Value) -> Value {
    let data = serde_json::from_value(request["fixture"].clone()).unwrap();
    let mut fixture = FixtureDomesticCnkiTransport::new(data);
    let mut observations = Vec::new();
    for operation in request["operations"].as_array().unwrap() {
        let result = match operation["op"].as_str().unwrap() {
            "resolve" => fixture.resolve_journal(&DomesticJournalLocator::new(serde_json::from_value(operation["titles"].clone()).unwrap(),serde_json::from_value(operation["issns"].clone()).unwrap())).map(|value|json!(value)),
            "years" => fixture.year_issues(&operation["journal"]).map(|value|json!(value)),
            "papers" => fixture.issue_articles(&operation["journal"],&operation["issue"],operation["page"].as_u64().unwrap() as usize).map(|value|json!({"articles":value.articles,"article_count":value.article_count,"page_index":value.page_index,"has_next_page":value.has_next_page})),
            "article" => fixture.article_detail(operation["url"].as_str().unwrap(),operation["platform_id"].as_str()),
            "drain" => Ok(json!(fixture.drain_attempts())),
            "reset" => fixture.reset_transient_state().map(|_|Value::Null),
            _ => panic!("unknown fixture operation"),
        };
        observations.push(json!({"result":outcome(result),"attempts":fixture.attempts()}));
    }
    json!(observations)
}

fn solve_sequence(request: &Value) -> Value {
    let mut session =
        DomesticCaptchaSession::with_budget(request["budget"].as_u64().unwrap() as usize);
    let mut solver = FixtureJfbymSolver::with_failures(
        request["distance"].as_f64().unwrap(),
        request["failures"].as_u64().unwrap_or(0) as usize,
    );
    let mut observations = Vec::new();
    for operation in request["operations"].as_array().unwrap() {
        let mut fetches = 0;
        let mut points = Vec::new();
        let accepted = operation["accept_after"].as_u64().unwrap_or(u64::MAX) as usize;
        let result = session.ensure_access(
            operation["text"].as_str().unwrap(),
            operation["url"].as_str().unwrap(),
            &mut solver,
            |url| {
                fetches += 1;
                if operation["fail"] == "fetch" {
                    return Err(DomesticCnkiSourceError::Request("fetch failed".into()));
                }
                parse_captcha_puzzle(url, &operation["body"])
            },
            |_, point| {
                points.push(point.to_string());
                if operation["fail"] == "submit" {
                    return Err(DomesticCnkiSourceError::Request("submit failed".into()));
                }
                Ok(points.len() >= accepted)
            },
        );
        let urls = request["attach"]
            .as_array()
            .unwrap()
            .iter()
            .map(|url| {
                outcome(
                    session
                        .attach_captcha_id(url.as_str().unwrap())
                        .map(|value| json!(value)),
                )
            })
            .collect::<Vec<_>>();
        observations.push(json!({"result":outcome(result.map(|_|Value::Null)),"remaining":session.remaining_budget(),"has_id":session.has_captcha_id(),"id_len":session.captcha_id_len(),"fetches":fetches,"points":points,"urls":urls}));
    }
    json!(observations)
}
