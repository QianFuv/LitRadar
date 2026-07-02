PRAGMA foreign_keys = ON;

CREATE TABLE journals (
    journal_id INTEGER PRIMARY KEY,
    library_id TEXT NOT NULL,
    platform_journal_id TEXT,
    title TEXT,
    issn TEXT,
    eissn TEXT,
    scimago_rank REAL,
    cover_url TEXT,
    available INTEGER,
    toc_data_approved_and_live INTEGER,
    has_articles INTEGER
);

CREATE TABLE journal_meta (
    journal_id INTEGER PRIMARY KEY,
    source_csv TEXT NOT NULL,
    area TEXT,
    csv_title TEXT,
    csv_issn TEXT,
    csv_library TEXT,
    resolved_source TEXT,
    resolved_source_id TEXT,
    resolved_title TEXT,
    resolved_issn TEXT,
    resolved_eissn TEXT,
    FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
);

CREATE TABLE issues (
    issue_id INTEGER PRIMARY KEY,
    journal_id INTEGER NOT NULL,
    publication_year INTEGER,
    title TEXT,
    volume TEXT,
    number TEXT,
    date TEXT,
    is_valid_issue INTEGER,
    suppressed INTEGER,
    embargoed INTEGER,
    within_subscription INTEGER,
    FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE
);

CREATE TABLE articles (
    article_id INTEGER PRIMARY KEY,
    journal_id INTEGER NOT NULL,
    issue_id INTEGER,
    title TEXT,
    date TEXT,
    authors TEXT,
    start_page TEXT,
    end_page TEXT,
    abstract TEXT,
    doi TEXT,
    pmid TEXT,
    permalink TEXT,
    suppressed INTEGER,
    in_press INTEGER,
    open_access INTEGER,
    platform_id TEXT,
    retraction_doi TEXT,
    within_library_holdings INTEGER,
    content_location TEXT,
    full_text_file TEXT,
    FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
    FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
);

CREATE TABLE article_listing (
    article_id INTEGER PRIMARY KEY,
    journal_id INTEGER NOT NULL,
    issue_id INTEGER,
    publication_year INTEGER,
    date TEXT,
    open_access INTEGER,
    in_press INTEGER,
    suppressed INTEGER,
    within_library_holdings INTEGER,
    doi TEXT,
    pmid TEXT,
    area TEXT,
    FOREIGN KEY (journal_id) REFERENCES journals(journal_id) ON DELETE CASCADE,
    FOREIGN KEY (issue_id) REFERENCES issues(issue_id) ON DELETE SET NULL
);

CREATE TABLE listing_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    status TEXT,
    updated_at TEXT
);

CREATE VIRTUAL TABLE article_search USING fts5(
    article_id UNINDEXED,
    title,
    abstract,
    doi,
    authors,
    journal_title
);

INSERT INTO journals (
    journal_id,
    library_id,
    platform_journal_id,
    title,
    issn,
    eissn,
    scimago_rank,
    cover_url,
    available,
    toc_data_approved_and_live,
    has_articles
) VALUES
    (
        9007199254740993,
        'scholarly',
        'S-GOLD',
        'Golden Systems Journal',
        '1234-5678',
        '8765-4321',
        1.5,
        'https://example.test/cover.png',
        1,
        1,
        1
    ),
    (
        9007199254740998,
        'cnki',
        'CNKI-GOLD',
        'CNKI Golden Journal',
        '2345-6789',
        NULL,
        NULL,
        NULL,
        1,
        1,
        1
    );

INSERT INTO journal_meta (
    journal_id,
    source_csv,
    area,
    csv_title,
    csv_issn,
    csv_library,
    resolved_source,
    resolved_source_id,
    resolved_title,
    resolved_issn,
    resolved_eissn
) VALUES
    (
        9007199254740993,
        'contracts.csv',
        'systems',
        'Golden Systems Journal',
        '1234-5678',
        'scholarly',
        'crossref',
        'S-GOLD',
        'Golden Systems Journal',
        '1234-5678',
        '8765-4321'
    ),
    (
        9007199254740998,
        'contracts.csv',
        'cnki',
        'CNKI Golden Journal',
        '2345-6789',
        'cnki',
        'cnki',
        'CNKI-GOLD',
        'CNKI Golden Journal',
        '2345-6789',
        NULL
    );

INSERT INTO issues (
    issue_id,
    journal_id,
    publication_year,
    title,
    volume,
    number,
    date,
    is_valid_issue,
    suppressed,
    embargoed,
    within_subscription
) VALUES
    (
        101,
        9007199254740993,
        2026,
        'Volume 42 Issue 1',
        '42',
        '1',
        '2026-06-30',
        1,
        0,
        0,
        1
    ),
    (
        201,
        9007199254740998,
        2026,
        '2026 Issue 6',
        '2026',
        '6',
        '2026-06-28',
        1,
        0,
        0,
        1
    );

INSERT INTO articles (
    article_id,
    journal_id,
    issue_id,
    title,
    date,
    authors,
    start_page,
    end_page,
    abstract,
    doi,
    pmid,
    permalink,
    suppressed,
    in_press,
    open_access,
    platform_id,
    retraction_doi,
    within_library_holdings,
    content_location,
    full_text_file
) VALUES
    (
        9007199254740995,
        9007199254740993,
        101,
        'Rust Golden Baseline',
        '2026-06-30',
        'Ada Example; Turing Test',
        '1',
        '12',
        'A contract baseline for Rust migration.',
        '10.1000/golden',
        NULL,
        'https://doi.org/10.1000/golden',
        0,
        0,
        1,
        'S-GOLD-1',
        NULL,
        1,
        'https://example.test/golden',
        'https://example.test/golden.pdf'
    ),
    (
        9007199254740994,
        9007199254740993,
        101,
        'Python Baseline Sentinel',
        '2025-01-01',
        'Grace Hopper',
        '13',
        '24',
        'A compatibility sentinel for Python behavior.',
        '10.1000/python',
        NULL,
        'https://doi.org/10.1000/python',
        0,
        0,
        0,
        'S-GOLD-2',
        NULL,
        1,
        'https://example.test/python',
        NULL
    ),
    (
        9007199254740996,
        9007199254740993,
        NULL,
        'In Press Contract Article',
        '2024-01-01',
        'Linus Example',
        NULL,
        NULL,
        'An in-press article that is always notifiable.',
        '10.1000/inpress',
        NULL,
        'https://doi.org/10.1000/inpress',
        0,
        1,
        1,
        'S-GOLD-3',
        NULL,
        1,
        'https://example.test/inpress',
        'https://example.test/inpress.pdf'
    ),
    (
        9007199254740997,
        9007199254740998,
        201,
        'CNKI Golden Article',
        '2026-06-28',
        'CNKI Author',
        '1',
        '8',
        'A CNKI full-text contract article.',
        NULL,
        NULL,
        'https://oversea.cnki.net/openlink/detail?filename=CNKI202606001',
        0,
        0,
        NULL,
        'CNKI202606001',
        NULL,
        1,
        'https://oversea.cnki.net/openlink/detail?filename=CNKI202606001',
        NULL
    );

INSERT INTO article_listing (
    article_id,
    journal_id,
    issue_id,
    publication_year,
    date,
    open_access,
    in_press,
    suppressed,
    within_library_holdings,
    doi,
    pmid,
    area
) VALUES
    (
        9007199254740995,
        9007199254740993,
        101,
        2026,
        '2026-06-30',
        1,
        0,
        0,
        1,
        '10.1000/golden',
        NULL,
        'systems'
    ),
    (
        9007199254740994,
        9007199254740993,
        101,
        2026,
        '2025-01-01',
        0,
        0,
        0,
        1,
        '10.1000/python',
        NULL,
        'systems'
    ),
    (
        9007199254740996,
        9007199254740993,
        NULL,
        NULL,
        '2024-01-01',
        1,
        1,
        0,
        1,
        '10.1000/inpress',
        NULL,
        'systems'
    ),
    (
        9007199254740997,
        9007199254740998,
        201,
        2026,
        '2026-06-28',
        NULL,
        0,
        0,
        1,
        NULL,
        NULL,
        'cnki'
    );

INSERT INTO listing_state (id, status, updated_at)
VALUES (1, 'ready', '2026-07-02T00:00:00Z');

INSERT INTO article_search (
    rowid,
    article_id,
    title,
    abstract,
    doi,
    authors,
    journal_title
) VALUES
    (
        9007199254740995,
        9007199254740995,
        'Rust Golden Baseline',
        'A contract baseline for Rust migration.',
        '10.1000/golden',
        'Ada Example; Turing Test',
        'Golden Systems Journal'
    ),
    (
        9007199254740994,
        9007199254740994,
        'Python Baseline Sentinel',
        'A compatibility sentinel for Python behavior.',
        '10.1000/python',
        'Grace Hopper',
        'Golden Systems Journal'
    ),
    (
        9007199254740996,
        9007199254740996,
        'In Press Contract Article',
        'An in-press article that is always notifiable.',
        '10.1000/inpress',
        'Linus Example',
        'Golden Systems Journal'
    ),
    (
        9007199254740997,
        9007199254740997,
        'CNKI Golden Article',
        'A CNKI full-text contract article.',
        '',
        'CNKI Author',
        'CNKI Golden Journal'
    );
