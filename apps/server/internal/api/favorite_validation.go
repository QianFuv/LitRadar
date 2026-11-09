package api

import (
	"strings"
	"unicode/utf8"
)

func positiveFavoriteId(name string, value int64) *apiError {
	if value <= 0 {
		return badRequest(name + " must be a positive integer")
	}
	return nil
}

// validateFavoriteRequest retains endpoint validation after authentication and before business admission.
func validateFavoriteRequest(name string, folder, article int64, query typedQuery, body map[string]any) *apiError {
	database := ""
	if value := query.text("db_name"); value != nil {
		database = *value
	}
	switch name {
	case "create_folder", "rename_folder":
		return validateFavoriteFolderName(body)
	case "set_tracking":
		return positiveFavoriteId("folder_id", body["folder_id"].(int64))
	case "list_folder_articles", "list_folder_article_page":
		return validateFavoriteListing(name, query)
	case "export_folder":
		return validateFavoriteExport(query)
	default:
		return validateFavoriteArticleRequest(name, folder, article, database, query, body)
	}
}

// validateFavoriteFolderName checks the trimmed folder name without mutating the request.
func validateFavoriteFolderName(body map[string]any) *apiError {
	value := strings.TrimSpace(body["name"].(string))
	if value == "" || utf8.RuneCountInString(value) > 100 {
		return badRequest("Folder name must be 1-100 characters")
	}
	return nil
}

// validateFavoriteListing preserves the distinct offset and page-limit rules.
func validateFavoriteListing(name string, query typedQuery) *apiError {
	limit := query.integerDefault("limit", 100)
	if limit < 1 || limit > 500 {
		return badRequest("limit must be between 1 and 500")
	}
	if name == "list_folder_articles" && query.integerDefault("offset", 0) < 0 {
		return badRequest("offset must be greater than or equal to 0")
	}
	return nil
}

// validateFavoriteExport rejects unsupported explicit formats before loading a snapshot.
func validateFavoriteExport(query typedQuery) *apiError {
	if format := query.text("format"); format != nil && *format != "bibtex" && *format != "ris" && *format != "endnote" {
		return badRequest("Invalid export format")
	}
	return nil
}

// validateFavoriteArticleRequest selects article and bulk checks without adding business-only validation.
func validateFavoriteArticleRequest(name string, folder, article int64, database string, query typedQuery, body map[string]any) *apiError {
	switch name {
	case "add_favorite":
		if failure := positiveFavoriteId("folder_id", folder); failure != nil {
			return failure
		}
		return validateFavoriteItem(body, true)
	case "remove_favorite":
		return validateFavoriteRemoval(folder, article, database)
	case "check_favorite":
		if failure := positiveFavoriteId("article_id", *query.integer("article_id")); failure != nil {
			return failure
		}
		return validateCharacters("db_name", database, 255)
	case "check_favorites_batch":
		if len(body["article_ids"].([]any)) > 500 {
			return badRequest("article_ids must contain at most 500 items")
		}
		return validateCharacters("db_name", body["db_name"].(string), 255)
	case "bulk_add", "bulk_remove", "bulk_move":
		return validateFavoriteBulk(name, folder, body)
	}
	return nil
}

// validateFavoriteRemoval checks folder, article and database in the original order.
func validateFavoriteRemoval(folder, article int64, database string) *apiError {
	if failure := positiveFavoriteId("folder_id", folder); failure != nil {
		return failure
	}
	if failure := positiveFavoriteId("article_id", article); failure != nil {
		return failure
	}
	return validateCharacters("db_name", database, 255)
}

// validateFavoriteBulk checks folder identities and item limits before ordered item validation.
func validateFavoriteBulk(name string, folder int64, body map[string]any) *apiError {
	label := "folder_id"
	if name == "bulk_move" {
		label = "source_folder_id"
	}
	if failure := positiveFavoriteId(label, folder); failure != nil {
		return failure
	}
	if name == "bulk_move" {
		if failure := positiveFavoriteId("target_folder_id", body["target_folder_id"].(int64)); failure != nil {
			return failure
		}
	}
	items := body["articles"].([]any)
	if len(items) > 500 {
		return badRequest("articles must contain at most 500 items")
	}
	for _, item := range items {
		if failure := validateFavoriteItem(item.(map[string]any), name == "bulk_add"); failure != nil {
			return failure
		}
	}
	return nil
}

func validateFavoriteItem(item map[string]any, hasNote bool) *apiError {
	if failure := positiveFavoriteId("article_id", item["article_id"].(int64)); failure != nil {
		return failure
	}
	if failure := validateCharacters("db_name", item["db_name"].(string), 255); failure != nil {
		return failure
	}
	if hasNote {
		return validateCharacters("note", item["note"].(string), 2000)
	}
	return nil
}
