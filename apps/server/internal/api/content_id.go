package api

import (
	"strconv"

	"github.com/danielgtaylor/huma/v2"
)

// contentID binds a decimal wire identifier to an exact SQLite integer.
type contentID struct{ value int64 }

// Schema describes positive decimal strings without signs or leading zeroes.
func (contentID) Schema(huma.Registry) *huma.Schema {
	return &huma.Schema{Type: "string", Pattern: "^[1-9][0-9]*$"}
}

// UnmarshalText parses IDs without converting through floating point.
func (id *contentID) UnmarshalText(text []byte) error {
	value, err := strconv.ParseInt(string(text), 10, 64)
	if err != nil {
		return err
	}
	id.value = value
	return nil
}

func (id contentID) pointer() *int64 {
	if id.value == 0 {
		return nil
	}
	return &id.value
}

func validateIDQueryValues(ctx huma.Context, names ...string) []error {
	location := ctx.URL()
	for _, name := range names {
		for _, value := range location.Query()[name] {
			if value == "" {
				return []error{&huma.ErrorDetail{Location: "query." + name, Message: "expected a positive decimal identifier", Value: value}}
			}
		}
	}
	return nil
}

// Resolve rejects explicitly empty IDs that Huma otherwise treats as absent query values.
func (input *ListArticlesInput) Resolve(ctx huma.Context) []error {
	return append(input.RequestMetadata.Resolve(ctx), validateIDQueryValues(ctx, "journal_id", "issue_id")...)
}

// Resolve distinguishes an omitted journal filter from an empty identifier.
func (input *ListIssuesInput) Resolve(ctx huma.Context) []error {
	return append(input.RequestMetadata.Resolve(ctx), validateIDQueryValues(ctx, "journal_id")...)
}
