//! Observe frozen Rust HTTP date parsing, UTF-8 replacement groups and JSON acceptance.
#[allow(dead_code)]
#[path = "../../../output/migration/execution/sources-oracle/http_retry.rs"]
mod http_retry;
#[allow(dead_code)]
#[path = "../../../crates/litradar-sources/src/response_body.rs"]
mod response_body;

use chrono::DateTime;
use serde_json::{Value, json};
use std::io::{BufRead, Read, Write};
use std::time::SystemTime;

fn main() -> std::io::Result<()> {
    let mut output = std::io::BufWriter::new(std::io::stdout());
    for line in std::io::stdin().lock().lines() {
        let mut request: Value = serde_json::from_str(&line?).unwrap();
        let result = match request["kind"].as_str().unwrap() {
            "fixture_decode" => {
                json!({"valid":serde_json::from_str::<litradar_sources::ScholarlyFixtureData>(request["input"].as_str().unwrap()).is_ok()})
            }
            "config_decode" => {
                json!({"valid":serde_json::from_str::<litradar_sources::LiveScholarlyConfig>(request["input"].as_str().unwrap()).is_ok()})
            }
            "wire" => {
                let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
                let address = listener.local_addr().unwrap();
                let response: Vec<u8> = request["response"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_u64().unwrap() as u8)
                    .collect();
                let server = std::thread::spawn(move || {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let mut captured = Vec::new();
                    let mut byte = [0u8; 1];
                    while !captured.ends_with(b"\r\n\r\n") {
                        if stream.read(&mut byte).unwrap() == 0 {
                            break;
                        };
                        captured.push(byte[0]);
                    }
                    let _ = stream.write_all(&response);
                    String::from_utf8(captured).unwrap()
                });
                let response = reqwest::blocking::Client::builder()
                    .no_proxy()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .unwrap()
                    .get(format!("http://{address}/"))
                    .send()
                    .unwrap();
                let body = response_body::bounded_response_text(
                    response,
                    request["limit"].as_u64().unwrap() as usize,
                );
                let captured = server.join().unwrap().to_ascii_lowercase();
                let result = match body {
                    Ok(text) => json!({"text":text}),
                    Err(error) => json!({"error":error.to_string()}),
                };
                json!({"body":result,"accept":captured.contains("\r\naccept: */*\r\n"),"gzip":captured.contains("\r\naccept-encoding: gzip\r\n")})
            }
            "number" => match serde_json::from_str::<Value>(request["input"].as_str().unwrap()) {
                Ok(Value::Number(value)) => {
                    json!({"valid":true,"text":value.to_string(),"bits":value.as_f64().unwrap().to_bits().to_string(),"signed":value.as_i64().map(|value|value.to_string()),"unsigned":value.as_u64().map(|value|value.to_string())})
                }
                _ => json!({"valid":false}),
            },
            "retry" => {
                let now = DateTime::parse_from_rfc3339(request["now"].as_str().unwrap()).unwrap();
                let delay = http_retry::parse_retry_after(
                    request["input"].as_str().unwrap(),
                    SystemTime::from(now),
                );
                json!({"delay":delay.map(|value|json!({"seconds":value.as_secs().to_string(),"nanoseconds":value.subsec_nanos()}))})
            }
            "text" => {
                let bytes: Vec<u8> = request["bytes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|value| value.as_u64().unwrap() as u8)
                    .collect();
                json!({"text":String::from_utf8_lossy(&bytes)})
            }
            "json" => {
                json!({"valid":serde_json::from_str::<Value>(request["input"].as_str().unwrap()).is_ok()})
            }
            "slider" => {
                let payload: Value =
                    serde_json::from_str(request["input"].as_str().unwrap()).unwrap();
                match litradar_sources::parse_slider_distance(&payload) {
                    Ok(value) => json!({"value":value}),
                    Err(error) => json!({"error":error.to_string()}),
                }
            }
            "points" => {
                let value = request["input"].as_str().unwrap().parse::<f64>().unwrap();
                match litradar_sources::point_x_candidates(value) {
                    Ok(value) => json!({"value":value}),
                    Err(error) => json!({"error":error.to_string()}),
                }
            }
            "encrypt" => match litradar_sources::encrypt_point_json(
                request["input"].as_str().unwrap(),
                request["x"].as_i64().unwrap() as i32,
                request["y"].as_i64().unwrap() as i32,
            ) {
                Ok(value) => json!({"value":value}),
                Err(error) => json!({"error":error.to_string()}),
            },
            "strip" => {
                json!({"value":litradar_sources::strip_data_url_base64(request["input"].as_str().unwrap())})
            }
            "proxy" => {
                match litradar_sources::ProviderProxy::explicit(request["input"].as_str().unwrap())
                {
                    Ok(value) => json!({"valid":true,"url":value.url()}),
                    Err(error) => json!({"valid":false,"error":error.to_string()}),
                }
            }
            _ => panic!("unknown observation"),
        };
        request["output"] = result;
        writeln!(output, "{request}")?;
    }
    output.flush()
}
