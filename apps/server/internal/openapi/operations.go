package openapi

import (
	"fmt"
	"github.com/danielgtaylor/huma/v2"
)

// Describe returns operation policy shared by live and offline registration.
func Describe(binding Operation) (huma.Operation, error) {
	operation := huma.Operation{Method: binding.Method, Path: binding.Path, OperationID: binding.ID, Responses: map[string]*huma.Response{}, Errors: []int{400, 401, 403, 404, 409, 413, 415, 422, 428, 429, 500, 502, 503}}
	switch binding.ID {
	case "list_announcements":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List all announcements for admin management."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "create_announcement":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Create an announcement."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "update_announcement":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Update an announcement."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "delete_announcement":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Delete an announcement."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_invite_codes":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List invite codes."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "create_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Create an admin-generated invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n* `body` - Optional bounded lifecycle overrides.\n\n# Returns\n\nCreated invite code metadata."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.RequestBody = &huma.RequestBody{Required: false}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "revoke_admin_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Irreversibly revoke an invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `code_id` - Invite code row identifier.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nOK response when an unrevoked invite was changed."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_provider_catalog":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Return built-in Provider capabilities and discovered catalog files."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_runtime_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List managed runtime settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "update_runtime_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Update managed runtime settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "list_scheduled_tasks":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List scheduled tasks."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "create_scheduled_task":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Create a scheduled task."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "update_scheduled_task":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Update a scheduled task."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "delete_scheduled_task":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Delete a scheduled task."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "scheduler_status":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Read durable scheduler cursor, worker heartbeat, and run status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "stats":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Return dashboard statistics."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_users":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "List all users with admin dashboard counts."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "delete_user":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Delete a user and associated data."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "set_admin":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Grant or revoke admin status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "reset_password":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Reset a user's password."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "get_announcements":
		operation.DefaultStatus = 200
		operation.Tags = []string{"announcements"}
		operation.Summary = "List enabled announcements from the existing auth database."
		operation.Description = "# Arguments\n\n* `state` - Shared API state containing storage paths.\n\n# Returns\n\nJSON announcement list ordered like the Python API."
		operation.Security = []map[string][]string{}
	case "list_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List articles with filters, FTS, and cursor pagination."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `raw_query` - Raw query string so repeated fields remain available.\n\n# Returns\n\nPaginated articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_article":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return one article record."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nArticle record."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_related_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return articles related to one article."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Source article identifier.\n* `query` - Database selector and result limit.\n\n# Returns\n\nArticles ranked by FTS5 bm25 similarity to the source title over titles and abstracts, excluding the source."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "redirect_article_abstract":
		operation.DefaultStatus = 307
		operation.Tags = []string{"index"}
		operation.Summary = "Resolve and redirect to an article abstract page online."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nNo-store temporary redirect."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_article_access":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return access actions for one article."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nArticle access response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "redirect_article_fulltext":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Redirect to an article full-text target."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `article_id` - Article identifier.\n* `query` - Database selector.\n\n# Returns\n\nTemporary redirect response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.Responses["307"] = &huma.Response{Description: "Temporary redirect to a full-text target."}
		operation.Responses["200"] = &huma.Response{Content: map[string]*huma.MediaType{"application/pdf": {}}}
	case "change_password":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Change the current user's password."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `body` - Password change request.\n\n# Returns\n\nOK response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "get_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Get the invite code generated by the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nInvite code response or null."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "generate_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Generate a one-time invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nInvite code response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "revoke_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Revoke the current user's unrevoked invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nOK response when an invite was revoked."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "rotate_invite_code":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Revoke any prior issuance and create one replacement invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nNewly issued replacement invite code."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "check_invite_required":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Return whether registration requires an invite code."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n\n# Returns\n\nInvite requirement response."
		operation.Security = []map[string][]string{}
	case "login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Sign in with username or email and password."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `body` - Login request.\n\n# Returns\n\nLogin response without the raw token."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "logout":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Logout the current session token and clear the browser cookie."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nLogout response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "logout_all":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Revoke every session and personal access token for the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `request_id` - Server-generated request identifier.\n\n# Returns\n\nLogout response after every token is durably revoked."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_me":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Return the current authenticated user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nCurrent user response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "register":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Register a new user account."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `body` - Registration request.\n\n# Returns\n\nCreated user response."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "get_tokens":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "List active access tokens for the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nActive token metadata."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "create_token":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Create an access token for the current user."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `body` - Token creation request.\n\n# Returns\n\nCreated token response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "delete_token":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Delete an access token by row id."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `token_id` - Token row identifier.\n\n# Returns\n\nOK response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_cfp_journals":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cfp"}
		operation.Summary = "List every maintained journal, including those with no indexed articles or CFP adapter."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_cfp_notices":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cfp"}
		operation.Summary = "Read original notices and backend-calculated availability at a cursor's fixed instant."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "poll_login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Poll a QR login session."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `body` - Polling parameters.\n\n# Returns\n\nPolling result and safe session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "start_login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Start a QR login session."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nQR login challenge and safe session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_session":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Return the current user's CNKI session status."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nSafe CNKI session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "clear_session":
		operation.DefaultStatus = 200
		operation.Tags = []string{"cnki"}
		operation.Summary = "Clear the current user's CNKI session."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nEmpty safe CNKI session status."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "check_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Check which folders contain an article."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "check_favorites_batch":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Batch check which folders contain articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "list_folders":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "List all folders for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "create_folder":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Create a new folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "rename_folder":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Rename an existing folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "delete_folder":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Delete an existing folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_folder_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "List favorited articles in a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "add_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Add one favorite to a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "bulk_add":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Bulk add favorites to a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "bulk_move":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Bulk move favorites between folders."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "bulk_remove":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Bulk remove favorites from a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "list_folder_article_page":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Return one stable cursor page from an owned favorite folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "remove_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Remove one favorite from a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "folder_count":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Get the favorite count for a folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "export_folder":
		operation.Responses["200"] = &huma.Response{Content: map[string]*huma.MediaType{"application/x-bibtex": {}, "application/x-research-info-systems": {}, "application/xml": {}}}
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Export one folder's favorites in a citation format."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_tracking":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Get the current tracking folder for the user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "set_tracking":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Set a folder as the current tracking folder."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "list_issues":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List issues with filters and offset pagination."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Issue list filters.\n\n# Returns\n\nPaginated issues."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_issue":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return one issue record."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `issue_id` - Issue identifier.\n* `query` - Database selector.\n\n# Returns\n\nIssue record."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_journals":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List journals with filters and offset pagination."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Journal list filters.\n* `raw_query` - Repeated rating fields; scalar parsing retains its existing contract.\n\n# Returns\n\nPaginated journals."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_journal":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Return one journal record."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `journal_id` - Journal identifier.\n* `query` - Database selector.\n\n# Returns\n\nJournal record."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_areas":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List journal area counts."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nJournal area counts."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_databases":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List index database filenames."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nJSON list of database filenames."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_journal_options":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List journal selector options."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nJournal option records."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_journal_ratings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List available journal grades with database-wide journal counts."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nFour exact rating groups, excluding unrated journals and counting journals without articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_ai_endpoints":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get the administrator-approved AI endpoint catalog."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_notification_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get the user's notification settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "update_notification_settings":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Create or update the user's notification settings."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "push_weekly_to_tracking":
		operation.DefaultStatus = 202
		operation.Tags = []string{"tracking"}
		operation.Summary = "Start one manual weekly-push job for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_push_weekly_run":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get one durable manual weekly-push run visible to its owner or an administrator."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "acknowledge_unknown_push_weekly_run":
		operation.DefaultStatus = 202
		operation.Tags = []string{"tracking"}
		operation.Summary = "Acknowledge the latest ambiguous manual push and enqueue a safe new attempt."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "cancel_push_weekly_run":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Request cancellation of one durable manual weekly-push run."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_push_weekly_status":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get the current manual weekly-push job status for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "status":
		operation.DefaultStatus = 200
		operation.Tags = []string{"tracking"}
		operation.Summary = "Get tracking status for the authenticated user."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_weekly_updates":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List weekly article updates grouped by database and journal."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nWeekly update response."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_weekly_update_articles":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List one bounded page of weekly articles from a fixed manifest window."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `raw_query` - Raw weekly page query.\n\n# Returns\n\nCursor-paginated weekly articles."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "get_weekly_updates_summary":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "Summarize weekly updates without returning article bodies."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n\n# Returns\n\nFixed-window database and journal counts."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "list_years":
		operation.DefaultStatus = 200
		operation.Tags = []string{"index"}
		operation.Summary = "List publication year summaries."
		operation.Description = "# Arguments\n\n* `state` - Shared API state.\n* `headers` - Request headers.\n* `query` - Database selector.\n\n# Returns\n\nYear summary records."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "live":
		operation.DefaultStatus = 200
		operation.Tags = []string{"health"}
		operation.Summary = "Return application event-loop liveness."
		operation.Description = "# Returns\n\nJSON health response."
		operation.Security = []map[string][]string{}
	case "ready":
		operation.DefaultStatus = 200
		operation.Tags = []string{"health"}
		operation.Summary = "Return whether the embedded scheduler has a recent persisted heartbeat."
		operation.Description = "# Arguments\n\n* `state` - API state containing the shared scheduler database path.\n\n# Returns\n\nHTTP 200 when ready or 503 until the embedded scheduler heartbeat is healthy."
		operation.Security = []map[string][]string{}
	case "list_relation_graph_favorites":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "List saved relation graph summaries."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "create_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Save a relation graph snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 3 * 1024 * 1024
	case "get_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Read an owned relation graph snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "update_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Update an owned relation graph title or snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 3 * 1024 * 1024
	case "delete_relation_graph_favorite":
		operation.DefaultStatus = 200
		operation.Tags = []string{"favorites"}
		operation.Summary = "Delete an owned relation graph snapshot."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "update_email":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Update the local account email."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "get_auth_providers":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "List enabled sign-in providers."
		operation.Security = []map[string][]string{}
	case "oidc_login":
		operation.DefaultStatus = 302
		operation.Tags = []string{"auth"}
		operation.Summary = "Start OIDC sign-in."
		operation.Security = []map[string][]string{}
	case "oidc_callback":
		operation.DefaultStatus = 302
		operation.Tags = []string{"auth"}
		operation.Summary = "Complete OIDC sign-in or linking."
		operation.Security = []map[string][]string{}
	case "oidc_link":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Link an OIDC identity to a local account."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "oidc_unlink":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Unlink the local account OIDC identity."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	case "native_login":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Sign in a native device with username or email and password."
		operation.Description = "Issues a new personal access token named device:<device_name> with a fixed 90-day TTL. Uses the password login rate limits and credential generation fence. Does not set cookies or replace the browser login session. Send the token as Authorization: Bearer; POST /api/auth/logout revokes that token."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "oidc_native_login":
		operation.DefaultStatus = 302
		operation.Tags = []string{"auth"}
		operation.Summary = "Start native OIDC sign-in using signed server state and client PKCE."
		operation.Security = []map[string][]string{}
	case "oidc_native_exchange":
		operation.DefaultStatus = 200
		operation.Tags = []string{"auth"}
		operation.Summary = "Exchange a one-time native OIDC authorization code for a 90-day device token."
		operation.Description = "Issues a new personal access token named device:<device_name> with a fixed 90-day TTL. Uses the password login rate limits and credential generation fence. Does not set cookies or replace the browser login session. Send the token as Authorization: Bearer; POST /api/auth/logout revokes that token."
		operation.Security = []map[string][]string{}
		operation.MaxBodyBytes = 2 * 1024 * 1024
	case "telemetry":
		operation.DefaultStatus = 200
		operation.Tags = []string{"admin"}
		operation.Summary = "Return anonymous hourly load and search-shape telemetry."
		operation.Security = []map[string][]string{{"bearer_auth": {}}, {"session_cookie": {}}}
	default:
		return huma.Operation{}, fmt.Errorf("unknown API operation: %s", binding.ID)
	}
	return operation, nil
}
