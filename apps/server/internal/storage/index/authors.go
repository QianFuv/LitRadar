//go:build go1.27

package index

import (
	json "encoding/json/v2"
	"errors"
	"fmt"

	domain "github.com/QianFuv/LitRadar/apps/server/internal/domain/sources"
)

func decodeCanonicalAuthors(text string) ([]domain.ArticleAuthorDraft, error) {
	var authors []domain.ArticleAuthorDraft
	if err := json.Unmarshal([]byte(text), &authors); err != nil {
		return nil, fmt.Errorf("invalid canonical article authors JSON: %w", err)
	}
	if authors == nil {
		return nil, errors.New("invalid canonical article authors JSON: expected an array of author objects")
	}
	return authors, nil
}
