package delivery

import (
	"math"
	"testing"
)

func TestStoredSubscriberResultsKeepDefaultsAndRejectAmbiguousTypes(t *testing.T) {
	assertSubscriberResultDefaults(t)
	result, err := ParseSubscriberResult([]byte(`{"folder_synced_count":18446744073709551615,"selected_article_ids":[9223372036854775807,-1],"message_id":""}`))
	if err != nil || result.FolderSyncedCount != math.MaxUint64 || result.SelectedArticleIds[0] != math.MaxInt64 || result.MessageId == nil {
		t.Fatal("integer/text identity changed", err)
	}
	for _, raw := range []string{`null`, `{"selected_article_ids":null}`, `{"selected_article_ids":[1.0]}`, `{"folder_synced_count":-1}`, `{"folder_synced_count":1.0}`, `{"message_id":1}`, `{"message_id":"\ud800"}`, `{"message_id":null,"message_id":"later"}`, `[[],0,null,1]`} {
		if _, err := ParseSubscriberResult([]byte(raw)); err == nil || err.Error() != "Stored subscriber result is invalid" {
			t.Fatalf("accepted corrupted subscriber result %s", raw)
		}
	}
}

func assertSubscriberResultDefaults(t *testing.T) {
	t.Helper()
	for _, raw := range []string{`{}`, `[]`, `[[]]`, `[[],0]`, `[[],0,null]`, `{"ignored":1e9999,"message_id":null}`, `{"ignored":"\ud800"}`} {
		result, err := ParseSubscriberResult([]byte(raw))
		if err != nil || result.SelectedArticleIds == nil || len(result.SelectedArticleIds) != 0 || result.FolderSyncedCount != 0 || result.MessageId != nil {
			t.Fatalf("default changed for %s: %v", raw, err)
		}
	}
}
