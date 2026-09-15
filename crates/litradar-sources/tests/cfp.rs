//! Publisher-boundary fixtures and discovery regression coverage.

use litradar_sources::cfp::{
    acquire_cfp_source, cfp_original_links, cfp_source_registry, extract_cfp_full_text,
    is_cfp_challenge, parse_cfp_page, CfpAdapter, CfpDocument, CfpSourceConfig, CfpSourceError,
    CfpTransport, CfpUrlRule,
};
use std::time::{Duration, Instant};

fn config(adapter: CfpAdapter) -> CfpSourceConfig {
    CfpSourceConfig {
        source_key: "journal:fixture".into(),
        catalog_ids: vec!["fixture".into()],
        journal_title: "Example Journal".into(),
        discovery_url: "https://example.org/calls".into(),
        adapter,
        config_version: 1,
        allowed_urls: vec![CfpUrlRule {
            host: "example.org".into(),
            path_prefix: "/".into(),
        }],
        identity_texts: vec!["Example Journal".into()],
        empty_statements: vec!["No current calls for special issues.".into()],
        capability_note: None,
        retains_previous_notices: false,
    }
}

fn document(text: &str) -> CfpDocument {
    CfpDocument {
        final_url: "https://example.org/calls".into(),
        text: text.into(),
        format: "html".into(),
    }
}

#[test]
fn cfp_html_keeps_only_journal_call_cards_and_discovers_new_original_titles() {
    let config = config(CfpAdapter::ElsevierCalls);
    let html="<html><head><script>fake call</script></head><body><h1>Example Journal</h1><h2>Latest articles</h2><h3>Published article</h3><h2>Call for papers</h2><h3>Original issue title</h3><p>Submission deadline: 15 December 2026</p><h3>New source-language topic</h3><p>Original AI research directions.</p><p>Submission deadline: 31 January 2027</p><footer><h2>Cookie Preference Center</h2><h3>Manage Consent Preferences</h3></footer></body></html>";
    let parsed = parse_cfp_page(&config, &document(html), "2026-09-15", true).unwrap();
    assert_eq!(parsed.sources.len(), 2);
    assert_eq!(parsed.sources[1].title, "New source-language topic");
    assert_eq!(parsed.sources[1].scope, "Original AI research directions.");
    assert_eq!(
        parsed.sources[1].date_text,
        "Submission deadline: 31 January 2027"
    );
}

#[test]
fn cfp_full_sections_retain_long_original_text_and_paragraph_boundaries() {
    let config = config(CfpAdapter::ElsevierCalls);
    let scope = format!(
        "{}\nFinal scope paragraph: multilingual evaluation and reproducibility.",
        "Original research directions include reliable multilingual models. "
            .repeat(18)
            .trim()
    );
    let requirements = format!(
        "{}\nFinal requirement: provide the complete experimental protocol.",
        "Authors must include original evidence and document their methods. "
            .repeat(18)
            .trim()
    );
    for heading in [
        "Submission instructions",
        "Submission guidelines",
        "Manuscript requirements",
        "投稿要求",
        "稿件要求",
    ] {
        let html = format!(
            "<h1>Special issue on multilingual models</h1><p>{scope}</p>\
             <h2>{heading}</h2><p>{requirements}</p>\
             <h2>Important dates</h2><p>Submission deadline: 31 December 2026</p>"
        );
        let parsed = parse_cfp_page(&config, &document(&html), "2026-09-15", false).unwrap();
        let source = &parsed.sources[0];
        assert_eq!(source.scope, scope, "{heading}");
        assert_eq!(source.requirements, requirements, "{heading}");
        let notice = litradar_domain::cfp::parse_cfp_source(source).unwrap();
        assert_eq!(notice.scope, scope, "{heading}");
        assert_eq!(notice.requirements, requirements, "{heading}");
    }
}

#[test]
fn cfp_full_text_reads_hidden_collection_paragraphs_and_plain_submission_labels() {
    let original = parse_cfp_page(
        &config(CfpAdapter::ElsevierCalls),
        &document("<h1>Example Journal</h1><h2>Call for papers</h2><h3>Source-language topic</h3><p>List preview...</p><p>Submission deadline: 31 December 2026</p>"),
        "2026-09-15", true,
    ).unwrap().sources.remove(0);
    let scope = format!(
        "{}\nTopics of interest\nFinal original research topic.",
        "Complete original research scope. ".repeat(40).trim()
    );
    let requirements = format!(
        "Submission Guidelines and Extended Versions\n{}\nFinal original submission requirement.",
        "Authors must document their methods. ".repeat(25).trim()
    );
    let html = format!("<h1 data-test='collection-title'>Source-language topic</h1><p>Participating journal: Example Journal</p><aside>Submission deadline: 31 December 2026</aside><div data-test='collection-description' style='max-height:100px;overflow:hidden'><p>{scope}</p><p>{requirements}</p></div><h2>Participating journal</h2><p>Unrelated journal metrics.</p>");
    let recovered = extract_cfp_full_text(&original, &document(&html)).unwrap();
    assert_eq!(recovered.scope, scope);
    assert_eq!(recovered.requirements, requirements);
    assert_eq!(recovered.date_text, original.date_text);
    assert!(!recovered.scope.contains("journal metrics"));
    let plain_labels = html.replace(
        "Submission Guidelines and Extended Versions",
        "Authors should prepare their manuscript using the original instructions.",
    );
    let recovered = extract_cfp_full_text(&original, &document(&plain_labels)).unwrap();
    assert_eq!(recovered.scope, scope);
    assert!(recovered.requirements.starts_with("Authors should prepare"));
}

#[test]
fn cfp_full_text_follows_title_links_and_rejects_previews_challenges_and_wrong_titles() {
    let original = parse_cfp_page(
        &config(CfpAdapter::ElsevierCalls),
        &document("<h1>Example Journal</h1><h2>Call for papers</h2><h3>Source-language topic</h3><p>List preview...</p><p>Submission deadline: 31 December 2026</p>"),
        "2026-09-15", true,
    ).unwrap().sources.remove(0);
    let list = document("<h1>Example Journal</h1><h2><a href='/collections/original'>Source-language topic</a></h2><p>List preview...</p><p>Submission deadline: 31 December 2026</p>");
    assert_eq!(
        cfp_original_links(&original, &list),
        vec!["https://example.org/collections/original"]
    );
    assert!(extract_cfp_full_text(&original, &list).is_err());
    let wrong = document("<h1 data-test='collection-title'>Different collection</h1><div data-test='collection-description'>Complete but unrelated original content.</div>");
    assert!(extract_cfp_full_text(&original, &wrong).is_err());
    let challenge = document(
        "<title>Client Challenge</title><script src='/_fs-ch-example/script.js'></script>",
    );
    assert!(is_cfp_challenge(&challenge));
    assert!(extract_cfp_full_text(&original, &challenge).is_err());
}

#[test]
fn cfp_full_text_excludes_sidebar_news_and_does_not_erase_unrecognized_requirements() {
    let mut original = parse_cfp_page(&config(CfpAdapter::ElsevierCalls), &document("<h1>Example Journal</h1><h2>Call for papers</h2><h3>Original topic</h3><p>Original preview.</p><p>Submission deadline: 31 December 2026</p>"), "2026-09-15", true).unwrap().sources.remove(0);
    original.requirements = "Previous instructions.".into();
    let html = "<h1>Original topic</h1><article class='general-post-content'><div class='prose'><p>Complete original research topics.</p><p><strong>Submission:</strong></p><p>Complete original manuscript requirements.</p></div><div>LATEST NEWS</div><p>Unrelated science news.</p><div>Read Next</div><p>Unrelated article recommendations.</p></article>";
    let recovered = extract_cfp_full_text(&original, &document(html)).unwrap();
    assert_eq!(recovered.scope, "Complete original research topics.");
    assert_eq!(
        recovered.requirements,
        "Submission:\nComplete original manuscript requirements."
    );
    assert!(extract_cfp_full_text(
        &original,
        &document(&html.replace("Submission:", "Unrecognized section label"))
    )
    .is_err());
    let mut pdf = document("Page 1\nThe Exchange\nOriginal topic\nRelevant topic.\nCall for Special Issue - Another topic\nUnrelated topic.\nAAEA Members in the News\nUnrelated news.");
    pdf.format = "pdf_text".into();
    assert!(extract_cfp_full_text(&original, &pdf).is_err());
    pdf.text = "Original topic\nComplete original research topics.\nSubmission:\nComplete original manuscript requirements.\nReferences:\nUnrelated reference entries.".into();
    let recovered = extract_cfp_full_text(&original, &pdf).unwrap();
    assert_eq!(recovered.scope, "Complete original research topics.");
    assert_eq!(
        recovered.requirements,
        "Submission:\nComplete original manuscript requirements."
    );
}

#[test]
fn cfp_full_text_recognizes_reviewed_submission_labels_and_scoped_chinese_bodies() {
    let mut original = parse_cfp_page(&config(CfpAdapter::ElsevierCalls), &document("<h1>Example Journal</h1><h2>Call for papers</h2><h3>Original topic</h3><p>Original preview.</p><p>Submission deadline: 31 December 2026</p>"), "2026-09-15", true).unwrap().sources.remove(0);
    original.requirements = "Previous requirements.".into();
    for label in [
        "Special Issue Submission and Review Process",
        "All manuscripts will be reviewed as a cohort",
    ] {
        let html = format!("<h1>Original topic</h1><div itemprop='articleBody'><p>Complete original scope.</p><h2>{label}</h2><p>Complete submission instructions.</p></div>");
        let recovered = extract_cfp_full_text(&original, &document(&html)).unwrap();
        assert_eq!(recovered.scope, "Complete original scope.");
        assert_eq!(
            recovered.requirements,
            format!("{label}\nComplete submission instructions.")
        );
    }
    original.catalog_ids = vec!["issn-0439-755x".into()];
    original.title = "原文征稿主题".into();
    let mut chinese = document("<div class='content_nr'><div class='item_biaoti'>原文征稿主题</div><div class='J_WenZhang'><p>完整的研究主题原文。</p><p>收稿形式与评审流程：</p><p>完整的投稿要求原文。</p></div><aside>无关推荐</aside></div>");
    chinese.final_url = "https://journal.psych.ac.cn/xlxb/CN/news/news64.shtml".into();
    let recovered = extract_cfp_full_text(&original, &chinese).unwrap();
    assert_eq!(recovered.scope, "完整的研究主题原文。");
    assert_eq!(
        recovered.requirements,
        "收稿形式与评审流程：\n完整的投稿要求原文。"
    );
    chinese.final_url = "https://example.org/news64.shtml".into();
    assert!(extract_cfp_full_text(&original, &chinese).is_err());
}

#[test]
fn cfp_comsoc_full_text_keeps_leading_dates_out_of_scope_and_requirements() {
    let mut original = parse_cfp_page(&config(CfpAdapter::ElsevierCalls), &document("<h1>Example Journal</h1><h2>Call for papers</h2><h3>Original topic</h3><p>Original preview.</p><p>Submission deadline: 31 December 2026</p>"), "2026-09-15", true).unwrap().sources.remove(0);
    original.catalog_ids = vec!["issn-0733-8716".into()];
    for body in [
        "<h2>Important Dates</h2><p>Manuscript Submission: 31 December 2026</p><h2>Scope</h2><p>Complete original research scope.</p><h2>Submission Guidelines</h2><p>Complete original submission requirements.</p><h2>Guest Editors</h2><p>Editorial names.</p>",
        "<h2>Call for Papers</h2><p>Complete original research scope.</p><h2>Submission Guidelines</h2><p>Complete original submission requirements.</p><h2>Important Dates</h2><p>Manuscript Submission: 31 December 2026</p><h2>Guest Editors</h2><p>Editorial names.</p>",
    ] {
        let mut page = document(&format!("<h1 class='h1--page-title'>Original topic</h1><article class='node--type-call-for-papers node--view-mode-full'><div class='paragraph-anchor-wrapper'><div class='text-long'>{body}</div></div></article>"));
        page.final_url = "https://www.comsoc.org/publications/journals/ieee-jsac/cfp/original-topic".into();
        let recovered = extract_cfp_full_text(&original, &page).unwrap();
        assert_eq!(recovered.scope, "Complete original research scope.");
        assert_eq!(recovered.requirements, "Submission Guidelines\nComplete original submission requirements.");
        page.final_url = "https://example.org/cfp/original-topic".into();
        assert!(extract_cfp_full_text(&original, &page).is_err());
    }
}

#[test]
fn cfp_springer_keeps_status_separate_from_deadlines_and_event_text() {
    let config = config(CfpAdapter::SpringerCollections);
    let parsed=parse_cfp_page(&config,&document("<h1>Example Journal</h1><h2>Collections</h2><h3>Source-language topic</h3><p>Original topic text.</p><div>Submission status <span>Open</span> Submission deadline <time>01 March 2027</time></div><h3>DISC 2024 (by invitation only)</h3><p>The conference was held October 28-Nov 1, 2024. Submissions are open now. The deadline to ...</p><p>Submission status Open</p>"),"2026-09-15",true).unwrap();
    assert_eq!(parsed.sources.len(), 2);
    assert_eq!(
        parsed.sources[0].date_text,
        "Submission deadline 01 March 2027"
    );
    assert!(parsed.sources[1].date_text.is_empty());
    assert!(parsed.sources[1]
        .status_text
        .as_ref()
        .unwrap()
        .contains("invitation only"));
}

#[test]
fn cfp_partial_springer_cards_fail_the_whole_snapshot() {
    let config = config(CfpAdapter::SpringerCollections);
    let html = "<h1>Example Journal</h1><h2>Valid original collection</h2><p>Submission status Open</p><p>Submission deadline: 1 December 2027</p><h2>Incomplete original collection</h2><p>Loading...</p>";
    assert!(matches!(
        parse_cfp_page(&config, &document(html), "2026-09-15", true),
        Err(CfpSourceError::Unrecognized)
    ));
}

#[test]
fn cfp_live_timeline_retains_opening_and_required_abstract_gates() {
    use litradar_domain::cfp::{parse_cfp_source, CfpState};
    let config = config(CfpAdapter::SpringerCollections);
    let html = "<h1>Example Journal</h1><h2>Worldwide topic</h2><p>World<span>wide</span> research.</p><p>Submission status Open</p><p>Submissions open on 01 October 2026</p><p>Submission deadline 31 March 2027</p><h2>Required abstract topic</h2><p>Submission status Open</p><p>Abstract deadline: 01 May 202<span>6</span></p><p>Submission deadline: 31 March 2027</p>";
    let parsed = parse_cfp_page(&config, &document(html), "2026-09-15", true).unwrap();
    assert_eq!(parsed.sources[0].scope, "Worldwide research.");
    let now = "2026-09-15T12:00:00Z".parse().unwrap();
    assert_eq!(
        parse_cfp_source(&parsed.sources[0]).unwrap().state(now),
        CfpState::Upcoming
    );
    assert_eq!(
        parse_cfp_source(&parsed.sources[1]).unwrap().state(now),
        CfpState::Closed
    );
}

#[test]
fn cfp_registry_preserves_reviewed_publisher_identity_and_preview_scope() {
    let registry = cfp_source_registry();
    let accident = registry
        .iter()
        .find(|config| config.journal_title == "Accident Analysis and Prevention")
        .unwrap();
    assert!(accident
        .identity_texts
        .contains(&"Accident Analysis & Prevention".to_owned()));
    let econometrics = registry
        .iter()
        .find(|config| config.journal_title == "Journal of Econometrics")
        .unwrap();
    assert!(econometrics.permits_url(
        &"https://www.sciencedirect.com/special-issue/336912/original"
            .parse()
            .unwrap()
    ));
    let fundamental = registry
        .iter()
        .find(|config| config.journal_title == "Fundamental Research")
        .unwrap();
    assert!(fundamental.retains_previous_notices);
    let keai = registry
        .iter()
        .find(|config| config.journal_title.contains("Virtual Reality"))
        .unwrap();
    let original = include_str!("fixtures/cfp-keai.txt")
        .lines()
        .skip(1)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(parse_cfp_page(
        keai,
        &CfpDocument {
            final_url: keai.discovery_url.clone(),
            text: original,
            format: "reviewed_text_fixture".into()
        },
        "2026-09-15",
        true
    )
    .is_ok());
}

#[test]
fn cfp_challenges_partial_lists_and_wrong_journals_cannot_be_verified_empty() {
    let config = config(CfpAdapter::ElsevierCalls);
    for html in ["<title>Just a moment...</title><p>Example Journal</p>","<h1>Different Journal</h1><h2>Call for papers</h2><h3>Example Journal study</h3><p>Original topic.</p><p>Submission deadline: 1 December 2027</p>","<h1>Example Journal</h1><h2>Call for papers</h2><h3>Only half a loaded card</h3>","<h1>Example Journal</h1><p>How to submit your Special Issue proposal:</p>","<h1>Example Journal</h1><p>No current calls for special issues.</p><a rel='next' href='/calls?page=2'>Next</a>"] {
        assert!(parse_cfp_page(&config,&document(html),"2026-09-15",true).is_err(),"{html}");
    }
    let parsed=parse_cfp_page(&config,&document("<h1>Example Journal</h1><p>No current calls for special issues.</p><p>Regular manuscript submission remains available.</p>"),"2026-09-15",true).unwrap();
    assert!(parsed.sources.is_empty());
    assert_eq!(
        parsed.empty_journals[0].source_statement,
        "No current calls for special issues."
    );
    assert!(parse_cfp_page(&config,&document("<h1>Original issue title</h1><p><a href='/original.pdf'>Download call for papers</a></p>"),"2026-09-15",false).is_err());
}

#[test]
fn cfp_saved_publisher_text_preserves_reviewed_card_boundaries() {
    for (needle, text, count) in [
        (
            "Acta Informatica",
            include_str!("fixtures/cfp-springer.txt"),
            3,
        ),
        (
            "Journal of Econometrics",
            include_str!("fixtures/cfp-elsevier.txt"),
            1,
        ),
        ("Virtual Reality", include_str!("fixtures/cfp-keai.txt"), 2),
    ] {
        let config = cfp_source_registry()
            .iter()
            .find(|config| config.journal_title.contains(needle))
            .unwrap();
        let document = CfpDocument {
            final_url: config.discovery_url.clone(),
            text: text.into(),
            format: "reviewed_text_fixture".into(),
        };
        let parsed = parse_cfp_page(config, &document, "2026-09-15", true)
            .unwrap_or_else(|error| panic!("{needle}: {error}"));
        assert_eq!(parsed.sources.len(), count, "{needle}");
        assert!(parsed
            .sources
            .iter()
            .all(|source| text.contains(&source.title)));
    }
}

#[test]
fn cfp_discovery_follows_required_details_and_propagates_incomplete_capture_failure() {
    struct Transport {
        has_detail: bool,
    }
    impl CfpTransport for Transport {
        fn fetch(
            &self,
            config: &CfpSourceConfig,
            url: &str,
            _deadline: Instant,
        ) -> Result<CfpDocument, CfpSourceError> {
            let text = if url == config.discovery_url {
                "<h1>Example Journal</h1><h2>Call for papers</h2><h3><a href='/new-call'>A newly announced issue</a></h3>"
            } else if self.has_detail {
                "<h1>A newly announced issue</h1><p>Original new topic.</p><p>Submission deadline: 1 December 2027</p>"
            } else {
                return Err(CfpSourceError::Request);
            };
            Ok(CfpDocument {
                final_url: url.into(),
                text: text.into(),
                format: "html".into(),
            })
        }
    }
    let config = config(CfpAdapter::ElsevierCalls);
    let acquired = acquire_cfp_source(
        &Transport { has_detail: true },
        &config,
        "2026-09-15",
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(acquired.documents.len(), 2);
    assert_eq!(acquired.sources.len(), 1);
    assert!(acquire_cfp_source(
        &Transport { has_detail: false },
        &config,
        "2026-09-15",
        Instant::now() + Duration::from_secs(2)
    )
    .is_err());
    assert_eq!(acquired.sources[0].title, "A newly announced issue");
    assert!(!config.permits_url(&"https://example.org.evil.test/calls".parse().unwrap()));
}

#[test]
fn cfp_linked_pdf_must_contain_the_original_call_title() {
    struct PdfTransport {
        is_matching: bool,
    }
    impl CfpTransport for PdfTransport {
        fn fetch(
            &self,
            config: &CfpSourceConfig,
            url: &str,
            _deadline: Instant,
        ) -> Result<CfpDocument, CfpSourceError> {
            if url == config.discovery_url {
                return Ok(document("<h1>Example Journal</h1><h2>Call for papers</h2><h3><a href='/original.pdf'>New original topic</a></h3>"));
            }
            Ok(CfpDocument {
                final_url: url.into(),
                text: format!(
                    "Example Journal\n{}\nSubmission deadline: 1 December 2027",
                    if self.is_matching {
                        "New original\ntopic"
                    } else {
                        "Different archived call"
                    }
                ),
                format: "pdf_text".into(),
            })
        }
    }
    let config = config(CfpAdapter::ElsevierCalls);
    let deadline = Instant::now() + Duration::from_secs(5);
    let acquired = acquire_cfp_source(
        &PdfTransport { is_matching: true },
        &config,
        "2026-09-15",
        deadline,
    )
    .unwrap();
    assert_eq!(acquired.sources[0].title, "New original topic");
    assert!(matches!(
        acquire_cfp_source(
            &PdfTransport { is_matching: false },
            &config,
            "2026-09-15",
            deadline
        ),
        Err(CfpSourceError::Unrecognized)
    ));
}
