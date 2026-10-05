package api

import (
	"net/http/httptest"
	"strings"
	"testing"
)

func BenchmarkRoute(b *testing.B) {
	groups := [][]route{
		(&authHandlers{}).routes(), (&indexHandlers{}).routes(), (&favoriteHandlers{}).routes(),
		(&publicHandlers{}).routes(), (&cnkiHandlers{}).routes(), (&adminHandlers{}).routes(),
		(&cfpHandlers{}).routes(), (&articleHandlers{}).routes(), (&trackingHandlers{}).routes(),
	}
	handler := &Handler{}
	for _, group := range groups {
		handler.routes = append(handler.routes, group...)
	}
	for _, scenario := range []struct {
		name  string
		index int
	}{{"First", 0}, {"Middle", len(handler.routes) / 2}, {"Last", len(handler.routes) - 1}, {"Missing", -1}} {
		b.Run(scenario.name, func(b *testing.B) {
			method, path, expected := "GET", "/api/benchmark-missing", ""
			if scenario.index >= 0 {
				operation := handler.routes[scenario.index].operation
				method, path, expected = operation.Method, operation.Path, operation.Path
				parts := strings.Split(path, "/")
				for index, part := range parts {
					if strings.HasPrefix(part, "{") {
						parts[index] = "123"
					}
				}
				path = strings.Join(parts, "/")
			}
			request := httptest.NewRequest(method, path, nil)
			probe := *request
			selected, pattern, _ := handler.route(&probe)
			if scenario.index >= 0 && (selected == nil || pattern != expected) || scenario.index < 0 && selected != nil {
				b.Fatalf("unexpected match: %s", pattern)
			}
			b.ReportAllocs()
			for b.Loop() {
				iteration := *request
				handler.route(&iteration)
			}
		})
	}
}
