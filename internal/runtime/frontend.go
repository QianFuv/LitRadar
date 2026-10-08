package runtime

import (
	"crypto/sha256"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"time"
	"unicode/utf8"
)

// frontend serves trusted assets with the application's characterized HTTP behavior.
type frontend struct {
	source     fs.FS
	validators map[string]string
}

// frontendFile supports independent ranged reads from disk and embedded files.
type frontendFile interface {
	fs.File
	io.ReaderAt
}

// newEmbeddedFrontend computes representation validators once for an immutable export.
func newEmbeddedFrontend(source fs.FS) (frontend, error) {
	files := frontend{source: source, validators: map[string]string{}}
	err := fs.WalkDir(source, ".", func(name string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		data, err := fs.ReadFile(source, name)
		if err != nil {
			return err
		}
		files.validators[name] = fmt.Sprintf("\"%x\"", sha256.Sum256(data))
		return nil
	})
	return files, err
}

// open acquires an asset that supports the existing section-reader response path.
func (files frontend) open(name string) (frontendFile, error) {
	file, err := files.source.Open(name)
	if err != nil {
		return nil, err
	}
	selected, ok := file.(frontendFile)
	if !ok {
		file.Close()
		return nil, errors.New("frontend asset does not support ranged reads")
	}
	return selected, nil
}

// ServeHTTP preserves method, path, representation and fallback admission.
func (files frontend) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	if request.Method != "GET" && request.Method != "HEAD" {
		writer.Header().Set("Allow", "GET,HEAD")
		writer.Header().Set("Content-Length", "0")
		writer.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	name, isValid := frontendPath(request.URL.EscapedPath())
	opened := files.openFrontendRequest(writer, request, name, isValid)
	if opened.isHandled {
		return
	}
	file, name := opened.file, opened.name
	isCompressed, isFallback := opened.isCompressed, opened.isFallback
	defer file.Close()
	metadata, err := file.Stat()
	if err != nil || !metadata.Mode().IsRegular() {
		writer.Header().Set("Content-Length", "0")
		writer.WriteHeader(http.StatusNotFound)
		return
	}
	validatorName := name
	if isCompressed {
		validatorName += ".gz"
	}
	serveFrontendFile(writer, request, file, metadata, name, isCompressed, isFallback, files.validators[validatorName])
}

func frontendPath(escaped string) (string, bool) {
	escaped = frontendExtensionPath(escaped)
	decoded, err := url.PathUnescape(strings.TrimLeft(escaped, "/"))
	if err != nil || !utf8.ValidString(decoded) || strings.ContainsAny(decoded, "\\:") || strings.HasPrefix(decoded, "/") {
		return "", false
	}
	components, isValid := frontendPathComponents(decoded)
	if !isValid {
		return "", false
	}
	if decoded == "" || strings.HasSuffix(decoded, "/") {
		components = append(components, "index.html")
	}
	name := strings.Join(components, "/")
	if !filepath.IsLocal(filepath.FromSlash(strings.ReplaceAll(name, "\x00", ""))) {
		return "", false
	}
	return name, true
}

// serveFrontendFile writes a selected representation with stable validators and ranged bodies.
func serveFrontendFile(writer http.ResponseWriter, request *http.Request, file frontendFile, metadata fs.FileInfo, name string, isCompressed, isFallback bool, contentEtag string) {
	size, modified := metadata.Size(), metadata.ModTime().UTC()
	etag := ""
	if modified.Unix() >= 0 {
		etag = fmt.Sprintf("\"%x.%08x-%x\"", modified.Unix(), modified.Nanosecond(), size)
	}
	if contentEtag != "" {
		etag, modified = contentEtag, time.Time{}
	}
	status := frontendPrecondition(request.Header, etag, modified)
	if writeFrontendPrecondition(writer, request, status, etag, modified, isFallback) {
		return
	}
	writer.Header().Set("Content-Type", frontendMediaType(name))
	writer.Header().Set("Accept-Ranges", "bytes")
	if isCompressed {
		writer.Header().Set("Content-Encoding", "gzip")
	}
	if !isFallback {
		writer.Header().Add("Vary", "accept-encoding")
	}
	start, length, status, isHandled := applyFrontendRange(writer, request, size, status, isFallback)
	if isHandled {
		return
	}
	writeFrontendStatus(writer, status, isFallback, length)
	if request.Method != "HEAD" {
		_, _ = io.CopyN(writer, io.NewSectionReader(file, start, length), length)
	}
}

func writeFrontendEmptyError(writer http.ResponseWriter, request *http.Request, status int, isFallback bool) {
	if request.Method == "HEAD" {
		if isFallback {
			status = http.StatusNotFound
		}
		writer.WriteHeader(status)
		return
	}
	writeFrontendStatus(writer, status, isFallback, 0)
}

func writeFrontendStatus(writer http.ResponseWriter, status int, isFallback bool, length int64) {
	if isFallback {
		status = http.StatusNotFound
	}
	writer.Header().Set("Content-Length", strconv.FormatInt(length, 10))
	writer.WriteHeader(status)
}

// frontendOpenedFile carries acquisition decisions back to the request's file owner.
type frontendOpenedFile struct {
	file                                frontendFile
	name                                string
	isCompressed, isFallback, isHandled bool
}

// openFrontendRequest retains path-error admission and fallback acquisition before caller-owned close.
func (files frontend) openFrontendRequest(writer http.ResponseWriter, request *http.Request, name string, isValid bool) frontendOpenedFile {
	var file frontendFile
	var err error
	isCompressed, isFallback := false, !isValid
	if isValid {
		if strings.ContainsRune(name, 0) {
			if request.Method == "HEAD" {
				writer.WriteHeader(http.StatusInternalServerError)
				return frontendOpenedFile{isHandled: true}
			}
			isFallback = true
		}
	}
	if isValid && !isFallback {
		opened := files.openFrontendPath(writer, request, name)
		if opened.isHandled {
			return opened
		}
		file, isCompressed, isFallback = opened.file, opened.isCompressed, opened.isFallback
	}
	if isFallback {
		name, isCompressed = "404.html", false
		file, err = files.open(name)
		if err != nil {
			writer.Header().Set("Content-Length", "0")
			writer.WriteHeader(http.StatusNotFound)
			return frontendOpenedFile{isHandled: true}
		}
	}
	return frontendOpenedFile{file: file, name: name, isCompressed: isCompressed, isFallback: isFallback}
}

// openFrontendPath preserves directory redirect, gzip selection and identity-open retry precedence.
func (files frontend) openFrontendPath(writer http.ResponseWriter, request *http.Request, name string) frontendOpenedFile {
	var file frontendFile
	var err error
	isCompressed, isFallback := false, false
	if redirectFrontendDirectory(writer, request, files.source, name) {
		return frontendOpenedFile{isHandled: true}
	}
	if prefersGzip(request.Header) {
		file, err = files.open(name + ".gz")
		isCompressed = err == nil
	}
	if file == nil && (err == nil || os.IsNotExist(err)) {
		file, err = files.open(name)
	}
	if err != nil {
		isFallback = isFrontendFallbackError(err)
		if !isFallback {
			writer.Header().Set("Content-Length", "0")
			writer.WriteHeader(http.StatusInternalServerError)
			return frontendOpenedFile{isHandled: true}
		}
	}
	return frontendOpenedFile{file: file, name: name, isCompressed: isCompressed, isFallback: isFallback}
}

// redirectFrontendDirectory retains escaped-path and raw-query spelling for directory redirects.
func redirectFrontendDirectory(writer http.ResponseWriter, request *http.Request, source fs.FS, filename string) bool {
	if info, metadataError := fs.Stat(source, filename); metadataError == nil && info.IsDir() {
		location := request.URL.EscapedPath() + "/"
		if request.URL.RawQuery != "" || request.URL.ForceQuery {
			location += "?" + request.URL.RawQuery
		}
		writer.Header().Set("Location", location)
		if request.Method != "HEAD" {
			writer.Header().Set("Content-Length", "0")
		}
		writer.WriteHeader(http.StatusTemporaryRedirect)
		return true
	}
	return false
}

// frontendPathComponents rejects parent components before dropping dots and empty components.
func frontendPathComponents(decoded string) ([]string, bool) {
	components := []string{}
	for _, component := range strings.Split(decoded, "/") {
		if component == ".." {
			return nil, false
		}
		if component != "" && component != "." {
			components = append(components, component)
		}
	}
	return components, true
}

// writeFrontendPrecondition publishes validators only after failed-precondition admission.
func writeFrontendPrecondition(writer http.ResponseWriter, request *http.Request, status int, etag string, modified time.Time, isFallback bool) bool {
	if status == http.StatusPreconditionFailed {
		writeFrontendEmptyError(writer, request, status, isFallback)
		return true
	}
	if etag != "" {
		writer.Header().Set("ETag", etag)
	}
	if !modified.IsZero() {
		writer.Header().Set("Last-Modified", modified.Format(http.TimeFormat))
	}
	if status == http.StatusNotModified {
		if isFallback {
			writeFrontendStatus(writer, status, true, 0)
		} else {
			writer.WriteHeader(status)
		}
		return true
	}
	return false
}

// applyFrontendRange preserves range admission and returns body positions only after successful headers.
func applyFrontendRange(writer http.ResponseWriter, request *http.Request, size int64, status int, isFallback bool) (int64, int64, int, bool) {
	start, length := int64(0), size
	if raw, exists := request.Header["Range"]; exists && len(raw) > 0 && isVisibleHeader(raw[0]) {
		ranges, isValid := frontendRanges(raw[0], uint64(size))
		if !isValid || len(ranges) != 1 {
			writeFrontendRangeError(writer, request, ranges, isValid, size, isFallback)
			return 0, 0, status, true
		}
		start = int64(ranges[0][0])
		length = int64(ranges[0][1]-ranges[0][0]) + 1
		if size == 0 {
			length = 0
		}
		status = http.StatusPartialContent
		writer.Header().Set("Content-Range", fmt.Sprintf("bytes %d-%d/%d", start, ranges[0][1], size))
	}
	return start, length, status, false
}

// writeFrontendRangeError retains multipart diagnostics and HEAD-specific empty response behavior.
func writeFrontendRangeError(writer http.ResponseWriter, request *http.Request, ranges [][2]uint64, isValid bool, size int64, isFallback bool) {
	writer.Header().Set("Content-Range", fmt.Sprintf("bytes */%d", size))
	body := ""
	if isValid && len(ranges) > 1 {
		body = "Cannot serve multipart range requests"
	}
	if body == "" {
		writeFrontendEmptyError(writer, request, http.StatusRequestedRangeNotSatisfiable, isFallback)
	} else {
		writeFrontendStatus(writer, http.StatusRequestedRangeNotSatisfiable, isFallback, int64(len(body)))
	}
	if request.Method != "HEAD" {
		_, _ = io.WriteString(writer, body)
	}
}

// frontendExtensionPath appends HTML using the escaped basename before path decoding.
func frontendExtensionPath(escaped string) string {
	last := path.Base(escaped)
	hasExtension := strings.LastIndex(last, ".") > 0
	if escaped != "/" && !strings.HasSuffix(escaped, "/") && !hasExtension {
		escaped += ".html"
	}
	return escaped
}

// isFrontendFallbackError retains the exact file-open categories admitted to the static fallback.
func isFrontendFallbackError(err error) bool {
	return os.IsNotExist(err) || os.IsPermission(err) || errors.Is(err, syscall.ENOTDIR) || errors.Is(err, os.ErrInvalid)
}
