// Package indexschema defines the canonical content layout and version-specific structural checks.
package indexschema

// Version is the canonical content layout produced by new and upgraded databases.
const Version = 9

// MinimumSupportedVersion is the oldest layout accepted without an ordinary startup upgrade.
const MinimumSupportedVersion = 6

// ContentTables defines the canonical persisted schema and table identities.
const ContentTables = `
    CREATE TABLE journals (
        journal_id INTEGER PRIMARY KEY,
        catalog_id TEXT NOT NULL UNIQUE,
        title TEXT NOT NULL,
        title_aliases_json TEXT NOT NULL,
        issns_json TEXT NOT NULL,
        issn TEXT,
        eissn TEXT,
        area TEXT,
        utd_rank TEXT,
        utd_rating TEXT,
        abs_rank TEXT,
        abs_rating TEXT,
        fms_rank TEXT,
        fms_rating TEXT,
        fmscn_rank TEXT,
        fmscn_rating TEXT
    );

    CREATE TABLE journal_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
        identity_value TEXT NOT NULL,
        canonical_catalog_id TEXT NOT NULL,
        PRIMARY KEY (identity_kind, identity_value)
    );

    CREATE TABLE issues (
        issue_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        publication_year INTEGER,
        title TEXT,
        volume TEXT,
        number TEXT,
        date TEXT,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
    );

    CREATE TABLE articles (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        title TEXT NOT NULL,
        publication_year INTEGER,
        date TEXT,
        authors_json TEXT NOT NULL,
        start_page TEXT,
        end_page TEXT,
        abstract_text TEXT,
        doi TEXT,
        pmid TEXT,
        open_access INTEGER,
        in_press INTEGER,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE TABLE article_retraction_dois (
        article_id INTEGER NOT NULL,
        retraction_doi TEXT NOT NULL,
        PRIMARY KEY (article_id, retraction_doi),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE TABLE article_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('doi', 'pmid', 'bibliographic')),
        identity_value TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        PRIMARY KEY (identity_kind, identity_value),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE TABLE article_listing (
        article_id INTEGER PRIMARY KEY,
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        publication_year INTEGER,
        date TEXT,
        open_access INTEGER,
        in_press INTEGER,
        doi TEXT,
        pmid TEXT,
        area TEXT,
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE,
        FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
        FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
    );

    CREATE VIRTUAL TABLE article_search
    USING fts5(
        article_id UNINDEXED,
        title,
        abstract_text,
        doi,
        pmid,
        authors,
        journal_title,
        content = '',
        contentless_delete = 1,
        tokenize = 'simple 0'
    );

    CREATE TABLE article_change_events (
        event_id INTEGER PRIMARY KEY,
        content_revision TEXT NOT NULL,
        article_id INTEGER NOT NULL,
        change_kind TEXT NOT NULL CHECK (change_kind IN ('upsert', 'remove')),
        journal_id INTEGER NOT NULL,
        issue_id INTEGER,
        in_press INTEGER NOT NULL CHECK (in_press IN (0, 1)),
        created_at TEXT NOT NULL
    );

    CREATE INDEX idx_journals_issn ON journals(issn);
    CREATE INDEX idx_journals_eissn ON journals(eissn);
    CREATE INDEX idx_journal_identity_keys_catalog
        ON journal_identity_keys(canonical_catalog_id);
    CREATE INDEX idx_issues_journal_year ON issues(journal_id, publication_year);
    CREATE INDEX idx_articles_journal ON articles(journal_id);
    CREATE INDEX idx_articles_issue ON articles(issue_id);
    CREATE INDEX idx_articles_date_id ON articles(date, article_id);
    CREATE INDEX idx_articles_doi ON articles(doi);
    CREATE INDEX idx_articles_pmid ON articles(pmid);
    CREATE INDEX idx_article_retraction_dois_doi
        ON article_retraction_dois(retraction_doi);
    CREATE INDEX idx_article_identity_keys_article ON article_identity_keys(article_id);
    CREATE INDEX idx_article_listing_date_id ON article_listing(date, article_id);
    CREATE INDEX idx_article_listing_journal_date_id
        ON article_listing(journal_id, date, article_id);
    CREATE INDEX idx_article_listing_issue ON article_listing(issue_id);
    CREATE UNIQUE INDEX idx_article_change_events_revision
        ON article_change_events(
            content_revision, article_id, change_kind, journal_id,
            COALESCE(issue_id, -1), in_press
        );
`

var columns = []struct {
	name    string
	columns []string
}{
	{"journals", []string{"journal_id", "catalog_id", "title", "title_aliases_json", "issns_json", "issn", "eissn", "area", "utd_rank", "utd_rating", "abs_rank", "abs_rating", "fms_rank", "fms_rating", "fmscn_rank", "fmscn_rating"}},
	{"journal_identity_keys", []string{"identity_kind", "identity_value", "canonical_catalog_id"}},
	{"issues", []string{"issue_id", "journal_id", "publication_year", "title", "volume", "number", "date"}},
	{"articles", []string{"article_id", "journal_id", "issue_id", "title", "publication_year", "date", "authors_json", "start_page", "end_page", "abstract_text", "doi", "pmid", "open_access", "in_press"}},
	{"article_retraction_dois", []string{"article_id", "retraction_doi"}},
	{"article_identity_keys", []string{"identity_kind", "identity_value", "article_id"}},
	{"article_listing", []string{"article_id", "journal_id", "issue_id", "publication_year", "date", "open_access", "in_press", "doi", "pmid", "area"}},
	{"article_search", []string{"article_id", "title", "abstract_text", "doi", "pmid", "authors", "journal_title"}},
	{"article_change_events", []string{"event_id", "content_revision", "article_id", "change_kind", "journal_id", "issue_id", "in_press", "created_at"}},
}

var indexes = []string{"idx_article_change_events_revision", "idx_article_identity_keys_article", "idx_article_listing_date_id", "idx_article_listing_issue", "idx_article_listing_journal_date_id", "idx_article_retraction_dois_doi", "idx_articles_date_id", "idx_articles_doi", "idx_articles_issue", "idx_articles_journal", "idx_articles_pmid", "idx_issues_journal_year", "idx_journal_identity_keys_catalog", "idx_journals_eissn", "idx_journals_issn"}
