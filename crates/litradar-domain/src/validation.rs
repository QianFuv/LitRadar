//! Shared business-input bounds used by HTTP, MCP, and storage callers.

use std::error::Error;
use std::fmt;

use crate::{FavoriteAdd, FavoriteArticleRef, NotificationSettingsUpdate};

/// Maximum number of article identifiers accepted by one favorite batch operation.
pub const MAX_BATCH_ARTICLE_IDS: usize = 500;
/// Maximum number of dynamic identifiers bound in one SQLite `IN` query chunk.
pub const SQLITE_IN_QUERY_CHUNK_SIZE: usize = 500;
/// Maximum folder-name length in Unicode scalar values.
pub const MAX_FOLDER_NAME_CHARS: usize = 100;
/// Maximum favorite-note length in Unicode scalar values.
pub const MAX_FAVORITE_NOTE_CHARS: usize = 2_000;
/// Maximum source database-name length in Unicode scalar values.
pub const MAX_DATABASE_NAME_CHARS: usize = 255;
/// Maximum number of notification keywords.
pub const MAX_NOTIFICATION_KEYWORDS: usize = 100;
/// Maximum number of notification research directions.
pub const MAX_NOTIFICATION_DIRECTIONS: usize = 100;
/// Maximum number of selected index databases.
pub const MAX_SELECTED_DATABASES: usize = 500;
/// Maximum keyword or research-direction length in Unicode scalar values.
pub const MAX_NOTIFICATION_PREFERENCE_CHARS: usize = 500;
/// Maximum notification URL length in Unicode scalar values.
pub const MAX_NOTIFICATION_URL_CHARS: usize = 2_048;
/// Maximum AI model-name length in Unicode scalar values.
pub const MAX_NOTIFICATION_MODEL_CHARS: usize = 200;
/// Maximum AI system-prompt length in Unicode scalar values.
pub const MAX_NOTIFICATION_PROMPT_CHARS: usize = 10_000;
/// Maximum PushPlus template length in Unicode scalar values.
pub const MAX_PUSHPLUS_TEMPLATE_CHARS: usize = 64;
/// Maximum PushPlus topic length in Unicode scalar values.
pub const MAX_PUSHPLUS_TOPIC_CHARS: usize = 200;
/// Maximum PushPlus channel length in Unicode scalar values.
pub const MAX_PUSHPLUS_CHANNEL_CHARS: usize = 64;
/// Maximum submitted notification-secret length in Unicode scalar values.
pub const MAX_NOTIFICATION_SECRET_CHARS: usize = 4_096;
/// Maximum announcement-title length in Unicode scalar values.
pub const MAX_ANNOUNCEMENT_TITLE_CHARS: usize = 200;
/// Maximum announcement-message length in Unicode scalar values.
pub const MAX_ANNOUNCEMENT_MESSAGE_CHARS: usize = 10_000;
/// Maximum announcement-priority length in Unicode scalar values.
pub const MAX_ANNOUNCEMENT_PRIORITY_CHARS: usize = 16;
/// Maximum text length accepted from one MCP string argument.
pub const MAX_MCP_TEXT_CHARS: usize = 2_048;
/// Maximum number of values accepted from one MCP array argument.
pub const MAX_MCP_ARRAY_ITEMS: usize = 500;
/// Maximum text length accepted by article-search filters.
pub const MAX_SEARCH_TEXT_CHARS: usize = MAX_MCP_TEXT_CHARS;
/// Maximum number of values accepted by one repeated article-search filter.
pub const MAX_SEARCH_FILTER_ITEMS: usize = MAX_MCP_ARRAY_ITEMS;

/// A stable, user-correctable business-input validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputValidationError {
    message: String,
}

impl InputValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for InputValidationError {
    /// Format the safe validation detail.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for InputValidationError {}

/// Validate a required string using Unicode scalar-value length.
///
/// # Arguments
///
/// * `label` - User-facing field label.
/// * `value` - Normalized string value.
/// * `maximum` - Inclusive character limit.
///
/// # Returns
///
/// Empty result when the value is non-empty and within the limit.
pub fn validate_required_characters(
    label: &str,
    value: &str,
    maximum: usize,
) -> Result<(), InputValidationError> {
    let count = value.chars().count();
    if count == 0 || count > maximum {
        return Err(InputValidationError::new(format!(
            "{label} must be 1-{maximum} characters"
        )));
    }
    Ok(())
}

/// Validate an optional or empty string using Unicode scalar-value length.
///
/// # Arguments
///
/// * `label` - User-facing field label.
/// * `value` - String value.
/// * `maximum` - Inclusive character limit.
///
/// # Returns
///
/// Empty result when the value is within the limit.
pub fn validate_characters(
    label: &str,
    value: &str,
    maximum: usize,
) -> Result<(), InputValidationError> {
    if value.chars().count() > maximum {
        return Err(InputValidationError::new(format!(
            "{label} must be at most {maximum} characters"
        )));
    }
    Ok(())
}

/// Validate a collection length.
///
/// # Arguments
///
/// * `label` - User-facing collection label.
/// * `count` - Submitted item count.
/// * `maximum` - Inclusive item limit.
///
/// # Returns
///
/// Empty result when the collection is within the limit.
pub fn validate_item_count(
    label: &str,
    count: usize,
    maximum: usize,
) -> Result<(), InputValidationError> {
    if count > maximum {
        return Err(InputValidationError::new(format!(
            "{label} must contain at most {maximum} items"
        )));
    }
    Ok(())
}

/// Validate a positive integer identifier.
///
/// # Arguments
///
/// * `label` - User-facing identifier label.
/// * `value` - Submitted integer.
///
/// # Returns
///
/// Empty result when the identifier is positive.
pub fn validate_positive_id(label: &str, value: i64) -> Result<(), InputValidationError> {
    if value <= 0 {
        return Err(InputValidationError::new(format!(
            "{label} must be a positive integer"
        )));
    }
    Ok(())
}

/// Validate a normalized folder name.
///
/// # Arguments
///
/// * `name` - Trimmed folder name.
///
/// # Returns
///
/// Empty result when the folder name is valid.
pub fn validate_folder_name(name: &str) -> Result<(), InputValidationError> {
    validate_required_characters("Folder name", name, MAX_FOLDER_NAME_CHARS)
}

/// Validate one favorite-add payload.
///
/// # Arguments
///
/// * `favorite` - Favorite payload.
///
/// # Returns
///
/// Empty result when the identifier and text fields are valid.
pub fn validate_favorite_add(favorite: &FavoriteAdd) -> Result<(), InputValidationError> {
    validate_positive_id("article_id", favorite.article_id.value())?;
    validate_characters("db_name", &favorite.db_name, MAX_DATABASE_NAME_CHARS)?;
    validate_characters("note", &favorite.note, MAX_FAVORITE_NOTE_CHARS)
}

/// Validate one favorite article reference.
///
/// # Arguments
///
/// * `favorite` - Favorite reference.
///
/// # Returns
///
/// Empty result when the identifier and database name are valid.
pub fn validate_favorite_article_ref(
    favorite: &FavoriteArticleRef,
) -> Result<(), InputValidationError> {
    validate_positive_id("article_id", favorite.article_id.value())?;
    validate_characters("db_name", &favorite.db_name, MAX_DATABASE_NAME_CHARS)
}

/// Validate every bounded notification-settings field.
///
/// # Arguments
///
/// * `settings` - Notification settings supplied by an API or storage caller.
///
/// # Returns
///
/// Empty result when all arrays and strings are within their shared limits.
pub fn validate_notification_settings(
    settings: &NotificationSettingsUpdate,
) -> Result<(), InputValidationError> {
    validate_item_count(
        "keywords",
        settings.keywords.len(),
        MAX_NOTIFICATION_KEYWORDS,
    )?;
    validate_item_count(
        "directions",
        settings.directions.len(),
        MAX_NOTIFICATION_DIRECTIONS,
    )?;
    validate_item_count(
        "selected_databases",
        settings.selected_databases.len(),
        MAX_SELECTED_DATABASES,
    )?;
    for keyword in &settings.keywords {
        validate_characters("keyword", keyword, MAX_NOTIFICATION_PREFERENCE_CHARS)?;
        let keyword = keyword.trim();
        if !keyword.is_empty() {
            validate_characters("keyword", keyword, MAX_NOTIFICATION_PREFERENCE_CHARS)?;
        }
    }
    for direction in &settings.directions {
        validate_characters("direction", direction, MAX_NOTIFICATION_PREFERENCE_CHARS)?;
        let direction = direction.trim();
        if !direction.is_empty() {
            validate_characters("direction", direction, MAX_NOTIFICATION_PREFERENCE_CHARS)?;
        }
    }
    for database in &settings.selected_databases {
        validate_characters("selected database", database, MAX_DATABASE_NAME_CHARS)?;
        let database = database.trim();
        if !database.is_empty() {
            validate_characters("selected database", database, MAX_DATABASE_NAME_CHARS)?;
        }
    }
    validate_required_characters("delivery_method", settings.delivery_method.trim(), 32)?;
    validate_characters("delivery_method", &settings.delivery_method, 32)?;
    validate_characters(
        "pushplus_template",
        &settings.pushplus_template,
        MAX_PUSHPLUS_TEMPLATE_CHARS,
    )?;
    validate_characters(
        "pushplus_topic",
        &settings.pushplus_topic,
        MAX_PUSHPLUS_TOPIC_CHARS,
    )?;
    validate_characters(
        "pushplus_channel",
        &settings.pushplus_channel,
        MAX_PUSHPLUS_CHANNEL_CHARS,
    )?;
    validate_characters(
        "ai_base_url",
        &settings.ai_base_url,
        MAX_NOTIFICATION_URL_CHARS,
    )?;
    validate_characters(
        "ai_backup_base_url",
        &settings.ai_backup_base_url,
        MAX_NOTIFICATION_URL_CHARS,
    )?;
    validate_characters("ai_model", &settings.ai_model, MAX_NOTIFICATION_MODEL_CHARS)?;
    validate_characters(
        "ai_backup_model",
        &settings.ai_backup_model,
        MAX_NOTIFICATION_MODEL_CHARS,
    )?;
    validate_characters(
        "ai_system_prompt",
        &settings.ai_system_prompt,
        MAX_NOTIFICATION_PROMPT_CHARS,
    )?;
    validate_characters(
        "ai_backup_system_prompt",
        &settings.ai_backup_system_prompt,
        MAX_NOTIFICATION_PROMPT_CHARS,
    )?;
    for (label, secret) in [
        ("pushplus_token", settings.pushplus_token.as_ref()),
        ("ai_api_key", settings.ai_api_key.as_ref()),
        ("ai_backup_api_key", settings.ai_backup_api_key.as_ref()),
    ] {
        if let Some(Some(secret)) = secret {
            validate_characters(label, secret, MAX_NOTIFICATION_SECRET_CHARS)?;
        }
    }
    Ok(())
}

/// Validate optional announcement mutation fields.
///
/// # Arguments
///
/// * `title` - Optional normalized title.
/// * `message` - Optional normalized message.
/// * `priority` - Optional normalized priority.
///
/// # Returns
///
/// Empty result when all supplied fields are valid.
pub fn validate_announcement_fields(
    title: Option<&str>,
    message: Option<&str>,
    priority: Option<&str>,
) -> Result<(), InputValidationError> {
    if let Some(title) = title {
        validate_characters("Title", title, MAX_ANNOUNCEMENT_TITLE_CHARS)?;
        validate_required_characters("Title", title.trim(), MAX_ANNOUNCEMENT_TITLE_CHARS)?;
    }
    if let Some(message) = message {
        validate_characters("Message", message, MAX_ANNOUNCEMENT_MESSAGE_CHARS)?;
        validate_required_characters("Message", message.trim(), MAX_ANNOUNCEMENT_MESSAGE_CHARS)?;
    }
    if let Some(priority) = priority {
        validate_characters("Priority", priority, MAX_ANNOUNCEMENT_PRIORITY_CHARS)?;
        let priority = priority.trim();
        validate_required_characters("Priority", priority, MAX_ANNOUNCEMENT_PRIORITY_CHARS)?;
        if !matches!(priority, "high" | "normal" | "low") {
            return Err(InputValidationError::new(
                "Priority must be high, normal, or low",
            ));
        }
    }
    Ok(())
}

/// Validate one MCP string argument.
///
/// # Arguments
///
/// * `label` - MCP argument name.
/// * `value` - Trimmed argument value.
///
/// # Returns
///
/// Empty result when the value is non-empty and bounded.
pub fn validate_mcp_text(label: &str, value: &str) -> Result<(), InputValidationError> {
    validate_required_characters(label, value, MAX_MCP_TEXT_CHARS)
}

/// Validate one MCP array argument.
///
/// # Arguments
///
/// * `label` - MCP argument name.
/// * `count` - Submitted item count.
///
/// # Returns
///
/// Empty result when the array is bounded.
pub fn validate_mcp_array(label: &str, count: usize) -> Result<(), InputValidationError> {
    validate_item_count(label, count, MAX_MCP_ARRAY_ITEMS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_limits_count_unicode_scalar_values() {
        validate_folder_name(&"文".repeat(MAX_FOLDER_NAME_CHARS))
            .expect("one hundred Unicode characters should pass");
        assert_eq!(
            validate_folder_name(&"文".repeat(MAX_FOLDER_NAME_CHARS + 1))
                .expect_err("one hundred and one Unicode characters should fail")
                .to_string(),
            "Folder name must be 1-100 characters"
        );
    }

    #[test]
    fn item_limits_accept_the_boundary_and_reject_one_more() {
        validate_item_count("article_ids", MAX_BATCH_ARTICLE_IDS, MAX_BATCH_ARTICLE_IDS)
            .expect("the documented favorite batch boundary should pass");
        assert_eq!(
            validate_item_count(
                "article_ids",
                MAX_BATCH_ARTICLE_IDS + 1,
                MAX_BATCH_ARTICLE_IDS,
            )
            .expect_err("one item over the favorite batch boundary should fail")
            .to_string(),
            "article_ids must contain at most 500 items"
        );
    }
}
