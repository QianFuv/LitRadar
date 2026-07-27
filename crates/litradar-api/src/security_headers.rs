//! Static export integrity validation and response security headers.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use axum::extract::{Request, State};
use axum::http::header::{
    CONTENT_SECURITY_POLICY, REFERRER_POLICY, STRICT_TRANSPORT_SECURITY, X_CONTENT_TYPE_OPTIONS,
    X_FRAME_OPTIONS,
};
use axum::http::{HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const CSP_MANIFEST_FILENAME: &str = "csp-hashes.json";
const CSP_MANIFEST_VERSION: u32 = 1;
const CSP_MANIFEST_ALGORITHM: &str = "sha256";
const MAX_CSP_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const PERMISSIONS_POLICY: HeaderName = HeaderName::from_static("permissions-policy");
const PERMISSIONS_POLICY_VALUE: &str =
    "camera=(), microphone=(), geolocation=(), payment=(), usb=()";
const HSTS_VALUE: &str = "max-age=31536000";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CspManifest {
    version: u32,
    algorithm: String,
    files: Vec<CspManifestFile>,
    script_hashes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CspManifestFile {
    path: String,
    html_sha256: String,
    inline_script_hashes: Vec<String>,
}

/// Validated response security policy derived from the deployed static export.
#[derive(Debug, Clone)]
pub(crate) struct SecurityHeaderPolicy {
    content_security_policy: HeaderValue,
    is_hsts_enabled: bool,
}

#[derive(Debug)]
pub(crate) struct SecurityHeaderError(String);

impl fmt::Display for SecurityHeaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for SecurityHeaderError {}

/// Load and verify the CSP manifest against every deployed HTML file.
///
/// # Arguments
///
/// * `web_root` - Root of the deployed static export.
/// * `is_hsts_enabled` - Whether hardened HTTPS deployment headers are required.
///
/// # Returns
///
/// A security policy that contains only hashes recomputed from deployed HTML.
pub(crate) fn load_security_header_policy(
    web_root: &Path,
    is_hsts_enabled: bool,
) -> Result<SecurityHeaderPolicy, SecurityHeaderError> {
    let expected_manifest = build_csp_manifest(web_root)?;
    let manifest_path = web_root.join(CSP_MANIFEST_FILENAME);
    let manifest_metadata = fs::symlink_metadata(&manifest_path).map_err(|error| {
        SecurityHeaderError(format!(
            "Unable to read CSP manifest metadata at {}: {error}",
            manifest_path.display()
        ))
    })?;
    if manifest_metadata.file_type().is_symlink() || !manifest_metadata.is_file() {
        return Err(SecurityHeaderError(format!(
            "CSP manifest must be a regular file: {}",
            manifest_path.display()
        )));
    }
    if manifest_metadata.len() > MAX_CSP_MANIFEST_BYTES {
        return Err(SecurityHeaderError(format!(
            "CSP manifest exceeds the {} byte limit",
            MAX_CSP_MANIFEST_BYTES
        )));
    }
    let manifest_bytes = fs::read(&manifest_path).map_err(|error| {
        SecurityHeaderError(format!(
            "Unable to read CSP manifest at {}: {error}",
            manifest_path.display()
        ))
    })?;
    let deployed_manifest: CspManifest = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| SecurityHeaderError(format!("Invalid CSP manifest: {error}")))?;
    if deployed_manifest != expected_manifest {
        return Err(SecurityHeaderError(
            "CSP manifest does not match the deployed static HTML".to_string(),
        ));
    }

    let mut policy = String::from(
        "default-src 'self'; base-uri 'self'; object-src 'none'; frame-ancestors 'none'; form-action 'self'; script-src 'self'",
    );
    for hash in &expected_manifest.script_hashes {
        policy.push_str(" '");
        policy.push_str(hash);
        policy.push('\'');
    }
    policy.push_str("; style-src 'self' 'unsafe-inline'");
    let content_security_policy = HeaderValue::from_str(&policy)
        .map_err(|_| SecurityHeaderError("Generated CSP header is invalid".to_string()))?;

    Ok(SecurityHeaderPolicy {
        content_security_policy,
        is_hsts_enabled,
    })
}

/// Apply baseline response security headers to every response.
///
/// # Arguments
///
/// * `policy` - Startup-validated static export policy.
/// * `request` - Incoming HTTP request.
/// * `next` - Remaining middleware and route service.
///
/// # Returns
///
/// Response with the deployment security policy applied.
pub(crate) async fn security_header_middleware(
    State(policy): State<SecurityHeaderPolicy>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_SECURITY_POLICY,
        policy.content_security_policy.clone(),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("same-origin"));
    headers.insert(
        PERMISSIONS_POLICY,
        HeaderValue::from_static(PERMISSIONS_POLICY_VALUE),
    );
    headers.insert(X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    if policy.is_hsts_enabled {
        headers.insert(
            STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static(HSTS_VALUE),
        );
    }
    response
}

fn build_csp_manifest(web_root: &Path) -> Result<CspManifest, SecurityHeaderError> {
    let mut html_paths = Vec::new();
    collect_html_paths(web_root, web_root, &mut html_paths)?;
    html_paths.sort_by(|left, right| left.0.cmp(&right.0));
    if html_paths.is_empty() {
        return Err(SecurityHeaderError(format!(
            "Static export contains no HTML files: {}",
            web_root.display()
        )));
    }

    let mut files = Vec::with_capacity(html_paths.len());
    let mut script_hashes = BTreeSet::new();
    for (relative_path, absolute_path) in html_paths {
        let html = fs::read(&absolute_path).map_err(|error| {
            SecurityHeaderError(format!(
                "Unable to read static HTML at {}: {error}",
                absolute_path.display()
            ))
        })?;
        std::str::from_utf8(&html).map_err(|_| {
            SecurityHeaderError(format!(
                "Static HTML must be valid UTF-8: {}",
                absolute_path.display()
            ))
        })?;
        let inline_script_hashes = extract_inline_script_hashes(&html)?;
        script_hashes.extend(inline_script_hashes.iter().cloned());
        files.push(CspManifestFile {
            path: relative_path,
            html_sha256: sha256_source(&html),
            inline_script_hashes,
        });
    }

    Ok(CspManifest {
        version: CSP_MANIFEST_VERSION,
        algorithm: CSP_MANIFEST_ALGORITHM.to_string(),
        files,
        script_hashes: script_hashes.into_iter().collect(),
    })
}

fn collect_html_paths(
    web_root: &Path,
    current_directory: &Path,
    html_paths: &mut Vec<(String, PathBuf)>,
) -> Result<(), SecurityHeaderError> {
    let entries = fs::read_dir(current_directory).map_err(|error| {
        SecurityHeaderError(format!(
            "Unable to traverse static export at {}: {error}",
            current_directory.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            SecurityHeaderError(format!("Unable to inspect static export entry: {error}"))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            SecurityHeaderError(format!(
                "Unable to inspect static export entry at {}: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_symlink() {
            return Err(SecurityHeaderError(format!(
                "Static export must not contain symbolic links: {}",
                entry.path().display()
            )));
        }
        if file_type.is_dir() {
            collect_html_paths(web_root, &entry.path(), html_paths)?;
        } else if file_type.is_file()
            && entry
                .path()
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("html"))
        {
            let relative_path = entry
                .path()
                .strip_prefix(web_root)
                .map_err(|_| {
                    SecurityHeaderError("Static HTML escaped the export root".to_string())
                })?
                .to_str()
                .ok_or_else(|| {
                    SecurityHeaderError("Static HTML path must be valid UTF-8".to_string())
                })?
                .replace('\\', "/");
            html_paths.push((relative_path, entry.path()));
        }
    }
    Ok(())
}

fn extract_inline_script_hashes(html: &[u8]) -> Result<Vec<String>, SecurityHeaderError> {
    let mut hashes = Vec::new();
    let mut cursor = 0;
    while let Some(opening_start) = find_script_opening(html, cursor) {
        let opening_end = find_opening_tag_end(html, opening_start + b"<script".len())?;
        let closing_start = find_script_closing(html, opening_end + 1).ok_or_else(|| {
            SecurityHeaderError("Static HTML contains an unterminated script element".to_string())
        })?;
        let closing_end = find_closing_tag_end(html, closing_start).ok_or_else(|| {
            SecurityHeaderError("Static HTML contains an invalid script closing tag".to_string())
        })?;
        if !has_src_attribute(&html[opening_start..=opening_end]) {
            hashes.push(sha256_source(&html[opening_end + 1..closing_start]));
        }
        cursor = closing_end;
    }
    Ok(hashes)
}

fn find_script_opening(html: &[u8], start: usize) -> Option<usize> {
    find_ascii_case_insensitive(html, b"<script", start).and_then(|index| {
        let boundary = html.get(index + b"<script".len()).copied()?;
        if is_html_whitespace(boundary) || matches!(boundary, b'/' | b'>') {
            Some(index)
        } else {
            find_script_opening(html, index + b"<script".len())
        }
    })
}

fn find_opening_tag_end(html: &[u8], start: usize) -> Result<usize, SecurityHeaderError> {
    let mut quote = None;
    for (index, byte) in html.iter().copied().enumerate().skip(start) {
        if let Some(expected_quote) = quote {
            if byte == expected_quote {
                quote = None;
            }
        } else if matches!(byte, b'\'' | b'"') {
            quote = Some(byte);
        } else if byte == b'>' {
            return Ok(index);
        }
    }
    Err(SecurityHeaderError(
        "Static HTML contains an unterminated script opening tag".to_string(),
    ))
}

fn find_script_closing(html: &[u8], start: usize) -> Option<usize> {
    let mut cursor = start;
    while let Some(index) = find_ascii_case_insensitive(html, b"</script", cursor) {
        let boundary = html.get(index + b"</script".len()).copied()?;
        if is_html_whitespace(boundary) || boundary == b'>' {
            return Some(index);
        }
        cursor = index + b"</script".len();
    }
    None
}

fn find_closing_tag_end(html: &[u8], closing_start: usize) -> Option<usize> {
    let mut cursor = closing_start + b"</script".len();
    while html
        .get(cursor)
        .is_some_and(|byte| is_html_whitespace(*byte))
    {
        cursor += 1;
    }
    (html.get(cursor) == Some(&b'>')).then_some(cursor + 1)
}

fn has_src_attribute(opening_tag: &[u8]) -> bool {
    let mut cursor = b"<script".len();
    while cursor + 1 < opening_tag.len() {
        while opening_tag
            .get(cursor)
            .is_some_and(|byte| is_html_whitespace(*byte) || *byte == b'/')
        {
            cursor += 1;
        }
        let name_start = cursor;
        while opening_tag
            .get(cursor)
            .is_some_and(|byte| !is_html_whitespace(*byte) && !matches!(*byte, b'=' | b'/' | b'>'))
        {
            cursor += 1;
        }
        if cursor == name_start {
            cursor += 1;
            continue;
        }
        if opening_tag[name_start..cursor].eq_ignore_ascii_case(b"src") {
            return true;
        }
        while opening_tag
            .get(cursor)
            .is_some_and(|byte| is_html_whitespace(*byte))
        {
            cursor += 1;
        }
        if opening_tag.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while opening_tag
            .get(cursor)
            .is_some_and(|byte| is_html_whitespace(*byte))
        {
            cursor += 1;
        }
        if let Some(quote @ (b'\'' | b'"')) = opening_tag.get(cursor).copied() {
            cursor += 1;
            while opening_tag.get(cursor).is_some_and(|byte| *byte != quote) {
                cursor += 1;
            }
            cursor += usize::from(cursor < opening_tag.len());
        } else {
            while opening_tag
                .get(cursor)
                .is_some_and(|byte| !is_html_whitespace(*byte) && *byte != b'>')
            {
                cursor += 1;
            }
        }
    }
    false
}

fn find_ascii_case_insensitive(haystack: &[u8], needle: &[u8], start: usize) -> Option<usize> {
    haystack
        .get(start..)?
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|offset| start + offset)
}

fn is_html_whitespace(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0c)
}

fn sha256_source(value: &[u8]) -> String {
    format!("sha256-{}", STANDARD.encode(Sha256::digest(value)))
}

#[cfg(test)]
pub(crate) fn write_test_web_root(project_root: &Path) {
    let web_root = project_root.join("web");
    fs::create_dir_all(&web_root).expect("test web root should be created");
    if !web_root.join("index.html").exists() {
        fs::write(
            web_root.join("index.html"),
            "<!doctype html><title>fixture</title><script>globalThis.fixture=true;</script>",
        )
        .expect("test index should write");
    }
    if !web_root.join("404.html").exists() {
        fs::write(
            web_root.join("404.html"),
            "<!doctype html><title>not found</title>",
        )
        .expect("test not-found page should write");
    }
    write_test_csp_manifest(&web_root);
}

#[cfg(test)]
pub(crate) fn write_test_csp_manifest(web_root: &Path) {
    let manifest = build_csp_manifest(web_root).expect("test CSP manifest should build");
    let mut payload =
        serde_json::to_vec_pretty(&manifest).expect("test CSP manifest should encode");
    payload.push(b'\n');
    fs::write(web_root.join(CSP_MANIFEST_FILENAME), payload)
        .expect("test CSP manifest should write");
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        build_csp_manifest, extract_inline_script_hashes, load_security_header_policy,
        sha256_source, write_test_csp_manifest, CspManifest, CSP_MANIFEST_FILENAME,
    };

    #[test]
    fn security_headers_hash_only_exact_inline_script_bytes() {
        let first = b"globalThis.first = '<tag>';";
        let second = "globalThis.second = '\u{4e2d}\u{6587}';".as_bytes();
        let html = format!(
            "<SCRIPT data-src='ignored'>{}</SCRIPT><script src='/external.js'></script><script>{}</script>",
            String::from_utf8_lossy(first),
            String::from_utf8_lossy(second)
        );

        let hashes = extract_inline_script_hashes(html.as_bytes())
            .expect("valid inline scripts should be hashed");

        assert_eq!(hashes, vec![sha256_source(first), sha256_source(second)]);
    }

    #[test]
    fn security_headers_reject_missing_and_stale_csp_manifests() {
        let temp_dir = tempdir().expect("temporary export should be created");
        fs::write(
            temp_dir.path().join("index.html"),
            "<!doctype html><script>globalThis.ready=true;</script>",
        )
        .expect("fixture HTML should write");

        let missing = load_security_header_policy(temp_dir.path(), false)
            .expect_err("missing CSP manifest should fail startup");
        assert!(missing.to_string().contains("CSP manifest metadata"));

        write_test_csp_manifest(temp_dir.path());
        load_security_header_policy(temp_dir.path(), false)
            .expect("matching CSP manifest should pass startup");

        fs::write(
            temp_dir.path().join("index.html"),
            "<!doctype html><script>globalThis.ready=false;</script>",
        )
        .expect("fixture HTML should mutate");
        let stale = load_security_header_policy(temp_dir.path(), false)
            .expect_err("stale CSP manifest should fail startup");
        assert_eq!(
            stale.to_string(),
            "CSP manifest does not match the deployed static HTML"
        );
    }

    #[test]
    #[ignore = "requires pnpm --dir app build"]
    fn security_headers_validate_production_static_export() {
        let web_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../app/out");
        let expected = build_csp_manifest(&web_root)
            .expect("built frontend HTML should produce a CSP manifest");
        let deployed: CspManifest = serde_json::from_slice(
            &fs::read(web_root.join(CSP_MANIFEST_FILENAME))
                .expect("built frontend CSP manifest should read"),
        )
        .expect("built frontend CSP manifest should decode");

        assert_eq!(deployed, expected);

        let policy = load_security_header_policy(&web_root, false)
            .expect("built frontend CSP manifest should match every exported HTML file");

        assert!(policy
            .content_security_policy
            .to_str()
            .expect("CSP should be ASCII")
            .contains("sha256-"));
    }
}
