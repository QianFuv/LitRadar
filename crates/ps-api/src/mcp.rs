//! Streamable HTTP MCP integration for the Rust API server.

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use ps_domain::{ArticleId, FavoriteAdd, OkResponse, UserId, UserResponse};
use ps_storage::{
    ArticleListParams, BusinessRepositoryError, DatabaseResolutionError, IndexRepositoryError,
    JournalListParams,
};
use rmcp::handler::server::{router::tool::ToolRouter, tool::Extension, wrapper::Parameters};
use rmcp::model::{CallToolResult, ContentBlock, ErrorData, ServerCapabilities, ServerInfo};
use rmcp::transport::streamable_http_server::{
    session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
};
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler};
use serde::{Deserialize, Serialize};
use tower::Service;

use crate::config::ApiConfig;
use crate::routes::auth;
use crate::state::ApiState;

type InnerMcpService = StreamableHttpService<PaperScannerMcp, LocalSessionManager>;

/// Build the authenticated Streamable HTTP MCP service.
///
/// # Arguments
///
/// * `config` - Runtime API configuration.
/// * `state` - Shared API state used for authentication.
///
/// # Returns
///
/// Tower service that rejects unauthenticated requests before MCP execution.
pub(crate) fn service(config: &ApiConfig, state: ApiState) -> AuthenticatedMcpService {
    let mcp_state = state.clone();
    let mcp_config = StreamableHttpServerConfig::default()
        .with_allowed_hosts(config.mcp_allowed_hosts.clone())
        .with_allowed_origins(config.mcp_allowed_origins.clone());
    let inner = StreamableHttpService::new(
        move || Ok(PaperScannerMcp::new(mcp_state.clone())),
        Arc::new(LocalSessionManager::default()),
        mcp_config,
    );

    AuthenticatedMcpService { state, inner }
}

#[derive(Clone)]
pub(crate) struct AuthenticatedMcpService {
    state: ApiState,
    inner: InnerMcpService,
}

impl Service<Request> for AuthenticatedMcpService {
    type Response = Response;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, mut request: Request) -> Self::Future {
        let state = self.state.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let (user, _) = match auth::require_current_user(&state, request.headers()) {
                Ok(context) => context,
                Err(error) => return Ok(error.into_response()),
            };
            request
                .extensions_mut()
                .insert(AuthenticatedMcpUser { user });

            match inner.call(request).await {
                Ok(response) => Ok(response.into_response()),
                Err(error) => match error {},
            }
        })
    }
}

#[derive(Debug, Clone)]
struct AuthenticatedMcpUser {
    user: UserResponse,
}

#[derive(Debug, Clone)]
struct PaperScannerMcp {
    state: ApiState,
    tool_router: ToolRouter<Self>,
}

impl PaperScannerMcp {
    fn new(state: ApiState) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl PaperScannerMcp {
    #[tool(
        name = "list_databases",
        description = "List available Paper Scanner SQLite databases."
    )]
    fn list_databases(&self) -> Result<CallToolResult, ErrorData> {
        self.index_tool(ps_storage::list_index_database_names(
            self.state.storage_config(),
        ))
    }

    #[tool(
        name = "list_areas",
        description = "List research areas for the selected Paper Scanner database."
    )]
    fn list_areas(
        &self,
        Parameters(input): Parameters<DatabaseInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let db = match optional_text("db", input.db) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::list_areas(
            self.state.storage_config(),
            db.as_deref(),
        ))
    }

    #[tool(
        name = "list_years",
        description = "List publication years for the selected Paper Scanner database."
    )]
    fn list_years(
        &self,
        Parameters(input): Parameters<DatabaseInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let db = match optional_text("db", input.db) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::list_years(
            self.state.storage_config(),
            db.as_deref(),
        ))
    }

    #[tool(
        name = "list_journal_options",
        description = "List journal filter options for the selected Paper Scanner database."
    )]
    fn list_journal_options(
        &self,
        Parameters(input): Parameters<DatabaseInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let db = match optional_text("db", input.db) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::list_journal_options(
            self.state.storage_config(),
            db.as_deref(),
        ))
    }

    #[tool(
        name = "list_sources",
        description = "List metadata source values for the selected Paper Scanner database."
    )]
    fn list_sources(
        &self,
        Parameters(input): Parameters<DatabaseInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let db = match optional_text("db", input.db) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::list_sources(
            self.state.storage_config(),
            db.as_deref(),
        ))
    }

    #[tool(
        name = "list_journals",
        description = "List journals from the selected Paper Scanner database."
    )]
    fn list_journals(
        &self,
        Parameters(input): Parameters<ListJournalsInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let (db, params) = match journal_list_params(input) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::list_journals(
            self.state.storage_config(),
            db.as_deref(),
            &params,
        ))
    }

    #[tool(name = "get_journal", description = "Get a single journal by ID.")]
    fn get_journal(
        &self,
        Parameters(input): Parameters<GetJournalInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let db = match optional_text("db", input.db) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        let journal_id = match required_positive_id("journal_id", input.journal_id) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::get_journal(
            self.state.storage_config(),
            db.as_deref(),
            journal_id,
        ))
    }

    #[tool(
        name = "search_articles",
        description = "Search articles in the Paper Scanner index."
    )]
    fn search_articles(
        &self,
        Parameters(input): Parameters<SearchArticlesInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let (db, params) = match article_list_params(input) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::list_articles(
            self.state.storage_config(),
            db.as_deref(),
            &params,
        ))
    }

    #[tool(name = "get_article", description = "Get a single article by ID.")]
    fn get_article(
        &self,
        Parameters(input): Parameters<GetArticleInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let db = match optional_text("db", input.db) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        let article_id = match required_positive_id("article_id", input.article_id) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.index_tool(ps_storage::get_article(
            self.state.storage_config(),
            db.as_deref(),
            article_id,
        ))
    }

    #[tool(
        name = "get_weekly_updates",
        description = "Get weekly update summaries across all Paper Scanner databases."
    )]
    fn get_weekly_updates(&self) -> Result<CallToolResult, ErrorData> {
        self.index_tool(ps_storage::get_weekly_updates(self.state.storage_config()))
    }

    #[tool(
        name = "list_folders",
        description = "List favorite folders for the authenticated Paper Scanner user."
    )]
    fn list_folders(
        &self,
        Extension(parts): Extension<Parts>,
    ) -> Result<CallToolResult, ErrorData> {
        let user_id = authenticated_user_id(&parts)?;
        self.business_tool(ps_storage::list_folders(
            self.state.storage_config().auth_db_path(),
            user_id,
        ))
    }

    #[tool(
        name = "add_favorite",
        description = "Add an article to a favorite folder for the authenticated user."
    )]
    fn add_favorite(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<FavoriteInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let user_id = authenticated_user_id(&parts)?;
        let favorite = match favorite_add(input) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        self.business_tool(ps_storage::add_favorite(
            self.state.storage_config().auth_db_path(),
            user_id,
            favorite.folder_id,
            &favorite.body,
        ))
    }

    #[tool(
        name = "remove_favorite",
        description = "Remove an article from a favorite folder for the authenticated user."
    )]
    fn remove_favorite(
        &self,
        Extension(parts): Extension<Parts>,
        Parameters(input): Parameters<FavoriteInput>,
    ) -> Result<CallToolResult, ErrorData> {
        let user_id = authenticated_user_id(&parts)?;
        let favorite = match favorite_add(input) {
            Ok(value) => value,
            Err(message) => return Ok(tool_error(message)),
        };
        match ps_storage::remove_favorite(
            self.state.storage_config().auth_db_path(),
            user_id,
            favorite.folder_id,
            favorite.body.article_id.value(),
            &favorite.body.db_name,
        ) {
            Ok(true) => json_tool_result(&OkResponse { ok: true }),
            Ok(false) => Ok(tool_error("Favorite not found")),
            Err(error) => Ok(tool_error(business_tool_error_message(&error))),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for PaperScannerMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Use Paper Scanner tools to query indexed papers and favorites.")
    }
}

impl PaperScannerMcp {
    fn index_tool<T: Serialize>(
        &self,
        result: Result<T, IndexRepositoryError>,
    ) -> Result<CallToolResult, ErrorData> {
        match result {
            Ok(payload) => json_tool_result(&payload),
            Err(error) => Ok(tool_error(index_tool_error_message(&error))),
        }
    }

    fn business_tool<T: Serialize>(
        &self,
        result: Result<T, BusinessRepositoryError>,
    ) -> Result<CallToolResult, ErrorData> {
        match result {
            Ok(payload) => json_tool_result(&payload),
            Err(error) => Ok(tool_error(business_tool_error_message(&error))),
        }
    }
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct DatabaseInput {
    db: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ListJournalsInput {
    area: Option<String>,
    available: Option<bool>,
    db: Option<String>,
    has_articles: Option<bool>,
    library_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    scimago_max: Option<f64>,
    scimago_min: Option<f64>,
    sort: Option<String>,
    year: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetJournalInput {
    db: Option<String>,
    journal_id: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SearchArticlesInput {
    area: Option<StringOrStrings>,
    cursor: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    db: Option<String>,
    doi: Option<String>,
    include_total: Option<bool>,
    in_press: Option<bool>,
    issue_id: Option<i64>,
    journal_id: Option<StringOrStrings>,
    limit: Option<i64>,
    offset: Option<i64>,
    open_access: Option<bool>,
    pmid: Option<String>,
    q: Option<String>,
    sort: Option<String>,
    suppressed: Option<bool>,
    within_library_holdings: Option<bool>,
    year: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct GetArticleInput {
    article_id: String,
    db: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct FavoriteInput {
    article_id: String,
    db_name: Option<String>,
    folder_id: i64,
}

struct FavoriteAddInput {
    body: FavoriteAdd,
    folder_id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(untagged)]
enum StringOrStrings {
    Single(String),
    Multiple(Vec<String>),
}

impl StringOrStrings {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(value) => vec![value],
            Self::Multiple(values) => values,
        }
    }
}

fn journal_list_params(
    input: ListJournalsInput,
) -> Result<(Option<String>, JournalListParams), String> {
    Ok((
        optional_text("db", input.db)?,
        JournalListParams {
            area: optional_text("area", input.area)?,
            library_id: optional_text("library_id", input.library_id)?,
            available: input.available,
            has_articles: input.has_articles,
            year: optional_nonnegative_i64("year", input.year)?,
            scimago_min: input.scimago_min,
            scimago_max: input.scimago_max,
            sort: optional_text("sort", input.sort)?,
            limit: limit_or_default(input.limit)?,
            offset: offset_or_default(input.offset)?,
        },
    ))
}

fn article_list_params(
    input: SearchArticlesInput,
) -> Result<(Option<String>, ArticleListParams), String> {
    let mut params = ArticleListParams::default();
    params.journal_id = positive_id_vec("journal_id", input.journal_id)?;
    params.area = text_vec("area", input.area)?;
    params.issue_id = optional_nonnegative_i64("issue_id", input.issue_id)?;
    params.year = optional_nonnegative_i64("year", input.year)?;
    params.in_press = input.in_press;
    params.open_access = input.open_access;
    params.suppressed = input.suppressed;
    params.within_library_holdings = input.within_library_holdings;
    params.date_from = optional_text("date_from", input.date_from)?;
    params.date_to = optional_text("date_to", input.date_to)?;
    params.doi = optional_text("doi", input.doi)?;
    params.pmid = optional_text("pmid", input.pmid)?;
    params.q = optional_text("q", input.q)?;
    params.sort = optional_text("sort", input.sort)?.or(params.sort);
    params.limit = limit_or_default(input.limit)?;
    params.offset = offset_or_default(input.offset)?;
    params.cursor = optional_text("cursor", input.cursor)?;
    params.include_total = input.include_total.unwrap_or(params.include_total);
    Ok((optional_text("db", input.db)?, params))
}

fn favorite_add(input: FavoriteInput) -> Result<FavoriteAddInput, String> {
    let folder_id = positive_i64("folder_id", input.folder_id)?;
    let article_id = required_positive_id("article_id", input.article_id)?;
    let db_name = optional_text("db_name", input.db_name)?.unwrap_or_default();
    Ok(FavoriteAddInput {
        body: FavoriteAdd {
            article_id: ArticleId(article_id),
            db_name,
            note: String::new(),
        },
        folder_id,
    })
}

fn authenticated_user_id(parts: &Parts) -> Result<UserId, ErrorData> {
    parts
        .extensions
        .get::<AuthenticatedMcpUser>()
        .map(|context| context.user.id)
        .ok_or_else(|| ErrorData::internal_error("Authenticated MCP user is missing", None))
}

fn json_tool_result<T: Serialize>(payload: &T) -> Result<CallToolResult, ErrorData> {
    let text = serde_json::to_string_pretty(payload)
        .map_err(|_| ErrorData::internal_error("Failed to serialize MCP tool response", None))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(text)]))
}

fn tool_error(message: impl Into<String>) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message.into())])
}

fn index_tool_error_message(error: &IndexRepositoryError) -> String {
    match error {
        IndexRepositoryError::DatabaseResolution(DatabaseResolutionError::Io(_))
        | IndexRepositoryError::Sqlite(_)
        | IndexRepositoryError::Io(_)
        | IndexRepositoryError::Json(_)
        | IndexRepositoryError::Cnki(_) => "Internal Server Error".to_string(),
        _ => error.to_string(),
    }
}

fn business_tool_error_message(error: &BusinessRepositoryError) -> String {
    match error {
        BusinessRepositoryError::DuplicateFolderName
        | BusinessRepositoryError::FolderNotFound
        | BusinessRepositoryError::SourceFolderNotFound
        | BusinessRepositoryError::TargetFolderNotFound
        | BusinessRepositoryError::SourceAndTargetFoldersSame
        | BusinessRepositoryError::InvalidScheduledJob(_)
        | BusinessRepositoryError::InvalidScheduledTask(_)
        | BusinessRepositoryError::LegacyScheduledTaskCannotBeEnabled => error.to_string(),
        BusinessRepositoryError::Sqlite(_)
        | BusinessRepositoryError::Io(_)
        | BusinessRepositoryError::Json(_)
        | BusinessRepositoryError::UnknownRuntimeSetting(_)
        | BusinessRepositoryError::InvalidRuntimeBoolean(_) => "Internal Server Error".to_string(),
    }
}

fn optional_text(name: &str, value: Option<String>) -> Result<Option<String>, String> {
    match value {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                Err(format!("{name} must not be empty"))
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        None => Ok(None),
    }
}

fn text_vec(name: &str, value: Option<StringOrStrings>) -> Result<Vec<String>, String> {
    match value {
        Some(value) => value
            .into_vec()
            .into_iter()
            .map(|entry| required_text(name, entry))
            .collect(),
        None => Ok(Vec::new()),
    }
}

fn required_text(name: &str, value: String) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(format!("{name} must not be empty"))
    } else {
        Ok(trimmed.to_string())
    }
}

fn required_positive_id(name: &str, value: String) -> Result<i64, String> {
    let value = required_text(name, value)?;
    let id = value
        .parse::<i64>()
        .map_err(|_| format!("{name} must be a positive integer"))?;
    if id > 0 {
        Ok(id)
    } else {
        Err(format!("{name} must be a positive integer"))
    }
}

fn positive_i64(name: &str, value: i64) -> Result<i64, String> {
    if value > 0 {
        Ok(value)
    } else {
        Err(format!("{name} must be a positive integer"))
    }
}

fn positive_id_vec(name: &str, value: Option<StringOrStrings>) -> Result<Vec<i64>, String> {
    match value {
        Some(value) => value
            .into_vec()
            .into_iter()
            .map(|entry| required_positive_id(name, entry))
            .collect(),
        None => Ok(Vec::new()),
    }
}

fn optional_nonnegative_i64(name: &str, value: Option<i64>) -> Result<Option<i64>, String> {
    match value {
        Some(value) if value < 0 => Err(format!("{name} must be greater than or equal to 0")),
        Some(value) => Ok(Some(value)),
        None => Ok(None),
    }
}

fn limit_or_default(value: Option<i64>) -> Result<i64, String> {
    match value {
        Some(value) if !(1..=200).contains(&value) => {
            Err("limit must be between 1 and 200".to_string())
        }
        Some(value) => Ok(value),
        None => Ok(50),
    }
}

fn offset_or_default(value: Option<i64>) -> Result<i64, String> {
    match value {
        Some(value) if value < 0 => Err("offset must be greater than or equal to 0".to_string()),
        Some(value) => Ok(value),
        None => Ok(0),
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{to_bytes, Body};
    use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, COOKIE, HOST};
    use axum::http::{Method, Request, StatusCode};
    use axum::response::Response;
    use axum::Router;
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::test_support::TestBackend;

    const INITIALIZE_BODY: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"ps-api-test","version":"0.1.0"}}}"#;
    const INITIALIZED_BODY: &str = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
    const TOOLS_LIST_BODY: &str = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_initialize_requires_authentication() {
        let backend = TestBackend::new();
        let app = backend.router();

        let response = send_mcp_post(&app, "localhost", None, None, None, INITIALIZE_BODY).await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_initialize_rejects_unconfigured_host() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_host_user", false);
        let app = backend.router();

        let response = send_mcp_post(
            &app,
            "paper.example",
            Some(&user.authorization_header()),
            None,
            None,
            INITIALIZE_BODY,
        )
        .await;

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_initialize_accepts_session_cookie_authentication() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_cookie_user", false);
        let app = backend.router();

        let response = send_mcp_post(
            &app,
            "localhost",
            None,
            Some(&user.cookie_header()),
            None,
            INITIALIZE_BODY,
        )
        .await;
        let status = response.status();
        let headers = response.headers().clone();
        let body = response_body(response).await;

        assert_eq!(status, StatusCode::OK);
        assert!(headers.contains_key("mcp-session-id"));
        assert!(body.contains(r#""id":1"#));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_tools_list_accepts_bearer_authentication() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_bearer_user", false);
        let app = backend.router();
        let authorization = user.authorization_header();
        let session_id = initialize_mcp_session(&app, &authorization).await;

        let payload = mcp_tools_list(&app, &authorization, &session_id).await;
        let tools = tool_names(&payload);

        assert!(tools.contains(&"list_databases".to_string()));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_index_tools_list_includes_read_only_tools() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_index_list_user", false);
        let app = backend.router();
        let authorization = user.authorization_header();
        let session_id = initialize_mcp_session(&app, &authorization).await;

        let payload = mcp_tools_list(&app, &authorization, &session_id).await;
        let tools = tool_names(&payload);

        for name in [
            "list_databases",
            "list_areas",
            "list_years",
            "list_journal_options",
            "list_sources",
            "list_journals",
            "get_journal",
            "search_articles",
            "get_article",
            "get_weekly_updates",
        ] {
            assert!(tools.contains(&name.to_string()), "missing MCP tool {name}");
        }
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_index_read_only_tools_return_fixture_data() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_index_reader", false);
        let index_database = backend.create_index_database("fixture.sqlite");
        let app = backend.router();
        let authorization = user.authorization_header();
        let session_id = initialize_mcp_session(&app, &authorization).await;

        let databases = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            10,
            "list_databases",
            json!({}),
        )
        .await;
        let database_payload = tool_payload(&databases);
        assert_eq!(database_payload, json!(["fixture.sqlite"]));

        let areas = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            11,
            "list_areas",
            json!({ "db": index_database.db_name }),
        )
        .await;
        let area_payload = tool_payload(&areas);
        assert_eq!(area_payload[0]["value"], "Medicine");

        let journal = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            12,
            "get_journal",
            json!({ "db": "fixture.sqlite", "journal_id": "101" }),
        )
        .await;
        let journal_payload = tool_payload(&journal);
        assert_eq!(journal_payload["title"], "Fixture Journal");

        let articles = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            13,
            "search_articles",
            json!({
                "area": "Medicine",
                "db": "fixture.sqlite",
                "include_total": true,
                "journal_id": ["101"],
                "limit": 1,
                "q": "Fixture"
            }),
        )
        .await;
        let articles_payload = tool_payload(&articles);
        assert_eq!(articles_payload["page"]["total"], 1);
        assert_eq!(articles_payload["items"][0]["title"], "Fixture Article");

        let article = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            14,
            "get_article",
            json!({ "article_id": "9001", "db": "fixture.sqlite" }),
        )
        .await;
        let article_payload = tool_payload(&article);
        assert_eq!(article_payload["doi"], "10.1234/fixture");

        let weekly_updates = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            15,
            "get_weekly_updates",
            json!({}),
        )
        .await;
        let weekly_payload = tool_payload(&weekly_updates);
        assert_eq!(weekly_payload["databases"], json!([]));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_index_errors_are_tool_level_results() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_index_error_user", false);
        backend.create_index_database("first.sqlite");
        backend.create_index_database("second.sqlite");
        let app = backend.router();
        let authorization = user.authorization_header();
        let session_id = initialize_mcp_session(&app, &authorization).await;

        let missing_db = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            20,
            "list_years",
            json!({}),
        )
        .await;
        assert_eq!(missing_db["result"]["isError"], true);
        assert!(tool_text(&missing_db).contains("Multiple databases found"));

        let invalid_id = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            21,
            "get_article",
            json!({ "article_id": "0", "db": "first.sqlite" }),
        )
        .await;
        assert_eq!(invalid_id["result"]["isError"], true);
        assert!(tool_text(&invalid_id).contains("article_id must be a positive integer"));
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_favorites_tools_list_includes_favorite_tools() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_favorites_list_user", false);
        let app = backend.router();
        let authorization = user.authorization_header();
        let session_id = initialize_mcp_session(&app, &authorization).await;

        let payload = mcp_tools_list(&app, &authorization, &session_id).await;
        let tools = tool_names(&payload);

        for name in ["list_folders", "add_favorite", "remove_favorite"] {
            assert!(tools.contains(&name.to_string()), "missing MCP tool {name}");
        }
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_favorites_add_remove_use_authenticated_user_scope() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_favorites_owner", false);
        let other_user = backend.authenticated_user("mcp_favorites_other", false);
        let folder =
            ps_storage::create_folder(backend.auth_db_path(), user.user_id(), "Reading", false)
                .expect("owner folder should be created");
        let other_folder =
            ps_storage::create_folder(backend.auth_db_path(), other_user.user_id(), "Other", false)
                .expect("other folder should be created");
        let app = backend.router();
        let authorization = user.authorization_header();
        let other_authorization = other_user.authorization_header();
        let session_id = initialize_mcp_session(&app, &authorization).await;
        let other_session_id = initialize_mcp_session(&app, &other_authorization).await;

        let initial_folders = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            30,
            "list_folders",
            json!({}),
        )
        .await;
        let initial_payload = tool_payload(&initial_folders);
        assert_eq!(
            folder_by_id(&initial_payload, folder.id)["article_count"],
            0
        );
        assert!(maybe_folder_by_id(&initial_payload, other_folder.id).is_none());

        let added = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            31,
            "add_favorite",
            json!({
                "article_id": "9001",
                "db_name": "fixture.sqlite",
                "folder_id": folder.id
            }),
        )
        .await;
        let added_payload = tool_payload(&added);
        assert_eq!(added_payload["folder_id"], folder.id);
        assert_eq!(added_payload["article_id"], "9001");

        let updated_folders = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            32,
            "list_folders",
            json!({}),
        )
        .await;
        let updated_payload = tool_payload(&updated_folders);
        assert_eq!(
            folder_by_id(&updated_payload, folder.id)["article_count"],
            1
        );

        let other_folders = call_mcp_tool(
            &app,
            &other_authorization,
            &other_session_id,
            33,
            "list_folders",
            json!({}),
        )
        .await;
        let other_payload = tool_payload(&other_folders);
        assert_eq!(
            folder_by_id(&other_payload, other_folder.id)["article_count"],
            0
        );
        assert!(maybe_folder_by_id(&other_payload, folder.id).is_none());

        let cross_user_add = call_mcp_tool(
            &app,
            &other_authorization,
            &other_session_id,
            34,
            "add_favorite",
            json!({
                "article_id": "9001",
                "db_name": "fixture.sqlite",
                "folder_id": folder.id
            }),
        )
        .await;
        assert_eq!(cross_user_add["result"]["isError"], true);
        assert!(tool_text(&cross_user_add).contains("Folder not found"));

        let removed = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            35,
            "remove_favorite",
            json!({
                "article_id": "9001",
                "db_name": "fixture.sqlite",
                "folder_id": folder.id
            }),
        )
        .await;
        assert_eq!(tool_payload(&removed), json!({ "ok": true }));

        let final_folders = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            36,
            "list_folders",
            json!({}),
        )
        .await;
        let final_payload = tool_payload(&final_folders);
        assert_eq!(folder_by_id(&final_payload, folder.id)["article_count"], 0);
    }

    #[tokio::test]
    #[cfg_attr(
        miri,
        ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
    )]
    async fn mcp_favorites_invalid_inputs_are_tool_level_results() {
        let backend = TestBackend::new();
        let user = backend.authenticated_user("mcp_favorites_error_user", false);
        let app = backend.router();
        let authorization = user.authorization_header();
        let session_id = initialize_mcp_session(&app, &authorization).await;

        let invalid_folder = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            40,
            "add_favorite",
            json!({
                "article_id": "9001",
                "db_name": "fixture.sqlite",
                "folder_id": 0
            }),
        )
        .await;
        assert_eq!(invalid_folder["result"]["isError"], true);
        assert!(tool_text(&invalid_folder).contains("folder_id must be a positive integer"));

        let invalid_article = call_mcp_tool(
            &app,
            &authorization,
            &session_id,
            41,
            "remove_favorite",
            json!({
                "article_id": "0",
                "db_name": "fixture.sqlite",
                "folder_id": 1
            }),
        )
        .await;
        assert_eq!(invalid_article["result"]["isError"], true);
        assert!(tool_text(&invalid_article).contains("article_id must be a positive integer"));
    }

    async fn initialize_mcp_session(app: &Router, authorization: &str) -> String {
        let initialize_response = send_mcp_post(
            app,
            "localhost",
            Some(authorization),
            None,
            None,
            INITIALIZE_BODY,
        )
        .await;
        assert_eq!(initialize_response.status(), StatusCode::OK);
        let session_id = initialize_response
            .headers()
            .get("mcp-session-id")
            .expect("initialize response should include MCP session")
            .to_str()
            .expect("MCP session id should be visible ASCII")
            .to_string();

        let initialized_response = send_mcp_post(
            app,
            "localhost",
            Some(authorization),
            None,
            Some(&session_id),
            INITIALIZED_BODY,
        )
        .await;
        assert_eq!(initialized_response.status(), StatusCode::ACCEPTED);
        session_id
    }

    async fn mcp_tools_list(app: &Router, authorization: &str, session_id: &str) -> Value {
        let tools_response = send_mcp_post(
            app,
            "localhost",
            Some(authorization),
            None,
            Some(session_id),
            TOOLS_LIST_BODY,
        )
        .await;
        assert_eq!(tools_response.status(), StatusCode::OK);
        sse_response_json(&response_body(tools_response).await)
    }

    async fn call_mcp_tool(
        app: &Router,
        authorization: &str,
        session_id: &str,
        id: i64,
        name: &str,
        arguments: Value,
    ) -> Value {
        let response = send_mcp_post(
            app,
            "localhost",
            Some(authorization),
            None,
            Some(session_id),
            &json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": arguments
                }
            })
            .to_string(),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        sse_response_json(&response_body(response).await)
    }

    fn sse_response_json(body: &str) -> Value {
        body.lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(str::trim)
            .filter(|data| !data.is_empty())
            .map(|data| serde_json::from_str::<Value>(data).expect("SSE data should be JSON"))
            .next_back()
            .expect("SSE response should include JSON data")
    }

    fn tool_names(payload: &Value) -> Vec<String> {
        payload["result"]["tools"]
            .as_array()
            .expect("tools result should be an array")
            .iter()
            .map(|tool| {
                tool["name"]
                    .as_str()
                    .expect("tool should include a name")
                    .to_string()
            })
            .collect()
    }

    fn tool_payload(payload: &Value) -> Value {
        serde_json::from_str(tool_text(payload)).expect("tool text should be JSON")
    }

    fn tool_text(payload: &Value) -> &str {
        payload["result"]["content"][0]["text"]
            .as_str()
            .expect("tool result should include text content")
    }

    fn folder_by_id(payload: &Value, folder_id: i64) -> &Value {
        maybe_folder_by_id(payload, folder_id).expect("folder should exist in payload")
    }

    fn maybe_folder_by_id(payload: &Value, folder_id: i64) -> Option<&Value> {
        payload
            .as_array()
            .expect("folders should be an array")
            .iter()
            .find(|folder| folder["id"].as_i64() == Some(folder_id))
    }

    async fn send_mcp_post(
        app: &Router,
        host: &str,
        authorization: Option<&str>,
        cookie: Option<&str>,
        session_id: Option<&str>,
        body: &str,
    ) -> Response {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri("/mcp")
            .header(HOST, host)
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/event-stream");
        if let Some(value) = authorization {
            builder = builder.header(AUTHORIZATION, value);
        }
        if let Some(value) = cookie {
            builder = builder.header(COOKIE, value);
        }
        if let Some(value) = session_id {
            builder = builder
                .header("mcp-session-id", value)
                .header("mcp-protocol-version", "2025-06-18");
        }

        app.clone()
            .oneshot(
                builder
                    .body(Body::from(body.to_string()))
                    .expect("request should build"),
            )
            .await
            .expect("response should be returned")
    }

    async fn response_body(response: Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        String::from_utf8(body.to_vec()).expect("body should be UTF-8")
    }
}
