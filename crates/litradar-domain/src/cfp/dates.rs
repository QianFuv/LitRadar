//! Strict calendar parsing for literal Chinese and English submission timelines.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, LazyLock, RwLock};

use chrono::NaiveDate;
use regex::{Captures, Regex};
use unicode_normalization::UnicodeNormalization;

use super::{clean_cfp_text, CfpDate, CfpDateStage};

const MONTH_NAMES: &str = "Jan(?:uary)?|Feb(?:ruary)?|Mar(?:ch)?|Apr(?:il)?|May|Jun(?:e)?|Jul(?:y)?|Aug(?:ust)?|Sep(?:t(?:ember)?)?|Oct(?:ober)?|Nov(?:ember)?|Dec(?:ember)?";
static REGEXES: LazyLock<RwLock<HashMap<String, Arc<Regex>>>> = LazyLock::new(RwLock::default);
static DATE_PATTERN: LazyLock<Arc<Regex>> = LazyLock::new(|| {
    regex(&format!(
        r"(?i)(\d{{4}})\s*[-/.年]\s*(\d{{1,2}})\s*[-/.月]\s*(\d{{1,2}})(?:\s*日)?|(\d{{1,2}})(?:st|nd|rd|th)?\s*,?\s+(?:of\s+)?({MONTH_NAMES})\.?\s*,?\s*(\d{{4}})|\b({MONTH_NAMES})\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?(?:\s*,\s*|\s+)(\d{{4}})"
    ))
});

pub(super) fn regex(pattern: &str) -> Arc<Regex> {
    if let Some(compiled) = REGEXES.read().expect("regex cache lock").get(pattern) {
        return compiled.clone();
    }
    let compiled = Arc::new(Regex::new(pattern).expect("CFP parser regex must compile"));
    REGEXES
        .write()
        .expect("regex cache lock")
        .insert(pattern.into(), compiled.clone());
    compiled
}

pub(super) fn matches(pattern: &str, value: &str) -> bool {
    regex(&format!("(?i){pattern}")).is_match(value)
}

fn calendar_date(year: i32, month: u32, day: u32) -> Option<String> {
    NaiveDate::from_ymd_opt(year, month, day).map(|date| date.to_string())
}

fn month_number(value: &str) -> u32 {
    let prefix = &value.to_ascii_lowercase()[..3];
    [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|month| *month == prefix)
    .expect("matched month") as u32
        + 1
}

fn has_numeric_boundaries(text: &str, start: usize, end: usize) -> bool {
    !text[..start]
        .chars()
        .next_back()
        .is_some_and(|value| value.is_ascii_digit())
        && !text[end..]
            .chars()
            .next()
            .is_some_and(|value| value.is_ascii_digit())
}

fn matching_text(value: &str) -> String {
    let normalized: String = value.nfkc().collect();
    let normalized = regex(r"(?i)\^\{(st|nd|rd|th)\}").replace_all(&normalized, "$1");
    let pattern = regex(r"(\d{1,2})[/.](\d{1,2})[/.](\d{4})");
    pattern
        .replace_all(&normalized, |capture: &Captures<'_>| {
            let full = capture.get(0).expect("full match");
            let first: u32 = capture[1].parse().expect("numeric date");
            let second: u32 = capture[2].parse().expect("numeric date");
            if !has_numeric_boundaries(&normalized, full.start(), full.end())
                || (first <= 12 && second <= 12)
            {
                full.as_str().to_owned()
            } else {
                let (day, month) = if first > 12 {
                    (first, second)
                } else {
                    (second, first)
                };
                format!("{}-{month}-{day}", &capture[3])
            }
        })
        .into_owned()
}

fn date_stage(context: &str, fallback: CfpDateStage) -> Option<CfpDateStage> {
    use CfpDateStage::*;
    let normalized = matching_text(context);
    let prefix = DATE_PATTERN
        .find(&normalized)
        .map(|found| &normalized[..found.start()]);
    let context = prefix.filter(|prefix| matches(
        r"deadline|due|submit|submission|abstract|proposal|paper|manuscript|open|start|revis|decision|notification|registration|publication|publish|截[稿止]|提交|投稿|摘要|开始|开放", prefix,
    )).unwrap_or(context);
    let rules = [
        (r"registration|payment|报名|缴费", Registration),
        (
            r"revis(?:ed|ion)|resubmission|final manuscript (?:due|submission)|final paper due|accepted papers?|camera.ready|修改稿|修订稿|返修|终稿",
            Revision,
        ),
        (
            r"decision|notification|review completed|editorial review|review sent|will let.+know|录用|通知|审稿|评审结果",
            Decision,
        ),
        (r"publication|publish|出版|刊出", Publication),
        (r"conference date|colloquium date|会议日期|举办时间", Event),
        (
            r"open(?:s|ing)?\s*(?:for (?:new )?submissions?|date|:)|submissions?\s+(?:open|start|begin|from)|(?:begin|start)(?:s|ing)?\s*(?:on|date|:)|开始|开放|起始",
            Opens,
        ),
        (r"abstract|摘要", Abstract),
        (r"proposal|提案|建议书", Proposal),
        (
            r"full\s+(?:paper|manuscript|draft)|completed manuscripts?|(?:paper|manuscript) submission|完整论文|全文|论文投稿",
            Paper,
        ),
        (
            r"submission|submit|deadline|due|close|截[稿止]|投稿|来稿|征稿时间|征文时间|收稿时间|发送.*论文|论文.*发送",
            fallback,
        ),
    ];
    rules
        .into_iter()
        .find_map(|(pattern, stage)| matches(pattern, context).then_some(stage))
}

fn clauses(value: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut start = 0;
    for (position, character) in value.char_indices() {
        if matches!(character, '\n' | ';' | '；') {
            if start < position {
                result.push(value[start..position].to_owned());
            }
            start = position + character.len_utf8();
        } else if matches!(character, '。' | '！' | '？') {
            let end = position + character.len_utf8();
            result.push(value[start..end].to_owned());
            start = end;
        } else if matches!(character, '.' | '!' | '?') {
            let end = position + 1;
            let remaining = &value[end..];
            if remaining.starts_with(char::is_whitespace)
                && remaining
                    .trim_start()
                    .starts_with(|character: char| character.is_ascii_uppercase())
            {
                result.push(value[start..end].to_owned());
                start = end + remaining.len() - remaining.trim_start().len();
            }
        }
    }
    if start < value.len() {
        result.push(value[start..].to_owned());
    }
    result
}

fn window_dates(
    text: &str,
    context: &str,
    stage: CfpDateStage,
) -> Option<Result<(String, String), ()>> {
    if stage != CfpDateStage::Opens && !stage.is_submission() {
        return None;
    }
    let month_first = regex(&format!(
        r"(?i)({MONTH_NAMES})\.?\s+(\d{{1,2}})(?:st|nd|rd|th)?\s*(?:[–—-]|\band\b)\s*(?:({MONTH_NAMES})\.?\s+)?(\d{{1,2}})(?:st|nd|rd|th)?(?:\s*,\s*|\s+)(\d{{4}})"
    ));
    let window = matches(r"window|period|窗口|期间", context);
    let month_range = regex(&format!(
        r"(?i)(\d{{1,2}})\s+({MONTH_NAMES})\s*[–—-]\s*(\d{{1,2}})\s+({MONTH_NAMES})\s+(\d{{4}})"
    ));
    let day_range = regex(&format!(
        r"(?i)(\d{{1,2}})\s*[–—-]\s*(\d{{1,2}})\s+({MONTH_NAMES})\s+(\d{{4}})"
    ));
    let components = if let Some(capture) = month_first.captures(text) {
        let full = capture.get(0).expect("full range");
        if !has_numeric_boundaries(text, full.start(), full.end()) {
            return None;
        }
        if matches(r"\band\b", full.as_str())
            && !matches(r"between|window|period|期间|窗口", context)
        {
            return Some(Err(()));
        }
        Some((
            capture[5].parse::<i32>().ok()?,
            month_number(&capture[1]),
            capture[2].parse::<u32>().ok()?,
            month_number(capture.get(3).map_or(&capture[1], |value| value.as_str())),
            capture[4].parse::<u32>().ok()?,
        ))
    } else if let Some(capture) = month_range.captures(text).filter(|_| window) {
        let full = capture.get(0).expect("full range");
        if !has_numeric_boundaries(text, full.start(), full.end()) {
            return None;
        }
        Some((
            capture[5].parse().ok()?,
            month_number(&capture[2]),
            capture[1].parse().ok()?,
            month_number(&capture[4]),
            capture[3].parse().ok()?,
        ))
    } else if let Some(capture) = day_range.captures(text).filter(|_| window) {
        let full = capture.get(0).expect("full range");
        if !has_numeric_boundaries(text, full.start(), full.end()) {
            return None;
        }
        Some((
            capture[4].parse().ok()?,
            month_number(&capture[3]),
            capture[1].parse().ok()?,
            month_number(&capture[3]),
            capture[2].parse().ok()?,
        ))
    } else {
        None
    };
    components.map(|(year, start_month, start_day, end_month, end_day)| {
        let start = calendar_date(
            year - i32::from(start_month > end_month),
            start_month,
            start_day,
        )
        .ok_or(())?;
        let end = calendar_date(year, end_month, end_day).ok_or(())?;
        if start > end {
            Err(())
        } else {
            Ok((start, end))
        }
    })
}

/// Parse labeled timelines, rejecting conflicting or underspecified admission gates.
pub fn parse_cfp_dates(text: &str, entry_stage: CfpDateStage) -> Option<Vec<CfpDate>> {
    use CfpDateStage::*;
    let mut dates = Vec::new();
    let mut unresolved = BTreeSet::new();
    let clauses = clauses(&clean_cfp_text(text));
    for (index, clause) in clauses.iter().enumerate() {
        let text = matching_text(clause);
        let context = if date_stage(clause, entry_stage).is_some() {
            clause.clone()
        } else {
            format!(
                "{} {clause}",
                index
                    .checked_sub(1)
                    .and_then(|previous| clauses.get(previous))
                    .map_or("", String::as_str)
            )
        };
        let Some(stage) = date_stage(&context, entry_stage) else {
            continue;
        };
        let submission_stage = if stage.is_submission() {
            stage
        } else {
            entry_stage
        };
        let is_optional = matches(
            r"optional|if desired|not required|not a prerequisite|可选|自愿|非必需",
            &context,
        );
        if let Some(window) = window_dates(&text, &context, stage) {
            let (start, end) = window.ok()?;
            for (date, stage) in [(start, Opens), (end, submission_stage)] {
                dates.push(CfpDate {
                    date,
                    stage,
                    original_text: clause.clone(),
                    is_exclusive: false,
                    is_optional,
                });
            }
            continue;
        }
        let captures: Vec<_> = DATE_PATTERN
            .captures_iter(&text)
            .filter(|capture| {
                let found = capture.get(0).expect("full date");
                has_numeric_boundaries(&text, found.start(), found.end())
            })
            .collect();
        if captures.is_empty()
            && (matches(
                r"following.{0,30}(?:timetable|timeline|schedule)|submission (?:stage|information)",
                &context,
            ) || !matches(
                r"deadline|due\b|no later than|\bbefore\b|\bby\b|close|submissions? (?:open|window|period)|截[稿止]|投稿时间|征稿时间|TBD|to be (?:announced|determined)",
                &context,
            ))
        {
            continue;
        }
        if captures.is_empty() && !is_optional {
            unresolved.insert(stage);
        }
        if captures.is_empty()
            && (stage == Opens || stage.is_submission())
            && matches(r"\d", clause)
        {
            return None;
        }
        for (match_index, capture) in captures.iter().enumerate() {
            let year = capture
                .get(1)
                .or_else(|| capture.get(6))
                .or_else(|| capture.get(9))?
                .as_str()
                .parse()
                .ok()?;
            let month = if let Some(month) = capture.get(2) {
                month.as_str().parse().ok()?
            } else {
                month_number(capture.get(5).or_else(|| capture.get(7))?.as_str())
            };
            let day = capture
                .get(3)
                .or_else(|| capture.get(4))
                .or_else(|| capture.get(8))?
                .as_str()
                .parse()
                .ok()?;
            let Some(date) = calendar_date(year, month, day) else {
                if stage == Opens || stage.is_submission() {
                    return None;
                }
                continue;
            };
            let resolved_stage = if captures.len() == 2
                && matches(r"window|period|between|窗口|期间", &context)
            {
                if match_index == 0 {
                    Opens
                } else {
                    submission_stage
                }
            } else {
                stage
            };
            dates.push(CfpDate {
                date,
                stage: resolved_stage,
                original_text: clause.clone(),
                is_optional,
                is_exclusive: matches(r"日前|日之前", clause)
                    || matches(
                        r"\bbefore(?:\s+the)?(?:\s+\w+day,?)?\s*$",
                        &text[..capture.get(0)?.start()],
                    ),
            });
        }
    }
    let mut unique: Vec<CfpDate> = Vec::new();
    for date in dates {
        if let Some(existing) = unique
            .iter_mut()
            .find(|existing| existing.stage == date.stage && existing.date == date.date)
        {
            *existing = date;
        } else {
            unique.push(date);
        }
    }
    for stage in [Opens, Abstract, Proposal] {
        if unresolved.contains(&stage)
            && !unique.iter().any(|date| date.stage == stage)
            && unique.iter().any(|date| date.stage.is_submission())
        {
            return None;
        }
    }
    for stage in [Opens, Paper, Abstract, Proposal] {
        let same_stage: Vec<_> = unique
            .iter()
            .filter(|date| date.stage == stage && !date.is_optional)
            .collect();
        if same_stage.len() > 1 {
            let extensions: Vec<_> = same_stage
                .iter()
                .filter(|date| matches(r"extended|延期|延长", &date.original_text))
                .collect();
            if extensions.len() != 1 {
                return None;
            }
            let extension = (*extensions[0]).clone();
            unique.retain(|date| date.stage != stage || *date == extension);
        }
    }
    if let (Some(start), Some(end)) = (
        unique.iter().find(|date| date.stage == Opens),
        unique.iter().find(|date| date.stage == entry_stage),
    ) {
        if start.date > end.date {
            return None;
        }
    }
    Some(unique)
}
