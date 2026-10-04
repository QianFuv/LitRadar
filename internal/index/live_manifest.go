package index

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"log/slog"
	"os"
	"path/filepath"
	"strconv"
	"time"

	jsoncompat "github.com/QianFuv/LitRadar/internal/domain/auth"
	storage "github.com/QianFuv/LitRadar/internal/storage/index"
)

func prepareCatalogManifest(ctx context.Context, config LiveConfig, input storage.CatalogInput, outcome storage.BatchCatalogOutcome) (storage.ManifestIntent, error) {
	content, err := storage.OpenContent(ctx, catalogContentPath(config, input))
	if err != nil {
		return storage.ManifestIntent{}, fmt.Errorf("index database %s cannot be used: %w", catalogContentPath(config, input), err)
	}
	defer content.Close()
	generated := strconv.FormatInt(time.Now().Unix(), 10)
	prepared, err := storage.PrepareContentChangeManifest(ctx, content.Conn, catalogDatabaseName(input), outcome.RunId, generated)
	if err != nil {
		return storage.ManifestIntent{}, err
	}
	return storage.NewManifestIntent(prepared.Payload, prepared.ThroughEventId, catalogManifestPath(input), outcome.RunId, generated)
}
func validateManifestRecovery(input storage.CatalogInput, outcome storage.BatchCatalogOutcome, intent storage.ManifestIntent) error {
	expected := catalogManifestPath(input)
	if intent.RunId != outcome.RunId || intent.Path != expected || outcome.ManifestPath != nil && *outcome.ManifestPath != expected {
		return &storage.BatchError{Kind: "state", Reason: "manifest recovery metadata does not match the frozen catalog"}
	}
	return nil
}
func shouldPublishManifest(config LiveConfig, input storage.CatalogInput, outcome storage.BatchCatalogOutcome, intent storage.ManifestIntent) (bool, error) {
	if intent.ThroughEventId != nil || outcome.ManifestPath != nil {
		return true, nil
	}
	path := filepath.Join(config.ProjectRoot, intent.Path)
	metadata, err := os.Stat(path)
	if errors.Is(err, os.ErrNotExist) {
		return true, nil
	}
	if err != nil {
		return false, err
	}
	if !metadata.Mode().IsRegular() || metadata.Size() > 64*1024*1024 {
		return false, invalidWorker("existing change manifest is not a bounded regular file")
	}
	body, err := os.ReadFile(path)
	if err != nil {
		return false, err
	}
	var manifest map[string]json.RawMessage
	if !jsoncompat.ValidJson(string(body)) {
		return false, invalidWorker("existing change manifest is not valid LitRadar JSON")
	}
	if json.Unmarshal(body, &manifest) != nil {
		return false, invalidWorker("existing change manifest does not match the selected catalog")
	}
	var run, generated, database string
	var summary map[string]json.RawMessage
	if json.Unmarshal(manifest["run_id"], &run) != nil || run == "" || json.Unmarshal(manifest["generated_at"], &generated) != nil || generated == "" || json.Unmarshal(manifest["db_name"], &database) != nil || database != catalogDatabaseName(input) || json.Unmarshal(manifest["summary"], &summary) != nil || summary == nil {
		return false, invalidWorker("existing change manifest does not match the selected catalog")
	}
	return false, nil
}
func publishCatalogManifest(ctx context.Context, config LiveConfig, input storage.CatalogInput, intent storage.ManifestIntent) error {
	content, err := storage.OpenContent(ctx, catalogContentPath(config, input))
	if err != nil {
		return fmt.Errorf("index database %s cannot be used: %w", catalogContentPath(config, input), err)
	}
	defer content.Close()
	history := catalogHistoryDirectory(config, input)
	if intent.ThroughEventId != nil {
		if err := storage.PublishContentChangeHistory(filepath.Join(history, intent.Sha256+".changes.json"), intent.Payload); err != nil {
			return err
		}
	}
	if err := storage.PublishContentChangeManifest(filepath.Join(config.ProjectRoot, intent.Path), intent.Payload); err != nil {
		return err
	}
	if intent.ThroughEventId != nil {
		if _, err := storage.AcknowledgeContentChangeEvents(ctx, content.Conn, *intent.ThroughEventId); err != nil {
			return err
		}
	}
	removed, err := storage.PruneContentChangeHistory(history, time.Now().Unix()-8*24*60*60)
	if err != nil {
		slog.Warn("index.batch.manifest_history_cleanup_failed", "component", "index", "catalog", input.CatalogName)
	} else if removed > 0 {
		slog.Info("index.batch.manifest_history_pruned", "component", "index", "catalog", input.CatalogName, "removed", removed)
	}
	return nil
}
