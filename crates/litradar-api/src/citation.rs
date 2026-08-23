//! Format-specific serializers for favorite citation exports.

use litradar_storage::business::FavoriteCitationRecord;

/// Citation output exceeded its caller-supplied UTF-8 byte limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct CitationOutputLimitExceeded;

/// Serialize favorite articles as structurally safe BibTeX records.
///
/// # Arguments
///
/// * `articles` - Favorite citation records in export order.
/// * `maximum_bytes` - Inclusive final UTF-8 byte limit.
///
/// # Returns
///
/// BibTeX text with one `article` entry per input record, or a size-limit failure.
pub(crate) fn serialize_bibtex(
    articles: &[FavoriteCitationRecord],
    maximum_bytes: usize,
) -> Result<String, CitationOutputLimitExceeded> {
    let mut serializer = BibtexSerializer::new(maximum_bytes);
    for (index, article) in articles.iter().enumerate() {
        serializer.start_entry(article.doi.as_deref().unwrap_or(""), index + 1)?;
        serializer.field("title", article.title.as_deref().unwrap_or(""), true)?;
        serializer.authors(&article.authors)?;
        serializer.field(
            "journal",
            article.journal_title.as_deref().unwrap_or(""),
            true,
        )?;
        serializer.field("year", article.date.as_deref().unwrap_or(""), true)?;
        serializer.field("doi", article.doi.as_deref().unwrap_or(""), false)?;
        serializer.finish_entry()?;
    }
    Ok(serializer.finish())
}

/// Serialize favorite articles as structurally safe RIS records.
///
/// # Arguments
///
/// * `articles` - Favorite citation records in export order.
/// * `maximum_bytes` - Inclusive final UTF-8 byte limit.
///
/// # Returns
///
/// RIS text with one `TY`/`ER` record per input article, or a size-limit failure.
pub(crate) fn serialize_ris(
    articles: &[FavoriteCitationRecord],
    maximum_bytes: usize,
) -> Result<String, CitationOutputLimitExceeded> {
    let mut serializer = RisSerializer::new(maximum_bytes);
    for article in articles {
        serializer.start_record()?;
        serializer.field("TI", article.title.as_deref().unwrap_or(""))?;
        serializer.authors(&article.authors)?;
        serializer.field("JO", article.journal_title.as_deref().unwrap_or(""))?;
        serializer.field("PY", article.date.as_deref().unwrap_or(""))?;
        serializer.field("DO", article.doi.as_deref().unwrap_or(""))?;
        serializer.finish_record()?;
    }
    Ok(serializer.finish())
}

/// Serialize favorite articles as structurally safe EndNote XML records.
///
/// # Arguments
///
/// * `articles` - Favorite citation records in export order.
/// * `maximum_bytes` - Inclusive final UTF-8 byte limit.
///
/// # Returns
///
/// UTF-8 EndNote XML text with escaped XML 1.0 text, or a size-limit failure.
pub(crate) fn serialize_endnote_xml(
    articles: &[FavoriteCitationRecord],
    maximum_bytes: usize,
) -> Result<String, CitationOutputLimitExceeded> {
    let mut serializer = EndnoteXmlSerializer::new(maximum_bytes)?;
    for article in articles {
        serializer.record(article)?;
    }
    serializer.finish()
}

struct BibtexSerializer {
    output: BoundedString,
    entry_count: usize,
}

impl BibtexSerializer {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            output: BoundedString::new(maximum_bytes),
            entry_count: 0,
        }
    }

    fn start_entry(
        &mut self,
        doi: &str,
        sequence: usize,
    ) -> Result<(), CitationOutputLimitExceeded> {
        if self.entry_count > 0 {
            self.output.push_str("\n\n")?;
        }
        self.output.push_str("@article{")?;
        let mut has_key_character = false;
        for character in doi
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
        {
            self.output.push(character)?;
            has_key_character = true;
        }
        if !has_key_character {
            self.output.push_str("favorite")?;
        }
        self.output.push_str(&sequence.to_string())?;
        self.output.push_str(",\n")?;
        self.entry_count += 1;
        Ok(())
    }

    fn authors(&mut self, authors: &[String]) -> Result<(), CitationOutputLimitExceeded> {
        self.output.push_str("  author = {")?;
        for (index, author) in authors.iter().enumerate() {
            if index > 0 {
                self.output.push_str(" and ")?;
            }
            push_bibtex_value(&mut self.output, author)?;
        }
        self.output.push_str("},\n")
    }

    fn field(
        &mut self,
        name: &str,
        value: &str,
        has_trailing_comma: bool,
    ) -> Result<(), CitationOutputLimitExceeded> {
        self.output.push_str("  ")?;
        self.output.push_str(name)?;
        self.output.push_str(" = {")?;
        push_bibtex_value(&mut self.output, value)?;
        self.output
            .push_str(if has_trailing_comma { "},\n" } else { "}\n" })
    }

    fn finish_entry(&mut self) -> Result<(), CitationOutputLimitExceeded> {
        self.output.push('}')
    }

    fn finish(self) -> String {
        self.output.finish()
    }
}

struct RisSerializer {
    output: BoundedString,
    record_count: usize,
}

impl RisSerializer {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            output: BoundedString::new(maximum_bytes),
            record_count: 0,
        }
    }

    fn start_record(&mut self) -> Result<(), CitationOutputLimitExceeded> {
        if self.record_count > 0 {
            self.output.push_str("\n\n")?;
        }
        self.field("TY", "JOUR")?;
        self.record_count += 1;
        Ok(())
    }

    fn authors(&mut self, authors: &[String]) -> Result<(), CitationOutputLimitExceeded> {
        if authors.is_empty() {
            return self.field("AU", "");
        }
        for author in authors {
            self.field("AU", author)?;
        }
        Ok(())
    }

    fn field(&mut self, tag: &str, value: &str) -> Result<(), CitationOutputLimitExceeded> {
        self.output.push_str(tag)?;
        self.output.push_str("  - ")?;
        for character in value.chars() {
            self.output.push(if is_structural_line_break(character) {
                ' '
            } else {
                character
            })?;
        }
        self.output.push('\n')
    }

    fn finish_record(&mut self) -> Result<(), CitationOutputLimitExceeded> {
        self.output.push_str("ER  -")
    }

    fn finish(self) -> String {
        self.output.finish()
    }
}

struct EndnoteXmlSerializer {
    output: BoundedString,
}

impl EndnoteXmlSerializer {
    fn new(maximum_bytes: usize) -> Result<Self, CitationOutputLimitExceeded> {
        let mut output = BoundedString::new(maximum_bytes);
        output.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?><xml><records>")?;
        Ok(Self { output })
    }

    fn record(
        &mut self,
        article: &FavoriteCitationRecord,
    ) -> Result<(), CitationOutputLimitExceeded> {
        self.output.push_str("<record><titles><title>")?;
        push_xml_text(&mut self.output, article.title.as_deref().unwrap_or(""))?;
        self.output
            .push_str("</title></titles><contributors><authors>")?;
        if article.authors.is_empty() {
            self.output.push_str("<author></author>")?;
        } else {
            for author in &article.authors {
                self.output.push_str("<author>")?;
                push_xml_text(&mut self.output, author)?;
                self.output.push_str("</author>")?;
            }
        }
        self.output
            .push_str("</authors></contributors><dates><year>")?;
        push_xml_text(&mut self.output, article.date.as_deref().unwrap_or(""))?;
        self.output
            .push_str("</year></dates><electronic-resource-num>")?;
        push_xml_text(&mut self.output, article.doi.as_deref().unwrap_or(""))?;
        self.output.push_str("</electronic-resource-num></record>")
    }

    fn finish(mut self) -> Result<String, CitationOutputLimitExceeded> {
        self.output.push_str("</records></xml>")?;
        Ok(self.output.finish())
    }
}

fn push_bibtex_value(
    output: &mut BoundedString,
    value: &str,
) -> Result<(), CitationOutputLimitExceeded> {
    for character in value.chars() {
        match character {
            '{' => output.push_str("\\char123{}")?,
            '}' => output.push_str("\\char125{}")?,
            '\\' => output.push_str("\\char92{}")?,
            '%' => output.push_str("\\%")?,
            '#' => output.push_str("\\#")?,
            '_' => output.push_str("\\_")?,
            '&' => output.push_str("\\&")?,
            '$' => output.push_str("\\$")?,
            '~' => output.push_str("\\char126{}")?,
            '^' => output.push_str("\\char94{}")?,
            character if is_structural_line_break(character) => output.push(' ')?,
            character => output.push(character)?,
        }
    }
    Ok(())
}

fn is_structural_line_break(character: char) -> bool {
    character.is_control() || character.is_whitespace() && character != ' '
}

fn push_xml_text(
    output: &mut BoundedString,
    value: &str,
) -> Result<(), CitationOutputLimitExceeded> {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;")?,
            '<' => output.push_str("&lt;")?,
            '>' => output.push_str("&gt;")?,
            '"' => output.push_str("&quot;")?,
            '\'' => output.push_str("&apos;")?,
            character if is_xml_1_0_character(character) => output.push(character)?,
            _ => output.push(' ')?,
        }
    }
    Ok(())
}

struct BoundedString {
    output: String,
    maximum_bytes: usize,
}

impl BoundedString {
    fn new(maximum_bytes: usize) -> Self {
        Self {
            output: String::new(),
            maximum_bytes,
        }
    }

    fn push_str(&mut self, value: &str) -> Result<(), CitationOutputLimitExceeded> {
        if value.len() > self.maximum_bytes.saturating_sub(self.output.len()) {
            return Err(CitationOutputLimitExceeded);
        }
        self.output.push_str(value);
        Ok(())
    }

    fn push(&mut self, character: char) -> Result<(), CitationOutputLimitExceeded> {
        let mut buffer = [0_u8; 4];
        self.push_str(character.encode_utf8(&mut buffer))
    }

    fn finish(self) -> String {
        self.output
    }
}

fn is_xml_1_0_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}

#[cfg(test)]
mod tests {
    use litradar_domain::ArticleId;
    use litradar_storage::business::FavoriteCitationRecord;

    use super::{
        serialize_bibtex as serialize_bibtex_bounded,
        serialize_endnote_xml as serialize_endnote_xml_bounded,
        serialize_ris as serialize_ris_bounded, CitationOutputLimitExceeded,
    };

    #[test]
    fn bibtex_normal_records_keep_existing_fields_and_semantics() {
        let article = favorite_article(
            1,
            Some("A & B <Genome>"),
            &["Alice", "Bob"],
            Some("Journal One"),
            Some("2026-01-05"),
            Some("10.1000/abc-def"),
        );

        assert_eq!(
            serialize_bibtex(&[article]),
            "@article{101000abcdef1,\n  title = {A \\& B <Genome>},\n  author = {Alice and Bob},\n  journal = {Journal One},\n  year = {2026-01-05},\n  doi = {10.1000/abc-def}\n}"
        );
        assert!(
            serialize_bibtex(&[favorite_article(2, None, &[], None, None, None)])
                .starts_with("@article{favorite1,\n  title = {},\n  author = {}")
        );
    }

    #[test]
    fn bibtex_reserved_characters_and_newlines_stay_inside_one_balanced_record() {
        let article = favorite_article(
            1,
            Some("Title } { \\ % # _ & $ ~ ^\r\n@article{injected,"),
            &["Author\n},\n@book{injected"],
            Some("Journal {unsafe}"),
            Some("2026\n}"),
            Some("{}"),
        );

        let output = serialize_bibtex(&[article]);

        assert_eq!(
            output
                .lines()
                .filter(|line| line.starts_with("@article{"))
                .count(),
            1
        );
        assert_eq!(output.lines().count(), 7);
        assert!(output.contains("\\char125{} \\char123{} \\char92{} \\% \\# \\_ \\& \\$"));
        assert!(output.contains("\\char126{} \\char94{}"));
        assert_balanced_braces(&output);
    }

    #[test]
    fn ris_normal_records_keep_existing_tags_and_author_lines() {
        let article = favorite_article(
            1,
            Some("Clinical Data"),
            &["Carol", "Dan"],
            Some("Alpha Journal"),
            Some("2026"),
            Some("10.1000/clinical"),
        );

        assert_eq!(
            serialize_ris(&[article]),
            "TY  - JOUR\nTI  - Clinical Data\nAU  - Carol\nAU  - Dan\nJO  - Alpha Journal\nPY  - 2026\nDO  - 10.1000/clinical\nER  -"
        );
    }

    #[test]
    fn ris_line_breaks_cannot_inject_tags_or_records() {
        let article = favorite_article(
            1,
            Some("Title\r\nER  -\nTY  - BOOK"),
            &["Author\u{2028}ER  -"],
            Some("Journal\u{85}AU  - attacker"),
            Some("2026\rPY  - 1900"),
            Some("10.1000/test\nER  -"),
        );

        let output = serialize_ris(&[article]);
        let lines = output.lines().collect::<Vec<_>>();

        assert_eq!(
            lines.iter().filter(|line| **line == "TY  - JOUR").count(),
            1
        );
        assert_eq!(lines.iter().filter(|line| **line == "ER  -").count(), 1);
        assert_eq!(lines.len(), 7);
        assert!(!output.contains('\r'));
        assert!(!output.contains('\u{2028}'));
        assert!(lines.iter().all(|line| {
            ["TY", "TI", "AU", "JO", "PY", "DO", "ER"]
                .iter()
                .any(|tag| line.starts_with(&format!("{tag}  -")))
        }));
    }

    #[test]
    fn endnote_xml_escapes_markup_and_replaces_forbidden_xml_characters() {
        let article = favorite_article(
            1,
            Some("</title><record>A&B\0"),
            &["Alice O'Neil</author>"],
            Some("Unused Journal"),
            Some("2026<1900"),
            Some("10.1000/a&b"),
        );

        let output = serialize_endnote_xml(&[article]);

        assert!(output.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert_eq!(output.matches("<record>").count(), 1);
        assert_eq!(output.matches("</record>").count(), 1);
        assert!(output.contains("&lt;/title&gt;&lt;record&gt;A&amp;B "));
        assert!(output.contains("Alice O&apos;Neil&lt;/author&gt;"));
        assert!(output.contains("2026&lt;1900"));
        assert!(output.contains("10.1000/a&amp;b"));
        assert!(!output.contains('\0'));
        assert!(output.ends_with("</records></xml>"));
    }

    #[test]
    fn citation_output_accepts_exact_byte_limit_and_rejects_the_next_byte() {
        const MAXIMUM_BYTES: usize = 8 * 1024 * 1024;

        let empty = serialize_bibtex(&[favorite_article(1, Some(""), &[], None, None, None)]);
        let title = "x".repeat(MAXIMUM_BYTES - empty.len());
        let exact_record = favorite_article(1, Some(&title), &[], None, None, None);

        let exact = serialize_bibtex_bounded(std::slice::from_ref(&exact_record), MAXIMUM_BYTES)
            .expect("exact byte boundary should serialize");

        assert_eq!(exact.len(), MAXIMUM_BYTES);
        assert_eq!(
            serialize_bibtex_bounded(&[exact_record], MAXIMUM_BYTES - 1),
            Err(CitationOutputLimitExceeded)
        );

        let unicode_record = favorite_article(2, Some("研究"), &[], None, None, None);
        let unicode = serialize_bibtex(std::slice::from_ref(&unicode_record));
        assert_eq!(
            serialize_bibtex_bounded(std::slice::from_ref(&unicode_record), unicode.len())
                .expect("exact Unicode byte boundary should serialize"),
            unicode
        );
        assert_eq!(
            serialize_bibtex_bounded(&[unicode_record], unicode.len() - 1),
            Err(CitationOutputLimitExceeded)
        );
    }

    fn serialize_bibtex(articles: &[FavoriteCitationRecord]) -> String {
        serialize_bibtex_bounded(articles, usize::MAX)
            .expect("unbounded BibTeX test serialization should succeed")
    }

    fn serialize_ris(articles: &[FavoriteCitationRecord]) -> String {
        serialize_ris_bounded(articles, usize::MAX)
            .expect("unbounded RIS test serialization should succeed")
    }

    fn serialize_endnote_xml(articles: &[FavoriteCitationRecord]) -> String {
        serialize_endnote_xml_bounded(articles, usize::MAX)
            .expect("unbounded EndNote test serialization should succeed")
    }

    fn favorite_article(
        id: i64,
        title: Option<&str>,
        authors: &[&str],
        journal_title: Option<&str>,
        date: Option<&str>,
        doi: Option<&str>,
    ) -> FavoriteCitationRecord {
        FavoriteCitationRecord {
            article_id: ArticleId(1_000 + id),
            db_name: "fixture.sqlite".to_string(),
            title: title.map(str::to_string),
            date: date.map(str::to_string),
            authors: authors.iter().map(|author| (*author).to_string()).collect(),
            doi: doi.map(str::to_string),
            journal_title: journal_title.map(str::to_string),
        }
    }

    fn assert_balanced_braces(value: &str) {
        let mut depth = 0_i64;
        for character in value.chars() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    assert!(depth >= 0, "BibTeX braces must never close below zero");
                }
                _ => {}
            }
        }
        assert_eq!(depth, 0, "BibTeX braces must balance");
    }
}
