package runtime

import (
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"path"
	"path/filepath"
	"strconv"
	"strings"
	"syscall"
	"unicode/utf8"
)

type frontend struct{ root string }

func (files frontend) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	if request.Method != "GET" && request.Method != "HEAD" {
		writer.Header().Set("Allow", "GET,HEAD")
		writer.Header().Set("Content-Length", "0")
		writer.WriteHeader(http.StatusMethodNotAllowed)
		return
	}
	name, isValid := frontendPath(request.URL.EscapedPath())
	var file *os.File
	var err error
	isCompressed, isFallback := false, !isValid
	if isValid {
		if strings.ContainsRune(name, 0) {
			if request.Method == "HEAD" {
				writer.WriteHeader(http.StatusInternalServerError)
				return
			}
			isFallback = true
		}
	}
	if isValid && !isFallback {
		filename := filepath.Join(files.root, filepath.FromSlash(name))
		if info, metadataError := os.Stat(filename); metadataError == nil && info.IsDir() {
			location := request.URL.EscapedPath() + "/"
			if request.URL.RawQuery != "" || request.URL.ForceQuery {
				location += "?" + request.URL.RawQuery
			}
			writer.Header().Set("Location", location)
			if request.Method != "HEAD" {
				writer.Header().Set("Content-Length", "0")
			}
			writer.WriteHeader(http.StatusTemporaryRedirect)
			return
		}
		if prefersGzip(request.Header) {
			file, err = os.Open(filename + ".gz")
			isCompressed = err == nil
		}
		if file == nil && (err == nil || os.IsNotExist(err)) {
			file, err = os.Open(filename)
		}
		if err != nil {
			isFallback = os.IsNotExist(err) || os.IsPermission(err) || errors.Is(err, syscall.ENOTDIR) || errors.Is(err, os.ErrInvalid)
			if !isFallback {
				writer.Header().Set("Content-Length", "0")
				writer.WriteHeader(http.StatusInternalServerError)
				return
			}
		}
	}
	if isFallback {
		name, isCompressed = "404.html", false
		file, err = os.Open(filepath.Join(files.root, name))
		if err != nil {
			writer.Header().Set("Content-Length", "0")
			writer.WriteHeader(http.StatusNotFound)
			return
		}
	}
	defer file.Close()
	metadata, err := file.Stat()
	if err != nil || !metadata.Mode().IsRegular() {
		writer.Header().Set("Content-Length", "0")
		writer.WriteHeader(http.StatusNotFound)
		return
	}
	serveFrontendFile(writer, request, file, metadata, name, isCompressed, isFallback)
}

func frontendPath(escaped string) (string, bool) {
	last := path.Base(escaped)
	hasExtension := strings.LastIndex(last, ".") > 0
	if escaped != "/" && !strings.HasSuffix(escaped, "/") && !hasExtension {
		escaped += ".html"
	}
	decoded, err := url.PathUnescape(strings.TrimLeft(escaped, "/"))
	if err != nil || !utf8.ValidString(decoded) || strings.ContainsAny(decoded, "\\:") || strings.HasPrefix(decoded, "/") {
		return "", false
	}
	components := []string{}
	for _, component := range strings.Split(decoded, "/") {
		if component == ".." {
			return "", false
		}
		if component != "" && component != "." {
			components = append(components, component)
		}
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

func serveFrontendFile(writer http.ResponseWriter, request *http.Request, file *os.File, metadata os.FileInfo, name string, isCompressed, isFallback bool) {
	size, modified := metadata.Size(), metadata.ModTime().UTC()
	etag := ""
	if modified.Unix() >= 0 {
		etag = fmt.Sprintf("\"%x.%08x-%x\"", modified.Unix(), modified.Nanosecond(), size)
	}
	status := frontendPrecondition(request.Header, etag, modified)
	if status == http.StatusPreconditionFailed {
		writeFrontendEmptyError(writer, request, status, isFallback)
		return
	}
	if etag != "" {
		writer.Header().Set("ETag", etag)
	}
	writer.Header().Set("Last-Modified", modified.Format(http.TimeFormat))
	if status == http.StatusNotModified {
		if isFallback {
			writeFrontendStatus(writer, status, true, 0)
		} else {
			writer.WriteHeader(status)
		}
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
	start, length := int64(0), size
	if raw, exists := request.Header["Range"]; exists && len(raw) > 0 && isVisibleHeader(raw[0]) {
		ranges, isValid := frontendRanges(raw[0], uint64(size))
		if !isValid || len(ranges) != 1 {
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
			return
		}
		start = int64(ranges[0][0])
		length = int64(ranges[0][1]-ranges[0][0]) + 1
		if size == 0 {
			length = 0
		}
		status = http.StatusPartialContent
		writer.Header().Set("Content-Range", fmt.Sprintf("bytes %d-%d/%d", start, ranges[0][1], size))
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
