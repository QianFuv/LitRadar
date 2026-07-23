//! Domestic NZKPT CNKI metadata client parsers, fixtures, and captcha session.
//!
//! This module is intentionally unregistered as a product provider until later
//! tasks wire index and abstract capabilities under the runtime name `cnki`.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Deserialize;
use serde_json::{json, Value};

use crate::jfbym::{
    encrypt_point_json, point_x_candidates, strip_data_url_base64, JfbymError, JfbymSolver,
};
use crate::scholarly::SourceError;

/// Domestic navigation host used by NZKPT journal search and detail pages.
pub const DOMESTIC_NAVI_BASE_URL: &str = "https://navi.cnki.net";
/// Domestic knowledge-network host used by article abstract pages.
pub const DOMESTIC_KNS_BASE_URL: &str = "https://kns.cnki.net";
const DOMESTIC_PLATFORM: &str = "NZKPT";
const DOMESTIC_LANGUAGE: &str = "CHS";
const DOMESTIC_JOURNAL_PARENT_CODE: &str = "SQN63324";
const DOMESTIC_SEARCH_PRODUCT_CODE: &str = "OYXNO5VW";
const DEFAULT_PCODE: &str = "CJFD,CCJD";
/// Maximum fresh captcha puzzle solves allowed per domestic session budget.
pub const DOMESTIC_CAPTCHA_SOLVE_BUDGET: usize = 5;
const DOMESTIC_POINT_JSON_Y: i32 = 5;

/// Fixture payload used by domestic NZKPT source replay.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DomesticCnkiFixtureData {
    /// Journal search HTML body.
    #[serde(default)]
    pub journal_search_html: String,
    /// Journal detail HTML page.
    #[serde(default)]
    pub journal_detail_html: String,
    /// Year issue tree HTML.
    #[serde(default)]
    pub year_issues_html: String,
    /// Issue article HTML keyed by year-issue id such as `202512`.
    #[serde(default)]
    pub issue_articles_html: BTreeMap<String, String>,
    /// Article detail HTML keyed by platform id.
    #[serde(default)]
    pub article_detail_html: BTreeMap<String, String>,
    /// Optional endpoint forced to return a parser error.
    #[serde(default)]
    pub fail_endpoint: Option<String>,
}

/// Errors returned by the domestic CNKI source parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomesticCnkiSourceError {
    /// Domestic CNKI returned a blocked or verification page.
    Request(String),
    /// HTML could not be parsed into the expected payload.
    Parse(String),
    /// Fixture data is missing a required response.
    MissingFixture(String),
    /// Shared source error.
    Source(SourceError),
}

impl fmt::Display for DomesticCnkiSourceError {
    /// Format the domestic CNKI source error.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Request(message) | Self::Parse(message) | Self::MissingFixture(message) => {
                formatter.write_str(message)
            }
            Self::Source(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for DomesticCnkiSourceError {
    /// Return the underlying source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            _ => None,
        }
    }
}

impl From<SourceError> for DomesticCnkiSourceError {
    /// Convert shared source errors into domestic CNKI source errors.
    fn from(error: SourceError) -> Self {
        Self::Source(error)
    }
}

/// Build the HAR-shaped domestic journal search form fields.
///
/// # Arguments
///
/// * `keyword` - Journal title or ISSN search value.
/// * `field_name` - Search field name such as `TI` or `SN`.
///
/// # Returns
///
/// Ordered form fields for `application/x-www-form-urlencoded`.
pub fn domestic_journal_search_form(keyword: &str, field_name: &str) -> BTreeMap<String, String> {
    let search_state = json!({
        "StateID": "",
        "Platfrom": "",
        "QueryTime": "",
        "Account": "knavi",
        "ClientToken": "",
        "Language": "",
        "CNode": {
            "PCode": DOMESTIC_SEARCH_PRODUCT_CODE,
            "SMode": "",
            "OperateT": ""
        },
        "QNode": {
            "SelectT": "",
            "Select_Fields": "",
            "S_DBCodes": "",
            "Subscribed": "",
            "QGroup": [{
                "Key": "subject",
                "Logic": 1,
                "Items": [],
                "ChildItems": [{
                    "Key": "txt",
                    "Logic": 1,
                    "Items": [{
                        "Key": "txt_1",
                        "Title": "",
                        "Logic": 1,
                        "Name": field_name,
                        "Operate": "%",
                        "Value": keyword,
                        "ExtendType": 0,
                        "ExtendValue": "",
                        "Value2": ""
                    }],
                    "ChildItems": []
                }]
            }],
            "OrderBy": "OTA|DESC",
            "GroupBy": "",
            "Additon": ""
        }
    });
    let mut form = BTreeMap::new();
    form.insert(
        "searchStateJson".to_string(),
        serde_json::to_string(&search_state).unwrap_or_else(|_| "{}".to_string()),
    );
    form.insert("displaymode".to_string(), "1".to_string());
    form.insert("pageindex".to_string(), "1".to_string());
    form.insert("pagecount".to_string(), "21".to_string());
    form.insert("index".to_string(), "JSTMWT6S".to_string());
    form.insert("searchType".to_string(), "刊名(曾用刊名)".to_string());
    form.insert(
        "parentcode".to_string(),
        DOMESTIC_JOURNAL_PARENT_CODE.to_string(),
    );
    form.insert("clickName".to_string(), String::new());
    form.insert("switchdata".to_string(), "search".to_string());
    form
}

/// Parse domestic journal search HTML into candidate detail payloads.
///
/// # Arguments
///
/// * `text` - Search result HTML body.
///
/// # Returns
///
/// Candidate journals with title, ISSN, and detail URL.
pub fn parse_domestic_journal_search_results(
    text: &str,
) -> Result<Vec<Value>, DomesticCnkiSourceError> {
    checked_text(text, "journal_search")?;
    let mut candidates = Vec::new();
    let mut seen = Vec::<String>::new();
    for tag in tags(text, "a") {
        let tag_attrs = attrs(&tag);
        let Some(href) = tag_attrs.get("href") else {
            continue;
        };
        if !href.contains("/knavi/detail?") {
            continue;
        }
        let detail_url = absolute_domestic_url(href);
        if contains_overseas_host(&detail_url) {
            return Err(DomesticCnkiSourceError::Parse(
                "domestic journal search returned overseas host".to_string(),
            ));
        }
        if seen.iter().any(|value| value == &detail_url) {
            continue;
        }
        seen.push(detail_url.clone());
        let title = tag_attrs
            .get("title")
            .cloned()
            .and_then(|value| non_empty(&value))
            .or_else(|| non_empty(&strip_tags(&tag)))
            .unwrap_or_default();
        let issn = journal_result_issn_near(text, href).unwrap_or_default();
        candidates.push(json!({
            "title": title,
            "issn": issn,
            "detail_url": with_domestic_platform(&detail_url),
        }));
    }
    Ok(candidates)
}

/// Parse domestic journal detail HTML.
///
/// # Arguments
///
/// * `text` - Journal detail HTML.
///
/// # Returns
///
/// Journal detail payload with `pykm` and product codes.
pub fn parse_domestic_journal_detail(text: &str) -> Result<Value, DomesticCnkiSourceError> {
    checked_text(text, "journal_detail")?;
    let pykm = input_value(text, "pykm").ok_or_else(|| {
        DomesticCnkiSourceError::Parse("domestic journal detail missing pykm".to_string())
    })?;
    let pcode = input_value(text, "pCode").unwrap_or_else(|| DEFAULT_PCODE.to_string());
    let visible_text = strip_tags(text);
    let detail_url = with_domestic_platform(&format!(
        "{DOMESTIC_NAVI_BASE_URL}/knavi/detail?pykm={pykm}"
    ));
    if contains_overseas_host(&detail_url) {
        return Err(DomesticCnkiSourceError::Parse(
            "domestic journal detail produced overseas host".to_string(),
        ));
    }
    Ok(json!({
        "detail_url": detail_url,
        "pykm": pykm,
        "pcode": pcode,
        "time": input_value(text, "time"),
        "title": input_value(text, "shareChName").or_else(|| title_text(text)),
        "issn": label_value(&visible_text, &["ISSN"]),
        "cn": label_value(&visible_text, &["CN"]),
        "raw_text": visible_text,
        "platform": DOMESTIC_PLATFORM,
    }))
}

/// Parse domestic year-issue tree HTML.
///
/// # Arguments
///
/// * `text` - Year issue HTML.
///
/// # Returns
///
/// Parsed issue payloads.
pub fn parse_domestic_year_issues(text: &str) -> Result<Vec<Value>, DomesticCnkiSourceError> {
    checked_text(text, "year_issues")?;
    let mut issues = Vec::new();
    for tag in tags(text, "a") {
        let tag_attrs = attrs(&tag);
        let element_id = tag_attrs.get("id").cloned().unwrap_or_default();
        if !element_id.starts_with("yq") {
            continue;
        }
        let key = &element_id[2..];
        let Some(year) = key.get(..4).and_then(|value| value.parse::<i64>().ok()) else {
            continue;
        };
        let label = strip_tags(&tag);
        let Some(year_issue) = tag_attrs.get("value").cloned() else {
            continue;
        };
        issues.push(json!({
            "year": year,
            "number": issue_number(key, &label),
            "title": label,
            "year_issue": decode_html(&year_issue),
            "year_issue_id": key,
        }));
    }
    Ok(issues)
}

/// Parse domestic issue article HTML.
///
/// # Arguments
///
/// * `text` - Issue article HTML.
/// * `issue` - Issue payload.
///
/// # Returns
///
/// Article summary payloads.
pub fn parse_domestic_issue_articles(
    text: &str,
    issue: &Value,
) -> Result<Vec<Value>, DomesticCnkiSourceError> {
    checked_text(text, "issue_articles")?;
    let mut articles = Vec::new();
    let mut current_section = String::new();
    let mut cursor = 0;
    while let Some((start, tag_name)) = next_article_block(text, cursor) {
        if tag_name == "dt" {
            if let Some((block, end)) = tag_block_at(text, "dt", start) {
                current_section = strip_tags(&block);
                cursor = end;
            } else {
                break;
            }
        } else if let Some((block, end)) = tag_block_at(text, "dd", start) {
            if let Some(article) = parse_article_row(&block, issue, &current_section) {
                if contains_overseas_host(
                    &json_text(article.get("article_url")).unwrap_or_default(),
                ) {
                    return Err(DomesticCnkiSourceError::Parse(
                        "domestic issue article returned overseas host".to_string(),
                    ));
                }
                articles.push(article);
            }
            cursor = end;
        } else {
            break;
        }
    }
    Ok(articles)
}

/// Parse one domestic article detail HTML page.
///
/// # Arguments
///
/// * `text` - Article detail HTML.
/// * `article_url` - Original article URL.
///
/// # Returns
///
/// Article detail payload.
pub fn parse_domestic_article_detail(
    text: &str,
    article_url: &str,
) -> Result<Value, DomesticCnkiSourceError> {
    checked_text(text, article_url)?;
    if contains_overseas_host(article_url) {
        return Err(DomesticCnkiSourceError::Parse(
            "domestic article detail used overseas host".to_string(),
        ));
    }
    let filename =
        input_value(text, "paramfilename").or_else(|| input_value(text, "param-filename"));
    let dbcode = input_value(text, "paramdbcode").or_else(|| input_value(text, "param-dbcode"));
    let dbname = input_value(text, "paramdbname").or_else(|| input_value(text, "param-dbname"));
    let title = first_block_text(text, "h1", "title")
        .or_else(|| first_block_text(text, "p", "title-one"))
        .or_else(|| title_text(text));
    let abstract_text = input_value(text, "abstract_text")
        .or_else(|| summary_text(text))
        .map(|value| strip_tags(&decode_html(&value)))
        .and_then(|value| non_empty(&value));
    let online_time =
        row_value(text, "在线公开时间").or_else(|| row_value(text, "Online Release Time"));
    let permalink = with_domestic_platform(article_url);
    Ok(json!({
        "article_url": permalink,
        "platform_id": filename,
        "dbcode": dbcode,
        "dbname": dbname,
        "title": title,
        "authors": author_text(text).or_else(|| span_title(text, "author")),
        "abstract": abstract_text,
        "doi": row_value(text, "DOI"),
        "online_release_date": online_time.and_then(|value| date_part(&value)),
        "pages": label_value(&strip_tags(text), &["页码", "Pages"]),
        "platform": DOMESTIC_PLATFORM,
    }))
}

/// Validate domestic CNKI response text.
///
/// # Arguments
///
/// * `text` - Response text.
/// * `url` - Request URL or fixture key.
///
/// # Returns
///
/// Ok when the response appears usable.
pub fn checked_text(text: &str, url: &str) -> Result<(), DomesticCnkiSourceError> {
    let lowered = text.to_lowercase();
    if (lowered.contains("captcha")
        || text.contains("访问异常")
        || text.contains("安全验证")
        || text.contains("\"code\":-403")
        || text.contains("/verify/home"))
        && !looks_like_domestic_content(text)
    {
        return Err(DomesticCnkiSourceError::Request(format!(
            "domestic CNKI verification required: {url}"
        )));
    }
    Ok(())
}

/// Return whether a URL or body fragment points at overseas CNKI.
///
/// # Arguments
///
/// * `value` - URL or text fragment.
///
/// # Returns
///
/// True when the overseas host is present.
pub fn contains_overseas_host(value: &str) -> bool {
    value.to_lowercase().contains("oversea.cnki.net")
}

/// Puzzle payload returned by CNKI `/verify-api/get`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomesticCaptchaPuzzle {
    /// Challenge page URL.
    pub challenge_url: String,
    /// Captcha type, typically `blockPuzzle`.
    pub captcha_type: String,
    /// Challenge ident query value.
    pub ident: String,
    /// Memory-only captcha id retained after a successful solve.
    pub captcha_id: String,
    /// Encoded return URL from the challenge.
    pub return_url: String,
    /// AES secret key for pointJson encryption.
    pub secret_key: String,
    /// Puzzle token submitted with pointJson.
    pub token: String,
    /// Base64 background image.
    pub original_image_b64: String,
    /// Base64 jigsaw/slide image.
    pub jigsaw_image_b64: String,
}

impl fmt::Display for DomesticCaptchaPuzzle {
    /// Format puzzle metadata without secrets or image payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DomesticCaptchaPuzzle {{ captcha_type: {}, captcha_id_len: {}, secret_key_len: {}, token_len: {}, original_len: {}, jigsaw_len: {} }}",
            self.captcha_type,
            self.captcha_id.len(),
            self.secret_key.len(),
            self.token.len(),
            self.original_image_b64.len(),
            self.jigsaw_image_b64.len()
        )
    }
}

/// Memory-only captcha session state shared by domestic index and abstract calls.
#[derive(Debug, Clone)]
pub struct DomesticCaptchaSession {
    captcha_id: Option<String>,
    solve_attempts: usize,
    solve_budget: usize,
}

impl Default for DomesticCaptchaSession {
    /// Build a captcha session with the default solve budget.
    fn default() -> Self {
        Self::new()
    }
}

impl DomesticCaptchaSession {
    /// Build a captcha session with the default solve budget.
    ///
    /// # Returns
    ///
    /// Empty session that has not solved a challenge yet.
    pub fn new() -> Self {
        Self::with_budget(DOMESTIC_CAPTCHA_SOLVE_BUDGET)
    }

    /// Build a captcha session with an explicit solve budget.
    ///
    /// # Arguments
    ///
    /// * `solve_budget` - Maximum fresh puzzle solves allowed.
    ///
    /// # Returns
    ///
    /// Empty session with the provided budget.
    pub fn with_budget(solve_budget: usize) -> Self {
        Self {
            captcha_id: None,
            solve_attempts: 0,
            solve_budget: solve_budget.max(1),
        }
    }

    /// Return whether the solve budget has remaining capacity.
    ///
    /// # Returns
    ///
    /// True when another puzzle solve may be attempted.
    pub fn has_budget(&self) -> bool {
        self.solve_attempts < self.solve_budget
    }

    /// Return remaining puzzle solves.
    ///
    /// # Returns
    ///
    /// Remaining budget count.
    pub fn remaining_budget(&self) -> usize {
        self.solve_budget.saturating_sub(self.solve_attempts)
    }

    /// Return whether a captcha id is currently retained in memory.
    ///
    /// # Returns
    ///
    /// True when a successful solve stored a captcha id.
    pub fn has_captcha_id(&self) -> bool {
        self.captcha_id
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    }

    /// Return the memory-only captcha id length without exposing the value.
    ///
    /// # Returns
    ///
    /// Length of the stored captcha id, or zero.
    pub fn captcha_id_len(&self) -> usize {
        self.captcha_id.as_ref().map(String::len).unwrap_or(0)
    }

    /// Attach a captcha id query parameter when one is present.
    ///
    /// # Arguments
    ///
    /// * `url` - Domestic request URL.
    ///
    /// # Returns
    ///
    /// URL with `captchaId` appended when available.
    pub fn attach_captcha_id(&self, url: &str) -> String {
        let Some(captcha_id) = self.captcha_id.as_deref().filter(|value| !value.is_empty()) else {
            return url.to_string();
        };
        if url.contains("captchaId=") {
            return url.to_string();
        }
        let separator = if url.contains('?') { '&' } else { '?' };
        format!("{url}{separator}captchaId={captcha_id}")
    }

    /// Clear the retained captcha id after a failed authenticated request.
    pub fn clear_captcha_id(&mut self) {
        self.captcha_id = None;
    }

    /// Detect and solve a captcha challenge using a jfbym dual-image solver.
    ///
    /// # Arguments
    ///
    /// * `response_text` - Domestic response body that may embed a challenge.
    /// * `response_url` - Request URL associated with the response.
    /// * `solver` - Dual-image solver implementation.
    /// * `fetch_puzzle` - Loads puzzle images and keys for one challenge URL.
    /// * `submit_point` - Submits encrypted pointJson for one candidate x.
    ///
    /// # Returns
    ///
    /// Ok when no challenge was present or a solve succeeded.
    pub fn ensure_access<S, F, C>(
        &mut self,
        response_text: &str,
        response_url: &str,
        solver: &mut S,
        mut fetch_puzzle: F,
        mut submit_point: C,
    ) -> Result<(), DomesticCnkiSourceError>
    where
        S: JfbymSolver,
        F: FnMut(&str) -> Result<DomesticCaptchaPuzzle, DomesticCnkiSourceError>,
        C: FnMut(&DomesticCaptchaPuzzle, &str) -> Result<bool, DomesticCnkiSourceError>,
    {
        if !looks_like_captcha_challenge(response_text, response_url) {
            return Ok(());
        }
        let challenge_url =
            extract_challenge_url(response_text, response_url).ok_or_else(|| {
                DomesticCnkiSourceError::Request(
                    "domestic CNKI verification required but challenge URL is missing".to_string(),
                )
            })?;
        self.solve_challenge(&challenge_url, solver, &mut fetch_puzzle, &mut submit_point)
    }

    /// Solve one captcha challenge within the remaining budget.
    ///
    /// # Arguments
    ///
    /// * `challenge_url` - CNKI `/verify/home` challenge URL.
    /// * `solver` - Dual-image solver implementation.
    /// * `fetch_puzzle` - Loads puzzle images and keys for one challenge URL.
    /// * `submit_point` - Submits encrypted pointJson for one candidate x.
    ///
    /// # Returns
    ///
    /// Ok when a candidate is accepted and the captcha id is retained.
    pub fn solve_challenge<S, F, C>(
        &mut self,
        challenge_url: &str,
        solver: &mut S,
        mut fetch_puzzle: F,
        mut submit_point: C,
    ) -> Result<(), DomesticCnkiSourceError>
    where
        S: JfbymSolver,
        F: FnMut(&str) -> Result<DomesticCaptchaPuzzle, DomesticCnkiSourceError>,
        C: FnMut(&DomesticCaptchaPuzzle, &str) -> Result<bool, DomesticCnkiSourceError>,
    {
        while self.has_budget() {
            self.solve_attempts += 1;
            let puzzle = fetch_puzzle(challenge_url)?;
            validate_puzzle(&puzzle)?;
            let distance = solver
                .solve_dual_image(&puzzle.jigsaw_image_b64, &puzzle.original_image_b64)
                .map_err(map_jfbym_error)?;
            // One candidate per fresh puzzle: failed checks invalidate the puzzle token.
            let candidates = point_x_candidates(distance);
            let candidate_index = (self.solve_attempts - 1) % candidates.len();
            let x = candidates[candidate_index];
            let point_json = encrypt_point_json(&puzzle.secret_key, x, DOMESTIC_POINT_JSON_Y)
                .map_err(map_jfbym_error)?;
            if submit_point(&puzzle, &point_json)? {
                self.captcha_id = Some(puzzle.captcha_id);
                return Ok(());
            }
        }
        Err(DomesticCnkiSourceError::Request(format!(
            "domestic CNKI captcha solve budget exhausted after {} attempts",
            self.solve_attempts
        )))
    }
}

/// Return whether response text or URL indicates a captcha challenge.
///
/// # Arguments
///
/// * `text` - Response body.
/// * `url` - Request or redirect URL.
///
/// # Returns
///
/// True when verification is required.
pub fn looks_like_captcha_challenge(text: &str, url: &str) -> bool {
    if contains_overseas_host(text) || contains_overseas_host(url) {
        return false;
    }
    if looks_like_domestic_content(text) {
        return false;
    }
    let lowered = text.to_lowercase();
    lowered.contains("captcha")
        || text.contains("访问异常")
        || text.contains("安全验证")
        || text.contains("\"code\":-403")
        || text.contains("/verify/home")
        || url.contains("/verify/home")
        || url.to_lowercase().contains("captcha")
}

/// Extract a CNKI challenge URL from a blocked domestic response.
///
/// # Arguments
///
/// * `text` - Response body that may embed a challenge URL.
/// * `url` - Request or redirect URL.
///
/// # Returns
///
/// Challenge URL when present.
pub fn extract_challenge_url(text: &str, url: &str) -> Option<String> {
    if url.contains("/verify/home") {
        return Some(url.to_string());
    }
    if let Ok(payload) = serde_json::from_str::<Value>(text) {
        if let Some(message) = payload.get("message").and_then(Value::as_str) {
            if message.contains("/verify/home") {
                return Some(message.to_string());
            }
        }
    }
    let marker = "https://kns.cnki.net/verify/home?";
    if let Some(start) = text.find(marker) {
        let rest = &text[start..];
        let end = rest
            .find(|character: char| {
                character.is_whitespace() || character == '"' || character == '\''
            })
            .unwrap_or(rest.len());
        return Some(rest[..end].to_string());
    }
    let relative = "/verify/home?";
    if let Some(start) = text.find(relative) {
        let rest = &text[start..];
        let end = rest
            .find(|character: char| {
                character.is_whitespace() || character == '"' || character == '\''
            })
            .unwrap_or(rest.len());
        return Some(format!("{DOMESTIC_KNS_BASE_URL}{}", &rest[..end]));
    }
    None
}

/// Parse a `/verify-api/get` JSON body into a puzzle payload.
///
/// # Arguments
///
/// * `challenge_url` - Challenge page URL providing query fields.
/// * `body` - JSON response body from `/verify-api/get`.
///
/// # Returns
///
/// Parsed puzzle or a request/parse error.
pub fn parse_captcha_puzzle(
    challenge_url: &str,
    body: &Value,
) -> Result<DomesticCaptchaPuzzle, DomesticCnkiSourceError> {
    let query = query_map(challenge_url);
    let container = puzzle_container(body).ok_or_else(|| {
        DomesticCnkiSourceError::Parse(
            "domestic captcha puzzle missing image container".to_string(),
        )
    })?;
    let original = strip_data_url_base64(
        container
            .get("originalImageBase64")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
    .to_string();
    let jigsaw = strip_data_url_base64(
        container
            .get("jigsawImageBase64")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    )
    .to_string();
    let secret_key = container
        .get("secretKey")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let token = container
        .get("token")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let captcha_type = query
        .get("captchaType")
        .cloned()
        .or_else(|| {
            container
                .get("captchaType")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "blockPuzzle".to_string());
    let ident = query.get("ident").cloned().unwrap_or_default();
    let captcha_id = query
        .get("captchaId")
        .cloned()
        .or_else(|| {
            container
                .get("captchaId")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    let return_url = query.get("returnUrl").cloned().unwrap_or_default();
    let puzzle = DomesticCaptchaPuzzle {
        challenge_url: challenge_url.to_string(),
        captcha_type,
        ident,
        captcha_id,
        return_url,
        secret_key,
        token,
        original_image_b64: original,
        jigsaw_image_b64: jigsaw,
    };
    validate_puzzle(&puzzle)?;
    Ok(puzzle)
}

/// Build the JSON body for `/verify-api/get`.
///
/// # Arguments
///
/// * `challenge_url` - Challenge page URL.
///
/// # Returns
///
/// Request body for puzzle fetch.
pub fn captcha_get_request_body(challenge_url: &str) -> Value {
    let query = query_map(challenge_url);
    json!({
        "captchaType": query.get("captchaType").cloned().unwrap_or_else(|| "blockPuzzle".to_string()),
        "clientUid": client_uid_hex(),
        "ts": unix_millis(),
        "ident": query.get("ident").cloned().unwrap_or_default(),
        "captchaId": query.get("captchaId").cloned().unwrap_or_default(),
    })
}

/// Build the JSON body for `/verify-api/web/check`.
///
/// # Arguments
///
/// * `puzzle` - Captcha puzzle from `/verify-api/get`.
/// * `point_json` - Encrypted pointJson ciphertext.
///
/// # Returns
///
/// Request body for captcha check.
pub fn captcha_check_request_body(puzzle: &DomesticCaptchaPuzzle, point_json: &str) -> Value {
    json!({
        "captchaType": puzzle.captcha_type,
        "pointJson": point_json,
        "token": puzzle.token,
        "ident": puzzle.ident,
        "returnUrl": puzzle.return_url,
    })
}

/// Return whether a `/verify-api/web/check` response accepted the pointJson.
///
/// # Arguments
///
/// * `body` - JSON response body.
///
/// # Returns
///
/// True when the captcha check succeeded.
pub fn captcha_check_succeeded(body: &Value) -> bool {
    if matches!(body.get("success"), Some(Value::Bool(true))) {
        return true;
    }
    if body.get("success").and_then(Value::as_str) == Some("true") {
        return true;
    }
    if body.get("success").and_then(Value::as_i64) == Some(1) {
        return true;
    }
    let container = body
        .get("repData")
        .or_else(|| body.get("data"))
        .and_then(Value::as_object);
    if let Some(container) = container {
        if matches!(container.get("result"), Some(Value::Bool(true))) {
            return true;
        }
        if container.get("result").and_then(Value::as_str) == Some("true") {
            return true;
        }
        if container.get("result").and_then(Value::as_i64) == Some(1) {
            return true;
        }
    }
    false
}

fn validate_puzzle(puzzle: &DomesticCaptchaPuzzle) -> Result<(), DomesticCnkiSourceError> {
    if puzzle.original_image_b64.is_empty()
        || puzzle.jigsaw_image_b64.is_empty()
        || puzzle.secret_key.is_empty()
        || puzzle.token.is_empty()
        || puzzle.captcha_id.is_empty()
    {
        return Err(DomesticCnkiSourceError::Parse(
            "domestic captcha puzzle missing required fields".to_string(),
        ));
    }
    if puzzle.secret_key.len() != 16 {
        return Err(DomesticCnkiSourceError::Parse(format!(
            "domestic captcha secretKey length is {}, expected 16",
            puzzle.secret_key.len()
        )));
    }
    Ok(())
}

fn puzzle_container(body: &Value) -> Option<&Value> {
    if body.get("originalImageBase64").is_some() {
        return Some(body);
    }
    for key in ["repData", "data"] {
        if let Some(value) = body.get(key) {
            if value.get("originalImageBase64").is_some() {
                return Some(value);
            }
            if let Some(nested) = value.as_object() {
                for nested_value in nested.values() {
                    if nested_value.get("originalImageBase64").is_some() {
                        return Some(nested_value);
                    }
                }
            }
        }
    }
    body.as_object().and_then(|map| {
        map.values()
            .find(|value| value.get("originalImageBase64").is_some())
    })
}

fn query_map(url: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let Some(query) = url.split_once('?').map(|(_, query)| query) else {
        return map;
    };
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if key.is_empty() {
            continue;
        }
        map.insert(percent_decode(key), percent_decode(value));
    }
    map
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hex = &value[index + 1..index + 3];
                if let Ok(byte) = u8::from_str_radix(hex, 16) {
                    output.push(byte);
                    index += 3;
                } else {
                    output.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn client_uid_hex() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    format!("{nanos:032x}")[..32].to_string()
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn map_jfbym_error(error: JfbymError) -> DomesticCnkiSourceError {
    match error {
        JfbymError::Configuration(message) => DomesticCnkiSourceError::Request(message),
        JfbymError::Request(message) | JfbymError::InvalidResponse(message) => {
            DomesticCnkiSourceError::Request(message)
        }
    }
}

fn journal_result_issn_near(text: &str, href: &str) -> Option<String> {
    let encoded_href = href.replace('&', "&amp;");
    let href_index = text.find(href).or_else(|| text.find(&encoded_href))?;
    let matched_len = if text[href_index..].starts_with(href) {
        href.len()
    } else {
        encoded_href.len()
    };
    let window_start = href_index.saturating_sub(80);
    let window_end = (href_index + matched_len + 400).min(text.len());
    let window = &text[window_start..window_end];
    let visible = strip_tags(window);
    label_value(&visible, &["ISSN"])
}

fn parse_article_row(row_html: &str, issue: &Value, section: &str) -> Option<Value> {
    let link = tags(row_html, "a").into_iter().find_map(|tag| {
        let tag_attrs = attrs(&tag);
        let href = tag_attrs.get("href")?;
        if !(href.contains("/kcms2/article/abstract") || href.contains("/article/abstract")) {
            return None;
        }
        let title = tag_attrs
            .get("title")
            .cloned()
            .and_then(|value| non_empty(&value))
            .or_else(|| non_empty(&strip_tags(&tag)))?;
        Some((with_domestic_platform(&absolute_domestic_url(href)), title))
    })?;
    let platform_id = tags(row_html, "b").into_iter().find_map(|tag| {
        let tag_attrs = attrs(&tag);
        tag_attrs
            .get("name")
            .is_some_and(|value| value == "encrypt")
            .then(|| tag_attrs.get("id").cloned())
            .flatten()
            .and_then(|value| non_empty(&value))
    });
    let authors = span_title(row_html, "author");
    let pages = span_title(row_html, "company");
    Some(json!({
        "title": link.1,
        "article_url": link.0,
        "platform_id": platform_id,
        "authors": authors,
        "pages": pages,
        "section": non_empty(section),
        "year": issue.get("year").cloned(),
        "number": issue.get("number").cloned(),
        "platform": DOMESTIC_PLATFORM,
    }))
}

fn summary_text(text: &str) -> Option<String> {
    tags(text, "span")
        .into_iter()
        .find_map(|tag| {
            let tag_attrs = attrs(&tag);
            let is_summary = tag_attrs
                .get("id")
                .is_some_and(|value| value == "ChDivSummary")
                || tag_attrs.get("class").is_some_and(|value| {
                    value.split_whitespace().any(|item| item == "abstract-text")
                });
            is_summary.then(|| non_empty(&strip_tags(&tag))).flatten()
        })
        .or_else(|| input_value(text, "abstract_text"))
}

fn looks_like_domestic_content(text: &str) -> bool {
    text.contains("YearIssueTree")
        || text.contains("id=\"pykm\"")
        || text.contains("ChDivSummary")
        || text.contains("/knavi/detail?")
        || text.contains("class=\"row clearfix")
        || text.contains("param-filename")
        || text.contains("paramfilename")
}

fn next_article_block(text: &str, from: usize) -> Option<(usize, &'static str)> {
    let dt = text[from..].find("<dt").map(|index| (from + index, "dt"));
    let dd = text[from..].find("<dd").map(|index| (from + index, "dd"));
    match (dt, dd) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    }
}

fn tags(text: &str, tag_name: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut cursor = 0;
    while let Some((block, end)) = find_tag_block(text, tag_name, cursor) {
        tags.push(block);
        cursor = end;
    }
    tags
}

fn find_tag_block(text: &str, tag_name: &str, from: usize) -> Option<(String, usize)> {
    let start = text[from..].find(&format!("<{tag_name}"))? + from;
    tag_block_at(text, tag_name, start)
}

fn tag_block_at(text: &str, tag_name: &str, start: usize) -> Option<(String, usize)> {
    let open_end = text[start..].find('>')? + start + 1;
    let close_marker = format!("</{tag_name}>");
    let close_start = text[open_end..].find(&close_marker)? + open_end;
    let end = close_start + close_marker.len();
    Some((text[start..end].to_string(), end))
}

fn attrs(tag: &str) -> BTreeMap<String, String> {
    let header = tag.split('>').next().unwrap_or(tag);
    let mut output = BTreeMap::new();
    for quote in ['"', '\''] {
        let mut cursor = 0;
        while let Some(equals_index) = header[cursor..].find('=') {
            let equals_index = cursor + equals_index;
            if !header[equals_index + 1..].starts_with(quote) {
                cursor = equals_index + 1;
                continue;
            }
            let key_start = header[..equals_index]
                .rfind(|character: char| character.is_whitespace() || character == '<')
                .map(|index| index + 1)
                .unwrap_or(0);
            let key = header[key_start..equals_index].trim().to_lowercase();
            let value_start = equals_index + 2;
            let Some(value_end) = header[value_start..]
                .find(quote)
                .map(|index| value_start + index)
            else {
                break;
            };
            if !key.is_empty() {
                output.insert(key, decode_html(&header[value_start..value_end]));
            }
            cursor = value_end + 1;
        }
    }
    output
}

fn input_value(text: &str, element_id: &str) -> Option<String> {
    start_tags(text, "input").into_iter().find_map(|tag| {
        let tag_attrs = attrs(&tag);
        tag_attrs
            .get("id")
            .is_some_and(|value| value == element_id)
            .then(|| tag_attrs.get("value").cloned())
            .flatten()
            .and_then(|value| non_empty(&value))
    })
}

fn start_tags(text: &str, tag_name: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut cursor = 0;
    let open = format!("<{tag_name}");
    while let Some(start) = text[cursor..].find(&open).map(|index| cursor + index) {
        let Some(end) = text[start..].find('>').map(|index| start + index + 1) else {
            break;
        };
        tags.push(text[start..end].to_string());
        cursor = end;
    }
    tags
}

fn span_title(text: &str, class_name: &str) -> Option<String> {
    tags(text, "span").into_iter().find_map(|tag| {
        let tag_attrs = attrs(&tag);
        tag_attrs
            .get("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == class_name))
            .then(|| {
                tag_attrs
                    .get("title")
                    .cloned()
                    .and_then(|value| clean_text(&value))
                    .or_else(|| non_empty(&strip_tags(&tag)))
            })
            .flatten()
    })
}

fn author_text(text: &str) -> Option<String> {
    let block = tags(text, "h3").into_iter().find(|tag| {
        let tag_attrs = attrs(tag);
        tag_attrs
            .get("id")
            .is_some_and(|value| value == "authorpart")
            && tag_attrs
                .get("class")
                .is_some_and(|value| value.split_whitespace().any(|item| item == "author"))
    })?;
    let names = tags(&block, "span")
        .into_iter()
        .filter_map(|tag| non_empty(&strip_tags(&tag)))
        .collect::<Vec<_>>();
    (!names.is_empty()).then(|| names.join("; "))
}

fn row_value(text: &str, label: &str) -> Option<String> {
    let mut cursor = 0;
    while let Some(start) = text[cursor..].find("<span").map(|index| cursor + index) {
        let Some((span, end)) = tag_block_at(text, "span", start) else {
            break;
        };
        let span_attrs = attrs(&span);
        if span_attrs
            .get("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == "rowtit"))
            && strip_tags(&span)
                .trim()
                .trim_end_matches([':', '：'])
                .trim()
                == label
        {
            if let Some((paragraph, _)) = find_tag_block(text, "p", end) {
                return non_empty(&strip_tags(&paragraph));
            }
            if let Some((next_span, _)) = find_tag_block(text, "span", end) {
                return non_empty(&strip_tags(&next_span));
            }
        }
        cursor = end;
    }
    None
}

fn first_block_text(text: &str, tag_name: &str, class_name: &str) -> Option<String> {
    tags(text, tag_name).into_iter().find_map(|tag| {
        attrs(&tag)
            .get("class")
            .is_some_and(|value| value.split_whitespace().any(|item| item == class_name))
            .then(|| non_empty(&strip_tags(&tag)))
            .flatten()
    })
}

fn title_text(text: &str) -> Option<String> {
    find_tag_block(text, "title", 0).and_then(|(block, _)| {
        non_empty(
            &strip_tags(&block)
                .replace(" - 中国知网", "")
                .replace("-中国知网", ""),
        )
    })
}

fn label_value(text: &str, labels: &[&str]) -> Option<String> {
    for label in labels {
        for separator in [':', '：'] {
            let marker = format!("{label}{separator}");
            if let Some(index) = text.find(&marker) {
                let rest = text[index + marker.len()..].trim_start();
                let value = rest
                    .split(|character: char| character.is_whitespace() || character == '；')
                    .next()
                    .unwrap_or_default();
                if let Some(clean) = non_empty(value) {
                    return Some(clean);
                }
            }
        }
    }
    None
}

fn issue_number(key: &str, label: &str) -> String {
    if let Some(digits) = key.get(4..).filter(|value| !value.is_empty()) {
        let trimmed = digits.trim_start_matches('0');
        return if trimmed.is_empty() {
            "0".to_string()
        } else {
            trimmed.to_string()
        };
    }
    let digits = label
        .chars()
        .filter(|character| character.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        label.to_string()
    } else {
        digits
    }
}

fn date_part(value: &str) -> Option<String> {
    non_empty(value.split_whitespace().next().unwrap_or_default())
}

fn strip_tags(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    for character in value.chars() {
        match character {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => output.push(character),
            _ => {}
        }
    }
    decode_html(&output)
}

fn clean_text(value: &str) -> Option<String> {
    non_empty(&decode_html(value).replace('\u{a0}', " ").trim())
}

fn decode_html(value: &str) -> String {
    value
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn json_text(value: Option<&Value>) -> Option<String> {
    value.and_then(|item| item.as_str().map(str::to_string))
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn absolute_domestic_url(value: &str) -> String {
    if value.starts_with("http://") || value.starts_with("https://") {
        value.to_string()
    } else if value.starts_with("/kcms") || value.starts_with("/starter") {
        format!("{DOMESTIC_KNS_BASE_URL}{value}")
    } else if value.starts_with('/') {
        format!("{DOMESTIC_NAVI_BASE_URL}{value}")
    } else {
        value.to_string()
    }
}

fn with_domestic_platform(url: &str) -> String {
    let absolute = absolute_domestic_url(url);
    let mut parts = absolute.splitn(2, '?');
    let path = parts.next().unwrap_or_default();
    let query = parts.next().unwrap_or_default();
    let mut pairs = query
        .split('&')
        .filter(|part| !part.is_empty())
        .filter(|part| {
            let key = part.split('=').next().unwrap_or_default().to_lowercase();
            key != "language" && key != "uniplatform"
        })
        .map(str::to_string)
        .collect::<Vec<_>>();
    pairs.push(format!("uniplatform={DOMESTIC_PLATFORM}"));
    pairs.push(format!("language={DOMESTIC_LANGUAGE}"));
    format!("{path}?{}", pairs.join("&"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_HTML: &str = r#"
        <div class="detials">
          <a target="_blank"
             href="https://navi.cnki.net/knavi/detail?p=TOKEN&amp;uniplatform=NZKPT&amp;language=CHS"
             title="世界经济">
            <span class="mask"></span>
          </a>
          <p>ISSN：1002-9621</p>
        </div>
        <a href="https://navi.cnki.net/knavi/detail?p=OTHER&uniplatform=NZKPT&language=CHS"
           title="世界经济研究">世界经济研究</a>
        <p>ISSN：1007-6964</p>
    "#;

    const DETAIL_HTML: &str = r#"
        <html><head><title>世界经济 - 中国知网</title></head>
        <body>
          <input id="pykm" type="hidden" value="SJJJ"/>
          <input id="pCode" type="hidden" value="CJFD,CCJD"/>
          <input type="hidden" id="shareChName" name="shareChName" value="世界经济"/>
          <span>ISSN：1002-9621</span>
          <span>CN：11-1138/F</span>
        </body></html>
    "#;

    const YEAR_HTML: &str = r#"
        <div id="YearIssueTree">
          <a id="yq202512"
             onclick="JournalDetail.BindIssueClick(this)"
             value="opaque-issue-token">No.12</a>
          <a id="yq202511"
             onclick="JournalDetail.BindIssueClick(this)"
             value="opaque-issue-token-11">No.11</a>
        </div>
    "#;

    const PAPERS_HTML: &str = r#"
        <dt class="tit">Articles</dt>
        <dd class="row clearfix">
          <span class="name">
            <a target="_blank"
               href="https://kns.cnki.net/kcms2/article/abstract?v=TOKEN&amp;uniplatform=NZKPT&amp;language=CHS">
              建立互利共赢的标准化合作伙伴关系
            </a>
            <b name="encrypt" id="SJJJ202512002"></b>
          </span>
          <span class="author" title="侯俊军;丁琪琪;">侯俊军;丁琪琪;</span>
          <span class="company" title="3-31">3-31</span>
        </dd>
    "#;

    const ABSTRACT_HTML: &str = r#"
        <html><head><title>建立互利共赢的标准化合作伙伴关系 - 中国知网</title></head>
        <body>
          <input type="hidden" id="param-dbcode" value="CJFQ">
          <input type="hidden" id="param-dbname" value="CJFDLAST2026">
          <input type="hidden" id="param-filename" value="SJJJ202512002">
          <input id="paramdbcode" type="hidden" value="CJFQ"/>
          <input id="abstract_text" type="hidden" value="&lt;正&gt;摘要正文样本"/>
          <span class="rowtit">摘要：</span>
          <span id="ChDivSummary" name="ChDivSummary" class="abstract-text">摘要正文样本</span>
          <span class="rowtit">DOI：</span><p>10.1000/domestic.sample</p>
        </body></html>
    "#;

    #[test]
    fn domestic_search_form_matches_har_shape_without_overseas_hosts() {
        let form = domestic_journal_search_form("世界经济", "TI");
        assert_eq!(form.get("parentcode").map(String::as_str), Some("SQN63324"));
        assert_eq!(form.get("switchdata").map(String::as_str), Some("search"));
        let state = form.get("searchStateJson").expect("search state");
        assert!(state.contains("OYXNO5VW"));
        assert!(state.contains("世界经济"));
        assert!(!state.contains("oversea.cnki.net"));
        assert!(!form
            .values()
            .any(|value| value.contains("oversea.cnki.net")));
    }

    #[test]
    fn parses_domestic_search_detail_year_papers_and_abstract() {
        let search = parse_domestic_journal_search_results(SEARCH_HTML).expect("search");
        assert_eq!(search.len(), 2);
        assert_eq!(search[0]["title"], "世界经济");
        assert_eq!(search[0]["issn"], "1002-9621");
        assert!(search[0]["detail_url"]
            .as_str()
            .unwrap_or_default()
            .starts_with("https://navi.cnki.net/knavi/detail?"));
        assert!(!contains_overseas_host(
            search[0]["detail_url"].as_str().unwrap_or_default()
        ));

        let detail = parse_domestic_journal_detail(DETAIL_HTML).expect("detail");
        assert_eq!(detail["pykm"], "SJJJ");
        assert_eq!(detail["pcode"], "CJFD,CCJD");
        assert_eq!(detail["title"], "世界经济");
        assert_eq!(detail["issn"], "1002-9621");
        assert_eq!(detail["platform"], "NZKPT");

        let issues = parse_domestic_year_issues(YEAR_HTML).expect("years");
        assert_eq!(issues.len(), 2);
        assert_eq!(issues[0]["year"], 2025);
        assert_eq!(issues[0]["number"], "12");
        assert_eq!(issues[0]["year_issue_id"], "202512");
        assert_eq!(issues[0]["year_issue"], "opaque-issue-token");

        let articles = parse_domestic_issue_articles(PAPERS_HTML, &issues[0]).expect("papers");
        assert_eq!(articles.len(), 1);
        assert_eq!(articles[0]["platform_id"], "SJJJ202512002");
        assert_eq!(articles[0]["authors"], "侯俊军;丁琪琪;");
        assert_eq!(articles[0]["pages"], "3-31");
        assert!(articles[0]["article_url"]
            .as_str()
            .unwrap_or_default()
            .starts_with("https://kns.cnki.net/kcms2/article/abstract?"));
        assert!(!contains_overseas_host(
            articles[0]["article_url"].as_str().unwrap_or_default()
        ));

        let article_url = articles[0]["article_url"].as_str().unwrap_or_default();
        let abstract_page =
            parse_domestic_article_detail(ABSTRACT_HTML, article_url).expect("abstract");
        assert_eq!(abstract_page["platform_id"], "SJJJ202512002");
        assert_eq!(abstract_page["dbcode"], "CJFQ");
        assert_eq!(abstract_page["dbname"], "CJFDLAST2026");
        assert_eq!(abstract_page["abstract"], "摘要正文样本");
        assert_eq!(abstract_page["doi"], "10.1000/domestic.sample");
        assert_eq!(abstract_page["platform"], "NZKPT");
        assert!(!contains_overseas_host(
            abstract_page["article_url"].as_str().unwrap_or_default()
        ));
    }

    #[test]
    fn rejects_captcha_and_overseas_hosts() {
        let captcha = checked_text(
            r#"{"code":-403,"message":"https://kns.cnki.net/verify/home?captchaType=blockPuzzle"}"#,
            "journal_search",
        );
        assert!(matches!(captcha, Err(DomesticCnkiSourceError::Request(_))));

        let overseas = parse_domestic_journal_search_results(
            r#"<a href="https://oversea.cnki.net/knavi/detail?p=1" title="x">x</a>"#,
        );
        assert!(matches!(overseas, Err(DomesticCnkiSourceError::Parse(_))));

        let overseas_detail = parse_domestic_article_detail(
            ABSTRACT_HTML,
            "https://oversea.cnki.net/kcms2/article/abstract?v=1",
        );
        assert!(matches!(
            overseas_detail,
            Err(DomesticCnkiSourceError::Parse(_))
        ));
    }

    #[test]
    fn fixtures_do_not_embed_captcha_secrets() {
        for sample in [
            SEARCH_HTML,
            DETAIL_HTML,
            YEAR_HTML,
            PAPERS_HTML,
            ABSTRACT_HTML,
        ] {
            let lowered = sample.to_lowercase();
            assert!(!lowered.contains("token="));
            assert!(!lowered.contains("secretkey"));
            assert!(!lowered.contains("captchaid"));
            assert!(!lowered.contains("jfbym"));
            assert!(!lowered.contains("api.jfbym.com"));
        }
    }

    #[test]
    fn detects_challenge_and_extracts_verify_url() {
        let body = r#"{"code":-403,"message":"https://kns.cnki.net/verify/home?captchaType=blockPuzzle&ident=eea05a&captchaId=2222b8cc-69e3-42a9-b1d2-07f08ff6dd54&returnUrl=opaque"}"#;
        assert!(looks_like_captcha_challenge(
            body,
            "https://navi.cnki.net/knavi/journals/searchbaseinfo"
        ));
        let challenge = extract_challenge_url(body, "search").expect("challenge");
        assert!(challenge.contains("/verify/home?"));
        assert!(challenge.contains("captchaId="));
        assert!(!challenge.contains("oversea.cnki.net"));
    }

    #[test]
    fn captcha_session_solves_within_budget_and_redacts_debug() {
        use crate::jfbym::FixtureJfbymSolver;

        let challenge = "https://kns.cnki.net/verify/home?captchaType=blockPuzzle&ident=eea05a&captchaId=2222b8cc-69e3-42a9-b1d2-07f08ff6dd54&returnUrl=opaque";
        let puzzle_body = json!({
            "repData": {
                "originalImageBase64": "AAAA",
                "jigsawImageBase64": "BBBB",
                "secretKey": "0123456789abcdef",
                "token": "tokentokentokentokentokentoken12"
            }
        });
        let puzzle = parse_captcha_puzzle(challenge, &puzzle_body).expect("puzzle");
        let debug = format!("{puzzle}");
        assert!(debug.contains("captcha_id_len: 36"));
        assert!(!debug.contains("2222b8cc"));
        assert!(!debug.contains("0123456789abcdef"));
        assert!(!debug.contains("tokentokentokentokentokentoken12"));
        assert!(!debug.contains("AAAA"));

        let mut session = DomesticCaptchaSession::with_budget(2);
        let mut solver = FixtureJfbymSolver::new(261.0);
        let mut accepted_x = None;
        session
            .solve_challenge(
                challenge,
                &mut solver,
                |_| Ok(puzzle.clone()),
                |puzzle, point_json| {
                    assert_eq!(puzzle.secret_key.len(), 16);
                    assert!(!point_json.is_empty());
                    assert!(!point_json.contains("261"));
                    accepted_x = Some(point_json.to_string());
                    Ok(true)
                },
            )
            .expect("solve");
        assert!(session.has_captcha_id());
        assert_eq!(session.captcha_id_len(), 36);
        let attached = session.attach_captcha_id("https://navi.cnki.net/knavi/journals/index");
        assert!(attached.contains("captchaId="));
        assert!(accepted_x.is_some());

        let get_body = captcha_get_request_body(challenge);
        assert_eq!(get_body["captchaType"], "blockPuzzle");
        assert_eq!(get_body["ident"], "eea05a");
        assert_eq!(get_body["clientUid"].as_str().unwrap().len(), 32);

        let check_body = captcha_check_request_body(&puzzle, "cipher");
        assert_eq!(check_body["pointJson"], "cipher");
        assert_eq!(check_body["token"], "tokentokentokentokentokentoken12");
        assert!(captcha_check_succeeded(&json!({"success": true})));
        assert!(captcha_check_succeeded(&json!({"data": {"result": true}})));
        assert!(!captcha_check_succeeded(&json!({"success": false})));
    }

    #[test]
    fn captcha_session_exhausts_budget_without_success() {
        use crate::jfbym::FixtureJfbymSolver;

        let challenge = "https://kns.cnki.net/verify/home?captchaType=blockPuzzle&ident=eea05a&captchaId=2222b8cc-69e3-42a9-b1d2-07f08ff6dd54&returnUrl=opaque";
        let puzzle_body = json!({
            "data": {
                "originalImageBase64": "AAAA",
                "jigsawImageBase64": "BBBB",
                "secretKey": "0123456789abcdef",
                "token": "tokentokentokentokentokentoken12"
            }
        });
        let puzzle = parse_captcha_puzzle(challenge, &puzzle_body).expect("puzzle");
        let mut session = DomesticCaptchaSession::with_budget(1);
        let mut solver = FixtureJfbymSolver::new(10.0);
        let error = session
            .solve_challenge(
                challenge,
                &mut solver,
                |_| Ok(puzzle.clone()),
                |_puzzle, _point| Ok(false),
            )
            .expect_err("budget");
        assert!(error.to_string().contains("budget exhausted"));
        assert!(!session.has_captcha_id());
    }
}
