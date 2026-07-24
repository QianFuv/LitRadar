//! Domestic NZKPT CNKI metadata client parsers, fixtures, and captcha session.
//!
//! This module is intentionally unregistered as a product provider until later
//! tasks wire index and abstract capabilities under the runtime name `cnki`.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::thread;
use std::time::Duration;

use crate::jfbym::{
    encrypt_point_json, point_x_candidates, strip_data_url_base64, JfbymError, JfbymSolver,
};
use crate::scholarly::{SourceAttempt, SourceError};
use litradar_domain::{
    normalize_bibliographic_text, normalize_contract_issn, normalize_contract_text,
};

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
const DOMESTIC_PAPERS_CONTINUATION_COUNT: usize = 10;
const DOMESTIC_REDIRECT_LIMIT: usize = 10;
const DOMESTIC_REQUEST_ATTEMPT_LIMIT: usize = 3;
/// Current stable domestic CNKI traversal checkpoint version.
pub const DOMESTIC_CNKI_CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Default)]
struct DomesticRequestBudget {
    ordinary_attempts: usize,
    captcha_replays: usize,
    has_pending_captcha_replay: bool,
}

struct DomesticAttempt<'a> {
    endpoint: &'a str,
    method: &'a str,
    request_url: &'a str,
    status_code: Option<u16>,
    did_succeed: bool,
    did_retry: bool,
    error: Option<&'a str>,
}

/// Ordered domestic journal identity candidates used by index and abstract resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomesticJournalLocator {
    titles: Vec<String>,
    issns: Vec<String>,
    normalized_titles: BTreeSet<String>,
    normalized_issns: BTreeSet<String>,
}

/// One validated page of domestic issue article summaries.
#[derive(Debug, Clone, PartialEq)]
pub struct DomesticIssueArticlePage {
    /// Parsed article summaries in upstream order.
    pub articles: Vec<Value>,
    /// Zero-based page index used for this response.
    pub page_index: usize,
    /// Validated upstream article count for this page.
    pub article_count: usize,
    /// Whether the fixed-size page requires a following page request.
    pub has_next_page: bool,
}

impl DomesticJournalLocator {
    /// Build a locator from canonical and alias titles plus print/electronic ISSNs.
    ///
    /// # Arguments
    ///
    /// * `titles` - Ordered title search candidates.
    /// * `issns` - Ordered ISSN search candidates.
    ///
    /// # Returns
    ///
    /// Locator with empty, invalid, and normalized duplicate candidates removed.
    pub fn new(titles: Vec<String>, issns: Vec<String>) -> Self {
        let mut normalized_titles = BTreeSet::new();
        let titles = titles
            .into_iter()
            .filter_map(|title| normalize_contract_text(&title))
            .filter(|title| normalized_titles.insert(normalize_bibliographic_text(title)))
            .collect();
        let mut normalized_issns = BTreeSet::new();
        let issns = issns
            .into_iter()
            .filter_map(|issn| normalize_contract_issn(&issn))
            .filter(|issn| normalized_issns.insert(issn.clone()))
            .collect();
        Self {
            titles,
            issns,
            normalized_titles,
            normalized_issns,
        }
    }

    /// Return ordered title queries after normalized deduplication.
    ///
    /// # Returns
    ///
    /// Canonical title followed by distinct aliases.
    pub fn titles(&self) -> &[String] {
        &self.titles
    }

    /// Return ordered canonical ISSN queries after deduplication.
    ///
    /// # Returns
    ///
    /// Distinct print/electronic ISSNs in caller order.
    pub fn issns(&self) -> &[String] {
        &self.issns
    }
}

impl DomesticRequestBudget {
    fn next_attempt(&mut self) -> Option<bool> {
        if self.has_pending_captcha_replay {
            if self.captcha_replays >= DOMESTIC_CAPTCHA_SOLVE_BUDGET {
                return None;
            }
            self.has_pending_captcha_replay = false;
            self.captcha_replays += 1;
            return Some(true);
        }
        if self.ordinary_attempts >= DOMESTIC_REQUEST_ATTEMPT_LIMIT {
            return None;
        }
        self.ordinary_attempts += 1;
        Some(false)
    }

    fn schedule_captcha_replay(&mut self) -> Result<(), DomesticCnkiSourceError> {
        if self.captcha_replays >= DOMESTIC_CAPTCHA_SOLVE_BUDGET {
            return Err(DomesticCnkiSourceError::Request(
                "domestic CNKI captcha replay budget exhausted".to_string(),
            ));
        }
        self.has_pending_captcha_replay = true;
        Ok(())
    }

    fn can_retry_ordinary(&self) -> bool {
        self.ordinary_attempts < DOMESTIC_REQUEST_ATTEMPT_LIMIT
    }

    fn did_retry(&self) -> bool {
        self.ordinary_attempts + self.captcha_replays > 1
    }
}

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
    /// Issue article HTML pages keyed by year-issue id such as `202512`.
    #[serde(default)]
    pub issue_article_pages: BTreeMap<String, Vec<String>>,
    /// Article detail HTML keyed by platform id.
    #[serde(default)]
    pub article_detail_html: BTreeMap<String, String>,
    /// Optional article detail HTTP status keyed by platform id.
    #[serde(default)]
    pub article_detail_status_codes: BTreeMap<String, u16>,
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
    /// One article detail is explicitly and permanently unavailable.
    PermanentArticleMissing,
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
            Self::PermanentArticleMissing => {
                formatter.write_str("domestic CNKI article is permanently unavailable")
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
    validate_domestic_response("journal_search", text)?;
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
        let detail_url = with_domestic_platform(href)?;
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
            "detail_url": detail_url,
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
    validate_domestic_response("journal_detail", text)?;
    let pykm = input_value(text, "pykm").ok_or_else(|| {
        DomesticCnkiSourceError::Parse("domestic journal detail missing pykm".to_string())
    })?;
    let pcode = input_value(text, "pCode").unwrap_or_else(|| DEFAULT_PCODE.to_string());
    let visible_text = strip_tags(text);
    let detail_url = with_domestic_platform(&format!(
        "{DOMESTIC_NAVI_BASE_URL}/knavi/detail?pykm={pykm}"
    ))?;
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
    validate_domestic_response("year_issues", text)?;
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
/// * `page_index` - Zero-based papers page index.
///
/// # Returns
///
/// Article summary payloads.
pub fn parse_domestic_issue_articles(
    text: &str,
    issue: &Value,
    page_index: usize,
) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError> {
    let is_explicit_empty_page = is_explicitly_empty_issue_page(text, page_index);
    if is_explicit_empty_page {
        checked_text(text, "issue_articles")?;
    } else {
        validate_domestic_response("issue_articles", text)?;
    }
    let article_count = match input_value(text, "articleCount") {
        Some(value) => value.parse::<usize>().map_err(|_| {
            DomesticCnkiSourceError::Parse(
                "domestic issue article page has invalid articleCount".to_string(),
            )
        })?,
        None if is_explicit_empty_page => 0,
        None => {
            return Err(DomesticCnkiSourceError::Parse(
                "domestic issue article page missing articleCount".to_string(),
            ));
        }
    };
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
            if let Some(article) = parse_article_row(&block, issue, &current_section)? {
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
    if articles.len() != article_count {
        return Err(DomesticCnkiSourceError::Parse(
            "domestic issue article count does not match parsed rows".to_string(),
        ));
    }
    Ok(DomesticIssueArticlePage {
        articles,
        page_index,
        article_count,
        has_next_page: article_count == DOMESTIC_PAPERS_CONTINUATION_COUNT,
    })
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
    validate_domestic_response("article_detail", text)?;
    let article_url = absolute_domestic_url(article_url)?;
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
    let permalink = with_domestic_platform(&article_url)?;
    Ok(json!({
        "article_url": permalink.clone(),
        "platform_id": filename,
        "dbcode": dbcode,
        "dbname": dbname,
        "title": title,
        "authors": author_text(text).or_else(|| span_title(text, "author")),
        "abstract": abstract_text,
        "doi": row_value(text, "DOI"),
        "online_release_date": online_time.and_then(|value| date_part(&value)),
        "pages": label_value(&strip_tags(text), &["页码", "Pages"]),
        "permalink": permalink,
        "platform": DOMESTIC_PLATFORM,
    }))
}

/// Validate domestic CNKI response text.
///
/// # Arguments
///
/// * `text` - Response text.
/// * `_url` - Request URL or fixture key retained for API compatibility.
///
/// # Returns
///
/// Ok when the response appears usable.
pub fn checked_text(text: &str, _url: &str) -> Result<(), DomesticCnkiSourceError> {
    if text.trim().is_empty() {
        return Err(DomesticCnkiSourceError::Parse(
            "domestic CNKI returned an empty response".to_string(),
        ));
    }
    let lowered = text.to_lowercase();
    if (lowered.contains("captcha")
        || text.contains("访问异常")
        || text.contains("安全验证")
        || text.contains("\"code\":-403")
        || text.contains("/verify/home"))
        && !looks_like_domestic_content(text)
    {
        return Err(DomesticCnkiSourceError::Request(
            "domestic CNKI verification required".to_string(),
        ));
    }
    Ok(())
}

impl DomesticCnkiSourceError {
    /// Return the upstream HTTP status when this is a status-aware request failure.
    ///
    /// # Returns
    ///
    /// Exact status code, or `None` for transport, captcha, fixture, and parse failures.
    pub fn http_status(&self) -> Option<u16> {
        let Self::Request(message) = self else {
            return None;
        };
        message
            .strip_prefix("domestic CNKI HTTP status ")
            .and_then(|value| value.parse().ok())
    }
}

fn validate_domestic_response(endpoint: &str, text: &str) -> Result<(), DomesticCnkiSourceError> {
    checked_text(text, endpoint)?;
    if endpoint == "article_detail" && is_explicitly_missing_article(text) {
        return Err(DomesticCnkiSourceError::PermanentArticleMissing);
    }
    let has_expected_structure = match endpoint {
        "journal_search" => text.contains("/knavi/detail?") || has_explicit_empty_marker(text),
        "journal_detail" => input_value(text, "pykm").is_some() || has_explicit_empty_marker(text),
        "year_issues" => text.contains("YearIssueTree"),
        "issue_articles" => {
            text.contains("articleCount")
                || text.contains("class=\"row clearfix")
                || is_domestic_update_placeholder(text)
        }
        "article_detail" => {
            input_value(text, "paramfilename").is_some()
                || input_value(text, "param-filename").is_some()
        }
        _ => false,
    };
    if !has_expected_structure {
        return Err(DomesticCnkiSourceError::Parse(format!(
            "domestic CNKI {endpoint} response is structurally incomplete"
        )));
    }
    Ok(())
}

fn is_explicitly_missing_article(text: &str) -> bool {
    let lowered = text.to_lowercase();
    [
        "记录已删除",
        "文献不存在",
        "该文献不存在",
        "record has been deleted",
        "record does not exist",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

fn has_explicit_empty_marker(text: &str) -> bool {
    let lowered = text.to_lowercase();
    [
        "暂无数据",
        "暂无相关",
        "未检索到",
        "没有找到",
        "无相关记录",
        "共 0 条结果",
        "共0条结果",
        "找到 0 条结果",
        "找到0条结果",
        "no results",
    ]
    .iter()
    .any(|marker| lowered.contains(marker))
}

fn is_explicitly_empty_issue_page(text: &str, page_index: usize) -> bool {
    if input_value(text, "articleCount").is_some() || text.contains("class=\"row clearfix") {
        return false;
    }
    if has_explicit_empty_marker(text) {
        return true;
    }
    page_index > 0 && is_domestic_update_placeholder(text)
}

fn is_domestic_update_placeholder(text: &str) -> bool {
    strip_tags(&decode_html(text)).contains("该刊数据正在更新中，请耐心等待")
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
#[derive(Clone, PartialEq, Eq)]
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
            "DomesticCaptchaPuzzle {{ captcha_type_len: {}, captcha_id_len: {}, secret_key_len: {}, token_len: {}, original_len: {}, jigsaw_len: {} }}",
            self.captcha_type.len(),
            self.captcha_id.len(),
            self.secret_key.len(),
            self.token.len(),
            self.original_image_b64.len(),
            self.jigsaw_image_b64.len()
        )
    }
}

impl fmt::Debug for DomesticCaptchaPuzzle {
    /// Format puzzle metadata without secrets or image payloads.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// Memory-only captcha session state shared by domestic index and abstract calls.
#[derive(Clone)]
pub struct DomesticCaptchaSession {
    captcha_id: Option<String>,
    solve_attempts: usize,
    solve_budget: usize,
}

impl fmt::Debug for DomesticCaptchaSession {
    /// Format captcha session state without exposing the retained captcha id.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DomesticCaptchaSession")
            .field("has_captcha_id", &self.has_captcha_id())
            .field("captcha_id_len", &self.captcha_id_len())
            .field("solve_attempts", &self.solve_attempts)
            .field("solve_budget", &self.solve_budget)
            .finish()
    }
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
    pub fn attach_captcha_id(&self, url: &str) -> Result<String, DomesticCnkiSourceError> {
        let mut parsed = parse_domestic_url(url)?;
        let Some(captcha_id) = self.captcha_id.as_deref().filter(|value| !value.is_empty()) else {
            return Ok(parsed.to_string());
        };
        if parsed.query_pairs().any(|(key, _)| key == "captchaId") {
            return Ok(parsed.to_string());
        }
        parsed
            .query_pairs_mut()
            .append_pair("captchaId", captcha_id);
        Ok(parsed.to_string())
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
            extract_challenge_url(response_text, response_url)?.ok_or_else(|| {
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
            let candidates = point_x_candidates(distance).map_err(map_jfbym_error)?;
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
    let lowered_url_path = url.split(['?', '#']).next().unwrap_or(url).to_lowercase();
    lowered.contains("captcha")
        || text.contains("访问异常")
        || text.contains("安全验证")
        || text.contains("\"code\":-403")
        || text.contains("/verify/home")
        || lowered_url_path.contains("/verify/home")
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
pub fn extract_challenge_url(
    text: &str,
    url: &str,
) -> Result<Option<String>, DomesticCnkiSourceError> {
    if url.contains("/verify/home") {
        return parse_challenge_url(url).map(Some);
    }
    if let Ok(payload) = serde_json::from_str::<Value>(text) {
        if let Some(message) = payload.get("message").and_then(Value::as_str) {
            if message.contains("/verify/home") {
                return parse_challenge_url(message).map(Some);
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
        return parse_challenge_url(&rest[..end]).map(Some);
    }
    let relative = "/verify/home?";
    if let Some(start) = text.find(relative) {
        let rest = &text[start..];
        let end = rest
            .find(|character: char| {
                character.is_whitespace() || character == '"' || character == '\''
            })
            .unwrap_or(rest.len());
        return parse_challenge_url(&rest[..end]).map(Some);
    }
    Ok(None)
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
    let challenge_url = parse_challenge_url(challenge_url)?;
    let query = query_map(&challenge_url)?;
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
        challenge_url,
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
pub fn captcha_get_request_body(challenge_url: &str) -> Result<Value, DomesticCnkiSourceError> {
    let query = query_map(challenge_url)?;
    Ok(json!({
        "captchaType": query.get("captchaType").cloned().unwrap_or_else(|| "blockPuzzle".to_string()),
        "clientUid": client_uid_hex(),
        "ts": unix_millis(),
        "ident": query.get("ident").cloned().unwrap_or_default(),
        "captchaId": query.get("captchaId").cloned().unwrap_or_default(),
    }))
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

fn query_map(url: &str) -> Result<BTreeMap<String, String>, DomesticCnkiSourceError> {
    let parsed = parse_domestic_url(&decode_html(url))?;
    Ok(parsed
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect())
}

fn parse_challenge_url(value: &str) -> Result<String, DomesticCnkiSourceError> {
    let decoded = decode_html(value);
    let absolute = if decoded.starts_with("/verify/home") {
        format!("{DOMESTIC_KNS_BASE_URL}{decoded}")
    } else {
        decoded
    };
    let parsed = parse_domestic_url(&absolute)?;
    if parsed.host_str() != Some("kns.cnki.net") || parsed.path() != "/verify/home" {
        return Err(DomesticCnkiSourceError::Request(
            "domestic CNKI challenge URL is not allowed".to_string(),
        ));
    }
    Ok(parsed.to_string())
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
    let window_start = previous_char_boundary(text, href_index.saturating_sub(80));
    let window_end = next_char_boundary(text, (href_index + matched_len + 400).min(text.len()));
    let window = &text[window_start..window_end];
    let visible = strip_tags(window);
    label_value(&visible, &["ISSN"])
}

fn parse_article_row(
    row_html: &str,
    issue: &Value,
    section: &str,
) -> Result<Option<Value>, DomesticCnkiSourceError> {
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
        Some((href.clone(), title))
    });
    let Some((href, title)) = link else {
        return Ok(None);
    };
    let link = (with_domestic_platform(&href)?, title);
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
    Ok(Some(json!({
        "title": link.1,
        "article_url": link.0,
        "platform_id": platform_id,
        "authors": authors,
        "pages": pages,
        "section": non_empty(section),
        "year": issue.get("year").cloned(),
        "number": issue.get("number").cloned(),
        "platform": DOMESTIC_PLATFORM,
    })))
}

fn previous_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn next_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
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
        || has_explicit_empty_marker(text)
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
    non_empty(decode_html(value).replace('\u{a0}', " ").trim())
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

fn absolute_domestic_url(value: &str) -> Result<String, DomesticCnkiSourceError> {
    parse_domestic_url(value).map(|url| url.to_string())
}

fn parse_domestic_url(value: &str) -> Result<Url, DomesticCnkiSourceError> {
    let value = value.trim();
    if value.is_empty() || value.starts_with("//") {
        return Err(DomesticCnkiSourceError::Request(
            "domestic CNKI URL is invalid".to_string(),
        ));
    }
    let parsed = if let Ok(url) = Url::parse(value) {
        url
    } else {
        if !value.starts_with('/') {
            return Err(DomesticCnkiSourceError::Request(
                "domestic CNKI relative URL is invalid".to_string(),
            ));
        }
        let base = if value.starts_with("/kcms")
            || value.starts_with("/starter")
            || value.starts_with("/verify")
        {
            DOMESTIC_KNS_BASE_URL
        } else {
            DOMESTIC_NAVI_BASE_URL
        };
        Url::parse(base)
            .and_then(|base| base.join(value))
            .map_err(|_| {
                DomesticCnkiSourceError::Request("domestic CNKI URL is invalid".to_string())
            })?
    };
    validate_domestic_url(&parsed)?;
    Ok(parsed)
}

fn validate_domestic_url(url: &Url) -> Result<(), DomesticCnkiSourceError> {
    if !is_allowed_domestic_url(url) {
        return Err(DomesticCnkiSourceError::Request(
            "domestic CNKI URL is not allowed".to_string(),
        ));
    }
    Ok(())
}

fn is_allowed_domestic_url(url: &Url) -> bool {
    url.scheme() == "https"
        && matches!(url.host_str(), Some("navi.cnki.net") | Some("kns.cnki.net"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none_or(|port| port == 443)
}

fn with_domestic_platform(url: &str) -> Result<String, DomesticCnkiSourceError> {
    let mut parsed = parse_domestic_url(url)?;
    let pairs = parsed
        .query_pairs()
        .filter(|(key, _)| !key.eq_ignore_ascii_case("language"))
        .filter(|(key, _)| !key.eq_ignore_ascii_case("uniplatform"))
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    parsed.set_query(None);
    {
        let mut query = parsed.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(&key, &value);
        }
        query.append_pair("uniplatform", DOMESTIC_PLATFORM);
        query.append_pair("language", DOMESTIC_LANGUAGE);
    }
    Ok(parsed.to_string())
}

fn redact_domestic_url(url: &str) -> String {
    let Ok(mut parsed) = Url::parse(url) else {
        return "[REDACTED INVALID DOMESTIC URL]".to_string();
    };
    let pairs = parsed
        .query_pairs()
        .map(|(key, value)| {
            let should_redact = matches!(
                key.to_ascii_lowercase().as_str(),
                "captchaid"
                    | "ident"
                    | "returnurl"
                    | "token"
                    | "secretkey"
                    | "pointjson"
                    | "originalimagebase64"
                    | "jigsawimagebase64"
            );
            (
                key.into_owned(),
                if should_redact {
                    "[REDACTED]".to_string()
                } else {
                    value.into_owned()
                },
            )
        })
        .collect::<Vec<_>>();
    parsed.set_query(None);
    if !pairs.is_empty() {
        let mut query = parsed.query_pairs_mut();
        for (key, value) in pairs {
            query.append_pair(&key, &value);
        }
    }
    parsed.to_string()
}

fn domestic_http_status_error(status_code: u16) -> DomesticCnkiSourceError {
    DomesticCnkiSourceError::Request(format!("domestic CNKI HTTP status {status_code}"))
}

fn validate_domestic_http_response(response: &Response) -> Result<(), DomesticCnkiSourceError> {
    validate_domestic_url(response.url())?;
    if !response.status().is_success() {
        return Err(domestic_http_status_error(response.status().as_u16()));
    }
    Ok(())
}

fn parse_domestic_json_response(
    response: Response,
    endpoint: &str,
) -> Result<Value, DomesticCnkiSourceError> {
    validate_domestic_http_response(&response)?;
    response.json().map_err(|_| {
        DomesticCnkiSourceError::Parse(format!(
            "domestic CNKI {endpoint} response is not valid JSON"
        ))
    })
}

/// Domestic NZKPT source transport abstraction.
pub trait DomesticCnkiTransport {
    /// Resolve one journal locator to domestic journal details.
    ///
    /// # Arguments
    ///
    /// * `locator` - Ordered canonical/alias title and ISSN candidates.
    ///
    /// # Returns
    ///
    /// Parsed journal details when found.
    fn resolve_journal(
        &mut self,
        locator: &DomesticJournalLocator,
    ) -> Result<Option<Value>, DomesticCnkiSourceError>;

    /// Fetch publication issues for one journal.
    ///
    /// # Arguments
    ///
    /// * `journal` - Domestic journal details.
    ///
    /// # Returns
    ///
    /// Parsed issue payloads.
    fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, DomesticCnkiSourceError>;

    /// Fetch article summaries for one issue.
    ///
    /// # Arguments
    ///
    /// * `journal` - Domestic journal details.
    /// * `issue` - Domestic issue payload.
    /// * `page_index` - Zero-based papers page index.
    ///
    /// # Returns
    ///
    /// Validated article summary page.
    fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
        page_index: usize,
    ) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError>;

    /// Fetch one article detail payload.
    ///
    /// # Arguments
    ///
    /// * `article_url` - Article URL from issue summary.
    /// * `platform_id` - Optional platform id from issue summary.
    ///
    /// # Returns
    ///
    /// Article detail payload.
    fn article_detail(
        &mut self,
        article_url: &str,
        platform_id: Option<&str>,
    ) -> Result<Value, DomesticCnkiSourceError>;

    /// Return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured source attempts.
    fn attempts(&self) -> &[SourceAttempt];

    /// Remove and return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured attempts, leaving the transport buffer empty.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt>;
}

/// Deterministic fixture transport for domestic NZKPT tests.
#[derive(Debug, Clone)]
pub struct FixtureDomesticCnkiTransport {
    data: DomesticCnkiFixtureData,
    attempts: Vec<SourceAttempt>,
}

impl FixtureDomesticCnkiTransport {
    /// Build a fixture transport from response data.
    ///
    /// # Arguments
    ///
    /// * `data` - Domestic fixture response payloads.
    ///
    /// # Returns
    ///
    /// Fixture transport.
    pub fn new(data: DomesticCnkiFixtureData) -> Self {
        Self {
            data,
            attempts: Vec::new(),
        }
    }

    fn record_attempt(
        &mut self,
        endpoint: &str,
        key: Option<&str>,
        did_succeed: bool,
        error: Option<String>,
    ) {
        self.attempts.push(SourceAttempt {
            service: "cnki".to_string(),
            endpoint: endpoint.to_string(),
            method: if endpoint == "journal_detail" || endpoint == "article_detail" {
                "GET".to_string()
            } else {
                "POST".to_string()
            },
            url: domestic_fixture_url(endpoint, key),
            status_code: Some(if did_succeed { 200 } else { 500 }),
            did_succeed,
            did_retry: false,
            error,
        });
    }
}

impl DomesticCnkiTransport for FixtureDomesticCnkiTransport {
    /// Resolve one fixture journal locator against observed detail identities.
    fn resolve_journal(
        &mut self,
        locator: &DomesticJournalLocator,
    ) -> Result<Option<Value>, DomesticCnkiSourceError> {
        if self
            .data
            .fail_endpoint
            .as_deref()
            .is_some_and(|value| value == "journal_detail")
        {
            let message = "domestic CNKI fixture failed for journal_detail".to_string();
            self.record_attempt("journal_detail", None, false, Some(message.clone()));
            return Err(DomesticCnkiSourceError::Parse(message));
        }
        if self.data.journal_detail_html.trim().is_empty() {
            self.record_attempt("journal_detail", None, true, None);
            return Ok(None);
        }
        let details = parse_domestic_journal_detail(&self.data.journal_detail_html)?;
        self.record_attempt("journal_detail", None, true, None);
        if domestic_journal_detail_matches(&details, locator) {
            Ok(Some(details))
        } else {
            Ok(None)
        }
    }

    /// Fetch fixture year issues for one journal.
    fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, DomesticCnkiSourceError> {
        let _ = journal;
        if self
            .data
            .fail_endpoint
            .as_deref()
            .is_some_and(|value| value == "year_issues")
        {
            let message = "domestic CNKI fixture failed for year_issues".to_string();
            self.record_attempt("year_issues", None, false, Some(message.clone()));
            return Err(DomesticCnkiSourceError::Parse(message));
        }
        let issues = parse_domestic_year_issues(&self.data.year_issues_html)?;
        self.record_attempt("year_issues", None, true, None);
        Ok(issues)
    }

    /// Fetch fixture article summaries for one issue.
    fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
        page_index: usize,
    ) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError> {
        let _ = journal;
        let year_issue_id = json_text(issue.get("year_issue_id"))
            .or_else(|| json_text(issue.get("year_issue")))
            .ok_or_else(|| {
                DomesticCnkiSourceError::Parse(
                    "domestic CNKI issue missing year_issue_id".to_string(),
                )
            })?;
        if self
            .data
            .fail_endpoint
            .as_deref()
            .is_some_and(|value| value == "issue_articles")
        {
            let message = "domestic CNKI fixture failed for issue_articles".to_string();
            self.record_attempt(
                "issue_articles",
                Some(&year_issue_id),
                false,
                Some(message.clone()),
            );
            return Err(DomesticCnkiSourceError::Parse(message));
        }
        let text = self
            .data
            .issue_article_pages
            .get(&year_issue_id)
            .and_then(|pages| pages.get(page_index))
            .cloned()
            .ok_or_else(|| {
                DomesticCnkiSourceError::MissingFixture(format!(
                    "domestic CNKI fixture missing issue_articles page {page_index} for {year_issue_id}"
                ))
            })?;
        let page = parse_domestic_issue_articles(&text, issue, page_index)?;
        let attempt_key = format!("{year_issue_id}:{page_index}");
        self.record_attempt("issue_articles", Some(&attempt_key), true, None);
        Ok(page)
    }

    /// Fetch one fixture article detail payload.
    fn article_detail(
        &mut self,
        article_url: &str,
        platform_id: Option<&str>,
    ) -> Result<Value, DomesticCnkiSourceError> {
        let key = platform_id.unwrap_or(article_url).to_string();
        if let Some(status_code) = self.data.article_detail_status_codes.get(&key).copied() {
            self.attempts.push(SourceAttempt {
                service: "cnki".to_string(),
                endpoint: "article_detail".to_string(),
                method: "GET".to_string(),
                url: domestic_fixture_url("article_detail", Some(&key)),
                status_code: Some(status_code),
                did_succeed: false,
                did_retry: false,
                error: Some("HTTP status".to_string()),
            });
            return Err(domestic_http_status_error(status_code));
        }
        if self
            .data
            .fail_endpoint
            .as_deref()
            .is_some_and(|value| value == "article_detail")
        {
            let message = "domestic CNKI fixture failed for article_detail".to_string();
            self.record_attempt("article_detail", Some(&key), false, Some(message.clone()));
            return Err(DomesticCnkiSourceError::Parse(message));
        }
        let text = self
            .data
            .article_detail_html
            .get(&key)
            .cloned()
            .ok_or_else(|| {
                DomesticCnkiSourceError::MissingFixture(format!(
                    "domestic CNKI fixture missing article_detail for {key}"
                ))
            })?;
        let detail = parse_domestic_article_detail(&text, article_url)?;
        self.record_attempt("article_detail", Some(&key), true, None);
        Ok(detail)
    }

    /// Return captured source attempts.
    fn attempts(&self) -> &[SourceAttempt] {
        &self.attempts
    }

    /// Remove and return captured source attempts.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        std::mem::take(&mut self.attempts)
    }
}

/// Domestic NZKPT metadata client using a transport implementation.
#[derive(Debug, Clone)]
pub struct DomesticCnkiClient<T> {
    transport: T,
}

impl<T> DomesticCnkiClient<T>
where
    T: DomesticCnkiTransport,
{
    /// Build a domestic CNKI client from a transport.
    ///
    /// # Arguments
    ///
    /// * `transport` - Domestic source transport.
    ///
    /// # Returns
    ///
    /// Domestic CNKI client.
    pub fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Resolve one journal locator to domestic journal details.
    ///
    /// # Arguments
    ///
    /// * `locator` - Ordered canonical/alias title and ISSN candidates.
    ///
    /// # Returns
    ///
    /// Parsed journal details when found.
    pub fn resolve_journal(
        &mut self,
        locator: &DomesticJournalLocator,
    ) -> Result<Option<Value>, DomesticCnkiSourceError> {
        self.transport.resolve_journal(locator)
    }

    /// Fetch publication issues for one journal.
    ///
    /// # Arguments
    ///
    /// * `journal` - Domestic journal details.
    ///
    /// # Returns
    ///
    /// Parsed issue payloads.
    pub fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, DomesticCnkiSourceError> {
        self.transport.year_issues(journal)
    }

    /// Fetch article summaries for one issue.
    ///
    /// # Arguments
    ///
    /// * `journal` - Domestic journal details.
    /// * `issue` - Domestic issue payload.
    /// * `page_index` - Zero-based papers page index.
    ///
    /// # Returns
    ///
    /// Validated article summary page.
    pub fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
        page_index: usize,
    ) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError> {
        self.transport.issue_articles(journal, issue, page_index)
    }

    /// Fetch one article detail payload.
    ///
    /// # Arguments
    ///
    /// * `article_url` - Article URL from issue summary.
    /// * `platform_id` - Optional platform id from issue summary.
    ///
    /// # Returns
    ///
    /// Article detail payload.
    pub fn article_detail(
        &mut self,
        article_url: &str,
        platform_id: Option<&str>,
    ) -> Result<Value, DomesticCnkiSourceError> {
        self.transport.article_detail(article_url, platform_id)
    }

    /// Return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured source attempts.
    pub fn attempts(&self) -> &[SourceAttempt] {
        self.transport.attempts()
    }

    /// Remove and return captured source attempts.
    ///
    /// # Returns
    ///
    /// Captured attempts, leaving the client buffer empty.
    pub fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        self.transport.drain_attempts()
    }
}

/// Opaque checkpoint for resumable domestic index walks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomesticCnkiCheckpoint {
    /// Checkpoint schema version.
    pub version: u32,
    /// Stable year-issue identifier for the next page to process.
    pub year_issue_id: String,
    /// Zero-based papers page index within the stable issue.
    pub page_index: usize,
}

fn domestic_fixture_url(endpoint: &str, key: Option<&str>) -> String {
    match (endpoint, key) {
        ("journal_detail", _) => format!("{DOMESTIC_NAVI_BASE_URL}/knavi/detail"),
        ("year_issues", _) => format!("{DOMESTIC_NAVI_BASE_URL}/knavi/journals/yearList"),
        ("issue_articles", Some(key)) => {
            format!("{DOMESTIC_NAVI_BASE_URL}/knavi/journals/papers?yearIssue={key}")
        }
        ("article_detail", Some(key)) => {
            format!("{DOMESTIC_KNS_BASE_URL}/kcms2/article/abstract?v={key}")
        }
        _ => format!("{DOMESTIC_NAVI_BASE_URL}/knavi/{endpoint}"),
    }
}

fn domestic_journal_detail_matches(details: &Value, locator: &DomesticJournalLocator) -> bool {
    let observed = DomesticJournalLocator::new(
        json_strings(details, "title", "title_aliases"),
        json_strings(details, "issn", "issns")
            .into_iter()
            .chain(json_text(details.get("eissn")))
            .collect(),
    );
    if !locator.normalized_issns.is_empty() && !observed.normalized_issns.is_empty() {
        return !locator
            .normalized_issns
            .is_disjoint(&observed.normalized_issns);
    }
    !locator
        .normalized_titles
        .is_disjoint(&observed.normalized_titles)
}

fn json_strings(value: &Value, scalar_field: &str, array_field: &str) -> Vec<String> {
    let mut values = json_text(value.get(scalar_field))
        .into_iter()
        .collect::<Vec<_>>();
    values.extend(
        value
            .get(array_field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_string)),
    );
    values
}

fn append_unique_domestic_detail_urls(
    detail_urls: &mut Vec<String>,
    seen_detail_urls: &mut BTreeSet<String>,
    candidates: Vec<Value>,
) {
    for candidate in candidates {
        let Some(detail_url) = json_text(candidate.get("detail_url")) else {
            continue;
        };
        if seen_detail_urls.insert(detail_url.clone()) {
            detail_urls.push(detail_url);
        }
    }
}

/// Live domestic CNKI transport configuration.
#[derive(Clone)]
pub struct LiveDomesticCnkiConfig {
    /// HTTP request timeout in seconds.
    pub timeout_seconds: u64,
    /// Optional jfbym token used when CNKI returns a captcha challenge.
    pub captcha_token: Option<String>,
}

impl fmt::Debug for LiveDomesticCnkiConfig {
    /// Format configuration without exposing captcha tokens.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveDomesticCnkiConfig")
            .field("timeout_seconds", &self.timeout_seconds)
            .field(
                "captcha_token",
                &self.captcha_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Blocking HTTP transport for live domestic NZKPT sources.
#[derive(Clone)]
pub struct LiveDomesticCnkiTransport {
    client: Client,
    captcha_token: Option<String>,
    captcha_session: SharedDomesticCaptchaSession,
    attempts: Vec<SourceAttempt>,
}

#[derive(Clone)]
struct SharedDomesticCaptchaSession {
    state: Arc<Mutex<LiveDomesticCaptchaState>>,
}

struct LiveDomesticCaptchaState {
    session: DomesticCaptchaSession,
    generation: u64,
}

impl SharedDomesticCaptchaSession {
    fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(LiveDomesticCaptchaState {
                session: DomesticCaptchaSession::new(),
                generation: 0,
            })),
        }
    }

    fn has_captcha_id(&self) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.session.has_captcha_id())
    }

    fn request_url(&self, base_url: &str) -> Result<(String, u64), DomesticCnkiSourceError> {
        let state = self.state.lock().map_err(|_| {
            DomesticCnkiSourceError::Request(
                "domestic CNKI captcha session is unavailable".to_string(),
            )
        })?;
        let request_url = state.session.attach_captcha_id(base_url)?;
        Ok((request_url, state.generation))
    }

    fn refresh<F>(
        &self,
        observed_generation: u64,
        refresh: F,
    ) -> Result<bool, DomesticCnkiSourceError>
    where
        F: FnOnce(&mut DomesticCaptchaSession) -> Result<(), DomesticCnkiSourceError>,
    {
        let mut state = self.state.lock().map_err(|_| {
            DomesticCnkiSourceError::Request(
                "domestic CNKI captcha session is unavailable".to_string(),
            )
        })?;
        if state.generation != observed_generation {
            return Ok(false);
        }
        refresh(&mut state.session)?;
        state.generation = state.generation.wrapping_add(1);
        Ok(true)
    }
}

impl fmt::Debug for LiveDomesticCnkiTransport {
    /// Format transport state without exposing captcha tokens.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LiveDomesticCnkiTransport")
            .field(
                "captcha_token",
                &self.captcha_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field("has_captcha_id", &self.captcha_session.has_captcha_id())
            .field("attempt_count", &self.attempts.len())
            .finish_non_exhaustive()
    }
}

impl LiveDomesticCnkiTransport {
    /// Build a live domestic CNKI transport.
    ///
    /// # Arguments
    ///
    /// * `config` - Live source configuration.
    ///
    /// # Returns
    ///
    /// Live domestic transport.
    pub fn new(config: LiveDomesticCnkiConfig) -> Result<Self, DomesticCnkiSourceError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds.max(1)))
            .cookie_store(true)
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= DOMESTIC_REDIRECT_LIMIT {
                    attempt.error("domestic CNKI redirect limit exceeded")
                } else if is_allowed_domestic_url(attempt.url()) {
                    attempt.follow()
                } else {
                    attempt.error("domestic CNKI redirect URL rejected")
                }
            }))
            .build()
            .map_err(|_| {
                DomesticCnkiSourceError::Request(
                    "domestic CNKI HTTP client initialization failed".to_string(),
                )
            })?;
        Ok(Self {
            client,
            captcha_token: config
                .captcha_token
                .filter(|value| !value.trim().is_empty()),
            captcha_session: SharedDomesticCaptchaSession::new(),
            attempts: Vec::new(),
        })
    }

    fn search_journals(
        &mut self,
        keyword: &str,
        field_name: &str,
    ) -> Result<Vec<Value>, DomesticCnkiSourceError> {
        let form = domestic_journal_search_form(keyword, field_name);
        let data: Vec<(String, String)> = form.into_iter().collect();
        let text = self.post_text(
            &format!("{DOMESTIC_NAVI_BASE_URL}/knavi/journals/searchbaseinfo"),
            &data,
            Some(&format!("{DOMESTIC_NAVI_BASE_URL}/knavi")),
            "journal_search",
        )?;
        parse_domestic_journal_search_results(&text)
    }

    fn get_journal_detail(
        &mut self,
        detail_url: &str,
    ) -> Result<Option<Value>, DomesticCnkiSourceError> {
        let text = self.get_text(detail_url, None, "journal_detail")?;
        if input_value(&text, "pykm").is_none() {
            return Ok(None);
        }
        parse_domestic_journal_detail(&text).map(Some)
    }

    fn get_text(
        &mut self,
        url: &str,
        referer: Option<&str>,
        endpoint: &str,
    ) -> Result<String, DomesticCnkiSourceError> {
        self.request_text("GET", url, &[], referer, endpoint)
    }

    fn post_text(
        &mut self,
        url: &str,
        data: &[(String, String)],
        referer: Option<&str>,
        endpoint: &str,
    ) -> Result<String, DomesticCnkiSourceError> {
        self.request_text("POST", url, data, referer, endpoint)
    }

    fn request_text(
        &mut self,
        method: &str,
        url: &str,
        data: &[(String, String)],
        referer: Option<&str>,
        endpoint: &str,
    ) -> Result<String, DomesticCnkiSourceError> {
        let base_url = absolute_domestic_url(url)?;
        let referer = referer
            .map(parse_domestic_url)
            .transpose()?
            .map(|url| url.to_string());
        let (mut request_url, mut captcha_generation) =
            self.captcha_session.request_url(&base_url)?;
        let mut budget = DomesticRequestBudget::default();
        while budget.next_attempt().is_some() {
            let did_retry = budget.did_retry();
            let mut builder = match method {
                "POST" => self.client.post(&request_url).form(data),
                _ => self.client.get(&request_url),
            };
            builder = builder.header(
                "User-Agent",
                "Mozilla/5.0 (compatible; LitRadar/0.1; +https://github.com/QianFuv/LitRadar)",
            );
            if let Some(referer) = referer.as_deref() {
                builder = builder.header("Referer", referer);
            }
            let response = builder.send();
            let response = match response {
                Ok(response) => response,
                Err(_) => {
                    self.record_attempt(DomesticAttempt {
                        endpoint,
                        method,
                        request_url: &request_url,
                        status_code: None,
                        did_succeed: false,
                        did_retry,
                        error: Some("request failed"),
                    });
                    if budget.can_retry_ordinary() {
                        thread::sleep(Duration::from_millis(
                            200 * budget.ordinary_attempts.max(1) as u64,
                        ));
                        continue;
                    }
                    return Err(DomesticCnkiSourceError::Request(
                        "domestic CNKI request failed".to_string(),
                    ));
                }
            };
            let status = response.status();
            let final_url = response.url().to_string();
            if validate_domestic_url(response.url()).is_err() {
                self.record_attempt(DomesticAttempt {
                    endpoint,
                    method,
                    request_url: &request_url,
                    status_code: Some(status.as_u16()),
                    did_succeed: false,
                    did_retry,
                    error: Some("redirect URL rejected"),
                });
                return Err(DomesticCnkiSourceError::Request(
                    "domestic CNKI redirect URL is not allowed".to_string(),
                ));
            }
            let text = match response.text() {
                Ok(text) => text,
                Err(_) => {
                    self.record_attempt(DomesticAttempt {
                        endpoint,
                        method,
                        request_url: &request_url,
                        status_code: Some(status.as_u16()),
                        did_succeed: false,
                        did_retry,
                        error: Some("response read failed"),
                    });
                    return Err(DomesticCnkiSourceError::Request(
                        "domestic CNKI response read failed".to_string(),
                    ));
                }
            };
            if looks_like_captcha_challenge(&text, &final_url) {
                self.record_attempt(DomesticAttempt {
                    endpoint,
                    method,
                    request_url: &request_url,
                    status_code: Some(status.as_u16()),
                    did_succeed: false,
                    did_retry,
                    error: Some("captcha challenge"),
                });
                self.solve_live_captcha(&text, &final_url, captcha_generation)?;
                (request_url, captcha_generation) = self.captcha_session.request_url(&base_url)?;
                budget.schedule_captcha_replay()?;
                continue;
            }
            if contains_overseas_host(&text) {
                self.record_attempt(DomesticAttempt {
                    endpoint,
                    method,
                    request_url: &request_url,
                    status_code: Some(status.as_u16()),
                    did_succeed: false,
                    did_retry,
                    error: Some("overseas host"),
                });
                return Err(DomesticCnkiSourceError::Request(
                    "domestic CNKI transport received overseas host".to_string(),
                ));
            }
            if !status.is_success() {
                self.record_attempt(DomesticAttempt {
                    endpoint,
                    method,
                    request_url: &request_url,
                    status_code: Some(status.as_u16()),
                    did_succeed: false,
                    did_retry,
                    error: Some("HTTP status"),
                });
                if !matches!(status.as_u16(), 404 | 410) && budget.can_retry_ordinary() {
                    continue;
                }
                return Err(domestic_http_status_error(status.as_u16()));
            }
            if let Err(error) = validate_domestic_response(endpoint, &text) {
                self.record_attempt(DomesticAttempt {
                    endpoint,
                    method,
                    request_url: &request_url,
                    status_code: Some(status.as_u16()),
                    did_succeed: false,
                    did_retry,
                    error: Some("invalid response"),
                });
                return Err(error);
            }
            self.record_attempt(DomesticAttempt {
                endpoint,
                method,
                request_url: &request_url,
                status_code: Some(status.as_u16()),
                did_succeed: true,
                did_retry,
                error: None,
            });
            return Ok(text);
        }
        Err(DomesticCnkiSourceError::Request(
            "domestic CNKI request retries exhausted".to_string(),
        ))
    }

    fn record_attempt(&mut self, attempt: DomesticAttempt<'_>) {
        self.attempts.push(SourceAttempt {
            service: "cnki".to_string(),
            endpoint: attempt.endpoint.to_string(),
            method: attempt.method.to_string(),
            url: redact_domestic_url(attempt.request_url),
            status_code: attempt.status_code,
            did_succeed: attempt.did_succeed,
            did_retry: attempt.did_retry,
            error: attempt.error.map(str::to_string),
        });
    }

    fn solve_live_captcha(
        &self,
        response_text: &str,
        response_url: &str,
        observed_generation: u64,
    ) -> Result<(), DomesticCnkiSourceError> {
        let token = self.captcha_token.clone().ok_or_else(|| {
            DomesticCnkiSourceError::Request("domestic CNKI captcha token is required".to_string())
        })?;
        let client = self.client.clone();
        self.captcha_session.refresh(observed_generation, |session| {
            let mut solver =
                crate::jfbym::LiveJfbymSolver::new(token, 30).map_err(map_jfbym_error)?;
            session.ensure_access(
                response_text,
                response_url,
                &mut solver,
                |challenge_url| {
                    let response = client
                        .get(challenge_url)
                        .header(
                            "User-Agent",
                            "Mozilla/5.0 (compatible; LitRadar/0.1; +https://github.com/QianFuv/LitRadar)",
                        )
                        .send()
                        .map_err(|_| {
                            DomesticCnkiSourceError::Request(
                                "domestic CNKI captcha page request failed".to_string(),
                            )
                        })?;
                    validate_domestic_http_response(&response)?;
                    let body = captcha_get_request_body(challenge_url)?;
                    let response = client
                        .post(format!("{DOMESTIC_KNS_BASE_URL}/verify-api/get"))
                        .header("Content-Type", "application/json;charset=UTF-8")
                        .header("Origin", DOMESTIC_KNS_BASE_URL)
                        .header("Referer", challenge_url)
                        .header("X-Requested-With", "XMLHttpRequest")
                        .json(&body)
                        .send()
                        .map_err(|_| {
                            DomesticCnkiSourceError::Request(
                                "domestic CNKI captcha puzzle request failed".to_string(),
                            )
                        })?;
                    let payload = parse_domestic_json_response(response, "captcha puzzle")?;
                    parse_captcha_puzzle(challenge_url, &payload)
                },
                |puzzle, point_json| {
                    let body = captcha_check_request_body(puzzle, point_json);
                    let response = client
                        .post(format!("{DOMESTIC_KNS_BASE_URL}/verify-api/web/check"))
                        .header("Content-Type", "application/json;charset=UTF-8")
                        .header("Origin", DOMESTIC_KNS_BASE_URL)
                        .header("Referer", &puzzle.challenge_url)
                        .header("X-Requested-With", "XMLHttpRequest")
                        .json(&body)
                        .send()
                        .map_err(|_| {
                            DomesticCnkiSourceError::Request(
                                "domestic CNKI captcha check request failed".to_string(),
                            )
                        })?;
                    let payload = parse_domestic_json_response(response, "captcha check")?;
                    Ok(captcha_check_succeeded(&payload))
                },
            )
        })?;
        Ok(())
    }
}

impl DomesticCnkiTransport for LiveDomesticCnkiTransport {
    /// Resolve one journal locator through domestic search and detail pages.
    fn resolve_journal(
        &mut self,
        locator: &DomesticJournalLocator,
    ) -> Result<Option<Value>, DomesticCnkiSourceError> {
        let mut detail_urls = Vec::new();
        let mut seen_detail_urls = BTreeSet::new();
        for title in locator.titles() {
            append_unique_domestic_detail_urls(
                &mut detail_urls,
                &mut seen_detail_urls,
                self.search_journals(title, "TI")?,
            );
        }
        for issn in locator.issns() {
            append_unique_domestic_detail_urls(
                &mut detail_urls,
                &mut seen_detail_urls,
                self.search_journals(issn, "SN")?,
            );
        }
        for detail_url in detail_urls {
            let Some(details) = self.get_journal_detail(&detail_url)? else {
                continue;
            };
            if domestic_journal_detail_matches(&details, locator) {
                return Ok(Some(details));
            }
        }
        Ok(None)
    }

    /// Fetch publication issues for one domestic journal.
    fn year_issues(&mut self, journal: &Value) -> Result<Vec<Value>, DomesticCnkiSourceError> {
        let pykm = json_text(journal.get("pykm")).ok_or_else(|| {
            DomesticCnkiSourceError::Parse("domestic CNKI journal missing pykm".to_string())
        })?;
        let data = vec![
            ("pIdx".to_string(), "0".to_string()),
            (
                "time".to_string(),
                json_text(journal.get("time")).unwrap_or_default(),
            ),
            ("isEpublish".to_string(), "0".to_string()),
            (
                "pcode".to_string(),
                json_text(journal.get("pcode")).unwrap_or_else(|| DEFAULT_PCODE.to_string()),
            ),
        ];
        let text = self.post_text(
            &format!("{DOMESTIC_NAVI_BASE_URL}/knavi/journals/{pykm}/yearList"),
            &data,
            json_text(journal.get("detail_url")).as_deref(),
            "year_issues",
        )?;
        parse_domestic_year_issues(&text)
    }

    /// Fetch article summaries for one issue.
    fn issue_articles(
        &mut self,
        journal: &Value,
        issue: &Value,
        page_index: usize,
    ) -> Result<DomesticIssueArticlePage, DomesticCnkiSourceError> {
        let pykm = json_text(journal.get("pykm")).ok_or_else(|| {
            DomesticCnkiSourceError::Parse("domestic CNKI journal missing pykm".to_string())
        })?;
        let year_issue = json_text(issue.get("year_issue"))
            .or_else(|| json_text(issue.get("year_issue_id")))
            .ok_or_else(|| {
                DomesticCnkiSourceError::Parse("domestic CNKI issue missing year_issue".to_string())
            })?;
        let data = vec![
            ("yearIssue".to_string(), year_issue),
            ("pageIdx".to_string(), page_index.to_string()),
            (
                "pcode".to_string(),
                json_text(journal.get("pcode")).unwrap_or_else(|| DEFAULT_PCODE.to_string()),
            ),
            ("isEpublish".to_string(), "0".to_string()),
            ("language".to_string(), DOMESTIC_LANGUAGE.to_string()),
            ("uniplatform".to_string(), DOMESTIC_PLATFORM.to_string()),
        ];
        let text = self.post_text(
            &format!("{DOMESTIC_NAVI_BASE_URL}/knavi/journals/{pykm}/papers"),
            &data,
            json_text(journal.get("detail_url")).as_deref(),
            "issue_articles",
        )?;
        parse_domestic_issue_articles(&text, issue, page_index)
    }

    /// Fetch one article detail payload.
    fn article_detail(
        &mut self,
        article_url: &str,
        platform_id: Option<&str>,
    ) -> Result<Value, DomesticCnkiSourceError> {
        let _ = platform_id;
        let text = self.get_text(article_url, None, "article_detail")?;
        parse_domestic_article_detail(&text, article_url)
    }

    /// Return captured source attempts.
    fn attempts(&self) -> &[SourceAttempt] {
        &self.attempts
    }

    /// Remove and return captured source attempts.
    fn drain_attempts(&mut self) -> Vec<SourceAttempt> {
        std::mem::take(&mut self.attempts)
    }
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
        <input id="articleCount" type="hidden" value="1">
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

        let page = parse_domestic_issue_articles(PAPERS_HTML, &issues[0], 0).expect("papers");
        assert_eq!(page.page_index, 0);
        assert_eq!(page.article_count, 1);
        assert_eq!(page.articles.len(), 1);
        assert!(!page.has_next_page);
        assert_eq!(page.articles[0]["platform_id"], "SJJJ202512002");
        assert_eq!(page.articles[0]["authors"], "侯俊军;丁琪琪;");
        assert_eq!(page.articles[0]["pages"], "3-31");
        assert!(page.articles[0]["article_url"]
            .as_str()
            .unwrap_or_default()
            .starts_with("https://kns.cnki.net/kcms2/article/abstract?"));
        assert!(!contains_overseas_host(
            page.articles[0]["article_url"].as_str().unwrap_or_default()
        ));

        let article_url = page.articles[0]["article_url"].as_str().unwrap_or_default();
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
    fn domestic_cnki_rejects_partial_papers_page() {
        let issues = parse_domestic_year_issues(YEAR_HTML).expect("years");
        let partial = PAPERS_HTML.replace("value=\"1\"", "value=\"2\"");

        assert!(parse_domestic_issue_articles(&partial, &issues[0], 0).is_err());
    }

    #[test]
    fn domestic_cnki_papers_pages_validate_counts_and_terminal_page() {
        let issue = json!({
            "year": 2025,
            "number": "12",
            "year_issue_id": "202512",
            "year_issue": "opaque"
        });
        let first_html = papers_fixture_page("FIRST", 10);
        let final_html = papers_fixture_page("FINAL", 2);
        let empty_html = papers_fixture_page("EMPTY", 0);

        let first = parse_domestic_issue_articles(&first_html, &issue, 0).expect("first page");
        assert_eq!(first.articles.len(), 10);
        assert_eq!(first.page_index, 0);
        assert_eq!(first.article_count, 10);
        assert!(first.has_next_page);
        let final_page = parse_domestic_issue_articles(&final_html, &issue, 1).expect("final page");
        assert_eq!(final_page.articles.len(), 2);
        assert_eq!(final_page.page_index, 1);
        assert_eq!(final_page.article_count, 2);
        assert!(!final_page.has_next_page);
        let empty = parse_domestic_issue_articles(&empty_html, &issue, 2).expect("empty terminal");
        assert!(empty.articles.is_empty());
        assert_eq!(empty.page_index, 2);
        assert_eq!(empty.article_count, 0);
        assert!(!empty.has_next_page);
        let expanded =
            parse_domestic_issue_articles(&papers_fixture_page("EXPANDED", 11), &issue, 0)
                .expect("structurally complete expanded page");
        assert_eq!(expanded.articles.len(), 11);
        assert!(!expanded.has_next_page);

        let mut transport = FixtureDomesticCnkiTransport::new(DomesticCnkiFixtureData {
            issue_article_pages: BTreeMap::from([(
                "202512".to_string(),
                vec![first_html, empty_html],
            )]),
            ..DomesticCnkiFixtureData::default()
        });
        assert!(
            transport
                .issue_articles(&json!({}), &issue, 0)
                .expect("fixture page zero")
                .has_next_page
        );
        assert!(
            !transport
                .issue_articles(&json!({}), &issue, 1)
                .expect("fixture empty terminal")
                .has_next_page
        );
        assert!(transport.issue_articles(&json!({}), &issue, 2).is_err());
    }

    #[test]
    fn domestic_cnki_journal_identity_matrix_rejects_conflicts_and_uses_fallbacks() {
        let locator = DomesticJournalLocator::new(
            vec![
                "世界经济".to_string(),
                "世界经济别名".to_string(),
                " 世界经济别名 ".to_string(),
            ],
            vec![
                "1002-9621".to_string(),
                "10029621".to_string(),
                "1234-5679".to_string(),
            ],
        );
        assert_eq!(locator.titles(), ["世界经济", "世界经济别名"]);
        assert_eq!(locator.issns(), ["1002-9621", "1234-5679"]);

        let conflicting = json!({
            "title": "世界经济",
            "issn": "2049-3630"
        });
        assert!(!domestic_journal_detail_matches(&conflicting, &locator));

        let alias_with_intersection = json!({
            "title": "世界经济别名",
            "issn": "1002-9621"
        });
        assert!(domestic_journal_detail_matches(
            &alias_with_intersection,
            &locator
        ));

        let provider_title_with_intersection = json!({
            "title": "Provider 自有题名",
            "issns": ["1002-9621", "2049-3630"]
        });
        assert!(domestic_journal_detail_matches(
            &provider_title_with_intersection,
            &locator
        ));

        let missing_observed_issn = json!({"title": "世界经济别名"});
        assert!(domestic_journal_detail_matches(
            &missing_observed_issn,
            &locator
        ));
        let title_only_locator =
            DomesticJournalLocator::new(vec!["世界经济".to_string()], vec!["invalid".to_string()]);
        let observed_with_only_issn = json!({
            "title": "世界经济",
            "issn": "2049-3630"
        });
        assert!(domestic_journal_detail_matches(
            &observed_with_only_issn,
            &title_only_locator
        ));
    }

    #[test]
    fn domestic_cnki_journal_detail_urls_are_deduplicated_in_search_order() {
        let mut detail_urls = Vec::new();
        let mut seen_detail_urls = BTreeSet::new();
        append_unique_domestic_detail_urls(
            &mut detail_urls,
            &mut seen_detail_urls,
            vec![
                json!({"detail_url": "https://navi.cnki.net/knavi/detail?p=1"}),
                json!({"detail_url": "https://navi.cnki.net/knavi/detail?p=2"}),
            ],
        );
        append_unique_domestic_detail_urls(
            &mut detail_urls,
            &mut seen_detail_urls,
            vec![
                json!({"detail_url": "https://navi.cnki.net/knavi/detail?p=1"}),
                json!({"detail_url": "https://navi.cnki.net/knavi/detail?p=3"}),
            ],
        );

        assert_eq!(
            detail_urls,
            [
                "https://navi.cnki.net/knavi/detail?p=1",
                "https://navi.cnki.net/knavi/detail?p=2",
                "https://navi.cnki.net/knavi/detail?p=3",
            ]
        );
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
        assert!(matches!(overseas, Err(DomesticCnkiSourceError::Request(_))));

        let overseas_detail = parse_domestic_article_detail(
            ABSTRACT_HTML,
            "https://oversea.cnki.net/kcms2/article/abstract?v=1",
        );
        assert!(matches!(
            overseas_detail,
            Err(DomesticCnkiSourceError::Request(_))
        ));
    }

    #[test]
    fn rejects_disallowed_absolute_urls_and_incomplete_success_pages() {
        for url in [
            "http://navi.cnki.net/knavi/detail?p=1",
            "https://navi.cnki.net.evil.example/knavi/detail?p=1",
            "https://127.0.0.1/knavi/detail?p=1",
            "https://user@navi.cnki.net/knavi/detail?p=1",
            "https://navi.cnki.net:444/knavi/detail?p=1",
        ] {
            let html = format!(r#"<a href="{url}" title="x">x</a><p>ISSN：1002-9621</p>"#);
            assert!(parse_domestic_journal_search_results(&html).is_err());
        }

        assert!(parse_domestic_journal_search_results(" ").is_err());
        assert!(parse_domestic_journal_detail("<html></html>").is_err());
        assert!(parse_domestic_year_issues("<html></html>").is_err());
        assert!(parse_domestic_issue_articles("<html></html>", &json!({}), 0).is_err());
        assert!(parse_domestic_article_detail(
            "<html></html>",
            "https://kns.cnki.net/kcms2/article/abstract?v=1"
        )
        .is_err());
    }

    #[test]
    fn allows_only_exact_domestic_https_origins_and_safe_relative_paths() {
        for valid in [
            "https://navi.cnki.net/knavi/detail?p=1",
            "https://kns.cnki.net:443/kcms2/article/abstract?v=1",
            "/knavi/detail?p=1",
            "/kcms2/article/abstract?v=1",
            "/verify/home?captchaType=blockPuzzle",
        ] {
            assert!(parse_domestic_url(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "http://navi.cnki.net/knavi/detail?p=1",
            "https://oversea.cnki.net/knavi/detail?p=1",
            "https://navi.cnki.net.evil.example/knavi/detail?p=1",
            "https://127.0.0.1/knavi/detail?p=1",
            "https://192.168.1.2/knavi/detail?p=1",
            "https://[::1]/knavi/detail?p=1",
            "https://user:password@navi.cnki.net/knavi/detail?p=1",
            "https://kns.cnki.net:444/kcms2/article/abstract?v=1",
            "//evil.example/path",
            "knavi/detail?p=1",
        ] {
            assert!(parse_domestic_url(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn accepts_only_explicit_structured_empty_responses() {
        assert!(parse_domestic_journal_search_results("<div>暂无数据</div>")
            .expect("explicit empty search")
            .is_empty());
        assert!(
            parse_domestic_year_issues("<div id=\"YearIssueTree\"></div>")
                .expect("empty issue tree")
                .is_empty()
        );
        assert!(parse_domestic_issue_articles(
            "<input id=\"articleCount\" value=\"0\">",
            &json!({}),
            0
        )
        .expect("explicit empty papers page")
        .articles
        .is_empty());
    }

    #[test]
    fn update_placeholder_is_terminal_only_after_the_first_issue_page() {
        let body = "<div>该刊数据正在<span>更新中</span>，请耐心等待</div>";

        assert!(validate_domestic_response("issue_articles", body).is_ok());
        let terminal = parse_domestic_issue_articles(body, &json!({}), 1)
            .expect("out-of-range update placeholder should terminate paging");
        assert!(terminal.articles.is_empty());
        assert!(!terminal.has_next_page);
        assert!(parse_domestic_issue_articles(body, &json!({}), 0).is_err());
    }

    #[test]
    fn explicit_empty_words_do_not_override_structured_issue_rows() {
        let body = papers_fixture_page("没有找到", 1);

        let page = parse_domestic_issue_articles(&body, &json!({}), 0)
            .expect("structured article row should take precedence");
        assert_eq!(page.articles.len(), 1);
    }

    #[test]
    fn redacts_domestic_attempt_urls_and_preserves_http_status() {
        let sensitive_url = "https://kns.cnki.net/verify/home?captchaId=captcha-secret-sentinel&ident=ident-secret-sentinel&returnUrl=return-secret-sentinel&language=CHS";
        let redacted = redact_domestic_url(sensitive_url);
        assert!(!redacted.contains("captcha-secret-sentinel"));
        assert!(!redacted.contains("ident-secret-sentinel"));
        assert!(!redacted.contains("return-secret-sentinel"));
        assert!(redacted.contains("language=CHS"));

        let mut transport = LiveDomesticCnkiTransport::new(LiveDomesticCnkiConfig {
            timeout_seconds: 1,
            captcha_token: Some("captcha-token-sentinel".to_string()),
        })
        .expect("transport");
        transport.record_attempt(DomesticAttempt {
            endpoint: "journal_search",
            method: "GET",
            request_url: sensitive_url,
            status_code: Some(403),
            did_succeed: false,
            did_retry: true,
            error: Some("captcha challenge"),
        });
        let diagnostic = format!(
            "{:?} {}",
            transport.attempts(),
            serde_json::to_string(transport.attempts()).expect("attempts should serialize")
        );
        assert!(!diagnostic.contains("captcha-secret-sentinel"));
        assert!(!diagnostic.contains("ident-secret-sentinel"));
        assert!(!diagnostic.contains("return-secret-sentinel"));
        assert!(!format!("{transport:?}").contains("captcha-token-sentinel"));

        let status_error = domestic_http_status_error(410);
        assert_eq!(status_error.http_status(), Some(410));
        assert_eq!(
            DomesticCnkiSourceError::Request("request failed".to_string()).http_status(),
            None
        );
    }

    #[test]
    fn final_ordinary_attempt_can_schedule_one_authenticated_replay() {
        let mut budget = DomesticRequestBudget::default();
        assert_eq!(budget.next_attempt(), Some(false));
        assert_eq!(budget.next_attempt(), Some(false));
        assert_eq!(budget.next_attempt(), Some(false));
        budget
            .schedule_captcha_replay()
            .expect("captcha replay should fit budget");
        assert_eq!(budget.next_attempt(), Some(true));
        assert_eq!(budget.next_attempt(), None);
    }

    #[test]
    fn parses_html_encoded_challenge_queries_exactly() {
        let body = r#"<a href="https://kns.cnki.net/verify/home?captchaType=blockPuzzle&amp;ident=ident-sentinel&amp;captchaId=captcha-sentinel&amp;returnUrl=return-sentinel">verify</a>"#;

        let challenge = extract_challenge_url(body, "search")
            .expect("challenge should parse")
            .expect("challenge should exist");
        let get_body = captcha_get_request_body(&challenge).expect("request body");

        assert_eq!(get_body["captchaType"], "blockPuzzle");
        assert_eq!(get_body["ident"], "ident-sentinel");
        assert_eq!(get_body["captchaId"], "captcha-sentinel");
        assert!(!challenge.contains("&amp;"));
    }

    #[test]
    fn parses_search_issn_window_at_utf8_boundaries() {
        let prefix = "中".repeat(31);
        let html = format!(
            r#"<div>{prefix}<a href="https://navi.cnki.net/knavi/detail?p=1" title="世界经济">世界经济</a><p>ISSN：1002-9621</p></div>"#
        );

        let candidates = parse_domestic_journal_search_results(&html).expect("search");

        assert_eq!(candidates[0]["issn"], "1002-9621");
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
        let challenge = extract_challenge_url(body, "search")
            .expect("challenge should parse")
            .expect("challenge should exist");
        assert!(challenge.contains("/verify/home?"));
        assert!(challenge.contains("captchaId="));
        assert!(!challenge.contains("oversea.cnki.net"));
    }

    #[test]
    fn authenticated_empty_search_result_is_not_a_captcha_challenge() {
        let body = r#"<div class="result-count">共 0 条结果</div><p>找到 0 条结果</p>"#;
        let url = "https://navi.cnki.net/knavi/journals/searchbaseinfo?captchaId=opaque";

        assert!(!looks_like_captcha_challenge(body, url));
        assert!(parse_domestic_journal_search_results(body)
            .expect("authenticated empty result should parse")
            .is_empty());
    }

    #[test]
    fn cloned_live_transport_sessions_share_one_captcha_refresh() {
        let parent = SharedDomesticCaptchaSession::new();
        let base_url = "https://kns.cnki.net/kcms2/article/abstract";
        let (_, generation) = parent.request_url(base_url).expect("initial captcha URL");
        let refresh_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let barrier = Arc::new(std::sync::Barrier::new(6));
        let refresh_results = thread::scope(|scope| {
            let handles = (0..6)
                .map(|_| {
                    let worker = parent.clone();
                    let refresh_count = Arc::clone(&refresh_count);
                    let barrier = Arc::clone(&barrier);
                    scope.spawn(move || {
                        barrier.wait();
                        worker
                            .refresh(generation, |session| {
                                refresh_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                                thread::sleep(Duration::from_millis(25));
                                session.captcha_id = Some("captcha-id-sentinel".to_string());
                                Ok(())
                            })
                            .expect("shared captcha refresh")
                    })
                })
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|handle| handle.join().expect("captcha worker"))
                .collect::<Vec<_>>()
        });
        let (attached, refreshed_generation) =
            parent.request_url(base_url).expect("parent captcha URL");

        assert_eq!(refresh_results.iter().filter(|result| **result).count(), 1);
        assert_eq!(refresh_count.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(refreshed_generation, generation.wrapping_add(1));
        assert!(attached.contains("captchaId="));
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
        let debug = format!("{puzzle:?}");
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
        let session_debug = format!("{session:?}");
        assert!(!session_debug.contains("2222b8cc"));
        let attached = session
            .attach_captcha_id("https://navi.cnki.net/knavi/journals/index")
            .expect("captcha id should attach");
        assert!(attached.contains("captchaId="));
        assert!(accepted_x.is_some());

        let get_body = captcha_get_request_body(challenge).expect("request body");
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

    #[test]
    fn captcha_session_rejects_invalid_solver_distance_without_panicking() {
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
        let mut solver = FixtureJfbymSolver::new(-1.0);

        let error = session
            .solve_challenge(
                challenge,
                &mut solver,
                |_| Ok(puzzle.clone()),
                |_puzzle, _point| Ok(true),
            )
            .expect_err("invalid distance should fail");

        assert!(error.to_string().contains("slider distance"));
        assert!(!session.has_captcha_id());
    }

    #[test]
    fn fixture_domestic_client_resolves_journal_and_articles() {
        let data = DomesticCnkiFixtureData {
            journal_search_html: SEARCH_HTML.to_string(),
            journal_detail_html: DETAIL_HTML.to_string(),
            year_issues_html: YEAR_HTML.to_string(),
            issue_article_pages: BTreeMap::from([(
                "202512".to_string(),
                vec![PAPERS_HTML.to_string()],
            )]),
            article_detail_html: BTreeMap::from([(
                "SJJJ202512002".to_string(),
                ABSTRACT_HTML.to_string(),
            )]),
            article_detail_status_codes: BTreeMap::new(),
            fail_endpoint: None,
        };
        let mut client = DomesticCnkiClient::new(FixtureDomesticCnkiTransport::new(data));
        let locator = DomesticJournalLocator::new(
            vec!["世界经济".to_string()],
            vec!["1002-9621".to_string()],
        );
        let journal = client
            .resolve_journal(&locator)
            .expect("journal")
            .expect("found");
        assert_eq!(journal["pykm"], "SJJJ");
        let issues = client.year_issues(&journal).expect("issues");
        assert_eq!(issues[0]["year_issue_id"], "202512");
        let articles = client
            .issue_articles(&journal, &issues[0], 0)
            .expect("articles");
        assert_eq!(articles.articles[0]["platform_id"], "SJJJ202512002");
        let detail = client
            .article_detail(
                articles.articles[0]["article_url"].as_str().unwrap(),
                Some("SJJJ202512002"),
            )
            .expect("detail");
        assert!(detail["permalink"]
            .as_str()
            .unwrap_or_default()
            .starts_with("https://kns.cnki.net/"));
        assert!(!detail["permalink"]
            .as_str()
            .unwrap_or_default()
            .contains("oversea.cnki.net"));
    }

    fn papers_fixture_page(prefix: &str, article_count: usize) -> String {
        let rows = (0..article_count)
            .map(|index| {
                let platform_id = format!("{prefix}{index:02}");
                format!(
                    r#"<dd class="row clearfix"><a href="https://kns.cnki.net/kcms2/article/abstract?v={platform_id}" title="Article {index}">Article {index}</a><b name="encrypt" id="{platform_id}"></b></dd>"#
                )
            })
            .collect::<String>();
        format!(
            "<dt class=\"tit\">Articles</dt>{rows}<input id=\"articleCount\" value=\"{article_count}\">"
        )
    }
}
