package index

const versionFiveSql = `
    CREATE TABLE journal_identity_keys (
        identity_kind TEXT NOT NULL CHECK (identity_kind IN ('catalog_id', 'issn')),
        identity_value TEXT NOT NULL,
        canonical_catalog_id TEXT NOT NULL,
        PRIMARY KEY (identity_kind, identity_value)
    );
    CREATE INDEX idx_journal_identity_keys_catalog
        ON journal_identity_keys(canonical_catalog_id);
`

const versionSixSql = `
    CREATE TABLE articles_v6 (
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

    INSERT INTO articles_v6 (
        article_id, journal_id, issue_id, title, publication_year, date, authors_json,
        start_page, end_page, abstract_text, doi, pmid, open_access, in_press
    )
    SELECT
        article_id, journal_id, issue_id, title, publication_year, date, authors_json,
        start_page, end_page, abstract_text, doi, pmid, open_access, in_press
    FROM articles;

    DROP TABLE articles;
    ALTER TABLE articles_v6 RENAME TO articles;

    CREATE TABLE article_retraction_dois (
        article_id INTEGER NOT NULL,
        retraction_doi TEXT NOT NULL,
        PRIMARY KEY (article_id, retraction_doi),
        FOREIGN KEY (article_id) REFERENCES articles(article_id) ON DELETE CASCADE
    );

    CREATE INDEX idx_articles_journal ON articles(journal_id);
    CREATE INDEX idx_articles_issue ON articles(issue_id);
    CREATE INDEX idx_articles_date_id ON articles(date, article_id);
    CREATE INDEX idx_articles_doi ON articles(doi);
    CREATE INDEX idx_articles_pmid ON articles(pmid);
    CREATE INDEX idx_article_retraction_dois_doi
        ON article_retraction_dois(retraction_doi);
`
