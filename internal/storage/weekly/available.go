package weekly

import (
	"context"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/domain/identity"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	storage "github.com/QianFuv/LitRadar/internal/storage/sqlite"
)

// LoadAvailable preserves source publication identity while pruning references absent from weekly listings.
func LoadAvailable(ctx context.Context, configuration config.Config, end Timestamp, selected []string) ([]Manifest, error) {
	manifests, err := LoadManifests(configuration, end.WindowStart(), end, nil)
	if err != nil {
		return nil, err
	}
	requested := map[string]map[int64]bool{}
	retained := make([]Manifest, 0, len(manifests))
	for _, manifest := range manifests {
		if len(selected) > 0 && !slices.Contains(selected, manifest.DbName) {
			continue
		}
		run := ""
		if manifest.RunId != nil {
			run = strings.TrimSpace(*manifest.RunId)
		}
		if run == "" {
			ids := make([]string, len(manifest.ArticleIds))
			for index, id := range manifest.ArticleIds {
				ids[index] = strconv.FormatInt(id, 10)
			}
			display := strings.Replace(strings.TrimSuffix(manifest.GeneratedAt.Format(true), "Z"), "T", " ", 1) + " UTC"
			value := manifest.DbName + ":" + display + ":[" + strings.Join(ids, ", ") + "]"
			run = "weekly-" + strconv.FormatInt(int64(identity.Stable(value, "weekly-manifest")), 10)
		}
		manifest.RunId = &run
		if requested[manifest.DbName] == nil {
			requested[manifest.DbName] = map[int64]bool{}
		}
		for _, id := range manifest.ArticleIds {
			requested[manifest.DbName][id] = true
		}
		retained = append(retained, manifest)
	}
	available := map[string]map[int64]bool{}
	for name, ids := range requested {
		filename := filepath.Join(configuration.IndexDir, name)
		if _, err := os.Stat(filename); err != nil {
			continue
		}
		existing, err := availableIds(ctx, filename, ids)
		if err != nil {
			return nil, err
		}
		available[name] = existing
	}
	result := make([]Manifest, 0, len(retained))
	for _, manifest := range retained {
		ids := make([]int64, 0, len(manifest.ArticleIds))
		for _, id := range manifest.ArticleIds {
			if available[manifest.DbName][id] {
				ids = append(ids, id)
			}
		}
		if len(ids) > 0 {
			manifest.ArticleIds = ids
			result = append(result, manifest)
		}
	}
	return result, nil
}

func availableIds(ctx context.Context, filename string, requested map[int64]bool) (map[int64]bool, error) {
	database, err := storage.Open(filename, false, 1)
	if err != nil {
		return nil, err
	}
	defer database.Close()
	ids := make([]any, 0, len(requested))
	for id := range requested {
		ids = append(ids, id)
	}
	existing := map[int64]bool{}
	for start := 0; start < len(ids); start += 500 {
		chunk := ids[start:min(start+500, len(ids))]
		marks := strings.TrimSuffix(strings.Repeat("?,", len(chunk)), ",")
		rows, err := database.QueryContext(ctx, "SELECT l.article_id FROM article_listing l JOIN journals j ON j.journal_id=l.journal_id WHERE l.article_id IN ("+marks+")", chunk...)
		if err != nil {
			return nil, err
		}
		for rows.Next() {
			var id storage.Integer
			if err := rows.Scan(&id); err != nil {
				rows.Close()
				return nil, err
			}
			existing[int64(id)] = true
		}
		err = rows.Err()
		rows.Close()
		if err != nil {
			return nil, err
		}
	}
	return existing, nil
}

// CountAvailable counts unique database/article pairs without imposing the legacy response ceiling.
func CountAvailable(ctx context.Context, configuration config.Config, end Timestamp, selected []string) (int, error) {
	manifests, err := LoadAvailable(ctx, configuration, end, selected)
	if err != nil {
		return 0, err
	}
	type reference struct {
		name string
		id   int64
	}
	unique := map[reference]bool{}
	for _, manifest := range manifests {
		for _, id := range manifest.ArticleIds {
			unique[reference{manifest.DbName, id}] = true
		}
	}
	return len(unique), nil
}
