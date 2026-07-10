//! Article access and full-text target repositories.

use super::shared::*;
use super::*;

/// Full-text route target.
#[derive(Debug, Clone, PartialEq)]
pub enum ArticleFulltextTarget {
    /// Browser redirect target.
    Redirect(String),
    /// Replay PDF response target.
    Pdf {
        /// Download filename.
        filename: String,
        /// HTTP content type.
        content_type: String,
        /// PDF bytes.
        content: Vec<u8>,
    },
    /// Live Zhejiang Library CNKI download target.
    Cnki(CnkiFulltextTarget),
}

/// Live CNKI full-text download target.
#[derive(Debug, Clone, PartialEq)]
pub struct CnkiFulltextTarget {
    /// Expected article title.
    pub title: String,
    /// Expected article authors.
    pub authors: String,
    /// Expected journal title.
    pub journal_title: String,
    /// Persisted CNKI session JSON.
    pub session_data: JsonValue,
    /// Stored QR UUID.
    pub qr_uuid: String,
}
/// Return article access capabilities.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
/// * `user_id` - Current user identifier.
///
/// # Returns
///
/// Article access response.
pub fn get_article_access(
    config: &StorageConfig,
    codec: &SecretCodec,
    db_name: Option<&str>,
    article_id: i64,
    user_id: UserId,
) -> Result<ArticleAccessResponse, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let row = get_article_access_row(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))?;
    article_access_response(&row, config, codec, user_id)
}

/// Return the full-text redirect URL for an article.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
/// * `user_id` - Current user identifier.
///
/// # Returns
///
/// Redirect URL.
pub fn article_fulltext_redirect_url(
    config: &StorageConfig,
    codec: &SecretCodec,
    db_name: Option<&str>,
    article_id: i64,
    user_id: UserId,
) -> Result<String, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let row = get_article_access_row(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))?;
    if is_cnki_article_row(&row) {
        if is_cnki_session_active(config, codec, user_id)? {
            return Err(IndexRepositoryError::NotFound(
                "CNKI full-text download is not migrated yet",
            ));
        }
        if let Some(permalink) = nonempty(row.permalink.as_deref()) {
            return Ok(with_cnki_chinese_language(permalink));
        }
        return Err(IndexRepositoryError::NotFound("Full text not available"));
    }
    if let Some(full_text_file) = nonempty(row.full_text_file.as_deref()) {
        if !is_cnki_protected_fulltext_url(full_text_file) {
            return Ok(with_cnki_chinese_language(full_text_file));
        }
    }
    if let Some(permalink) = nonempty(row.permalink.as_deref()) {
        return Ok(with_cnki_chinese_language(permalink));
    }
    if let Some(doi) = nonempty(row.doi.as_deref()) {
        return Ok(format!("https://doi.org/{doi}"));
    }
    Err(IndexRepositoryError::NotFound("Full text not available"))
}

/// Return the full-text route target for an article.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
/// * `user_id` - Current user identifier.
///
/// # Returns
///
/// Redirect or PDF response target.
pub fn article_fulltext_target(
    config: &StorageConfig,
    codec: &SecretCodec,
    db_name: Option<&str>,
    article_id: i64,
    user_id: UserId,
) -> Result<ArticleFulltextTarget, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let row = get_article_access_row(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))?;
    if is_cnki_article_row(&row) && is_cnki_session_active(config, codec, user_id)? {
        let session = get_cnki_session_data(config.auth_db_path(), codec, user_id)?
            .ok_or(IndexRepositoryError::NotFound("CNKI login is required"))?;
        return Ok(ArticleFulltextTarget::Cnki(CnkiFulltextTarget {
            title: row.title.unwrap_or_default(),
            authors: row.authors.unwrap_or_default(),
            journal_title: row.journal_title.unwrap_or_default(),
            session_data: session.session_data,
            qr_uuid: session.qr_uuid,
        }));
    }
    Ok(ArticleFulltextTarget::Redirect(
        article_fulltext_redirect_url(config, codec, db_name, article_id, user_id)?,
    ))
}

fn get_article_access_row(
    connection: &Connection,
    article_id: i64,
) -> Result<Option<ArticleAccessRow>, IndexRepositoryError> {
    connection
        .query_row(
            "SELECT a.doi, a.full_text_file, a.permalink, j.library_id, \
                    a.title, a.authors, j.title AS journal_title \
             FROM articles a \
             JOIN journals j ON j.journal_id = a.journal_id WHERE a.article_id = ?",
            [article_id],
            |row| {
                Ok(ArticleAccessRow {
                    doi: row.get(0)?,
                    full_text_file: row.get(1)?,
                    permalink: row.get(2)?,
                    library_id: row.get(3)?,
                    title: row.get(4)?,
                    authors: row.get(5)?,
                    journal_title: row.get(6)?,
                })
            },
        )
        .optional()
        .map_err(IndexRepositoryError::from)
}

fn article_access_response(
    row: &ArticleAccessRow,
    config: &StorageConfig,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<ArticleAccessResponse, IndexRepositoryError> {
    Ok(ArticleAccessResponse {
        detail: detail_access_action(row),
        fulltext: fulltext_access_action(row, config, codec, user_id)?,
    })
}

fn detail_access_action(row: &ArticleAccessRow) -> ArticleAccessAction {
    if let Some(permalink) = nonempty(row.permalink.as_deref()) {
        return ArticleAccessAction {
            available: true,
            label: if is_cnki_article_row(row) {
                CNKI_DETAIL_LABEL.to_string()
            } else {
                DETAIL_LABEL.to_string()
            },
            provider: Some(DETAIL_PROVIDER.to_string()),
            url: Some(with_cnki_chinese_language(permalink)),
            requires_login: false,
            message: None,
        };
    }
    if let Some(doi) = nonempty(row.doi.as_deref()) {
        return ArticleAccessAction {
            available: true,
            label: DETAIL_LABEL.to_string(),
            provider: Some(DOI_PROVIDER.to_string()),
            url: Some(format!("https://doi.org/{doi}")),
            requires_login: false,
            message: None,
        };
    }
    ArticleAccessAction {
        available: false,
        label: DETAIL_LABEL.to_string(),
        provider: None,
        url: None,
        requires_login: false,
        message: Some("Article detail is not available".to_string()),
    }
}

fn fulltext_access_action(
    row: &ArticleAccessRow,
    config: &StorageConfig,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<ArticleAccessAction, IndexRepositoryError> {
    if let Some(full_text_file) = nonempty(row.full_text_file.as_deref()) {
        if !is_cnki_protected_fulltext_url(full_text_file) {
            return Ok(ArticleAccessAction {
                available: true,
                label: FULLTEXT_LABEL.to_string(),
                provider: Some(STORED_FULLTEXT_PROVIDER.to_string()),
                url: None,
                requires_login: false,
                message: None,
            });
        }
    }
    if is_cnki_article_row(row) {
        let is_active = is_cnki_session_active(config, codec, user_id)?;
        return Ok(ArticleAccessAction {
            available: is_active,
            label: FULLTEXT_LABEL.to_string(),
            provider: Some(ZJLIB_CNKI_PROVIDER.to_string()),
            url: None,
            requires_login: !is_active,
            message: (!is_active).then(|| "需要先在设置中完成浙江图书馆扫码登录".to_string()),
        });
    }
    Ok(ArticleAccessAction {
        available: false,
        label: FULLTEXT_LABEL.to_string(),
        provider: None,
        url: None,
        requires_login: false,
        message: Some("Full text is not available".to_string()),
    })
}

fn is_cnki_article_row(row: &ArticleAccessRow) -> bool {
    row.library_id.trim().eq_ignore_ascii_case(CNKI_SOURCE)
}

fn is_cnki_session_active(
    config: &StorageConfig,
    codec: &SecretCodec,
    user_id: UserId,
) -> Result<bool, IndexRepositoryError> {
    if !config.auth_db_path().exists() {
        return Ok(false);
    }
    let status = get_cnki_session_status(config.auth_db_path(), codec, user_id)?;
    Ok(status.status == "active")
}

fn is_cnki_protected_fulltext_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains(CNKI_PROTECTED_FULLTEXT_HOST) && lower.contains(CNKI_PROTECTED_FULLTEXT_PATH)
}

fn with_cnki_chinese_language(url: &str) -> String {
    if !url.to_ascii_lowercase().contains("oversea.cnki.net") || url.contains("language=chs") {
        return url.to_string();
    }
    if url.contains('?') {
        format!("{url}&language=chs")
    } else {
        format!("{url}?language=chs")
    }
}

#[derive(Debug, Clone)]
struct ArticleAccessRow {
    doi: Option<String>,
    full_text_file: Option<String>,
    permalink: Option<String>,
    library_id: String,
    title: Option<String>,
    authors: Option<String>,
    journal_title: Option<String>,
}

#[cfg(test)]
mod tests {
    use ps_domain::UserId;
    use serde_json::json;

    use super::*;
    use crate::index::test_support::IndexFixture;

    #[test]
    fn article_access_and_fulltext_urls_cover_redirect_construction() {
        let fixture = IndexFixture::new(true);
        let user_id = UserId(1);

        let stored_access = get_article_access(
            &fixture.config,
            &fixture.secret_codec,
            Some(&fixture.db_name),
            1001,
            user_id,
        )
        .expect("stored full text access should resolve");
        assert!(stored_access.fulltext.available);
        assert_eq!(
            stored_access.fulltext.provider.as_deref(),
            Some("stored_url")
        );

        assert_eq!(
            article_fulltext_redirect_url(
                &fixture.config,
                &fixture.secret_codec,
                Some(&fixture.db_name),
                1001,
                user_id,
            )
            .expect("stored full text should redirect"),
            "https://files.example/fulltext.pdf"
        );
        assert_eq!(
            article_fulltext_redirect_url(
                &fixture.config,
                &fixture.secret_codec,
                Some(&fixture.db_name),
                1003,
                user_id,
            )
            .expect("CNKI permalink should redirect without an active session"),
            "https://oversea.cnki.net/kcms/detail/abc?foo=bar&language=chs"
        );
        assert_eq!(
            article_fulltext_redirect_url(
                &fixture.config,
                &fixture.secret_codec,
                Some(&fixture.db_name),
                1005,
                user_id,
            )
            .expect("DOI fallback should redirect"),
            "https://doi.org/10.1000/doi-only"
        );

        let missing_url = article_fulltext_redirect_url(
            &fixture.config,
            &fixture.secret_codec,
            Some(&fixture.db_name),
            1008,
            user_id,
        )
        .expect_err("missing full text should fail");
        assert!(matches!(
            missing_url,
            IndexRepositoryError::NotFound("Full text not available")
        ));

        let cnki_access = get_article_access(
            &fixture.config,
            &fixture.secret_codec,
            Some(&fixture.db_name),
            1003,
            user_id,
        )
        .expect("CNKI access should resolve");
        assert_eq!(cnki_access.detail.provider.as_deref(), Some("detail_url"));
        assert_eq!(
            cnki_access.detail.url.as_deref(),
            Some("https://oversea.cnki.net/kcms/detail/abc?foo=bar&language=chs")
        );
        assert!(!cnki_access.fulltext.available);
        assert!(cnki_access.fulltext.requires_login);
        assert_eq!(cnki_access.fulltext.provider.as_deref(), Some("zjlib_cnki"));

        match article_fulltext_target(
            &fixture.config,
            &fixture.secret_codec,
            Some(&fixture.db_name),
            1001,
            user_id,
        )
        .expect("stored full text target should resolve")
        {
            ArticleFulltextTarget::Redirect(url) => {
                assert_eq!(url, "https://files.example/fulltext.pdf");
            }
            ArticleFulltextTarget::Pdf { .. } => panic!("stored full text should redirect"),
            ArticleFulltextTarget::Cnki(_) => panic!("stored full text should not use CNKI"),
        }

        crate::upsert_cnki_session(
            fixture.config.auth_db_path(),
            &fixture.secret_codec,
            user_id,
            &json!({"bff_user_token":"x.eyJleHAiOjQxMDI0NDQ4MDB9.y"}),
            "active",
            None,
        )
        .expect("CNKI session should be stored");
        let active_cnki = get_article_access(
            &fixture.config,
            &fixture.secret_codec,
            Some(&fixture.db_name),
            1003,
            user_id,
        )
        .expect("active CNKI access should resolve");
        assert!(active_cnki.fulltext.available);
        assert!(!active_cnki.fulltext.requires_login);

        match article_fulltext_target(
            &fixture.config,
            &fixture.secret_codec,
            Some(&fixture.db_name),
            1003,
            user_id,
        )
        .expect("active CNKI full text target should resolve")
        {
            ArticleFulltextTarget::Cnki(target) => {
                assert_eq!(target.title, "CNKI Protected Knowledge");
                assert_eq!(target.authors, "Dan");
                assert_eq!(target.journal_title, "Beta CNKI");
                assert_eq!(
                    target.session_data["bff_user_token"],
                    "x.eyJleHAiOjQxMDI0NDQ4MDB9.y"
                );
            }
            ArticleFulltextTarget::Redirect(_) | ArticleFulltextTarget::Pdf { .. } => {
                panic!("active CNKI full text should use live CNKI target")
            }
        }
    }

    #[test]
    fn cnki_language_url_helpers_cover_query_variants() {
        assert_eq!(
            with_cnki_chinese_language("https://example.test/article"),
            "https://example.test/article"
        );
        assert_eq!(
            with_cnki_chinese_language("https://oversea.cnki.net/kcms/detail/abc"),
            "https://oversea.cnki.net/kcms/detail/abc?language=chs"
        );
        assert_eq!(
            with_cnki_chinese_language("https://oversea.cnki.net/kcms/detail/abc?foo=bar"),
            "https://oversea.cnki.net/kcms/detail/abc?foo=bar&language=chs"
        );
        assert_eq!(
            with_cnki_chinese_language("https://oversea.cnki.net/kcms/detail/abc?language=chs"),
            "https://oversea.cnki.net/kcms/detail/abc?language=chs"
        );
        assert!(is_cnki_protected_fulltext_url(
            "https://O.OVERSEA.CNKI.NET/barnew/download/order?id=abc"
        ));
    }
}
