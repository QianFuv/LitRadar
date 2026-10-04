//! Observe the original locked field matcher without a Go-derived expectation.
use std::io::{self, Read};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl io::Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> { self.0.lock().unwrap().extend_from_slice(bytes); Ok(bytes.len()) }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Self;
    fn make_writer(&'a self) -> Self { self.clone() }
}

fn record(span: &tracing::Span, value: &serde_json::Value) {
    match value["kind"].as_str().unwrap_or("empty") {
        "bool" => { span.record("value", value["value"].as_bool().unwrap()); }
        "u64" => { span.record("value", value["value"].as_str().unwrap().parse::<u64>().unwrap()); }
        "i64" => { span.record("value", value["value"].as_str().unwrap().parse::<i64>().unwrap()); }
        "f64" => { span.record("value", value["value"].as_str().unwrap().parse::<f64>().unwrap()); }
        "str" => { span.record("value", value["value"].as_str().unwrap()); }
        "debug" => { span.record("value", tracing::field::debug(value["value"].as_str().unwrap())); }
        _ => {}
    }
}

fn events(target: &str, phase: u64) {
    macro_rules! emit {
        ($target:literal) => {{
            tracing::error!(target:$target,event="probe",component="fixture",phase,value="sample");
            tracing::warn!(target:$target,event="probe",component="fixture",phase,value="sample");
            tracing::info!(target:$target,event="probe",component="fixture",phase,value="sample");
            tracing::debug!(target:$target,event="probe",component="fixture",phase,value="sample");
            tracing::trace!(target:$target,event="probe",component="fixture",phase,value="sample");
        }};
    }
    match target {
        "litradar" => emit!("litradar"),
        "litradar_api::http_observability" => emit!("litradar_api::http_observability"),
        "litradar_cli" => emit!("litradar_cli"),
        _ => emit!("other"),
    }
}

fn observe_filter(case: &serde_json::Value) -> serde_json::Value {
    let filter = case["filter"].as_str().unwrap();
    let parsed = tracing_subscriber::EnvFilter::try_new(filter);
    let Ok(parsed) = parsed else { return serde_json::json!({"valid":false}); };
    let capture = Capture(Arc::new(Mutex::new(Vec::new())));
    let subscriber = tracing_subscriber::fmt().with_env_filter(parsed).with_writer(capture.clone()).with_ansi(false).with_target(true).json().flatten_event(true).with_current_span(true).with_span_list(true).finish();
    tracing::subscriber::with_default(subscriber, || {
        let target = case["target"].as_str().unwrap_or("litradar");
        events(target,0);
        let command = case["command"].as_str().unwrap_or("serve");
        let span = tracing::info_span!(target:"litradar","process",component="runtime",command,value=tracing::field::Empty);
        record(&span,&case["initial"]);
        {
            let _entered = span.enter();
            events(target,1);
            record(&span,&case["update"]);
            events(target,2);
        }
        {
            let _entered = span.enter();
            events(target,3);
        }
        events(target,4);
    });
    let bytes = capture.0.lock().unwrap();
    let observations: Vec<_> = std::str::from_utf8(&bytes).unwrap().lines().map(|line| {
        let parsed: serde_json::Value = serde_json::from_str(line).unwrap();
        serde_json::json!({"phase":parsed["phase"],"level":parsed["level"],"span":parsed.get("span").and_then(|value| value.get("name"))})
    }).collect();
    serde_json::json!({"valid":true,"observations":observations})
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let request: serde_json::Value = serde_json::from_str(&input).unwrap();
    if request.get("compact").is_some() {
        let capture = Capture(Arc::new(Mutex::new(Vec::new())));
        let subscriber = tracing_subscriber::fmt().with_env_filter("trace").with_writer(capture.clone()).with_ansi(false).with_target(true).without_time().compact().finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::info!(target:"litradar",event="outside",component="runtime");
            let process = tracing::info_span!(target:"litradar","process",component="runtime",command="admin",version="0.1.0",process_id=123u32,parent_run_id=tracing::field::Empty);
            process.record("parent_run_id","parent-1");
            let _entered = process.enter();
            tracing::info!(target:"litradar",event="process.completed",component="runtime",outcome="success",duration_ms=7u128);
            let child = tracing::info_span!(target:"litradar_cli","cli.command",component="cli",command="admin");
            let _child = child.enter();
            tracing::error!(target:"litradar::observability",event="process.panicked",component="runtime",message="LitRadar process panicked");
        });
        let bytes = capture.0.lock().unwrap();
        println!("{}",serde_json::to_string(std::str::from_utf8(&bytes).unwrap()).unwrap());
        return;
    }
    if let Some(cases) = request.get("filters").and_then(|value| value.as_array()) {
        let result: Vec<_> = cases.iter().map(observe_filter).collect();
        println!("{}",serde_json::to_string(&result).unwrap());
        return;
    }
    let cases: Vec<serde_json::Value> = serde_json::from_value(request).unwrap();
    let observations: Vec<_> = cases.into_iter().map(|case| {
        let pattern = case["pattern"].as_str().unwrap();
        let values = case["inputs"].as_array().unwrap();
        match matchers::Pattern::new(pattern) {
            Ok(matcher) => serde_json::json!({"pattern": pattern, "valid": true, "observations": values.iter().map(|value| serde_json::json!({"input":value,"matched":matcher.matches(&value.as_str().unwrap())})).collect::<Vec<_>>()}),
            Err(_) => serde_json::json!({"pattern": pattern, "valid": false}),
        }
    }).collect();
    println!("{}", serde_json::to_string(&observations).unwrap());
}
