package main

import (
	"context"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"time"

	"github.com/QianFuv/LitRadar/internal/cfp"
	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
	storage "github.com/QianFuv/LitRadar/internal/storage/cfp"
)

func prepareCfp(root string) error {
	marker, err := os.ReadFile(filepath.Join(root, ".litradar-e2e-root"))
	if err != nil || string(marker) != "litradar-full-stack-e2e-v1\n" {
		return errors.New("synthetic root marker required")
	}
	var input struct {
		Config cfp.SourceConfig
		Seed   string
	}
	if err := json.NewDecoder(os.Stdin).Decode(&input); err != nil {
		return err
	}
	var seed domain.Seed
	if err := json.Unmarshal([]byte(input.Seed), &seed); err != nil {
		return err
	}
	repository, err := storage.Open(filepath.Join(root, "data/auth.sqlite"))
	if err != nil {
		return err
	}
	defer repository.Close()
	ctx := context.Background()
	imported, err := cfp.EnsureSeed(ctx, repository)
	if err != nil {
		return err
	}
	now := time.Now().Unix()
	lease, err := repository.BeginRefresh(ctx, input.Config.SourceKey, now, 60)
	if err != nil {
		return err
	}
	err = repository.PublishRefresh(ctx, lease, storage.Publication{
		Capture: input.Seed, CaptureFormat: "synthetic_originals_json",
		SourceUrl: input.Config.DiscoveryUrl, ConfigVersion: input.Config.ConfigVersion,
		Sources: seed.Sources, EmptyJournals: seed.EmptyJournals,
	}, now)
	if err != nil {
		return err
	}
	return json.NewEncoder(os.Stdout).Encode(imported)
}
