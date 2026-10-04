package cli

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"time"

	assets "github.com/QianFuv/LitRadar/assets/cfp"
	"github.com/QianFuv/LitRadar/internal/cfp"
	"github.com/QianFuv/LitRadar/internal/index"
	store "github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
)

var cfpUsage = map[string]any{
	"usage":    []string{"litradar cfp import --project-root PATH --input FILE", "litradar cfp refresh --project-root PATH (--db NAME | --catalog-id ID | --all) [--full-text] [--capture-dir PATH] [--resume-captures] [--obscura-path PATH] [--pdftotext-path PATH] [--source-timeout SECONDS] [--timeout SECONDS]"},
	"defaults": map[string]int{"source_timeout": 90, "timeout": 600, "concurrency": 2},
	"fullText": "Recapture stored notices and follow original-title detail links, including snapshot-only sources. Unresolved notices retain their previous originals.",
	"import":   "Additive, validated, immutable seed import. Existing source ownership and online refresh data are preserved.",
}

func runCfp(ctx context.Context, values []string, output io.Writer) error {
	if hasHelp(values) {
		return writeResult(output, cfpUsage)
	}
	args := arguments(slices.Clone(values))
	root, err := args.projectRoot()
	if err != nil {
		return err
	}
	if len(args) == 0 {
		return usageError(cfpUsage)
	}
	command := args[0]
	args = args[1:]
	configuration := config.FromProjectRoot(root)
	if command == "import" {
		result, err := importCfp(ctx, configuration, args)
		if err != nil {
			return err
		}
		return writeResult(output, result)
	}
	if command != "refresh" {
		return usageError(cfpUsage)
	}
	result, isIncomplete, err := refreshCfp(ctx, configuration, args)
	if err != nil {
		return err
	}
	if err := writeResult(output, result); err != nil {
		return err
	}
	if isIncomplete {
		return errors.New("CFP refresh completed with failed or unattempted sources; last-good data was retained")
	}
	return nil
}

func importCfp(ctx context.Context, configuration config.Config, args arguments) (store.ImportResult, error) {
	var empty store.ImportResult
	input, err := args.take("--input")
	if err != nil {
		return empty, err
	}
	if input == nil {
		return empty, usageError(cfpUsage)
	}
	if len(args) != 0 {
		return empty, fmt.Errorf("Unexpected CFP import arguments: %s", strings.Join(args, " "))
	}
	metadata, err := os.Stat(*input)
	if err != nil {
		return empty, err
	}
	if metadata.Size() > 16*1024*1024 {
		return empty, errors.New("CFP input exceeds 16 MiB")
	}
	data, err := os.ReadFile(*input)
	if err != nil {
		return empty, err
	}
	seed, err := store.PrepareSeed(data)
	if err != nil {
		return empty, err
	}
	if _, err := authmigration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		return empty, err
	}
	repository, err := store.Open(configuration.AuthDbPath)
	if err != nil {
		return empty, err
	}
	defer repository.Close()
	identity := assets.SeedId
	if !bytes.Equal(data, assets.Seed()) {
		if _, err := cfp.EnsureSeed(ctx, repository); err != nil {
			return empty, err
		}
		identity = "operator:" + seed.ContentHash()
	}
	return repository.ImportPrepared(ctx, identity, seed)
}

func refreshCfp(ctx context.Context, configuration config.Config, args arguments) (any, bool, error) {
	database, err := args.take("--db")
	if err != nil {
		return nil, false, err
	}
	catalog, err := args.take("--catalog-id")
	if err != nil {
		return nil, false, err
	}
	isAll, isFullText, shouldResume := args.flag("--all"), args.flag("--full-text"), args.flag("--resume-captures")
	capture, err := args.take("--capture-dir")
	if err != nil {
		return nil, false, err
	}
	options := cfp.DefaultRefreshOptions()
	for _, option := range []struct {
		name  string
		value **string
	}{{"--obscura-path", &options.ObscuraPath}, {"--pdftotext-path", &options.PdftotextPath}} {
		value, err := args.take(option.name)
		if err != nil {
			return nil, false, err
		}
		if value != nil {
			*option.value = value
		}
	}
	sourceTimeout, err := args.unsigned("--source-timeout", 90)
	if err != nil {
		return nil, false, err
	}
	timeout, err := args.unsigned("--timeout", 600)
	if err != nil {
		return nil, false, err
	}
	selections := 0
	for _, isSelected := range []bool{database != nil, catalog != nil, isAll} {
		if isSelected {
			selections++
		}
	}
	if len(args) != 0 || selections != 1 || capture != nil && !isFullText || shouldResume && (!isFullText || capture == nil) {
		return nil, false, usageError(cfpUsage)
	}
	if sourceTimeout == 0 || sourceTimeout > 600 || timeout == 0 || timeout > 3600 {
		return nil, false, errors.New("CFP source timeout must be 1..600 seconds and batch timeout 1..3600 seconds")
	}
	options.SourceTimeout, options.OverallTimeout = time.Duration(sourceTimeout)*time.Second, time.Duration(timeout)*time.Second
	sources, err := selectCfpSources(configuration, database, catalog, isAll)
	if err != nil {
		return nil, false, err
	}
	if _, err := authmigration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		return nil, false, err
	}
	repository, err := store.Open(configuration.AuthDbPath)
	if err != nil {
		return nil, false, err
	}
	defer repository.Close()
	counts := map[string]int{}
	if isFullText {
		results, err := cfp.RefreshFullTexts(ctx, repository, sources, options, capture, shouldResume)
		if err != nil {
			return nil, false, err
		}
		var recovered, updated uint64
		for _, result := range results {
			counts[result.Status]++
			recovered += result.Recovered
			updated += result.Updated
		}
		failed := counts["failed"] + counts["partial"]
		return map[string]any{"success": counts["success"], "partial": counts["partial"], "failed": failed, "noNotices": counts["no_notices"], "notAttempted": counts["not_attempted"], "recoveredNotices": recovered, "updatedNotices": updated, "sources": results}, failed+counts["not_attempted"] > 0, nil
	}
	results, err := cfp.RefreshSources(ctx, repository, sources, options)
	if err != nil {
		return nil, false, err
	}
	for _, result := range results {
		counts[result.Status]++
	}
	return map[string]any{"success": counts["success"], "failed": counts["failed"], "unsupported": counts["unsupported"], "notAttempted": counts["not_attempted"], "sources": results}, counts["failed"]+counts["not_attempted"] > 0, nil
}

func selectCfpSources(configuration config.Config, database, catalogId *string, isAll bool) ([]cfp.SourceConfig, error) {
	if isAll {
		return cfp.Registry(), nil
	}
	identities := map[string]bool{}
	hasDatabase := database == nil
	catalogs, err := configuration.ListProviderCatalogs()
	if err != nil {
		return nil, err
	}
	for _, catalog := range catalogs {
		if database != nil && *database != catalog.Stem+".sqlite" || catalog.CsvFilename == nil {
			continue
		}
		hasDatabase = true
		journals, err := index.ReadCatalogCsv(filepath.Join(configuration.MetaDir, *catalog.CsvFilename))
		if err != nil {
			return nil, err
		}
		for _, journal := range journals {
			if catalogId != nil && *catalogId != journal.CatalogId && !slices.Contains(journal.CatalogAliases, *catalogId) {
				continue
			}
			identities[journal.CatalogId] = true
			for _, alias := range journal.CatalogAliases {
				identities[alias] = true
			}
		}
	}
	if !hasDatabase {
		return nil, errors.New("CFP database catalog not found")
	}
	if len(identities) == 0 {
		return nil, errors.New("CFP journal catalog member not found")
	}
	sources := []cfp.SourceConfig{}
	for _, source := range cfp.Registry() {
		for _, id := range source.CatalogIds {
			if identities[id] {
				sources = append(sources, source)
				break
			}
		}
	}
	if len(sources) == 0 && catalogId != nil {
		return nil, errors.New("CFP journal is not yet adapted")
	}
	return sources, nil
}
