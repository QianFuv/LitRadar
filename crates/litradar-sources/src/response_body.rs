//! Bounded decoded response-body readers for blocking source transports.

use std::fmt;
use std::io::Read;

use reqwest::blocking::Response;
use serde_json::Value;

/// Fixed classifications for bounded source response failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseBodyError {
    /// The decoded body could not be read.
    ReadFailed,
    /// The decoded body exceeded its endpoint limit.
    TooLarge,
    /// The bounded body was not valid JSON.
    InvalidJson,
}

impl fmt::Display for ResponseBodyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ReadFailed => "response body could not be read",
            Self::TooLarge => "response body exceeded the configured size limit",
            Self::InvalidJson => "response body was not valid JSON",
        })
    }
}

/// Read one decoded response as bounded UTF-8-lossy text.
///
/// # Arguments
///
/// * `response` - Blocking response whose transparent content decoding is already configured.
/// * `maximum_bytes` - Maximum decoded bytes retained in memory.
///
/// # Returns
///
/// Bounded text or a fixed response-body classification.
pub(crate) fn bounded_response_text(
    response: Response,
    maximum_bytes: usize,
) -> Result<String, ResponseBodyError> {
    let bytes = bounded_response_bytes(response, maximum_bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Read and parse one decoded response as bounded JSON.
///
/// # Arguments
///
/// * `response` - Blocking response whose transparent content decoding is already configured.
/// * `maximum_bytes` - Maximum decoded bytes retained in memory.
///
/// # Returns
///
/// Parsed JSON or a fixed response-body classification.
pub(crate) fn bounded_response_json(
    response: Response,
    maximum_bytes: usize,
) -> Result<Value, ResponseBodyError> {
    let bytes = bounded_response_bytes(response, maximum_bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| ResponseBodyError::InvalidJson)
}

fn bounded_response_bytes(
    response: Response,
    maximum_bytes: usize,
) -> Result<Vec<u8>, ResponseBodyError> {
    let maximum_bytes_u64 = u64::try_from(maximum_bytes).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > maximum_bytes_u64)
    {
        return Err(ResponseBodyError::TooLarge);
    }
    let capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(maximum_bytes)
        .min(64 * 1024);
    let mut bytes = Vec::with_capacity(capacity);
    response
        .take(maximum_bytes_u64.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| ResponseBodyError::ReadFailed)?;
    if bytes.len() > maximum_bytes {
        return Err(ResponseBodyError::TooLarge);
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    use reqwest::blocking::{Client, Response};
    use reqwest::redirect::Policy;

    use super::{bounded_response_json, bounded_response_text, ResponseBodyError};

    fn raw_response(headers: &str, body: Vec<u8>) -> Response {
        let listener = TcpListener::bind("127.0.0.1:0").expect("body test listener should bind");
        let address = listener
            .local_addr()
            .expect("body test listener address should resolve");
        let headers = headers.to_string();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("body test request should connect");
            let mut request = [0_u8; 4_096];
            let _ = stream
                .read(&mut request)
                .expect("body test request should read");
            write!(
                stream,
                "HTTP/1.1 200 OK\r\n{headers}Connection: close\r\n\r\n"
            )
            .expect("body test headers should write");
            stream
                .write_all(&body)
                .expect("body test payload should write");
        });
        let response = Client::builder()
            .redirect(Policy::none())
            .gzip(true)
            .build()
            .expect("body test client should build")
            .get(format!("http://{address}/body"))
            .send()
            .expect("body test response should arrive");
        server.join().expect("body test server should finish");
        response
    }

    #[test]
    fn bounded_reader_rejects_header_chunked_and_decoded_gzip_oversize() {
        let content_length_response = raw_response("Content-Length: 65\r\n", vec![b'a'; 65]);
        assert_eq!(
            bounded_response_text(content_length_response, 64),
            Err(ResponseBodyError::TooLarge)
        );

        let decoded_chunk = vec![b'b'; 65];
        let mut chunked_body = format!("{:X}\r\n", decoded_chunk.len()).into_bytes();
        chunked_body.extend_from_slice(&decoded_chunk);
        chunked_body.extend_from_slice(b"\r\n0\r\n\r\n");
        let chunked_response = raw_response("Transfer-Encoding: chunked\r\n", chunked_body);
        assert_eq!(
            bounded_response_text(chunked_response, 64),
            Err(ResponseBodyError::TooLarge)
        );

        let gzip_body = vec![
            31, 139, 8, 0, 0, 0, 0, 0, 2, 10, 75, 76, 28, 217, 0, 0, 89, 54, 125, 176, 0, 1, 0, 0,
        ];
        let gzip_response = raw_response(
            &format!(
                "Content-Encoding: gzip\r\nContent-Length: {}\r\n",
                gzip_body.len()
            ),
            gzip_body,
        );
        assert_eq!(
            bounded_response_text(gzip_response, 64),
            Err(ResponseBodyError::TooLarge)
        );
    }

    #[test]
    fn bounded_json_reader_preserves_valid_payloads_and_rejects_invalid_json() {
        let valid_body = br#"{"ok":true}"#.to_vec();
        let valid_response = raw_response(
            &format!("Content-Length: {}\r\n", valid_body.len()),
            valid_body,
        );
        assert_eq!(
            bounded_response_json(valid_response, 64).expect("valid JSON should parse"),
            serde_json::json!({"ok": true})
        );

        let invalid_body = b"not-json".to_vec();
        let invalid_response = raw_response(
            &format!("Content-Length: {}\r\n", invalid_body.len()),
            invalid_body,
        );
        assert_eq!(
            bounded_response_json(invalid_response, 64),
            Err(ResponseBodyError::InvalidJson)
        );
    }
}
