package cnki

import (
	"reflect"
	"testing"
)

// TestArticleDetailFallbackPriorities preserves admission before cleaning and identifier alias precedence.
func TestArticleDetailFallbackPriorities(t *testing.T) {
	for _, scenario := range []struct {
		name, markup, field string
		expected            any
	}{
		{"canonical-id", `<input id="paramfilename" value="canonical">`, "platform_id", "canonical"},
		{"fallback-id", `<input id="paramfilename" value=" ">`, "platform_id", "fallback"},
		{"canonical-code", `<input id="param-dbcode" value="secondary"><input id="paramdbcode" value="primary">`, "dbcode", "primary"},
		{"fallback-code", `<input id="paramdbcode" value=" "><input id="param-dbcode" value="secondary">`, "dbcode", "secondary"},
		{"canonical-db", `<input id="param-dbname" value="secondary"><input id="paramdbname" value="primary">`, "dbname", "primary"},
		{"duplicate-empty-input", `<input id="paramdbcode" value=" "><input id="paramdbcode" value="later"><input id="param-dbcode" value="secondary">`, "dbcode", "later"},
		{"heading-title", `<title>Third - 中国知网</title><p class="title-one">Second</p><h1 class="title">First</h1>`, "title", "First"},
		{"paragraph-title", `<title>Third - 中国知网</title><p class="title-one">Second</p><h1 class="title"> </h1>`, "title", "Second"},
		{"page-title", `<title>Third - 中国知网</title><p class="title-one"> </p>`, "title", "Third"},
		{"missing-title", "", "title", nil},
		{"hidden-summary", `<span id="ChDivSummary">Visible</span><input id="abstract_text" value="Hidden">`, "abstract", "Hidden"},
		{"visible-summary", `<span id="ChDivSummary">Visible</span><input id="abstract_text" value=" ">`, "abstract", "Visible"},
		{"cleaned-empty-summary", `<span id="ChDivSummary">Visible</span><input id="abstract_text" value="&lt;i&gt;&lt;/i&gt;">`, "abstract", nil},
		{"missing-summary", "", "abstract", nil},
		{"chinese-date", `<span class="rowtit">在线公开时间：</span><p>2026-10-01 08:00</p><span class="rowtit">Online Release Time:</span><p>2025-01-01</p>`, "online_release_date", "2026-10-01"},
		{"english-date", `<span class="rowtit">Online Release Time:</span><p>2025-01-01 08:00</p>`, "online_release_date", "2025-01-01"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			value, err := ParseArticleDetail(scenario.markup+`<input id="param-filename" value="fallback">`, KnsBase+"/kcms2/article/abstract?v=synthetic")
			if err != nil {
				t.Fatal(err)
			}
			if actual := value.(map[string]any)[scenario.field]; !reflect.DeepEqual(actual, scenario.expected) {
				t.Fatal("detail fallback priority changed", actual, scenario.expected)
			}
		})
	}
}
