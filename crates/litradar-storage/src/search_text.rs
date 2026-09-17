//! Search-only text preparation for the non-pinyin simple FTS tokenizer.

use std::borrow::Cow;

use litradar_domain::ArticleSearchMode;
use rusqlite::{params, Connection, OptionalExtension};
use unicode_normalization::{char::is_combining_mark, UnicodeNormalization};

/// Check the actual search declaration without opening its virtual table.
///
/// Auth/control schema versions do not identify content databases, and old content schemas
/// must remain independent of native tokenizer assets.
pub fn uses_simple_search(connection: &Connection) -> rusqlite::Result<bool> {
    let declaration = connection
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name='article_search'",
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(declaration.is_some_and(|sql| {
        sql.chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase()
            .contains("tokenize='simple0'")
    }))
}

/// Preserve canonical text while preparing v9 search projections and query operands.
///
/// Latin accents and non-ASCII case are folded for simple, and punctuation keeps its
/// word-separator behavior. ASCII letters retain their spelling; advanced query syntax
/// is handled separately. Old unicode61 databases receive their input unchanged.
pub fn prepare_search_text(value: &str, uses_simple: bool) -> Cow<'_, str> {
    if !uses_simple
        || value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte.is_ascii_whitespace())
    {
        return Cow::Borrowed(value);
    }
    let mut normalized = String::with_capacity(value.len());
    for character in value
        .nfd()
        .filter(|character| !is_combining_mark(*character))
    {
        if !character.is_alphanumeric()
            && !matches!(character, '\u{e000}'..='\u{f8ff}' | '\u{f0000}'..='\u{ffffd}' | '\u{100000}'..='\u{10fffd}')
        {
            normalized.push(' ');
        } else if character.is_ascii() {
            normalized.push(character);
        } else {
            normalized.extend(character.to_lowercase());
        }
    }
    Cow::Owned(normalized)
}

/// Normalize FTS operands without changing operators, columns, or the presence of a filter.
///
/// Quoted strings and barewords are normalized separately in advanced mode. A changed
/// bareword is quoted so an accented literal cannot turn into a Boolean operator.
pub fn prepare_search_query(
    value: &str,
    uses_simple: bool,
    mode: ArticleSearchMode,
) -> Cow<'_, str> {
    if !uses_simple {
        return Cow::Borrowed(value);
    }
    if mode == ArticleSearchMode::Simple {
        let normalized = prepare_search_text(value, true);
        return if normalized.trim().is_empty() && !value.trim().is_empty() {
            Cow::Borrowed(value)
        } else {
            normalized
        };
    }
    let mut characters = value.chars().peekable();
    let mut output = String::with_capacity(value.len());
    let mut is_column_set = false;
    while let Some(character) = characters.next() {
        if character == '"' {
            let mut original = String::from("\"");
            let mut operand = String::new();
            let mut is_closed = false;
            while let Some(next) = characters.next() {
                original.push(next);
                if next == '"' {
                    if characters.peek() == Some(&'"') {
                        original.push(characters.next().unwrap());
                        operand.push('"');
                    } else {
                        is_closed = true;
                        break;
                    }
                } else {
                    operand.push(next);
                }
            }
            if !is_closed {
                return Cow::Borrowed(value);
            }
            let is_column = is_column_set
                || characters
                    .clone()
                    .find(|character| !character.is_ascii_whitespace())
                    == Some(':');
            if is_column {
                output.push_str(&original);
            } else {
                output.push('"');
                output.push_str(&prepare_search_text(&operand, true).replace('"', "\"\""));
                output.push('"');
            }
        } else if character.is_ascii_alphanumeric()
            || character == '_'
            || character == '\u{1a}'
            || !character.is_ascii()
        {
            let mut operand = String::from(character);
            while characters.peek().is_some_and(|next| {
                next.is_ascii_alphanumeric()
                    || *next == '_'
                    || *next == '\u{1a}'
                    || !next.is_ascii()
            }) {
                operand.push(characters.next().unwrap());
            }
            let is_column = is_column_set
                || characters
                    .clone()
                    .find(|character| !character.is_ascii_whitespace())
                    == Some(':');
            let normalized = prepare_search_text(&operand, true);
            if is_column || normalized == operand {
                output.push_str(&operand);
            } else {
                output.push('"');
                output.push_str(&normalized.replace('"', "\"\""));
                output.push('"');
            }
        } else {
            if character == '{' {
                is_column_set = true;
            }
            if character == '}' {
                is_column_set = false;
            }
            output.push(character);
        }
    }
    Cow::Owned(output)
}

/// Stream canonical records into the search projection inside the caller's transaction.
///
/// This does not change article text or identifiers and never collects the full corpus
/// in memory. The caller owns offline/transactional write exclusion and recovery.
pub fn rebuild_article_search(connection: &Connection) -> rusqlite::Result<()> {
    crate::sqlite::load_index_tokenizer(connection)?;
    let uses_simple = uses_simple_search(connection)?;
    connection.execute("DELETE FROM article_search", [])?;
    let mut statement = connection.prepare(
        "SELECT a.article_id,a.title,a.abstract_text,a.doi,a.pmid,a.authors_json,j.title
         FROM articles a JOIN journals j ON j.journal_id=a.journal_id ORDER BY a.article_id",
    )?;
    let mut rows = statement.query([])?;
    let mut insert = connection.prepare(
        "INSERT INTO article_search(rowid,article_id,title,abstract_text,doi,pmid,authors,journal_title)
         VALUES(?1,?1,?2,?3,?4,?5,?6,?7)",
    )?;
    while let Some(row) = rows.next()? {
        let authors_json: String = row.get(5)?;
        let authors = crate::article_authors::decode_article_author_names(&authors_json)
            .map_err(|_| rusqlite::Error::InvalidQuery)?
            .join("; ");
        insert.execute(params![
            row.get::<_, i64>(0)?,
            prepare_search_text(row.get_ref(1)?.as_str()?, uses_simple),
            prepare_search_text(
                row.get::<_, Option<String>>(2)?
                    .as_deref()
                    .unwrap_or_default(),
                uses_simple
            ),
            prepare_search_text(
                row.get::<_, Option<String>>(3)?
                    .as_deref()
                    .unwrap_or_default(),
                uses_simple
            ),
            prepare_search_text(
                row.get::<_, Option<String>>(4)?
                    .as_deref()
                    .unwrap_or_default(),
                uses_simple
            ),
            prepare_search_text(&authors, uses_simple),
            prepare_search_text(row.get_ref(6)?.as_str()?, uses_simple),
        ])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{prepare_search_query, prepare_search_text};
    use litradar_domain::ArticleSearchMode;

    #[test]
    fn normalization_preserves_operators_and_original_text() {
        let query = "Café AND résumé OR ΣΙΓΜΑ NOT 科技金融";
        assert_eq!(
            prepare_search_text(query, true),
            "Cafe AND resume OR σιγμα NOT 科技金融"
        );
        assert_eq!(prepare_search_text(query, false), query);
        assert!(matches!(
            prepare_search_text("genome NOT preview", true),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn query_normalization_preserves_fts_grammar_and_empty_operands() {
        let advanced = ArticleSearchMode::Advanced;
        for (query, expected) in [
            ("gene ÓR cancer", "gene \"OR\" cancer"),
            ("ÀND", "\"AND\""),
            ("title:Café* NOT preview", "title:\"Cafe\"* NOT preview"),
            ("títle:genome", "títle:genome"),
            (
                "{títle abstract_text}:Café",
                "{títle abstract_text}:\"Cafe\"",
            ),
            ("\"títle\":genome", "\"títle\":genome"),
            ("\"Genome-sequencing\"", "\"Genome sequencing\""),
            ("\u{301}", "\"\""),
        ] {
            assert_eq!(prepare_search_query(query, true, advanced), expected);
        }
        assert_eq!(
            prepare_search_query("\u{301}", true, ArticleSearchMode::Simple),
            "\u{301}"
        );
        assert_eq!(
            prepare_search_text("Café, genome-sequencing", true),
            "Cafe  genome sequencing"
        );
    }
}
