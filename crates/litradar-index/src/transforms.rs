//! Canonical maintained-catalog parsing and validation.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::Path;

use litradar_domain::{
    normalize_contract_issn, normalize_contract_text, JournalCatalogEntry, JournalRankings,
};
use litradar_provider::conformance::validate_catalog_entry;

/// Normalized maintained-catalog CSV row.
pub type CsvRow = BTreeMap<String, String>;

/// Exact ordered column contract for maintained catalog CSV version 3.
pub const CATALOG_CSV_V3_COLUMNS: [&str; 16] = [
    "catalog_id",
    "catalog_aliases",
    "title",
    "issn",
    "eissn",
    "all_issns",
    "title_aliases",
    "area",
    "utd_rank",
    "utd_rating",
    "abs_rank",
    "abs_rating",
    "fms_rank",
    "fms_rating",
    "fmscn_rank",
    "fmscn_rating",
];

/// Canonical catalog parsing or validation failure.
#[derive(Debug)]
pub enum CatalogContractError {
    /// Reading the catalog file failed.
    Io(std::io::Error),
    /// The CSV shape or one canonical entry is invalid.
    Invalid(String),
}

impl CatalogContractError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
}

impl fmt::Display for CatalogContractError {
    /// Format the catalog validation diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl Error for CatalogContractError {
    /// Return the filesystem failure when present.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<std::io::Error> for CatalogContractError {
    /// Convert a catalog file read failure.
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Read and validate one canonical maintained-catalog CSV file.
///
/// # Arguments
///
/// * `path` - Version 3 catalog path.
///
/// # Returns
///
/// Canonical entries in source order.
pub fn read_catalog_csv(
    path: impl AsRef<Path>,
) -> Result<Vec<JournalCatalogEntry>, CatalogContractError> {
    parse_catalog_csv(&std::fs::read_to_string(path)?)
}

/// Parse and validate canonical maintained-catalog CSV text.
///
/// # Arguments
///
/// * `text` - Version 3 catalog CSV text.
///
/// # Returns
///
/// Canonical entries in source order.
pub fn parse_catalog_csv(text: &str) -> Result<Vec<JournalCatalogEntry>, CatalogContractError> {
    let mut lines = text.lines().filter(|line| !line.trim().is_empty());
    let header_line = lines
        .next()
        .ok_or_else(|| CatalogContractError::invalid("canonical catalog is empty"))?;
    let headers = parse_csv_line(header_line)?;
    let expected = CATALOG_CSV_V3_COLUMNS
        .iter()
        .map(|value| (*value).to_string())
        .collect::<Vec<_>>();
    if headers != expected {
        return Err(CatalogContractError::invalid(format!(
            "catalog must use exact v3 header {}; found {}",
            CATALOG_CSV_V3_COLUMNS.join(","),
            headers.join(",")
        )));
    }

    let mut rows = Vec::new();
    for (index, line) in lines.enumerate() {
        let values = parse_csv_line(line).map_err(|error| {
            CatalogContractError::invalid(format!("catalog row {}: {error}", index + 2))
        })?;
        if values.len() != headers.len() {
            return Err(CatalogContractError::invalid(format!(
                "catalog row {} has {} columns; expected {}",
                index + 2,
                values.len(),
                headers.len()
            )));
        }
        rows.push(headers.iter().cloned().zip(values).collect());
    }
    build_catalog_entries(&rows)
}

/// Build one canonical journal catalog entry from a version 3 CSV row.
///
/// # Arguments
///
/// * `csv_row` - CSV row keyed by its exact version 3 headers.
///
/// # Returns
///
/// Normalized canonical catalog entry or a validation failure.
pub fn build_catalog_entry(csv_row: &CsvRow) -> Result<JournalCatalogEntry, CatalogContractError> {
    validate_catalog_columns(csv_row)?;
    let catalog_id = required_catalog_id(csv_row)?;
    let catalog_aliases = catalog_id_list(csv_row, "catalog_aliases")?;
    let title = required_catalog_text(csv_row, "title")?;
    let issn = optional_catalog_issn(csv_row, "issn")?;
    let eissn = optional_catalog_issn(csv_row, "eissn")?;
    let all_issns = catalog_issn_list(csv_row, "all_issns")?;
    let title_aliases = catalog_text_list(csv_row, "title_aliases")?;
    let entry = JournalCatalogEntry {
        catalog_id,
        catalog_aliases,
        title,
        issn,
        eissn,
        all_issns,
        title_aliases,
        area: optional_catalog_text(csv_row, "area"),
        rankings: JournalRankings {
            utd_rank: optional_catalog_text(csv_row, "utd_rank"),
            utd_rating: optional_catalog_text(csv_row, "utd_rating"),
            abs_rank: optional_catalog_text(csv_row, "abs_rank"),
            abs_rating: optional_catalog_text(csv_row, "abs_rating"),
            fms_rank: optional_catalog_text(csv_row, "fms_rank"),
            fms_rating: optional_catalog_text(csv_row, "fms_rating"),
            fmscn_rank: optional_catalog_text(csv_row, "fmscn_rank"),
            fmscn_rating: optional_catalog_text(csv_row, "fmscn_rating"),
        },
    };
    validate_catalog_entry(&entry)
        .map_err(|error| CatalogContractError::invalid(error.to_string()))?;
    Ok(entry)
}

/// Validate and normalize all rows in one maintained catalog.
///
/// # Arguments
///
/// * `rows` - Version 3 CSV rows.
///
/// # Returns
///
/// Canonical entries with unique catalog identities and ISSN ownership.
pub fn build_catalog_entries(
    rows: &[CsvRow],
) -> Result<Vec<JournalCatalogEntry>, CatalogContractError> {
    if rows.is_empty() {
        return Err(CatalogContractError::invalid(
            "canonical catalog must contain at least one journal",
        ));
    }
    let mut catalog_owners = BTreeMap::new();
    let mut issn_owners = BTreeMap::new();
    let mut entries = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 2;
        let entry = build_catalog_entry(row).map_err(|error| {
            CatalogContractError::invalid(format!("catalog row {row_number}: {error}"))
        })?;
        for identity in std::iter::once(&entry.catalog_id).chain(&entry.catalog_aliases) {
            if let Some((owner_row, owner_catalog_id)) = catalog_owners.get(identity) {
                return Err(CatalogContractError::invalid(format!(
                    "catalog row {row_number} catalog identity {identity} is already owned by row {owner_row} ({owner_catalog_id})"
                )));
            }
            catalog_owners.insert(identity.clone(), (row_number, entry.catalog_id.clone()));
        }
        for issn in &entry.all_issns {
            if let Some((owner_row, owner_catalog_id)) = issn_owners.get(issn) {
                return Err(CatalogContractError::invalid(format!(
                    "catalog row {row_number} ISSN {issn} is already owned by row {owner_row} ({owner_catalog_id})"
                )));
            }
            issn_owners.insert(issn.clone(), (row_number, entry.catalog_id.clone()));
        }
        entries.push(entry);
    }
    Ok(entries)
}

fn parse_csv_line(line: &str) -> Result<Vec<String>, CatalogContractError> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut characters = line.trim_end_matches('\r').chars().peekable();
    let mut inside_quotes = false;
    while let Some(character) = characters.next() {
        match character {
            '"' if inside_quotes && characters.peek() == Some(&'"') => {
                current.push('"');
                characters.next();
            }
            '"' => inside_quotes = !inside_quotes,
            ',' if !inside_quotes => {
                values.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(character),
        }
    }
    if inside_quotes {
        return Err(CatalogContractError::invalid(
            "catalog CSV row has an unterminated quoted field",
        ));
    }
    values.push(current.trim().to_string());
    Ok(values)
}

fn validate_catalog_columns(csv_row: &CsvRow) -> Result<(), CatalogContractError> {
    let expected = CATALOG_CSV_V3_COLUMNS
        .iter()
        .map(|column| (*column).to_string())
        .collect::<BTreeSet<_>>();
    let actual = csv_row.keys().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        let missing = expected.difference(&actual).cloned().collect::<Vec<_>>();
        let unexpected = actual.difference(&expected).cloned().collect::<Vec<_>>();
        return Err(CatalogContractError::invalid(format!(
            "catalog row must use exact v3 columns; missing={missing:?}, unexpected={unexpected:?}"
        )));
    }
    Ok(())
}

fn required_catalog_id(csv_row: &CsvRow) -> Result<String, CatalogContractError> {
    let raw = csv_row
        .get("catalog_id")
        .expect("validated catalog row contains catalog_id");
    let normalized = normalize_contract_text(raw)
        .ok_or_else(|| CatalogContractError::invalid("catalog_id must not be blank"))?;
    if normalized != *raw {
        return Err(CatalogContractError::invalid(
            "catalog_id must already use canonical trimmed form",
        ));
    }
    Ok(normalized)
}

fn catalog_id_list(csv_row: &CsvRow, field: &str) -> Result<Vec<String>, CatalogContractError> {
    let mut values = Vec::new();
    for raw in csv_row
        .get(field)
        .expect("validated catalog row contains catalog ID list")
        .split(';')
    {
        let Some(value) = normalize_contract_text(raw) else {
            continue;
        };
        if value != raw {
            return Err(CatalogContractError::invalid(format!(
                "{field} must already use canonical trimmed form"
            )));
        }
        if values.contains(&value) {
            return Err(CatalogContractError::invalid(format!(
                "{field} contains a duplicate value"
            )));
        }
        values.push(value);
    }
    Ok(values)
}

fn required_catalog_text(csv_row: &CsvRow, field: &str) -> Result<String, CatalogContractError> {
    optional_catalog_text(csv_row, field)
        .ok_or_else(|| CatalogContractError::invalid(format!("{field} must not be blank")))
}

fn optional_catalog_text(csv_row: &CsvRow, field: &str) -> Option<String> {
    csv_row
        .get(field)
        .and_then(|value| normalize_contract_text(value))
}

fn optional_catalog_issn(
    csv_row: &CsvRow,
    field: &str,
) -> Result<Option<String>, CatalogContractError> {
    let Some(value) = optional_catalog_text(csv_row, field) else {
        return Ok(None);
    };
    normalize_contract_issn(&value)
        .map(Some)
        .ok_or_else(|| CatalogContractError::invalid(format!("{field} contains an invalid ISSN")))
}

fn catalog_issn_list(csv_row: &CsvRow, field: &str) -> Result<Vec<String>, CatalogContractError> {
    let mut values = Vec::new();
    for value in csv_row
        .get(field)
        .expect("validated catalog row contains ISSN list")
        .split(';')
    {
        let Some(value) = normalize_contract_text(value) else {
            continue;
        };
        let issn = normalize_contract_issn(&value).ok_or_else(|| {
            CatalogContractError::invalid(format!("{field} contains an invalid ISSN"))
        })?;
        if !values.contains(&issn) {
            values.push(issn);
        }
    }
    Ok(values)
}

fn catalog_text_list(csv_row: &CsvRow, field: &str) -> Result<Vec<String>, CatalogContractError> {
    let mut values = Vec::new();
    for value in csv_row
        .get(field)
        .expect("validated catalog row contains text list")
        .split(';')
    {
        let Some(value) = normalize_contract_text(value) else {
            continue;
        };
        if values.contains(&value) {
            return Err(CatalogContractError::invalid(format!(
                "{field} contains a duplicate value"
            )));
        }
        values.push(value);
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::Path;

    use super::{
        build_catalog_entries, build_catalog_entry, parse_catalog_csv, read_catalog_csv, CsvRow,
        CATALOG_CSV_V3_COLUMNS,
    };

    fn catalog_row() -> CsvRow {
        let mut row = CATALOG_CSV_V3_COLUMNS
            .iter()
            .map(|column| ((*column).to_string(), String::new()))
            .collect::<BTreeMap<_, _>>();
        row.insert("catalog_id".to_string(), "journal-1".to_string());
        row.insert("title".to_string(), "Canonical Journal".to_string());
        row.insert("issn".to_string(), "1234-5679".to_string());
        row.insert("all_issns".to_string(), "1234-5679".to_string());
        row
    }

    #[test]
    fn canonical_row_builds_provider_free_entry() {
        let entry = build_catalog_entry(&catalog_row()).expect("canonical row should pass");
        assert_eq!(entry.catalog_id, "journal-1");
        assert!(entry.catalog_aliases.is_empty());
        assert_eq!(entry.issn.as_deref(), Some("1234-5679"));
    }

    #[test]
    fn forbidden_or_missing_columns_fail_before_persistence() {
        let mut provider = catalog_row();
        provider.insert("provider".to_string(), "cnki".to_string());
        assert!(build_catalog_entry(&provider).is_err());

        let mut url = catalog_row();
        url.insert("url".to_string(), "https://example.test".to_string());
        assert!(build_catalog_entry(&url).is_err());

        let mut missing = catalog_row();
        missing.remove("catalog_id");
        assert!(build_catalog_entry(&missing).is_err());
    }

    #[test]
    fn duplicate_catalog_ids_fail_the_catalog() {
        let row = catalog_row();
        assert!(build_catalog_entries(&[row.clone(), row]).is_err());
    }

    #[test]
    fn catalog_alias_and_issn_ownership_collisions_fail_the_catalog() {
        let first = catalog_row();
        let mut alias_collision = catalog_row();
        alias_collision.insert("catalog_id".to_string(), "journal-2".to_string());
        alias_collision.insert("catalog_aliases".to_string(), "journal-1".to_string());
        alias_collision.insert("title".to_string(), "Second Journal".to_string());
        alias_collision.insert("issn".to_string(), "2049-3630".to_string());
        alias_collision.insert("all_issns".to_string(), "2049-3630".to_string());
        assert!(build_catalog_entries(&[first.clone(), alias_collision])
            .expect_err("catalog alias ownership collision should fail")
            .to_string()
            .contains("catalog identity journal-1"));

        let mut issn_collision = catalog_row();
        issn_collision.insert("catalog_id".to_string(), "journal-2".to_string());
        issn_collision.insert("title".to_string(), "Second Journal".to_string());
        assert!(build_catalog_entries(&[first, issn_collision])
            .expect_err("ISSN ownership collision should fail")
            .to_string()
            .contains("ISSN 1234-5679"));
    }

    #[test]
    fn self_or_noncanonical_catalog_aliases_fail_the_row() {
        let mut self_alias = catalog_row();
        self_alias.insert("catalog_aliases".to_string(), "journal-1".to_string());
        assert!(build_catalog_entry(&self_alias).is_err());

        let mut noncanonical_alias = catalog_row();
        noncanonical_alias.insert("catalog_aliases".to_string(), " journal-legacy".to_string());
        assert!(build_catalog_entry(&noncanonical_alias).is_err());
    }

    #[test]
    fn parser_requires_the_exact_v3_header_and_width() {
        let header = CATALOG_CSV_V3_COLUMNS.join(",");
        let mut values = vec![String::new(); CATALOG_CSV_V3_COLUMNS.len()];
        values[0] = "journal-1".to_string();
        values[2] = "Canonical Journal".to_string();
        values[3] = "1234-5679".to_string();
        values[5] = "1234-5679".to_string();
        let row = values.join(",");
        let entries =
            parse_catalog_csv(&format!("{header}\n{row}\n")).expect("exact catalog should parse");
        assert_eq!(entries.len(), 1);
        assert!(parse_catalog_csv(
            "catalog_id,title,issn,eissn,all_issns,title_aliases,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating\n"
        )
        .is_err());
        assert!(parse_catalog_csv("source,id,title\ncnki,x,Journal\n").is_err());
        assert!(parse_catalog_csv(&format!("{header}\nonly-one-column\n")).is_err());
    }

    #[test]
    fn parser_handles_quoted_commas_and_rejects_unterminated_quotes() {
        let header = CATALOG_CSV_V3_COLUMNS.join(",");
        let row = "journal-1,,\"Canonical, Journal\",1234-5679,,1234-5679,,,,,,,,,,";
        let entries =
            parse_catalog_csv(&format!("{header}\n{row}\n")).expect("quoted value should parse");
        assert_eq!(entries[0].title, "Canonical, Journal");
        assert!(parse_catalog_csv(&format!("{header}\n\"unterminated\n")).is_err());
    }

    #[test]
    fn committed_catalogs_preserve_canonical_identity_metadata() {
        let bundle_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/meta");
        let ccf = read_catalog_csv(bundle_dir.join("ccf_computer_journals.csv"))
            .expect("CCF catalog should parse");
        let chinese = read_catalog_csv(bundle_dir.join("chinese_journals.csv"))
            .expect("Chinese catalog should parse");
        let english = read_catalog_csv(bundle_dir.join("english_journals.csv"))
            .expect("English catalog should parse");

        assert_eq!(ccf.len(), 291);
        assert_eq!(chinese.len(), 94);
        assert_eq!(english.len(), 571);

        let environment = english
            .iter()
            .find(|entry| entry.catalog_id == "issn-1472-3409")
            .expect("Environment and Planning A should remain canonical");
        assert_eq!(environment.catalog_aliases, ["issn-0308-518x"]);
        assert_eq!(
            environment.title,
            "Environment and Planning A: Economy and Space"
        );
        assert_eq!(environment.issn.as_deref(), Some("0308-518X"));
        assert_eq!(environment.eissn.as_deref(), Some("1472-3409"));
        assert_eq!(environment.all_issns, ["1472-3409", "0308-518X"]);
        assert_eq!(environment.title_aliases, ["Environment and Planning A"]);
        assert_eq!(environment.rankings.abs_rank.as_deref(), Some("1545"));
        assert_eq!(environment.rankings.abs_rating.as_deref(), Some("3"));
        assert_eq!(environment.rankings.fms_rank.as_deref(), Some("1020"));
        assert_eq!(environment.rankings.fms_rating.as_deref(), Some("B"));

        let series_b = english
            .iter()
            .find(|entry| entry.catalog_id == "issn-1467-9868")
            .expect("JRSS Series B should remain canonical");
        assert_eq!(series_b.catalog_aliases, ["issn-1369-7412"]);
        assert_eq!(
            series_b.title,
            "Journal of the Royal Statistical Society Series B: Statistical Methodology"
        );
        assert_eq!(series_b.issn.as_deref(), Some("1369-7412"));
        assert_eq!(series_b.eissn.as_deref(), Some("1467-9868"));
        assert_eq!(series_b.all_issns, ["1467-9868", "1369-7412"]);
        assert!(series_b.title_aliases.is_empty());
        assert_eq!(series_b.area.as_deref(), Some("Statistics & Econometrics"));
        assert_eq!(series_b.rankings.abs_rank.as_deref(), Some("141"));
        assert_eq!(series_b.rankings.abs_rating.as_deref(), Some("4"));
        assert_eq!(series_b.rankings.fms_rank.as_deref(), Some("133"));
        assert_eq!(series_b.rankings.fms_rating.as_deref(), Some("A"));

        let transportation = english
            .iter()
            .find(|entry| entry.catalog_id == "issn-1879-2367")
            .expect("Transportation Research Part B should remain canonical");
        assert_eq!(transportation.catalog_aliases, ["issn-0191-2615"]);
        assert_eq!(
            transportation.title,
            "Transportation Research Part B: Methodological"
        );
        assert_eq!(transportation.issn.as_deref(), Some("0191-2615"));
        assert_eq!(transportation.eissn.as_deref(), Some("1879-2367"));
        assert_eq!(transportation.all_issns, ["1879-2367", "0191-2615"]);
        assert_eq!(
            transportation.title_aliases,
            ["Transportation Research, Series B: Methodological"]
        );
        assert_eq!(transportation.rankings.abs_rank.as_deref(), Some("1577"));
        assert_eq!(transportation.rankings.abs_rating.as_deref(), Some("4"));
        assert_eq!(transportation.rankings.fms_rank.as_deref(), Some("1150"));
        assert_eq!(transportation.rankings.fms_rating.as_deref(), Some("A"));

        for retired in ["issn-0308-518x", "issn-1369-7412", "issn-0191-2615"] {
            assert!(english.iter().all(|entry| entry.catalog_id != retired));
        }

        let signal_processing = ccf
            .iter()
            .find(|entry| entry.catalog_id == "issn-1053-587x")
            .expect("IEEE Transactions on Signal Processing should remain present");
        assert_eq!(
            signal_processing.title,
            "IEEE Transactions on Signal Processing"
        );
        assert!(signal_processing.catalog_aliases.is_empty());
        assert!(signal_processing.title_aliases.is_empty());
        assert_eq!(signal_processing.all_issns, ["1053-587X", "1941-0476"]);
    }
}
