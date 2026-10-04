package openapi

import (
	"embed"
	"net/http"
	"path"
	"strings"
	"unicode/utf8"
)

//go:embed swagger/*
var swaggerAssets embed.FS

// ServeDocs serves the original locked Swagger UI resources without filesystem redirects.
// The caller owns method dispatch, HEAD suppression and the common security headers.
func ServeDocs(writer http.ResponseWriter, request *http.Request) {
	if request.URL.EscapedPath() == "/docs" {
		writer.Header().Set("Location", "/docs/")
		writer.WriteHeader(http.StatusSeeOther)
		return
	}
	name := strings.TrimPrefix(request.URL.Path, "/docs/")
	if !utf8.ValidString(name) {
		writer.Header().Set("Content-Type", "text/plain; charset=utf-8")
		writer.WriteHeader(http.StatusBadRequest)
		_, _ = writer.Write([]byte("Invalid URL: Invalid UTF-8 in `rest`"))
		return
	}
	if name == "" || name == "/" {
		name = "index.html"
	}
	if strings.Contains(name, "/") || name == "LICENSE" || name == "NOTICE" || name == "README.md" {
		writer.WriteHeader(http.StatusNotFound)
		return
	}
	content, err := swaggerAssets.ReadFile("swagger/" + name)
	if err != nil {
		writer.WriteHeader(http.StatusNotFound)
		return
	}
	if name == "swagger-initializer.js" {
		content = []byte(strings.ReplaceAll(string(content), "{{config}}", "  \"dom_id\": \"#swagger-ui\",\n  \"url\": \"/openapi.json\",\n  \"deepLinking\": true,\n  \"layout\": \"StandaloneLayout\""))
	}
	contentType := map[string]string{".js": "text/javascript", ".map": "text/plain", ".css": "text/css", ".html": "text/html", ".png": "image/png"}[path.Ext(name)]
	if contentType == "" {
		contentType = "application/octet-stream"
	}
	writer.Header().Set("Content-Type", contentType)
	writer.WriteHeader(http.StatusOK)
	_, _ = writer.Write(content)
}
