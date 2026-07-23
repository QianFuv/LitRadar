//! jfbym dual-image slider solver used by domestic CNKI captcha handling.
//!
//! This module never logs API tokens, secret keys, captcha identifiers, or
//! image payloads. Debug formatting redacts credential-bearing fields.

use std::error::Error;
use std::fmt;
use std::time::Duration;

use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use reqwest::blocking::Client;
use serde_json::Value;

/// Official jfbym dual-image slider type used for CNKI `blockPuzzle`.
pub const JFBYM_DUAL_SLIDER_TYPE: &str = "20111";
/// Documented jfbym custom API endpoint (HTTP preferred by vendor docs).
pub const JFBYM_API_URL: &str = "http://api.jfbym.com/api/YmServer/customApi";
/// Success code returned by jfbym when recognition succeeds.
pub const JFBYM_SUCCESS_CODE: i64 = 10000;

/// Errors returned by the jfbym dual-image solver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JfbymError {
    /// Solver configuration is invalid or incomplete.
    Configuration(String),
    /// Solver HTTP or protocol request failed.
    Request(String),
    /// Solver returned an unusable payload.
    InvalidResponse(String),
}

impl fmt::Display for JfbymError {
    /// Format the jfbym solver error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message)
            | Self::Request(message)
            | Self::InvalidResponse(message) => formatter.write_str(message),
        }
    }
}

impl Error for JfbymError {}

/// Solver boundary used by domestic captcha sessions.
pub trait JfbymSolver {
    /// Solve a dual-image slider challenge.
    ///
    /// # Arguments
    ///
    /// * `slide_image_b64` - Base64 jigsaw/slide image without data-URL prefix.
    /// * `background_image_b64` - Base64 background image without data-URL prefix.
    ///
    /// # Returns
    ///
    /// Gap left-edge distance on the background image.
    fn solve_dual_image(
        &mut self,
        slide_image_b64: &str,
        background_image_b64: &str,
    ) -> Result<f64, JfbymError>;
}

/// Deterministic solver for unit tests.
#[derive(Debug, Clone)]
pub struct FixtureJfbymSolver {
    distance: f64,
    remaining_failures: usize,
}

impl FixtureJfbymSolver {
    /// Build a fixture solver that always returns one distance.
    ///
    /// # Arguments
    ///
    /// * `distance` - Gap left-edge x to return.
    ///
    /// # Returns
    ///
    /// Fixture solver.
    pub fn new(distance: f64) -> Self {
        Self {
            distance,
            remaining_failures: 0,
        }
    }

    /// Build a fixture solver that fails a fixed number of times first.
    ///
    /// # Arguments
    ///
    /// * `distance` - Gap left-edge x after failures are exhausted.
    /// * `failures` - Number of forced failures before success.
    ///
    /// # Returns
    ///
    /// Fixture solver with a failure budget.
    pub fn with_failures(distance: f64, failures: usize) -> Self {
        Self {
            distance,
            remaining_failures: failures,
        }
    }
}

impl JfbymSolver for FixtureJfbymSolver {
    /// Return the configured fixture distance or a forced failure.
    fn solve_dual_image(
        &mut self,
        slide_image_b64: &str,
        background_image_b64: &str,
    ) -> Result<f64, JfbymError> {
        if slide_image_b64.trim().is_empty() || background_image_b64.trim().is_empty() {
            return Err(JfbymError::InvalidResponse(
                "jfbym fixture requires non-empty dual images".to_string(),
            ));
        }
        if self.remaining_failures > 0 {
            self.remaining_failures -= 1;
            return Err(JfbymError::Request(
                "jfbym fixture forced failure".to_string(),
            ));
        }
        Ok(self.distance)
    }
}

/// Live HTTP solver for the jfbym dual-image API.
pub struct LiveJfbymSolver {
    token: String,
    client: Client,
    api_url: String,
    type_code: String,
}

impl fmt::Debug for LiveJfbymSolver {
    /// Format the solver without exposing the API token.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveJfbymSolver")
            .field("token", &"[REDACTED]")
            .field("api_url", &self.api_url)
            .field("type_code", &self.type_code)
            .finish()
    }
}

impl LiveJfbymSolver {
    /// Build a live jfbym solver.
    ///
    /// # Arguments
    ///
    /// * `token` - jfbym API token.
    /// * `timeout_seconds` - HTTP timeout.
    ///
    /// # Returns
    ///
    /// Live solver, or a configuration error when the token is empty.
    pub fn new(token: impl Into<String>, timeout_seconds: u64) -> Result<Self, JfbymError> {
        let token = token.into();
        if token.trim().is_empty() {
            return Err(JfbymError::Configuration(
                "jfbym token is required".to_string(),
            ));
        }
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_seconds.max(1)))
            .build()
            .map_err(|error| JfbymError::Request(error.to_string()))?;
        Ok(Self {
            token,
            client,
            api_url: JFBYM_API_URL.to_string(),
            type_code: JFBYM_DUAL_SLIDER_TYPE.to_string(),
        })
    }
}

impl JfbymSolver for LiveJfbymSolver {
    /// POST dual-image recognition to jfbym and parse the gap distance.
    fn solve_dual_image(
        &mut self,
        slide_image_b64: &str,
        background_image_b64: &str,
    ) -> Result<f64, JfbymError> {
        let slide_image = strip_data_url_base64(slide_image_b64);
        let background_image = strip_data_url_base64(background_image_b64);
        if slide_image.is_empty() || background_image.is_empty() {
            return Err(JfbymError::InvalidResponse(
                "jfbym dual-image solve requires slide and background images".to_string(),
            ));
        }
        let payload = serde_json::json!({
            "token": self.token,
            "type": self.type_code,
            "slide_image": slide_image,
            "background_image": background_image,
        });
        let response = self
            .client
            .post(&self.api_url)
            .header("Content-Type", "application/json")
            .json(&payload)
            .send()
            .map_err(|error| JfbymError::Request(error.to_string()))?;
        let status = response.status();
        let body: Value = response
            .json()
            .map_err(|error| JfbymError::InvalidResponse(error.to_string()))?;
        if !status.is_success() {
            return Err(JfbymError::Request(format!(
                "jfbym HTTP status {}",
                status.as_u16()
            )));
        }
        let code = body.get("code").and_then(Value::as_i64).or_else(|| {
            body.get("code")
                .and_then(Value::as_u64)
                .map(|value| value as i64)
        });
        if code != Some(JFBYM_SUCCESS_CODE) {
            let message = body
                .get("msg")
                .or_else(|| body.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("jfbym recognition failed");
            return Err(JfbymError::InvalidResponse(format!(
                "jfbym recognition failed with code {code:?}: {message}"
            )));
        }
        parse_slider_distance(&body).ok_or_else(|| {
            JfbymError::InvalidResponse("jfbym response missing slider distance".to_string())
        })
    }
}

/// Strip an optional `data:` URL prefix from a base64 image payload.
///
/// # Arguments
///
/// * `value` - Raw image value that may include a data-URL prefix.
///
/// # Returns
///
/// Base64 payload without the data-URL prefix.
pub fn strip_data_url_base64(value: &str) -> &str {
    let trimmed = value.trim();
    if trimmed.to_ascii_lowercase().starts_with("data:") {
        if let Some((_, payload)) = trimmed.split_once(',') {
            return payload.trim();
        }
    }
    trimmed
}

/// AES-128-ECB PKCS7 encrypt CNKI `pointJson` coordinates.
///
/// # Arguments
///
/// * `secret_key` - 16-byte UTF-8 AES key from the captcha puzzle.
/// * `x` - Horizontal gap coordinate accepted by CNKI.
/// * `y` - Vertical coordinate; live probes use `5`.
///
/// # Returns
///
/// Base64 ciphertext for the `pointJson` field.
pub fn encrypt_point_json(secret_key: &str, x: i32, y: i32) -> Result<String, JfbymError> {
    let key_bytes = secret_key.as_bytes();
    if key_bytes.len() != 16 {
        return Err(JfbymError::InvalidResponse(format!(
            "captcha secretKey must be 16 bytes, got {}",
            key_bytes.len()
        )));
    }
    let plain = serde_json::to_vec(&serde_json::json!({"x": x, "y": y}))
        .map_err(|error| JfbymError::InvalidResponse(error.to_string()))?;
    let ciphertext = encrypt_aes128_ecb_pkcs7(key_bytes, &plain)?;
    Ok(BASE64.encode(ciphertext))
}

/// Extract a numeric slider distance from a jfbym response payload.
///
/// # Arguments
///
/// * `payload` - Full jfbym JSON body or nested fragment.
///
/// # Returns
///
/// Parsed distance when present.
pub fn parse_slider_distance(payload: &Value) -> Option<f64> {
    match payload {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_distance_text(text),
        Value::Array(items) => items.iter().find_map(parse_slider_distance),
        Value::Object(map) => {
            for key in ["data", "result", "distance", "x", "value"] {
                if let Some(value) = map.get(key) {
                    if let Some(distance) = parse_slider_distance(value) {
                        return Some(distance);
                    }
                }
            }
            map.values().find_map(parse_slider_distance)
        }
        _ => None,
    }
}

/// Build integer x candidates from a jfbym gap-edge distance.
///
/// # Arguments
///
/// * `raw_distance` - Gap left-edge distance returned by jfbym.
///
/// # Returns
///
/// Ordered unique candidate x values: raw, then +/-1, then +/-2.
pub fn point_x_candidates(raw_distance: f64) -> Vec<i32> {
    let raw = raw_distance.round() as i32;
    let mut candidates = Vec::new();
    for offset in [0, 1, -1, 2, -2] {
        let value = raw + offset;
        if value < 0 {
            continue;
        }
        if !candidates.contains(&value) {
            candidates.push(value);
        }
    }
    candidates
}

fn parse_distance_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(value) = trimmed.parse::<f64>() {
        return Some(value);
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return parse_slider_distance(&value);
    }
    let number = trimmed.chars().enumerate().find_map(|(index, character)| {
        if character.is_ascii_digit() || character == '-' || character == '+' {
            Some(&trimmed[index..])
        } else {
            None
        }
    })?;
    let end = number
        .find(|character: char| {
            !(character.is_ascii_digit()
                || character == '.'
                || character == '-'
                || character == '+')
        })
        .unwrap_or(number.len());
    number[..end].parse().ok()
}

fn encrypt_aes128_ecb_pkcs7(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, JfbymError> {
    let cipher = Aes128::new_from_slice(key)
        .map_err(|error| JfbymError::InvalidResponse(error.to_string()))?;
    let pad_len = 16 - (plaintext.len() % 16);
    let mut padded = plaintext.to_vec();
    padded.extend(std::iter::repeat_n(pad_len as u8, pad_len));
    let mut output = Vec::with_capacity(padded.len());
    for chunk in padded.chunks_exact(16) {
        let mut block = *aes::cipher::generic_array::GenericArray::from_slice(chunk);
        cipher.encrypt_block(&mut block);
        output.extend_from_slice(&block);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockDecrypt;

    #[test]
    fn encrypts_point_json_with_aes128_ecb_pkcs7() {
        let secret_key = "0123456789abcdef";
        let encrypted = encrypt_point_json(secret_key, 261, 5).expect("encrypt");
        assert!(!encrypted.is_empty());
        assert!(!encrypted.contains("261"));
        assert!(!encrypted.contains(secret_key));

        let ciphertext = BASE64.decode(encrypted.as_bytes()).expect("base64");
        let cipher = Aes128::new_from_slice(secret_key.as_bytes()).expect("key");
        let mut plain = Vec::with_capacity(ciphertext.len());
        for chunk in ciphertext.chunks_exact(16) {
            let mut block = *aes::cipher::generic_array::GenericArray::from_slice(chunk);
            cipher.decrypt_block(&mut block);
            plain.extend_from_slice(&block);
        }
        let pad = *plain.last().expect("pad") as usize;
        plain.truncate(plain.len() - pad);
        let decoded: Value = serde_json::from_slice(&plain).expect("json");
        assert_eq!(decoded["x"], 261);
        assert_eq!(decoded["y"], 5);
    }

    #[test]
    fn parses_nested_jfbym_distance() {
        let body = serde_json::json!({
            "code": 10000,
            "msg": "ok",
            "data": {
                "code": 0,
                "data": "261",
                "time": 0.01
            }
        });
        assert_eq!(parse_slider_distance(&body), Some(261.0));
    }

    #[test]
    fn point_candidates_prefer_raw_then_small_offsets() {
        assert_eq!(point_x_candidates(261.4), vec![261, 262, 260, 263, 259]);
    }

    #[test]
    fn fixture_solver_rejects_empty_images_and_honors_failures() {
        let mut solver = FixtureJfbymSolver::with_failures(120.0, 1);
        assert!(solver.solve_dual_image("", "bg").is_err());
        assert!(solver.solve_dual_image("slide", "bg").is_err());
        assert_eq!(solver.solve_dual_image("slide", "bg").expect("ok"), 120.0);
    }

    #[test]
    fn live_solver_debug_redacts_token() {
        let solver = LiveJfbymSolver::new("super-secret-token", 10).expect("solver");
        let debug = format!("{solver:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("super-secret-token"));
    }

    #[test]
    fn strips_data_url_prefix() {
        assert_eq!(strip_data_url_base64("data:image/png;base64,AAAA"), "AAAA");
        assert_eq!(strip_data_url_base64("  BBBB  "), "BBBB");
    }
}
