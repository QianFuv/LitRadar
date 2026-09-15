//! CFP source-language and initial-submission regression fixtures.

use chrono::{DateTime, Utc};
use litradar_domain::cfp::{
    parse_cfp_dates, parse_cfp_source, CfpDateStage, CfpSeed, CfpSource, CfpState,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

fn source(date_text: &str) -> CfpSource {
    serde_json::from_value(serde_json::json!({
        "catalogIds":["issn-1063-6560","issn-1530-9304"],
        "journalTitle":"Evolutionary Computation", "title":"Example special issue",
        "typeText":"Special Issue", "dateText":date_text,
        "sourceUrl":"https://example.org/cfp", "checkedOn":"2026-09-15"
    }))
    .unwrap()
}

fn instant(value: &str) -> DateTime<Utc> {
    value.parse().unwrap()
}

#[test]
fn cfp_every_shipped_notice_matches_legacy_originals_and_fixed_instant_states() {
    #[derive(Deserialize)]
    struct Expected {
        id: String,
        digest: String,
        states: Vec<CfpState>,
    }
    #[derive(Deserialize)]
    struct Fixture {
        times: Vec<String>,
        expected: Vec<Expected>,
    }
    let seed: CfpSeed =
        serde_json::from_str(include_str!("../../litradar-sources/assets/cfp-seed.json")).unwrap();
    let fixture: Fixture = serde_json::from_str(include_str!("fixtures/cfp-legacy.json")).unwrap();
    assert_eq!(seed.sources.len(), 1622);
    assert_eq!(fixture.expected.len(), seed.sources.len());
    for (source, expected) in seed.sources.iter().zip(fixture.expected) {
        let notice = parse_cfp_source(source).unwrap_or_else(|| panic!("Rejected {source:?}"));
        assert_eq!(notice.id, expected.id);
        let canonical = serde_json::to_vec(&serde_json::to_value(&notice).unwrap()).unwrap();
        assert_eq!(
            Sha256::digest(&canonical)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            expected.digest,
            "Normalized parity: {} / {}\n{}",
            source.journal_title,
            source.title,
            String::from_utf8(canonical).unwrap()
        );
        for (now, expected_state) in fixture.times.iter().zip(expected.states) {
            assert_eq!(
                notice.state(instant(now)),
                expected_state,
                "{} at {now}",
                notice.id
            );
        }
    }
}

#[test]
fn cfp_preserves_original_text_and_rejects_invalid_sources() {
    let mut source = source("Submission deadline: 31 December 2026");
    source.title = "  Original English title  ".into();
    source.scope = "First topic\r\nSecond topic: 人工智能 & AI".into();
    let notice = parse_cfp_source(&source).unwrap();
    assert_eq!(notice.title, "Original English title");
    assert_eq!(notice.scope, "First topic\nSecond topic: 人工智能 & AI");
    assert!(notice.requirements.is_empty());
    for invalid in [
        "javascript:alert(1)",
        "https://",
        "https://user:password@example.org/cfp",
    ] {
        let mut invalid_source = source.clone();
        invalid_source.source_url = invalid.into();
        assert!(parse_cfp_source(&invalid_source).is_none());
    }
    source.time_zone = Some("invalid-zone".into());
    assert!(parse_cfp_source(&source).is_none());
    source.time_zone = None;
    source.checked_on = "2026-02-30".into();
    assert!(parse_cfp_source(&source).is_none());
}

#[test]
fn cfp_parses_original_calendar_formats_and_shared_year_windows() {
    for (text, expected) in [
        ("论文投稿截止日期 2026 年 12 月 31 日", "2026-12-31"),
        ("Submission deadline\n31 December 2026", "2026-12-31"),
        (
            "December 31, 2026: Manuscript submission deadline",
            "2026-12-31",
        ),
        ("Submission deadline: Sept. 15, 2026", "2026-09-15"),
        ("31, March, 2023: Paper submission deadline", "2023-03-31"),
        (
            "Final submission deadline: 31^{st} March 2027.",
            "2027-03-31",
        ),
        ("Deadline for Submissions: 31/01/2027", "2027-01-31"),
        (
            "Submission Deadline: October 30, 2026Contribute to this Special Topic",
            "2026-10-30",
        ),
        ("Submission deadline: February 29, 2028", "2028-02-29"),
    ] {
        assert_eq!(
            parse_cfp_dates(text, CfpDateStage::Paper).unwrap()[0].date,
            expected,
            "{text}"
        );
    }
    for (text, start, end) in [
        ("Submissions Sept 1 - 30, 2026", "2026-09-01", "2026-09-30"),
        (
            "Submission period: April 30, 2026 - October 31, 2026",
            "2026-04-30",
            "2026-10-31",
        ),
        (
            "Submission window: 15 January 2027–12 February 2027",
            "2027-01-15",
            "2027-02-12",
        ),
        (
            "Submit proposals between January 1 and January 15, 2027",
            "2027-01-01",
            "2027-01-15",
        ),
        (
            "Submission window: 1–31 December 2026",
            "2026-12-01",
            "2026-12-31",
        ),
        (
            "Submission window: 15 January – 15 February 2027",
            "2027-01-15",
            "2027-02-15",
        ),
        (
            "Submission window: December 15–January 15, 2027",
            "2026-12-15",
            "2027-01-15",
        ),
    ] {
        let dates = parse_cfp_dates(text, CfpDateStage::Proposal)
            .unwrap_or_else(|| panic!("Rejected window: {text}"));
        assert_eq!(dates.len(), 2, "{text}");
        assert_eq!(
            (&*dates[0].date, dates[0].stage),
            (start, CfpDateStage::Opens)
        );
        assert_eq!(
            (&*dates[1].date, dates[1].stage),
            (end, CfpDateStage::Proposal)
        );
    }
}

#[test]
fn cfp_rejects_impossible_ambiguous_or_conflicting_admission_dates() {
    for text in [
        "Submission deadline: 31 October",
        "Submission deadline: 31 June 2027",
        "Submission deadline: February 29, 2027",
        "Submission deadline: autumn 2027",
        "Submission deadline: 2026-12-310",
        "Submission deadline: 12026-12-31",
        "Submission deadline: 1 May 2027\nSubmission deadline: 1 June 2027",
        "Submission window: 1–31 December 20260",
        "Submission deadlines: January 1 and January 15, 2027",
        "Submission deadline: 01/02/2027",
        "Abstract deadline: TBD\nFull manuscript submission deadline: 1 December 2026",
    ] {
        assert!(parse_cfp_source(&source(text)).is_none(), "{text}");
    }
    let dates=parse_cfp_dates("Submission deadline: 15 September 2026\nSubmission deadline (now extended!): October 1, 2026",CfpDateStage::Paper).unwrap();
    assert_eq!(dates.len(), 1);
    assert_eq!(dates[0].date, "2026-10-01");
}

#[test]
fn cfp_required_initial_gates_are_not_reopened_by_later_milestones() {
    let now = instant("2026-09-15T12:00:00Z");
    for text in ["Deadline for one-page abstracts: 1 May 2026\nDeadline for full manuscript submissions: 1 December 2026","Abstract deadline: 1 December 2026\nProposal deadline: 1 May 2026","投稿截止日期：2026年8月31日\n缴费截止日期：2026年9月18日\n修改稿截止日期：2026年10月1日\n最终录用通知：2026年11月1日\n预计出版：2027年1月1日"] {
        assert_eq!(parse_cfp_source(&source(text)).unwrap().state(now),CfpState::Closed);
    }
    let notice=parse_cfp_source(&source("Optional proposal deadline: 1 May 2026\nFull manuscript submission deadline: 1 December 2026")).unwrap();
    assert_eq!(notice.entry_deadline().unwrap().date, "2026-12-01");
    assert_eq!(notice.state(now), CfpState::Open);
    let notice=parse_cfp_source(&source("Submission to workshop (if desired) by: 15 April 2026\nDeadline for final paper submissions: 30 November 2026")).unwrap();
    assert_eq!(notice.entry_deadline().unwrap().date, "2026-11-30");
    let notice=parse_cfp_source(&source("Manuscript submission due: September 15, 2026\nRevised manuscript due: December 31, 2026\nFinal manuscript due: February 15, 2027")).unwrap();
    assert_eq!(notice.entry_deadline().unwrap().date, "2026-09-15");
    assert_eq!(
        notice
            .dates
            .iter()
            .filter(|date| date.stage == CfpDateStage::Revision)
            .count(),
        2
    );
}

#[test]
fn cfp_preserves_timezone_uncertainty_exclusive_cutoffs_and_source_constraints() {
    let mut source = source("应征作者务请于2026年9月20日前将论文发送至邮箱");
    source.time_zone = Some("Asia/Shanghai".into());
    let notice = parse_cfp_source(&source).unwrap();
    assert!(notice.entry_deadline().unwrap().is_exclusive);
    assert_eq!(
        notice.state(instant("2026-09-19T15:59:59Z")),
        CfpState::Open
    );
    assert_eq!(
        notice.state(instant("2026-09-19T16:00:00Z")),
        CfpState::Closed
    );
    source.time_zone = None;
    source.date_text = "Submission deadline: 20 September 2026".into();
    let notice = parse_cfp_source(&source).unwrap();
    for (now, expected) in [
        ("2026-09-20T09:59:59Z", CfpState::Open),
        ("2026-09-20T10:00:00Z", CfpState::Uncertain),
        ("2026-09-21T12:00:00Z", CfpState::Closed),
    ] {
        assert_eq!(notice.state(instant(now)), expected);
    }
    source.time_zone = Some("America/New_York".into());
    source.date_text = "Submission deadline: 31 October 2026".into();
    assert_eq!(
        parse_cfp_source(&source)
            .unwrap()
            .state(instant("2026-11-01T02:00:00Z")),
        CfpState::Open
    );
    source.date_text.clear();
    source.raw_date_text = "Deadline for Submissions: December 2030".into();
    let now = instant("2026-09-15T12:00:00Z");
    assert_eq!(
        parse_cfp_source(&source).unwrap().state(now),
        CfpState::Uncertain
    );
    source.is_historical = true;
    assert_eq!(
        parse_cfp_source(&source).unwrap().state(now),
        CfpState::Historical
    );
    source.status_text = Some("Submission status Closed".into());
    assert_eq!(
        parse_cfp_source(&source).unwrap().state(now),
        CfpState::Closed
    );
    source.is_historical = false;
    source.status_text =
        Some("JCST now only accepts special issue proposals on the invitation basis.".into());
    assert_eq!(
        parse_cfp_source(&source).unwrap().state(now),
        CfpState::InvitationOnly
    );
    source.raw_date_text.clear();
    source.status_text = None;
    assert_eq!(
        parse_cfp_source(&source).unwrap().state(now),
        CfpState::Undated
    );
    source.date_text =
        "Submissions Open: October 1, 2026\nSubmissions Close: November 1, 2026".into();
    assert_eq!(
        parse_cfp_source(&source).unwrap().state(now),
        CfpState::Upcoming
    );
}
