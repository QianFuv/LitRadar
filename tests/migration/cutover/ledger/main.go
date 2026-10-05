// Command ledger serves only synthetic HTTPS responses on an isolated migration network.
package main

import (
	"crypto/tls"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

func main() {
	if len(os.Args) != 3 || os.Args[1] != "ledger" {
		panic("expected ledger and certificate directory")
	}
	if err := ledger(os.Args[2]); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func isolation() error {
	for _, address := range []string{"1.1.1.1:443", "11.253.253.1:443", "host.docker.internal:2375", "host.docker.internal:443"} {
		connection, err := net.DialTimeout("tcp", address, time.Second)
		if err == nil {
			connection.Close()
			return fmt.Errorf("unexpected external connection: %s", address)
		}
	}
	routes, err := os.ReadFile("/proc/net/route")
	if err != nil {
		return err
	}
	for _, line := range strings.Split(string(routes), "\n")[1:] {
		fields := strings.Fields(line)
		if len(fields) > 2 && fields[1] == "00000000" {
			return errors.New("unexpected default route")
		}
	}
	return json.NewEncoder(os.Stdout).Encode(map[string]any{"externalConnectionsDenied": true, "routes": string(routes)})
}

func ledger(root string) error {
	articleId, err := strconv.ParseInt(os.Getenv("FIXTURE_ARTICLE_ID"), 10, 64)
	if err != nil || articleId <= 0 {
		return errors.New("synthetic candidate ID required")
	}
	if err := isolation(); err != nil {
		return err
	}
	ai := http.NewServeMux()
	ai.HandleFunc("POST /v1/chat/completions", func(writer http.ResponseWriter, request *http.Request) {
		if request.Header.Get("Authorization") != "Bearer synthetic-ai-only" {
			writer.WriteHeader(403)
			return
		}
		body, err := io.ReadAll(http.MaxBytesReader(writer, request.Body, 1024*1024))
		if err != nil {
			writer.WriteHeader(400)
			return
		}
		content := fmt.Sprintf(`{"summary":"Synthetic summary","selected":[{"article_id":%d,"score":1}]}`, articleId)
		if strings.Contains(string(body), "selected_paper_summary") {
			content = `{"summary":"Synthetic summary"}`
		}
		fmt.Println(`{"event":"synthetic-ai"}`)
		writer.Header().Set("Content-Type", "application/json")
		json.NewEncoder(writer).Encode(map[string]any{"choices": []any{map[string]any{"message": map[string]string{"content": content}}}})
	})
	push := http.NewServeMux()
	push.HandleFunc("POST /send", func(writer http.ResponseWriter, request *http.Request) {
		if request.Host != "www.pushplus.plus" || request.TLS == nil || request.TLS.ServerName != "www.pushplus.plus" {
			writer.WriteHeader(400)
			return
		}
		var body map[string]any
		if json.NewDecoder(http.MaxBytesReader(writer, request.Body, 1024*1024)).Decode(&body) != nil {
			writer.WriteHeader(400)
			return
		}
		content, hasContent := body["content"].(string)
		title, hasTitle := body["title"].(string)
		if body["token"] != "synthetic-only" || body["template"] != "markdown" || body["channel"] != "wechat" || !hasContent || !hasTitle || title == "" || !strings.Contains(content, "Evidence Graphs for Living Literature Reviews") {
			writer.WriteHeader(400)
			return
		}
		fmt.Println(`{"event":"synthetic-delivery","host":"www.pushplus.plus","path":"/send"}`)
		if os.Getenv("FIXTURE_RESPONSE") == "hold" {
			select {
			case <-request.Context().Done():
			case <-time.After(90 * time.Second):
			}
			return
		}
		if os.Getenv("FIXTURE_RESPONSE") == "lost" {
			connection, _, err := writer.(http.Hijacker).Hijack()
			if err == nil {
				connection.Close()
			}
			return
		}
		writer.Header().Set("Content-Type", "application/json")
		io.WriteString(writer, `{"code":200,"data":"synthetic-message-id"}`)
	})
	errorsChannel := make(chan error, 2)
	for _, service := range []struct {
		port, kind, identity string
		handler              http.Handler
	}{{":8443", "ai", "valid", ai}, {":443", "push", os.Getenv("FIXTURE_CERT"), push}} {
		if service.identity == "" {
			service.identity = "valid"
		}
		certificate, err := tls.LoadX509KeyPair(filepath.Join(root, service.kind, service.identity+".pem"), filepath.Join(root, service.kind, service.identity+"-key.pem"))
		if err != nil {
			return err
		}
		listener, err := tls.Listen("tcp", service.port, &tls.Config{Certificates: []tls.Certificate{certificate}, MinVersion: tls.VersionTLS12})
		if err != nil {
			return err
		}
		defer listener.Close()
		go func() {
			server := &http.Server{Addr: service.port, Handler: service.handler, ReadHeaderTimeout: 5 * time.Second}
			errorsChannel <- server.Serve(listener)
		}()
	}
	fmt.Println(`{"event":"ledger-ready"}`)
	return <-errorsChannel
}
