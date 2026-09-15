//! Literal HTML heading/card boundaries for reviewed publisher CFP lists.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use litradar_domain::cfp::{
    clean_cfp_text, parse_cfp_dates, parse_cfp_source, CfpDateStage, CfpEmptyJournal, CfpSource,
};
use regex::Regex;
use reqwest::Url;
use scraper::{Html, Node, Selector};

use super::{CfpAdapter, CfpDocument, CfpSourceConfig, CfpSourceError, CFP_MAX_PAGE_BYTES};

static HEADINGS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^(#{1,6}) (.+)$").expect("heading regex"));
static STATUS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:Submission status|Status)[\s:]+(Open(?: for submissions)?|Closed|Upcoming)")
        .expect("status regex")
});
static DATE_LABEL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)deadline|submissions? (?:due|close|open|window|period)|(?:full )?(?:paper|manuscript|abstract|proposal) (?:submission|due)|(?:submit|submission) (?:by|before|until|deadline)|notification|final decision|camera.ready|revised manuscript|截[稿止]|投稿时间|征稿时间|来稿时间|提交时间|论文.*(?:发送|提交)|(?:发送|提交).*论文").expect("date label regex")
});
static DATE_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)20\d{2}|\b(?:Jan|Feb|Mar|Apr|May|Jun|Jul|Aug|Sep|Oct|Nov|Dec)\w*\b|\d+月|ongoing|rolling|TBD|to be (?:announced|determined)|待定").expect("date token regex")
});

/// One scoped page extraction before optional detail fetching.
pub struct CfpParsedPage {
    /// Literal records extracted from complete scoped cards or detail sections.
    pub sources: Vec<CfpSource>,
    /// Original verified scoped empty statements.
    pub empty_journals: Vec<CfpEmptyJournal>,
    /// Required detail links whose list cards did not contain a full announcement.
    pub detail_urls: Vec<String>,
    /// Literal list titles carried into a linked PDF that has no HTML heading structure.
    pub detail_titles: BTreeMap<String, String>,
}

fn pattern(pattern: &str, text: &str) -> bool {
    Regex::new(pattern)
        .expect("source parser pattern")
        .is_match(text)
}

fn visible_html(html: &Html) -> String {
    visible_html_excluding(html, None)
}

fn visible_html_excluding(html: &Html, exclusions: Option<&Selector>) -> String {
    let excluded = exclusions
        .into_iter()
        .flat_map(|selector| html.select(selector).map(|element| element.id()))
        .collect::<Vec<_>>();
    let mut text = String::new();
    let mut nodes = vec![(*html.root_element(), false)];
    while let Some((node, is_exiting)) = nodes.pop() {
        if excluded.contains(&node.id()) {
            continue;
        }
        match node.value() {
            Node::Text(value) => text.push_str(value),
            Node::Element(element) => {
                let name = element.name();
                if matches!(
                    name,
                    "script" | "style" | "head" | "noscript" | "svg" | "nav" | "footer"
                ) || element.attr("hidden").is_some()
                    || element.attr("aria-hidden") == Some("true")
                {
                    continue;
                }
                let is_heading = matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6");
                let is_block = is_heading
                    || matches!(
                        name,
                        "p" | "div"
                            | "section"
                            | "article"
                            | "li"
                            | "tr"
                            | "br"
                            | "dl"
                            | "dt"
                            | "dd"
                    );
                if is_block {
                    text.push('\n');
                }
                if is_exiting {
                    continue;
                }
                if is_heading {
                    text.push_str(&"#".repeat((name.as_bytes()[1] - b'0') as usize));
                    text.push(' ');
                }
                nodes.push((node, true));
                nodes.extend(
                    node.children()
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(|child| (child, false)),
                );
            }
            _ => {}
        }
    }
    clean_cfp_text(&text)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn plain(value: &str) -> String {
    clean_cfp_text(
        &value
            .lines()
            .map(|line| line.trim_start_matches('#').trim())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// Whether a captured page is an access challenge rather than publisher content.
pub fn is_cfp_challenge(document: &CfpDocument) -> bool {
    let html = Html::parse_document(&document.text);
    let title = html
        .select(&Selector::parse("title,h1").expect("challenge title selector"))
        .map(|element| element.text().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    if pattern(
        r"(?i)just a moment|client challenge|access denied|verify (?:that )?you are human|captcha|robot verification",
        &title,
    ) {
        return true;
    }
    let has_challenge_scripts = pattern(
        r"(?i)cf-chl-|challenge-platform|enable javascript and cookies to continue|/_fs-ch-",
        &document.text,
    );
    let visible = visible_html(&html);
    has_challenge_scripts
        && (visible.trim().is_empty()
            || pattern(
                r"(?im)^(?:#{1,6} )?(?:performing security verification|verifying you are human|enable javascript and cookies to continue|checking your browser before accessing|verification successful\. waiting for .+ to respond)[.!]?\s*$",
                &visible,
            ))
}

fn matching_title(first: &str, second: &str) -> bool {
    let normalize = |value: &str| {
        clean_cfp_text(value)
            .chars()
            .filter(|character| character.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect::<String>()
    };
    let first = normalize(first);
    !first.is_empty() && first == normalize(second)
}

fn springer_update_body<'a>(
    original: &CfpSource,
    document: &CfpDocument,
    html: &'a Html,
) -> Option<scraper::ElementRef<'a>> {
    let url = Url::parse(&document.final_url).ok()?;
    if url.host_str() != Some("link.springer.com")
        || !url.path().starts_with("/journal/10490/updates/")
        || !original.catalog_ids.iter().any(|id| id == "issn-0217-4561")
    {
        return None;
    }
    let body = html
        .select(&Selector::parse("#updates-content-body").expect("Springer update body selector"))
        .next()?;
    let title = body
        .select(&Selector::parse("h1").expect("Springer update title selector"))
        .next()?;
    let title = visible_html_excluding(
        &Html::parse_fragment(&title.inner_html()),
        Some(&Selector::parse(".u-visually-hidden").expect("Springer hidden title selector")),
    );
    matching_title(&title, &original.title).then_some(body)
}

/// Find original-title detail links and bounded journal-list navigation for an existing notice.
pub fn cfp_original_links(source: &CfpSource, document: &CfpDocument) -> Vec<String> {
    let html = Html::parse_document(&document.text);
    let Ok(base) = Url::parse(&document.final_url) else {
        return Vec::new();
    };
    let mut details = Vec::new();
    if let Some(body) = springer_update_body(source, document, &html) {
        for paragraph in
            body.select(&Selector::parse("p").expect("Springer full-call paragraph selector"))
        {
            if !pattern(
                r"(?i)read the full call for papers",
                &paragraph.text().collect::<String>(),
            ) {
                continue;
            }
            for link in paragraph
                .select(&Selector::parse("a[href]").expect("Springer full-call link selector"))
            {
                if let Some(url) = link
                    .value()
                    .attr("href")
                    .and_then(|href| base.join(href).ok())
                    .filter(|url| matches!(url.scheme(), "https" | "http"))
                {
                    details.push(url.to_string());
                }
            }
        }
        return details;
    }
    if base.host_str() == Some("www.poms.org")
        && base.path().trim_end_matches('/') == "/journal/announcements"
        && source.catalog_ids.iter().any(|id| id == "issn-1059-1478")
    {
        let title_selector =
            Selector::parse(".views-field-title .field-content").expect("POMS card title selector");
        let link_selector = Selector::parse(".views-field-field-submission-guidelines-docu a[href], .views-field-views-conditional-field a[href]")
            .expect("POMS card detail selector");
        for card in
            html.select(&Selector::parse(".poms-special-issues").expect("POMS card selector"))
        {
            if !card
                .select(&title_selector)
                .any(|title| matching_title(&title.text().collect::<String>(), &source.title))
            {
                continue;
            }
            for link in card.select(&link_selector) {
                if let Some(url) = link
                    .value()
                    .attr("href")
                    .and_then(|href| base.join(href).ok())
                    .filter(|url| {
                        matches!(url.scheme(), "https" | "http")
                            && url.host_str() == base.host_str()
                    })
                {
                    let url = url.to_string();
                    if !details.contains(&url) {
                        details.push(url);
                    }
                }
            }
        }
        return details;
    }
    if base.host_str() == Some("www.ieee-ras.org")
        && base.path().trim_end_matches('/') == "/publications/t-ase/special-issues-t-ase"
        && source.catalog_ids.iter().any(|id| id == "issn-1545-5955")
    {
        let title_selector = Selector::parse("h3.dynamic-content-for-elementor-acf")
            .expect("T-ASE card title selector");
        let link_selector =
            Selector::parse("a.elementor-button[href]").expect("T-ASE download selector");
        for card in html.select(
            &Selector::parse("div[data-elementor-type='container'][data-elementor-id='16977']")
                .expect("T-ASE card selector"),
        ) {
            if !card
                .select(&title_selector)
                .any(|title| matching_title(&title.text().collect::<String>(), &source.title))
            {
                continue;
            }
            for link in card.select(&link_selector) {
                if let Some(url) = link
                    .value()
                    .attr("href")
                    .and_then(|href| base.join(href).ok())
                    .filter(|url| {
                        matches!(url.scheme(), "https" | "http")
                            && url.host_str() == base.host_str()
                            && url.path().to_ascii_lowercase().ends_with(".pdf")
                    })
                {
                    let url = url.to_string();
                    if !details.contains(&url) {
                        details.push(url);
                    }
                }
            }
        }
        return details;
    }
    let mut indexes = Vec::new();
    let has_title = html
        .select(&Selector::parse("h1,h2,h3,h4,title").expect("original title selector"))
        .any(|element| matching_title(&element.text().collect::<String>(), &source.title));
    for element in html.select(&Selector::parse("a[href]").expect("original link selector")) {
        let label = clean_cfp_text(&element.text().collect::<String>());
        let Some(url) = element
            .value()
            .attr("href")
            .and_then(|href| base.join(href).ok())
            .filter(|url| {
                matches!(url.scheme(), "https" | "http") && url.as_str() != base.as_str()
            })
        else {
            continue;
        };
        if matching_title(&label, &source.title)
            || (has_title
                && url.path().to_ascii_lowercase().ends_with(".pdf")
                && pattern(r"(?i)call for papers|download|征稿|征文|pdf", &label))
        {
            details.push(url.to_string());
        } else if url.host_str() == base.host_str()
            && (element
                .value()
                .attr("rel")
                .is_some_and(|rel| rel.split_whitespace().any(|part| part == "next"))
                || pattern(
                    r"(?i)^(?:view all calls for papers|calls? for papers|collections|collections and calls for papers|next page|下一页|征稿启事|征稿通知|征文通知)$",
                    &label,
                ))
        {
            indexes.push(url.to_string());
        }
    }
    details.extend(indexes.into_iter().take(3));
    let mut seen = BTreeSet::new();
    details.retain(|url| seen.insert(url.clone()));
    details
}

fn full_text_sections(body: &str) -> (String, String) {
    let body = plain(body);
    let end = Regex::new(r"(?im)^(?:[一二三四五六七八九十\d]+[、.．\s]+)?(?:references|bibliography|参考文献)\s*[:：]?\s*$")
        .expect("reference section boundary");
    let body = end
        .find(&body)
        .map_or(body.as_str(), |boundary| &body[..boundary.start()])
        .trim();
    let requirements = Regex::new(r"(?im)^(?:(?:[一二三四五六七八九十\d]+)[、.．\s]+)?(?:submissions?\s*[:：]|submissions? (?:format|guidelines?|instructions|information|requirements|procedure|process)|special issue submission and review process|all manuscripts will be reviewed as a cohort|all submissions must be formatted|instructions for authors|manuscript (?:preparation|requirements|submission)|author (?:guidelines|instructions)|how to submit|paper submission|稿件要求|投稿要求|征稿要求|投稿方式|投稿渠道|投稿网址|投稿指南|论文要求|提交要求|征文要求|来稿要求|征文投稿说明|收稿形式与评审流程|稿件提交|authors should prepare|prospective authors should submit|authors are encouraged to contact the editorial team|submitted papers should|papers must (?:be submitted|follow))").expect("full-text requirements boundary");
    match requirements.find(body) {
        Some(boundary) => (
            body[..boundary.start()].trim().to_owned(),
            body[boundary.start()..].trim().to_owned(),
        ),
        None => (body.to_owned(), String::new()),
    }
}

/// Extract complete original sections from a verified existing announcement, never a list preview.
pub fn extract_cfp_full_text(
    original: &CfpSource,
    document: &CfpDocument,
) -> Result<CfpSource, CfpSourceError> {
    if document.text.len() > CFP_MAX_PAGE_BYTES || is_cfp_challenge(document) {
        return Err(CfpSourceError::Unrecognized);
    }
    let html = Html::parse_document(&document.text);
    let collection_title = html
        .select(
            &Selector::parse("[data-test='collection-title']").expect("collection title selector"),
        )
        .next();
    let body = if let Some(title) = collection_title {
        if !matching_title(&title.text().collect::<String>(), &original.title) {
            return Err(CfpSourceError::Unrecognized);
        }
        let description = html
            .select(
                &Selector::parse("[data-test='collection-description']")
                    .expect("collection description selector"),
            )
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        visible_html(&Html::parse_fragment(&description.inner_html()))
    } else if let Some(body) = springer_update_body(original, document, &html) {
        let body = visible_html_excluding(
            &Html::parse_fragment(&body.inner_html()),
            Some(
                &Selector::parse("h1, .u-visually-hidden")
                    .expect("Springer update title exclusion"),
            ),
        );
        if pattern(r"(?i)read the full call for papers", &body) {
            return Err(CfpSourceError::Unrecognized);
        }
        body
    } else if Url::parse(&document.final_url).is_ok_and(|url| {
        url.host_str() == Some("www.comsoc.org")
            && [
                ("ieee-jsac", "issn-0733-8716"),
                ("ieee-tnsm", "issn-1932-4537"),
            ]
            .iter()
            .any(|(journal, catalog_id)| {
                url.path()
                    .starts_with(&format!("/publications/journals/{journal}/cfp/"))
                    && original.catalog_ids.iter().any(|id| id == catalog_id)
            })
    }) {
        let has_title = html
            .select(&Selector::parse("h1.h1--page-title").expect("ComSoc title selector"))
            .any(|title| matching_title(&title.text().collect::<String>(), &original.title));
        if !has_title {
            return Err(CfpSourceError::Unrecognized);
        }
        let article = html
            .select(
                &Selector::parse("article.node--type-call-for-papers.node--view-mode-full")
                    .expect("ComSoc article selector"),
            )
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        let parts = article
            .select(
                &Selector::parse(".paragraph-anchor-wrapper .text-long")
                    .expect("ComSoc text selector"),
            )
            .map(|part| visible_html(&Html::parse_fragment(&part.inner_html())))
            .collect::<Vec<_>>()
            .join("\n");
        let text = plain(&parts);
        let scope_heading = Regex::new(r"(?im)^(?:scope|call for papers)\s*[:：]?\s*$")
            .expect("ComSoc scope heading")
            .find(&text);
        let scope_start = scope_heading.map_or(0, |heading| heading.end());
        if scope_heading.is_none() && text.to_lowercase().starts_with("important dates") {
            return Err(CfpSourceError::Unrecognized);
        }
        let requirement_heading =
            Regex::new(r"(?im)^submissions? (?:format|guidelines)\s*[:：]?\s*$")
                .expect("ComSoc requirement heading")
                .find(&text)
                .ok_or(CfpSourceError::Unrecognized)?;
        if scope_start >= requirement_heading.start() {
            return Err(CfpSourceError::Unrecognized);
        }
        let end = Regex::new(r"(?im)^(?:important dates|guest editors|references)\s*[:：]?\s*$")
            .expect("ComSoc section end");
        let scope = &text[scope_start..requirement_heading.start()];
        let scope = end
            .find(scope)
            .map_or(scope, |boundary| &scope[..boundary.start()])
            .trim();
        let requirements = &text[requirement_heading.start()..];
        let requirements = end
            .find(requirements)
            .map_or(requirements, |boundary| &requirements[..boundary.start()])
            .trim();
        format!("{scope}\n{requirements}")
    } else if Url::parse(&document.final_url).is_ok_and(|url| {
        url.host_str() == Some("journal.psych.ac.cn") && url.path().starts_with("/xlxb/CN/news/")
    }) && original.catalog_ids.iter().any(|id| id == "issn-0439-755x")
    {
        let title_selector =
            Selector::parse(".item_biaoti").expect("psychology notice title selector");
        let container = html
            .select(&Selector::parse(".content_nr").expect("psychology notice container selector"))
            .find(|container| {
                container
                    .select(&title_selector)
                    .any(|title| matching_title(&title.text().collect::<String>(), &original.title))
            })
            .ok_or(CfpSourceError::Unrecognized)?;
        let body = container
            .select(&Selector::parse(".J_WenZhang").expect("psychology notice body selector"))
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        visible_html(&Html::parse_fragment(&body.inner_html()))
    } else if Url::parse(&document.final_url).is_ok_and(|url| {
        url.host_str() == Some("www.resci.cn") && url.path().starts_with("/CN/news/")
    }) && original.catalog_ids.iter().any(|id| id == "issn-1007-7588")
    {
        let title_selector =
            Selector::parse(".newstitle").expect("resources notice title selector");
        let body = html
            .select(
                &Selector::parse(".content_nr > .news-content")
                    .expect("resources notice body selector"),
            )
            .find(|body| {
                body.select(&title_selector)
                    .any(|title| matching_title(&title.text().collect::<String>(), &original.title))
            })
            .ok_or(CfpSourceError::Unrecognized)?;
        visible_html_excluding(
            &Html::parse_fragment(&body.inner_html()),
            Some(
                &Selector::parse(".newstitle, .text-right")
                    .expect("resources notice metadata selector"),
            ),
        )
    } else if Url::parse(&document.final_url).is_ok_and(|url| {
        url.host_str() == Some("chinaifs.org.cn")
            && url.path().starts_with("/html/web/tongzhigonggao/")
    }) && original.catalog_ids.iter().any(|id| id == "issn-1006-1029")
    {
        let title_selector = Selector::parse(".news-title > h1, .news-title > h2")
            .expect("finance notice title selector");
        let container = html
            .select(&Selector::parse(".news-wrap").expect("finance notice container selector"))
            .find(|container| {
                let title = container
                    .select(&title_selector)
                    .map(|title| title.text().collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                matching_title(&title, &original.title)
            })
            .ok_or(CfpSourceError::Unrecognized)?;
        let body = container
            .select(&Selector::parse(".news-content").expect("finance notice body selector"))
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        visible_html(&Html::parse_fragment(&body.inner_html()))
    } else if Url::parse(&document.final_url).is_ok_and(|url| {
        url.host_str() == Some("www.jryj.org.cn") && url.path().starts_with("/CN/news/")
    }) && original.catalog_ids.iter().any(|id| id == "issn-1002-7246")
    {
        let title = html
            .select(&Selector::parse("td.news_biaoti").expect("financial research title selector"))
            .find(|title| matching_title(&title.text().collect::<String>(), &original.title))
            .ok_or(CfpSourceError::Unrecognized)?;
        let table = title
            .ancestors()
            .filter_map(scraper::ElementRef::wrap)
            .find(|element| element.value().name() == "table")
            .ok_or(CfpSourceError::Unrecognized)?;
        let body = table
            .select(&Selector::parse("span.J_WenZhang").expect("financial research body selector"))
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        visible_html(&Html::parse_fragment(&body.inner_html()))
    } else if Url::parse(&document.final_url).is_ok_and(|url| {
        url.host_str() == Some("kxxyj.magtechjournal.com")
            && url.path().starts_with("/kxxyj/CN/news/")
    }) && original.catalog_ids.iter().any(|id| id == "issn-1003-2053")
    {
        let title_selector =
            Selector::parse(".item_biaoti").expect("science notice title selector");
        let body = html
            .select(
                &Selector::parse(".content_nr > .item_con > ul")
                    .expect("science notice body selector"),
            )
            .find(|body| {
                body.select(&title_selector)
                    .any(|title| matching_title(&title.text().collect::<String>(), &original.title))
            })
            .ok_or(CfpSourceError::Unrecognized)?;
        visible_html_excluding(
            &Html::parse_fragment(&body.inner_html()),
            Some(&title_selector),
        )
    } else if Url::parse(&document.final_url)
        .is_ok_and(|url| url.host_str() == Some("www.poms.org") && url.path().starts_with("/node/"))
        && original.catalog_ids.iter().any(|id| id == "issn-1059-1478")
    {
        let title_selector = Selector::parse("h1.node__title").expect("POMS notice title selector");
        let article = html
            .select(
                &Selector::parse("article.node--type-call-for-papers.node--view-mode-full")
                    .expect("POMS notice container selector"),
            )
            .find(|article| {
                article
                    .select(&title_selector)
                    .any(|title| matching_title(&title.text().collect::<String>(), &original.title))
            })
            .ok_or(CfpSourceError::Unrecognized)?;
        let body = article
            .select(
                &Selector::parse(".field-name-field-submission-guidelines-summ")
                    .expect("POMS notice body selector"),
            )
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        visible_html(&Html::parse_fragment(&body.inner_html()))
    } else if Url::parse(&document.final_url).is_ok_and(|url| {
        url.host_str() == Some("www.grss-ieee.org")
            && url
                .path()
                .starts_with("/publications/author-resources/grsl-special-streams/")
    }) && original.catalog_ids.iter().any(|id| id == "issn-1545-598x")
    {
        let matches_title = |value: &str| {
            ["", "GRSS Special Stream on ", "GRSS Special Stream of the "]
                .iter()
                .any(|prefix| {
                    value
                        .trim()
                        .strip_prefix(prefix)
                        .is_some_and(|title| matching_title(title, &original.title))
                })
        };
        let title_selector =
            Selector::parse(".elementor-widget-theme-post-title h1").expect("GRSL title selector");
        let article = html
            .select(
                &Selector::parse(
                    "[data-elementor-type='single-post'].category-grsl-special-streams",
                )
                .expect("GRSL detail selector"),
            )
            .find(|article| {
                article
                    .select(&title_selector)
                    .any(|title| matches_title(&title.text().collect::<String>()))
            })
            .ok_or(CfpSourceError::Unrecognized)?;
        let body = article
            .select(
                &Selector::parse(
                    ".elementor-widget-theme-post-content > .elementor-widget-container",
                )
                .expect("GRSL body selector"),
            )
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        let text = plain(&visible_html(&Html::parse_fragment(&body.inner_html())));
        let text = text
            .split_once('\n')
            .filter(|(title, _)| matches_title(title))
            .map_or(text.as_str(), |(_, body)| body);
        let requirements = Regex::new(r"(?im)^all submissions must be formatted")
            .expect("GRSL requirements boundary")
            .find(text)
            .ok_or(CfpSourceError::Unrecognized)?;
        let scope = &text[..requirements.start()];
        let scope_end = Regex::new(r"(?im)^(?:guest editors?|schedule)\s*[:：]?\s*$")
            .expect("GRSL scope boundary");
        let scope = scope_end
            .find(scope)
            .map_or(scope, |boundary| &scope[..boundary.start()]);
        format!("{}\n{}", scope.trim(), text[requirements.start()..].trim())
    } else if document.format == "pdf_text" {
        let text = plain(&document.text);
        if pattern(
            r"(?im)^(?:the exchange|table of contents|contents|newsletter|members in the news|aaea members in the news|news and announcements|job announcements|anti.harassment and code of conduct policy)\s*$",
            &text,
        ) {
            return Err(CfpSourceError::Unrecognized);
        }
        let title = original
            .title
            .split_whitespace()
            .map(regex::escape)
            .collect::<Vec<_>>()
            .join(r"\s+");
        let title =
            Regex::new(&format!("(?i){title}")).map_err(|_| CfpSourceError::Unrecognized)?;
        let matched = title
            .find(&text)
            .or_else(|| {
                let is_reviewed = Url::parse(&document.final_url).is_ok_and(|url| {
                    (url.host_str() == Some("ieee-iotj.org")
                        && url.path().starts_with("/wp-content/uploads/")
                        && original.catalog_ids.iter().any(|id| id == "issn-2327-4662"))
                        || (url.host_str() == Some("www.poms.org")
                            && url
                                .path()
                                .starts_with("/sites/default/files/callforpapers/")
                            && original.catalog_ids.iter().any(|id| id == "issn-1059-1478"))
                });
                if !is_reviewed {
                    return None;
                }
                let typography = original
                    .title
                    .replace('&', "and")
                    .chars()
                    .filter(|character| character.is_alphanumeric())
                    .map(|character| regex::escape(&character.to_string()))
                    .collect::<Vec<_>>()
                    .join(r"[\s\p{P}]*");
                Regex::new(&format!("(?i){typography}")).ok()?.find(&text)
            })
            .ok_or(CfpSourceError::Unrecognized)?;
        if text[..matched.start()].chars().count() > 600 {
            return Err(CfpSourceError::Unrecognized);
        }
        let body = text[matched.end()..].trim();
        if Url::parse(&document.final_url).is_ok_and(|url| {
            url.host_str() == Some("www.poms.org")
                && url.path()
                    == "/sites/default/files/callforpapers/FlexMfgEcosystems-Revised_0.pdf"
        }) && original.catalog_ids.iter().any(|id| id == "issn-1059-1478")
        {
            let scope = Regex::new(r"(?im)^Background:")
                .expect("POMS background boundary")
                .find(body)
                .ok_or(CfpSourceError::Unrecognized)?;
            let dates = Regex::new(r"(?im)^Deadlines\s*$")
                .expect("POMS dates boundary")
                .find(body)
                .ok_or(CfpSourceError::Unrecognized)?;
            let requirements =
                Regex::new(r"(?im)^Authors are encouraged to contact the editorial team")
                    .expect("POMS submission boundary")
                    .find(body)
                    .ok_or(CfpSourceError::Unrecognized)?;
            let editors = Regex::new(r"(?im)^Guest Editors\s*$")
                .expect("POMS biography boundary")
                .find(body)
                .ok_or(CfpSourceError::Unrecognized)?;
            if !(scope.start() < dates.start()
                && dates.end() <= requirements.start()
                && requirements.start() < editors.start())
            {
                return Err(CfpSourceError::Unrecognized);
            }
            format!(
                "{}\n{}",
                body[scope.start()..dates.start()].trim(),
                body[requirements.start()..editors.start()].trim()
            )
        } else {
            body.to_owned()
        }
    } else {
        let has_title = html
            .select(&Selector::parse("h1,h2").expect("article title selector"))
            .any(|element| matching_title(&element.text().collect::<String>(), &original.title));
        if !has_title {
            return Err(CfpSourceError::Unrecognized);
        }
        let article = html.select(&Selector::parse("article.general-post-content .prose, [itemprop='articleBody'], .entry-content, .article-content, .c-article-body").expect("original article body selector"))
            .next()
            .ok_or(CfpSourceError::Unrecognized)?;
        let body = visible_html(&Html::parse_fragment(&article.inner_html()));
        let end = Regex::new(r"(?im)^(?:#{1,6} )?(?:journal navigation|related articles|related content|latest articles|latest news|read next|about springer nature link|footer navigation|navigation|search|references|cookie preferences)\s*$").expect("announcement end boundary");
        let body = end
            .find(&body)
            .map_or(body.as_str(), |boundary| &body[..boundary.start()]);
        body.trim().to_owned()
    };
    let (mut scope, mut requirements) = full_text_sections(&body);
    if original.catalog_ids.iter().any(|id| id == "issn-1007-7588") {
        let notes = Regex::new(r"(?m)^(?:重点注意事项|注意事项|时间节点)\s*[:：]?\s*$")
            .expect("resources submission notes boundary");
        if let Some(start) = notes.find(&scope).map(|boundary| boundary.start()) {
            requirements = format!("{}\n{}", scope[start..].trim(), requirements)
                .trim()
                .to_owned();
            scope = scope[..start].trim().to_owned();
        }
    }
    if (scope.is_empty() && requirements.is_empty())
        || (!original.requirements.is_empty() && requirements.is_empty())
        || pattern(r"(?m)(?:\.{3}|…)\s*$", &scope)
        || pattern(r"(?m)(?:\.{3}|…)\s*$", &requirements)
        || (collection_title.is_none()
            && !document.format.eq("pdf_text")
            && html
                .select(&Selector::parse("a[href]").expect("detail link selector"))
                .any(|link| {
                    matching_title(&link.text().collect::<String>(), &original.title)
                        && link.value().attr("href").is_some_and(|href| {
                            Url::parse(&document.final_url)
                                .ok()
                                .and_then(|base| base.join(href).ok())
                                .is_some_and(|url| {
                                    url.as_str().split('#').next()
                                        != document.final_url.split('#').next()
                                })
                        })
                }))
    {
        return Err(CfpSourceError::Unrecognized);
    }
    let mut source = original.clone();
    source.scope = scope;
    source.requirements = requirements;
    source.source_url = document.final_url.clone();
    parse_cfp_source(&source).ok_or(CfpSourceError::Unrecognized)?;
    Ok(source)
}

fn blocks(text: &str, min: usize, max: usize) -> Vec<(String, String)> {
    let headings: Vec<_> = HEADINGS.captures_iter(text).collect();
    headings
        .iter()
        .enumerate()
        .filter_map(|(index, heading)| {
            let level = heading[1].len();
            if !(min..=max).contains(&level) {
                return None;
            }
            let end = headings[index + 1..]
                .iter()
                .find(|later| later[1].len() <= level)
                .map_or(text.len(), |later| {
                    later.get(0).expect("full heading").start()
                });
            Some((
                plain(&heading[2]),
                text[heading.get(0).expect("full heading").end()..end]
                    .trim()
                    .to_owned(),
            ))
        })
        .collect()
}

fn date_clauses(body: &str) -> String {
    let text = plain(body);
    let text = Regex::new(r"([.!?])\s+([A-Z])")
        .expect("sentence boundary")
        .replace_all(&text, "$1\n$2");
    let lines: Vec<_> = text.lines().collect();
    let mut clauses = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !DATE_LABEL.is_match(line) {
            continue;
        }
        let clause = if DATE_TOKEN.is_match(line) {
            (*line).to_owned()
        } else if let Some(next) = lines
            .get(index + 1)
            .filter(|line| DATE_TOKEN.is_match(line))
        {
            format!("{line} {next}")
        } else {
            continue;
        };
        if !clauses.contains(&clause) {
            clauses.push(clause);
        }
    }
    clauses.join("\n")
}

fn record(
    config: &CfpSourceConfig,
    document: &CfpDocument,
    checked_on: &str,
    title: &str,
    body: &str,
    status: Option<String>,
) -> Result<CfpSource, CfpSourceError> {
    let type_text = if pattern(r"(?i)proposals?|提案", title) {
        "Call for Proposals"
    } else {
        "Special Issue"
    };
    let plain_body = plain(body);
    let timeline_body = STATUS.replace_all(&plain_body, "");
    let mut date_text = date_clauses(&timeline_body);
    let stage = if type_text == "Call for Proposals" {
        CfpDateStage::Proposal
    } else {
        CfpDateStage::Paper
    };
    let raw_date_text = if parse_cfp_dates(&date_text, stage).is_none() {
        std::mem::take(&mut date_text)
    } else {
        String::new()
    };
    let requirements = blocks(body, 2, 6)
        .into_iter()
        .find(|(title, _)| {
            pattern(
                r"(?i)submission (?:instructions|guidelines)|manuscript requirements|投稿要求|稿件要求",
                title,
            )
        })
        .map_or(String::new(), |(_, body)| plain(&body));
    let scope_lines: Vec<_> = plain(body)
        .lines()
        .take_while(|line| {
            !DATE_LABEL.is_match(line)
                && !STATUS.is_match(line)
                && !pattern(
                    r"(?i)^guest editors?|^submission (?:instructions|guidelines)|^manuscript requirements|^投稿要求|^稿件要求",
                    line,
                )
        })
        .filter(|line| !pattern(r"(?i)^(?:[0-3]?\d\s+[a-z]{3,9}\s+20\d{2}|[a-z]{3,9}\s+[0-3]?\d,?\s+20\d{2}|20\d{2}[-/]\d{1,2}[-/]\d{1,2})$", line.trim()))
        .map(str::to_owned)
        .collect();
    let mut status = status.unwrap_or_default();
    if let Some(invitation)=plain(&format!("{title}\n{body}")).lines().find(|line|pattern(r"(?i)invit(?:ation|e|ed)[ -]*only|invitation required|invited (?:submissions|papers) only|仅限受邀",line)) {status.push('\n');status.push_str(invitation);}
    let source = CfpSource {
        catalog_ids: config.catalog_ids.clone(),
        journal_title: config.journal_title.clone(),
        title: plain(title),
        scope: plain(&scope_lines.join("\n")),
        requirements,
        type_text: type_text.into(),
        date_text,
        source_url: document.final_url.clone(),
        checked_on: checked_on.into(),
        entry_stage: None,
        time_zone: None,
        status_text: (!status.is_empty()).then_some(status),
        is_historical: false,
        raw_date_text,
    };
    parse_cfp_source(&source).ok_or(CfpSourceError::Unrecognized)?;
    Ok(source)
}

fn has_journal_identity(config: &CfpSourceConfig, html: &Html, text: &str) -> bool {
    let first_card = HEADINGS
        .captures_iter(text)
        .find(|heading| {
            heading[1].len()
                >= if config.adapter == CfpAdapter::SpringerCollections {
                    2
                } else {
                    3
                }
        })
        .map_or(text.len(), |heading| {
            heading.get(0).expect("heading boundary").start()
        });
    let prefix = &text[..first_card];
    let titles = html
        .select(&Selector::parse("title").expect("identity title selector"))
        .map(|element| element.text().collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    let identity_text = format!("{titles}\n{prefix}");
    config.identity_texts.iter().any(|identity| {
        if pattern(r"^\d{4}-\d{3}[0-9X]$", identity) {
            return pattern(
                &format!(
                    r"(?i)\b(?:e-?)?issn\s*[:：]?\s*{}\b",
                    regex::escape(identity)
                ),
                prefix,
            );
        }
        identity_text
            .lines()
            .flat_map(|line| line.split(" | "))
            .flat_map(|line| line.split(" - "))
            .flat_map(|line| line.split(" — "))
            .any(|line| {
                line.trim_start_matches('#')
                    .trim()
                    .eq_ignore_ascii_case(identity)
            })
    })
}

/// Extract only verified publisher cards; a missing/partial layout is never an empty result.
pub fn parse_cfp_page(
    config: &CfpSourceConfig,
    document: &CfpDocument,
    checked_on: &str,
    is_discovery: bool,
) -> Result<CfpParsedPage, CfpSourceError> {
    if document.text.len() > CFP_MAX_PAGE_BYTES {
        return Err(CfpSourceError::TooLarge);
    }
    if !config
        .permits_url(&Url::parse(&document.final_url).map_err(|_| CfpSourceError::DisallowedUrl)?)
    {
        return Err(CfpSourceError::DisallowedUrl);
    }
    let html = Html::parse_document(&document.text);
    let text = if document.format == "html" {
        visible_html(&html)
    } else {
        document.text.clone()
    };
    if is_cfp_challenge(document) {
        return Err(CfpSourceError::Challenge);
    }
    if is_discovery && !has_journal_identity(config, &html, &text) {
        return Err(CfpSourceError::Unrecognized);
    }
    if is_discovery
        && html
            .select(&Selector::parse("a[rel~=next][href]").expect("pagination selector"))
            .next()
            .is_some()
    {
        return Err(CfpSourceError::Unrecognized);
    }
    let mut sources = Vec::new();
    let mut detail_urls = BTreeSet::new();
    let mut detail_titles = BTreeMap::new();
    let call_blocks = match config.adapter {
        CfpAdapter::SnapshotOnly => return Err(CfpSourceError::Unsupported),
        CfpAdapter::SpringerCollections if is_discovery => blocks(
            text.split("\n## Journal navigation")
                .next()
                .unwrap_or(&text),
            2,
            3,
        ),
        CfpAdapter::ElsevierCalls | CfpAdapter::KeaiCalls if is_discovery => {
            let section = blocks(&text, 1, 2)
                .into_iter()
                .find(|(title, _)| title.eq_ignore_ascii_case("Call for papers"));
            section.map_or(Vec::new(), |(_, body)| blocks(&body, 3, 3))
        }
        _ => blocks(&text, 1, 1).into_iter().take(1).collect(),
    };
    let mut has_ambiguous_card = false;
    for (title, body) in call_blocks {
        if pattern(
            r"(?i)^(book reviews?|corrections?|editorials?|archive|meet the editors|latest articles|cookie|manage consent)",
            &title,
        ) || pattern(
            r"(?i)^(collections|open collections|collections and calls for papers|calls? for papers|special issue calls? for papers|submission (?:instructions|guidelines|status|deadline)|guest editors?|filter by|journal navigation)$",
            &title,
        ) || title == config.journal_title
        {
            continue;
        }
        if config.adapter == CfpAdapter::SpringerCollections
            && is_discovery
            && !STATUS.is_match(&body)
        {
            has_ambiguous_card = true;
            continue;
        }
        if config.adapter == CfpAdapter::SpringerCollections
            && is_discovery
            && blocks(&body, 2, 3)
                .iter()
                .any(|(_, body)| STATUS.is_match(body))
        {
            continue;
        }
        let link = html
            .select(&Selector::parse("a[href]").expect("link selector"))
            .find(|element| clean_cfp_text(&element.text().collect::<String>()) == title)
            .and_then(|element| element.value().attr("href"))
            .and_then(|href| Url::parse(&document.final_url).ok()?.join(href).ok());
        if body.trim().is_empty() && is_discovery {
            if let Some(link) = link.clone().filter(|link| config.permits_url(link)) {
                detail_titles.insert(link.to_string(), title.clone());
                detail_urls.insert(link.to_string());
                continue;
            }
        }
        if title.trim().is_empty() || body.trim().is_empty() {
            has_ambiguous_card = true;
            continue;
        }
        if !is_discovery
            && !pattern(
                r"(?i)call for|submission|deadline|special issue|截[稿止]|征稿",
                &format!("{title}\n{body}"),
            )
        {
            return Err(CfpSourceError::Unrecognized);
        }
        let status = STATUS
            .find(&plain(&body))
            .map(|found| found.as_str().to_owned());
        let mut source = record(config, document, checked_on, &title, &body, status)?;
        if !is_discovery
            && source.date_text.is_empty()
            && source.raw_date_text.is_empty()
            && pattern(
                r"(?i)(?:download|view).{0,40}(?:call for papers|pdf)",
                &body,
            )
            && html
                .select(&Selector::parse("a[href]").expect("PDF link selector"))
                .any(|link| {
                    link.value().attr("href").is_some_and(|href| {
                        href.split('?')
                            .next()
                            .unwrap_or(href)
                            .to_ascii_lowercase()
                            .ends_with(".pdf")
                    })
                })
        {
            return Err(CfpSourceError::Unrecognized);
        }
        if is_discovery
            && config.adapter != CfpAdapter::SpringerCollections
            && source.date_text.is_empty()
            && source.raw_date_text.is_empty()
            && source.scope.is_empty()
        {
            if let Some(link) = link.clone().filter(|link| config.permits_url(link)) {
                detail_titles.insert(link.to_string(), title.clone());
                detail_urls.insert(link.to_string());
                continue;
            }
            has_ambiguous_card = true;
            continue;
        }
        if let Some(link) = link.filter(|link| {
            config.permits_url(link)
                && link.path()
                    != Url::parse(&document.final_url)
                        .expect("validated URL")
                        .path()
        }) {
            source.source_url = link.to_string();
        }
        sources.push(source);
    }
    if has_ambiguous_card {
        return Err(CfpSourceError::Unrecognized);
    }
    let mut empty_journals = Vec::new();
    if sources.is_empty() && detail_urls.is_empty() {
        if let Some(statement) = config
            .empty_statements
            .iter()
            .find(|statement| text.contains(statement.as_str()))
        {
            empty_journals.push(CfpEmptyJournal {
                catalog_ids: config.catalog_ids.clone(),
                journal_title: config.journal_title.clone(),
                checked_on: checked_on.into(),
                source_url: document.final_url.clone(),
                source_statement: statement.clone(),
                notices: Vec::new(),
            });
        } else {
            return Err(CfpSourceError::Unrecognized);
        }
    }
    Ok(CfpParsedPage {
        sources,
        empty_journals,
        detail_urls: detail_urls.into_iter().collect(),
        detail_titles,
    })
}
