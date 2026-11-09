package nativehost

import (
	"context"
	"crypto/subtle"
	"net"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"
)

const (
	// loopbackShutdownTimeout bounds how long closing waits for the completion
	// page to finish writing.
	loopbackShutdownTimeout = 2 * time.Second
	loopbackHeaderTimeout   = 10 * time.Second
)

// loopbackCompletionPage tells the user to return to the app after a callback.
const loopbackCompletionPage = `<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>LitRadar</title>
<style>body{margin:0;min-height:100vh;display:grid;place-items:center;font-family:system-ui,sans-serif;background:#f6f6f6;color:#1b1b1b}@media (prefers-color-scheme:dark){body{background:#141414;color:#ededed}}main{max-width:28rem;padding:2rem;text-align:center}h1{font-size:1.25rem}p{line-height:1.6;opacity:.75}</style>
</head>
<body><main><h1>%TITLE%</h1><p>%MESSAGE%</p></main></body>
</html>
`

// LoopbackOidcCallbacks receives sign-in callbacks on a one-shot HTTP listener
// bound to 127.0.0.1, as the desktop hosts do.
type LoopbackOidcCallbacks struct{}

// Listen implements [OidcCallbackSource]. The listener answers only GET
// requests to [OidcCallbackPath] whose state matches, accepts the first such
// request, and stops when it is closed.
func (LoopbackOidcCallbacks) Listen(state string) (OidcCallbackListener, error) {
	socket, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		return nil, err
	}
	port := socket.Addr().(*net.TCPAddr).Port
	listener := &loopbackListener{
		state:     state,
		host:      "127.0.0.1:" + strconv.Itoa(port),
		callbacks: make(chan url.Values, 1),
	}
	listener.server = &http.Server{Handler: listener, ReadHeaderTimeout: loopbackHeaderTimeout}
	go func() {
		_ = listener.server.Serve(socket)
	}()
	return listener, nil
}

// ConsumeInterruption implements [OidcCallbackSource]; a loopback callback
// cannot outlive the process that listens for it.
func (LoopbackOidcCallbacks) ConsumeInterruption() bool {
	return false
}

// loopbackListener is the HTTP handler and lifecycle of one loopback callback.
type loopbackListener struct {
	state     string
	host      string
	server    *http.Server
	callbacks chan url.Values

	deliverOnce sync.Once
	closeOnce   sync.Once
}

// RedirectURI implements [OidcCallbackListener].
func (listener *loopbackListener) RedirectURI() string {
	return "http://" + listener.host + OidcCallbackPath
}

// Wait implements [OidcCallbackListener].
func (listener *loopbackListener) Wait(ctx context.Context) (url.Values, error) {
	select {
	case query := <-listener.callbacks:
		return query, nil
	case <-ctx.Done():
		return nil, ctx.Err()
	}
}

// Close implements [OidcCallbackListener]. It lets an in-flight completion
// page finish before the socket closes.
func (listener *loopbackListener) Close() {
	listener.closeOnce.Do(func() {
		ctx, cancel := context.WithTimeout(context.Background(), loopbackShutdownTimeout)
		defer cancel()
		if listener.server.Shutdown(ctx) != nil {
			_ = listener.server.Close()
		}
	})
}

// ServeHTTP answers the browser redirect of the sign-in.
func (listener *loopbackListener) ServeHTTP(writer http.ResponseWriter, request *http.Request) {
	header := writer.Header()
	header.Set("Cache-Control", "no-store")
	header.Set("Content-Security-Policy", "default-src 'none'; style-src 'unsafe-inline'")
	header.Set("Referrer-Policy", "no-referrer")
	header.Set("X-Content-Type-Options", "nosniff")
	if request.Method != http.MethodGet || request.URL.Path != OidcCallbackPath || request.Host != listener.host {
		http.NotFound(writer, request)
		return
	}
	query := request.URL.Query()
	states := query["state"]
	if len(states) != 1 || subtle.ConstantTimeCompare([]byte(states[0]), []byte(listener.state)) != 1 {
		writeCompletionPage(writer, http.StatusBadRequest, "登录请求无效", "此链接与 LitRadar 当前的登录请求不匹配，请回到 LitRadar 重新发起登录。")
		return
	}
	isFirst := false
	listener.deliverOnce.Do(func() { isFirst = true })
	if !isFirst {
		writeCompletionPage(writer, http.StatusGone, "登录已处理", "请回到 LitRadar 继续。")
		return
	}
	if _, failed := query["error"]; failed {
		writeCompletionPage(writer, http.StatusOK, "登录未完成", "请回到 LitRadar 查看原因并重试。现在可以关闭此页面。")
	} else {
		writeCompletionPage(writer, http.StatusOK, "登录完成", "请回到 LitRadar 继续使用。现在可以关闭此页面。")
	}
	listener.callbacks <- query
}

// writeCompletionPage writes the static page shown in the browser after a callback.
func writeCompletionPage(writer http.ResponseWriter, status int, title, message string) {
	page := strings.NewReplacer("%TITLE%", title, "%MESSAGE%", message).Replace(loopbackCompletionPage)
	writer.Header().Set("Content-Type", "text/html; charset=utf-8")
	writer.WriteHeader(status)
	_, _ = writer.Write([]byte(page))
}
