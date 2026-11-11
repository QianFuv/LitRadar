package runtime

import (
	"context"
	"os"
	"testing"
	"time"

	"github.com/QianFuv/LitRadar/apps/server/internal/storage/settings"
	"github.com/QianFuv/LitRadar/apps/server/internal/telemetry"
)

// TestTelemetryRuntimePolicy checks defaults and restart-applied persisted values.
func TestTelemetryRuntimePolicy(t *testing.T) {
	configuration := runtimeConfiguration(t)
	if !configuration.IsTelemetryEnabled || configuration.TelemetryRetentionDays != 90 {
		t.Fatal(configuration)
	}
	if err := configuration.ApplyRuntimeSettings([]settings.Value{{Field: "telemetry_enabled", Value: "off"}, {Field: "telemetry_retention_days", Value: "7"}}); err != nil {
		t.Fatal(err)
	}
	if configuration.IsTelemetryEnabled || configuration.TelemetryRetentionDays != 7 {
		t.Fatal(configuration)
	}
	for _, value := range []string{"0", "3651", "bad"} {
		if err := configuration.ApplyRuntimeSettings([]settings.Value{{Field: "telemetry_retention_days", Value: value}}); err == nil {
			t.Fatal(value)
		}
	}
}

// TestTelemetryDrainsOnPreparedClose verifies the resource owner persists pending requests at shutdown.
func TestTelemetryDrainsOnPreparedClose(t *testing.T) {
	configuration := runtimeConfiguration(t)
	prepared, err := prepareResources(context.Background(), configuration)
	if err != nil {
		t.Fatal(err)
	}
	prepared.services.Telemetry.Mcp("list_databases", false, time.Millisecond)
	if err := prepared.Close(); err != nil {
		t.Fatal(err)
	}
	collector, err := telemetry.Open(context.Background(), configuration.Storage.TelemetryDbPath(), 90)
	if err != nil {
		t.Fatal(err)
	}
	defer collector.Close()
	now := time.Now().UTC().Truncate(time.Hour)
	report, err := collector.Read(context.Background(), now, now.Add(time.Hour), "mcp")
	if err != nil || len(report.Series) != 1 || report.Series[0].Calls != 1 {
		t.Fatal(report, err)
	}
}

// TestDisabledTelemetryDoesNotCreateDatabase verifies persisted settings disable recording and I/O.
func TestDisabledTelemetryDoesNotCreateDatabase(t *testing.T) {
	configuration := runtimeConfiguration(t)
	ctx := context.Background()
	if err := prepareRuntimeStorage(ctx, configuration); err != nil {
		t.Fatal(err)
	}
	bootstrap := &Prepared{configuration: configuration}
	if err := bootstrap.prepareRuntimeCatalogs(ctx, configuration); err != nil {
		t.Fatal(err)
	}
	value := "false"
	if _, err := settings.New(bootstrap.services.Auth, bootstrap.services.Codec, true).Update(ctx, nil, map[string]*string{"telemetry_enabled": &value}, nil, nil); err != nil {
		t.Fatal(err)
	}
	if err := bootstrap.Close(); err != nil {
		t.Fatal(err)
	}
	prepared, err := prepareResources(ctx, configuration)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := prepared.Close(); err != nil {
			t.Errorf("prepared.Close: %v", err)
		}
	}()
	if prepared.services.Telemetry != nil {
		t.Fatal("disabled collector opened")
	}
	if _, err := os.Stat(configuration.Storage.TelemetryDbPath()); !os.IsNotExist(err) {
		t.Fatal("disabled telemetry created a database", err)
	}
}

// TestTelemetryOpenFailureDoesNotFailService checks best-effort admission of optional load storage.
func TestTelemetryOpenFailureDoesNotFailService(t *testing.T) {
	configuration := runtimeConfiguration(t)
	if err := os.MkdirAll(configuration.Storage.TelemetryDbPath(), 0700); err != nil {
		t.Fatal(err)
	}
	prepared, err := prepareResources(context.Background(), configuration)
	if err != nil {
		t.Fatal(err)
	}
	defer func() {
		if err := prepared.Close(); err != nil {
			t.Errorf("prepared.Close: %v", err)
		}
	}()
	if prepared.services.Telemetry != nil || prepared.handler == nil {
		t.Fatal("telemetry storage failure affected service admission")
	}
}
