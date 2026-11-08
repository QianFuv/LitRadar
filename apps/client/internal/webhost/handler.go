package webhost

import (
	"bytes"
	"errors"
	"io/fs"
	"mime"
	"net/http"
	"path"
	"strings"
	"time"
)

const (
	indexFile = "index.html"
	// hashedAssetPrefix holds Vite's content-hashed build output.
	hashedAssetPrefix = "assets/"

	cacheImmutable  = "public, max-age=31536000, immutable"
	cacheRevalidate = "no-cache"
	cacheStatic     = "public, max-age=86400"
)

// contentTypes covers build outputs whose types Go may not know without a system
// MIME table, as in a distroless image.
var contentTypes = map[string]string{
	".css":   "text/css; charset=utf-8",
	".html":  "text/html; charset=utf-8",
	".ico":   "image/x-icon",
	".js":    "text/javascript; charset=utf-8",
	".json":  "application/json",
	".map":   "application/json",
	".png":   "image/png",
	".svg":   "image/svg+xml",
	".txt":   "text/plain; charset=utf-8",
	".webp":  "image/webp",
	".woff2": "font/woff2",
}

// NewHandler serves the built client from dist. Existing files are served with a
// caching policy matched to their name; other GET or HEAD navigations without a
// file extension fall back to index.html so client-side routes such as /login
// load the application. Missing files with an extension stay 404 so a stale
// chunk never receives HTML.
func NewHandler(dist fs.FS) (http.Handler, error) {
	index, err := fs.ReadFile(dist, indexFile)
	if err != nil {
		return nil, errors.New("client build is missing index.html; run the frontend build first")
	}
	return &spaHandler{dist: dist, index: index}, nil
}

type spaHandler struct {
	dist  fs.FS
	index []byte
}

// ServeHTTP implements [http.Handler].
func (handler *spaHandler) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	if request.Method != http.MethodGet && request.Method != http.MethodHead {
		writer.Header().Set("Allow", "GET, HEAD")
		http.Error(writer, http.StatusText(http.StatusMethodNotAllowed), http.StatusMethodNotAllowed)
		return
	}
	name := strings.TrimPrefix(path.Clean("/"+request.URL.Path), "/")
	if name == "" || name == indexFile {
		handler.serveIndex(writer, request)
		return
	}
	content, err := fs.ReadFile(handler.dist, name)
	if err == nil {
		handler.serveFile(writer, request, name, content)
		return
	}
	if path.Ext(name) != "" || strings.HasPrefix(name, hashedAssetPrefix) {
		http.NotFound(writer, request)
		return
	}
	handler.serveIndex(writer, request)
}

func (handler *spaHandler) serveIndex(writer http.ResponseWriter, request *http.Request) {
	writer.Header().Set("Cache-Control", cacheRevalidate)
	writer.Header().Set("Content-Type", contentTypes[".html"])
	http.ServeContent(writer, request, indexFile, time.Time{}, bytes.NewReader(handler.index))
}

func (handler *spaHandler) serveFile(writer http.ResponseWriter, request *http.Request, name string, content []byte) {
	headers := writer.Header()
	switch {
	case strings.HasPrefix(name, hashedAssetPrefix):
		headers.Set("Cache-Control", cacheImmutable)
	case path.Ext(name) == ".js" || path.Ext(name) == ".html":
		headers.Set("Cache-Control", cacheRevalidate)
	default:
		headers.Set("Cache-Control", cacheStatic)
	}
	extension := path.Ext(name)
	if contentType, ok := contentTypes[extension]; ok {
		headers.Set("Content-Type", contentType)
	} else if contentType := mime.TypeByExtension(extension); contentType != "" {
		headers.Set("Content-Type", contentType)
	} else {
		headers.Set("Content-Type", "application/octet-stream")
	}
	http.ServeContent(writer, request, name, time.Time{}, bytes.NewReader(content))
}
