package runtime

import (
	"bytes"
	"context"
	"errors"
	"io"
	"log"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"

	"github.com/QianFuv/LitRadar/internal/runtime/observability"
)

func TestHttpPanicsCloseConnectionWithoutPayloadOrStack(t *testing.T) {
	var output, serverErrors bytes.Buffer
	guard, logger, err := observability.New("info", "json", &output)
	if err != nil {
		t.Fatal(err)
	}
	previous := slog.Default()
	slog.SetDefault(logger)
	defer slog.SetDefault(previous)
	server := httptest.NewUnstartedServer(redactHttpPanics(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { panic("synthetic-private-panic-sentinel") })))
	server.Config.ErrorLog = log.New(&serverErrors, "", 0)
	server.Start()
	response, err := server.Client().Get(server.URL)
	if response != nil {
		response.Body.Close()
	}
	server.Close()
	guard.Shutdown()
	if err == nil {
		t.Fatal("panic did not abort the HTTP exchange")
	}
	if serverErrors.Len() != 0 || strings.Contains(output.String(), "synthetic-private") || strings.Contains(output.String(), "goroutine") || !strings.Contains(output.String(), `"event":"process.panicked"`) {
		t.Fatal("panic escaped fixed diagnostic boundary", output.String(), serverErrors.String())
	}
}

func TestServiceLifecycleLogsOnlySuccessfulJoinedShutdown(t *testing.T) {
	for _, name := range []string{"signal", "unexpected", "failure"} {
		t.Run(name, func(t *testing.T) {
			var output bytes.Buffer
			guard, logger, err := observability.New("info", "json", &output)
			if err != nil {
				t.Fatal(err)
			}
			previous := slog.Default()
			slog.SetDefault(logger)
			defer slog.SetDefault(previous)
			ctx, cancel := context.WithCancel(context.Background())
			defer cancel()
			operation := func(ctx context.Context) error { <-ctx.Done(); return nil }
			if name == "signal" {
				cancel()
			} else {
				operation = func(context.Context) error {
					if name == "failure" {
						return errors.New("private failure")
					}
					return nil
				}
			}
			err = coordinate(ctx, []component{{"api", operation}})
			guard.Shutdown()
			text := output.String()
			if name == "signal" {
				if err != nil || !strings.Contains(text, "service.shutdown.requested") || !strings.Contains(text, "service.shutdown.completed") || strings.Contains(text, "service.component.failed") {
					t.Fatal(err, text)
				}
			} else if err == nil || !strings.Contains(text, "service.component.failed") || strings.Contains(text, "service.shutdown.completed") || strings.Contains(text, "private failure") {
				t.Fatal(err, text)
			}
		})
	}
}

func TestHttpAbortHandlerStaysSilent(t *testing.T) {
	var output bytes.Buffer
	server := httptest.NewUnstartedServer(redactHttpPanics(http.HandlerFunc(func(http.ResponseWriter, *http.Request) { panic(http.ErrAbortHandler) })))
	server.Config.ErrorLog = log.New(&output, "", 0)
	server.Start()
	response, err := server.Client().Get(server.URL)
	if response != nil {
		_, _ = io.Copy(io.Discard, response.Body)
		response.Body.Close()
	}
	server.Close()
	if err == nil || output.Len() != 0 {
		t.Fatal("intentional abort became a diagnostic", err, output.String())
	}
}
