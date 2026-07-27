//! Format-specific serializers for favorite citation exports.

use litradar_domain::FavoriteArticleResponse;

/// Serialize favorite articles as structurally safe BibTeX records.
///
/// # Arguments
///
/// * `articles` - Favorite article records in export order.
///
/// # Returns
///
/// BibTeX text with one `article` entry per input record.
pub(crate) fn serialize_bibtex(articles: &[FavoriteArticleResponse]) -> String {
    let mut serializer = BibtexSerializer::new();
    for (index, article) in articles.iter().enumerate() {
        let key = citation_key(article.doi.as_deref().unwrap_or(""), index + 1);
        serializer.start_entry(&key);
        serializer.field("title", article.title.as_deref().unwrap_or(""));
        serializer.authors(article.authors.as_deref().unwrap_or(&[]));
        serializer.field("journal", article.journal_title.as_deref().unwrap_or(""));
        serializer.field("year", article.date.as_deref().unwrap_or(""));
        serializer.field("doi", article.doi.as_deref().unwrap_or(""));
        serializer.finish_entry();
    }
    serializer.finish()
}

/// Serialize favorite articles as structurally safe RIS records.
///
/// # Arguments
///
/// * `articles` - Favorite article records in export order.
///
/// # Returns
///
/// RIS text with one `TY`/`ER` record per input article.
pub(crate) fn serialize_ris(articles: &[FavoriteArticleResponse]) -> String {
    let mut serializer = RisSerializer::new();
    for article in articles {
        serializer.start_record();
        serializer.field("TI", article.title.as_deref().unwrap_or(""));
        serializer.authors(article.authors.as_deref().unwrap_or(&[]));
        serializer.field("JO", article.journal_title.as_deref().unwrap_or(""));
        serializer.field("PY", article.date.as_deref().unwrap_or(""));
        serializer.field("DO", article.doi.as_deref().unwrap_or(""));
        serializer.finish_record();
    }
    serializer.finish()
}

/// Serialize favorite articles as structurally safe EndNote XML records.
///
/// # Arguments
///
/// * `articles` - Favorite article records in export order.
///
/// # Returns
///
/// UTF-8 EndNote XML text with escaped XML 1.0 character data.
pub(crate) fn serialize_endnote_xml(articles: &[FavoriteArticleResponse]) -> String {
    let mut serializer = EndnoteXmlSerializer::new();
    for article in articles {
        serializer.record(article);
    }
    serializer.finish()
}

struct BibtexSerializer {
    output: String,
    entry_count: usize,
}

impl BibtexSerializer {
    fn new() -> Self {
        Self {
            output: String::new(),
            entry_count: 0,
        }
    }

    fn start_entry(&mut self, key: &str) {
        if self.entry_count > 0 {
            self.output.push_str("\n\n");
        }
        self.output.push_str("@article{");
        self.output.push_str(key);
        self.output.push_str(",\n");
        self.entry_count += 1;
    }

    fn authors(&mut self, authors: &[String]) {
        let value = authors
            .iter()
            .map(|author| escape_bibtex_value(author))
            .collect::<Vec<_>>()
            .join(" and ");
        self.escaped_field("author", &value);
    }

    fn field(&mut self, name: &str, value: &str) {
        self.escaped_field(name, &escape_bibtex_value(value));
    }

    fn escaped_field(&mut self, name: &str, value: &str) {
        self.output.push_str("  ");
        self.output.push_str(name);
        self.output.push_str(" = {");
        self.output.push_str(value);
        self.output.push_str("},\n");
    }

    fn finish_entry(&mut self) {
        let trailing_separator = self
            .output
            .rfind(",\n")
            .expect("a BibTeX entry always contains fields");
        self.output
            .replace_range(trailing_separator..trailing_separator + 1, "");
        self.output.push('}');
    }

    fn finish(self) -> String {
        self.output
    }
}

struct RisSerializer {
    output: String,
    record_count: usize,
}

impl RisSerializer {
    fn new() -> Self {
        Self {
            output: String::new(),
            record_count: 0,
        }
    }

    fn start_record(&mut self) {
        if self.record_count > 0 {
            self.output.push_str("\n\n");
        }
        self.field("TY", "JOUR");
        self.record_count += 1;
    }

    fn authors(&mut self, authors: &[String]) {
        if authors.is_empty() {
            self.field("AU", "");
            return;
        }
        for author in authors {
            self.field("AU", author);
        }
    }

    fn field(&mut self, tag: &str, value: &str) {
        self.output.push_str(tag);
        self.output.push_str("  - ");
        self.output.push_str(&normalize_line_value(value));
        self.output.push('\n');
    }

    fn finish_record(&mut self) {
        self.output.push_str("ER  -");
    }

    fn finish(self) -> String {
        self.output
    }
}

struct EndnoteXmlSerializer {
    output: String,
}

impl EndnoteXmlSerializer {
    fn new() -> Self {
        Self {
            output: String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?><xml><records>"),
        }
    }

    fn record(&mut self, article: &FavoriteArticleResponse) {
        self.output.push_str("<record><titles><title>");
        push_xml_text(&mut self.output, article.title.as_deref().unwrap_or(""));
        self.output
            .push_str("</title></titles><contributors><authors>");
        let authors = article.authors.as_deref().unwrap_or(&[]);
        if authors.is_empty() {
            self.output.push_str("<author></author>");
        } else {
            for author in authors {
                self.output.push_str("<author>");
                push_xml_text(&mut self.output, author);
                self.output.push_str("</author>");
            }
        }
        self.output
            .push_str("</authors></contributors><dates><year>");
        push_xml_text(&mut self.output, article.date.as_deref().unwrap_or(""));
        self.output
            .push_str("</year></dates><electronic-resource-num>");
        push_xml_text(&mut self.output, article.doi.as_deref().unwrap_or(""));
        self.output.push_str("</electronic-resource-num></record>");
    }

    fn finish(mut self) -> String {
        self.output.push_str("</records></xml>");
        self.output
    }
}

fn citation_key(value: &str, sequence: usize) -> String {
    let sanitized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .collect::<String>();
    let base = if sanitized.is_empty() {
        "favorite"
    } else {
        sanitized.as_str()
    };
    format!("{base}{sequence}")
}

fn escape_bibtex_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '{' => escaped.push_str("\\char123{}"),
            '}' => escaped.push_str("\\char125{}"),
            '\\' => escaped.push_str("\\char92{}"),
            '%' => escaped.push_str("\\%"),
            '#' => escaped.push_str("\\#"),
            '_' => escaped.push_str("\\_"),
            '&' => escaped.push_str("\\&"),
            '$' => escaped.push_str("\\$"),
            '~' => escaped.push_str("\\char126{}"),
            '^' => escaped.push_str("\\char94{}"),
            character if is_structural_line_break(character) => escaped.push(' '),
            character => escaped.push(character),
        }
    }
    escaped
}

fn normalize_line_value(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if is_structural_line_break(character) {
                ' '
            } else {
                character
            }
        })
        .collect()
}

fn is_structural_line_break(character: char) -> bool {
    character.is_control() || character.is_whitespace() && character != ' '
}

fn push_xml_text(output: &mut String, value: &str) {
    for character in value.chars() {
        match character {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&apos;"),
            character if is_xml_1_0_character(character) => output.push(character),
            _ => output.push(' '),
        }
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
    use litradar_domain::{ArticleId, FavoriteArticleResponse, JournalId};

    use super::{serialize_bibtex, serialize_endnote_xml, serialize_ris};

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

    fn favorite_article(
        id: i64,
        title: Option<&str>,
        authors: &[&str],
        journal_title: Option<&str>,
        date: Option<&str>,
        doi: Option<&str>,
    ) -> FavoriteArticleResponse {
        FavoriteArticleResponse {
            id,
            folder_id: 10,
            article_id: ArticleId(1_000 + id),
            db_name: "fixture.sqlite".to_string(),
            note: String::new(),
            created_at: 1.0,
            journal_id: Some(JournalId(1)),
            issue_id: Some(20),
            title: title.map(str::to_string),
            publication_year: Some(2026),
            date: date.map(str::to_string),
            authors: Some(authors.iter().map(|author| (*author).to_string()).collect()),
            abstract_text: None,
            doi: doi.map(str::to_string),
            journal_title: journal_title.map(str::to_string),
            open_access: None,
            in_press: None,
            volume: None,
            number: None,
            issn: None,
            eissn: None,
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
