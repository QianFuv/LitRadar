package runtime

import (
	"context"
	"errors"
	"log/slog"
	"net"
	"path/filepath"
	"sync"
	"time"

	"github.com/QianFuv/LitRadar/internal/api"
	"github.com/QianFuv/LitRadar/internal/cfp"
	"github.com/QianFuv/LitRadar/internal/platform/executor"
	"github.com/QianFuv/LitRadar/internal/sources"
	"github.com/QianFuv/LitRadar/internal/storage/auth"
	cfpstorage "github.com/QianFuv/LitRadar/internal/storage/cfp"
	"github.com/QianFuv/LitRadar/internal/storage/config"
	"github.com/QianFuv/LitRadar/internal/storage/delivery"
	"github.com/QianFuv/LitRadar/internal/storage/maintenance"
	"github.com/QianFuv/LitRadar/internal/storage/meta"
	authmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/auth"
	indexmigration "github.com/QianFuv/LitRadar/internal/storage/migrations/index"
	"github.com/QianFuv/LitRadar/internal/storage/scheduler"
	"github.com/QianFuv/LitRadar/internal/storage/secrets"
	"github.com/QianFuv/LitRadar/internal/storage/settings"
)

// Prepared owns the listener, API resources and deployment key until all service work drains.
type Prepared struct {
	configuration Config
	services      api.Services
	handler       *api.Handler
	listener      net.Listener
	closeOnce     sync.Once
	closeError    error
}

// PreflightStorage preserves startup migration ordering before any consumer opens storage.
func PreflightStorage(ctx context.Context, configuration config.Config) error {
	if err := maintenance.CheckInterrupted(configuration); err != nil {
		return err
	}
	if _, err := authmigration.Migrate(ctx, configuration.AuthDbPath); err != nil {
		return err
	}
	if _, err := delivery.ImportLegacyFiles(ctx, configuration, float64(time.Now().UnixNano())/1e9); err != nil {
		return err
	}
	return indexmigration.PreflightExisting(ctx, configuration)
}

// Prepare validates and initializes the entire HTTP service before binding its listener.
// Background components start only when the prepared service is run.
func Prepare(ctx context.Context, configuration Config) (*Prepared, error) {
	prepared, err := prepareResources(ctx, configuration)
	if err != nil {
		return nil, err
	}
	prepared.listener, err = net.Listen("tcp", configuration.BindAddress())
	if err != nil {
		_ = prepared.Close()
		return nil, err
	}
	address := prepared.listener.Addr().(*net.TCPAddr)
	family := "ipv6"
	if address.IP.To4() != nil {
		family = "ipv4"
	}
	slog.InfoContext(ctx, "service.listener.ready", "event", "service.listener.ready", "component", "api", "address_family", family, "port", address.Port)
	return prepared, nil
}

func prepareResources(ctx context.Context, configuration Config) (_ *Prepared, err error) {
	if err := configuration.ValidateDevelopment(); err != nil {
		return nil, err
	}
	if err := PreflightStorage(ctx, configuration.Storage); err != nil {
		return nil, err
	}
	if configuration.BundledMetaDir != "" {
		report, err := meta.Prepare(ctx, configuration.Storage, configuration.BundledMetaDir)
		if err != nil {
			return nil, err
		}
		ReportManagedMeta(ctx, report, "api_startup")
	}
	prepared := &Prepared{configuration: configuration, services: api.Services{Storage: configuration.Storage}}
	defer func() {
		if err != nil {
			_ = prepared.Close()
		}
	}()
	prepared.services.Codec, err = secrets.Load(configuration.SecretKeyFile)
	if err != nil {
		return nil, err
	}
	if _, err = secrets.Verify(ctx, configuration.Storage.AuthDbPath, prepared.services.Codec); err != nil {
		return nil, err
	}
	prepared.services.Cfp, err = cfpstorage.Open(configuration.Storage.AuthDbPath)
	if err != nil {
		return nil, err
	}
	if _, err = cfp.EnsureSeed(ctx, prepared.services.Cfp); err != nil {
		return nil, err
	}
	prepared.services.Auth, err = auth.Open(configuration.Storage.AuthDbPath)
	if err != nil {
		return nil, err
	}
	values, err := settings.New(prepared.services.Auth, prepared.services.Codec).Load(ctx)
	if err != nil {
		return nil, err
	}
	if err = configuration.ApplyRuntimeSettings(values); err != nil {
		return nil, err
	}
	configuration.ApiOptions.ProviderProxy, err = sources.ProxySelectionFromRuntime(values)
	if err != nil {
		return nil, err
	}
	configuration.ApiOptions.ContentSecurityPolicy = developmentCsp
	if !configuration.IsDevelopment {
		webRoot := filepath.Join(configuration.Storage.ProjectRoot, "web")
		configuration.ApiOptions.ContentSecurityPolicy, err = loadSecurityPolicy(webRoot)
		if err != nil {
			return nil, err
		}
		configuration.ApiOptions.IsHstsEnabled = configuration.AreSecureCookiesRequired
		configuration.ApiOptions.Frontend = frontend{webRoot}
	}
	prepared.services.Delivery, err = delivery.Open(configuration.Storage.AuthDbPath)
	if err != nil {
		return nil, err
	}
	prepared.services.Scheduler, err = scheduler.Open(configuration.Storage.AuthDbPath)
	if err != nil {
		return nil, err
	}
	prepared.services.StoragePool = executor.New(8, 30*time.Second)
	prepared.services.UpstreamPool = executor.New(4, 30*time.Second)
	prepared.services.KdfPool = executor.New(2, 30*time.Second)
	prepared.handler, err = api.New(prepared.services, configuration.ApiOptions)
	if err != nil {
		return nil, err
	}
	prepared.configuration = configuration
	return prepared, nil
}

// Address returns the actual bound address, including an operating-system-assigned port.
func (prepared *Prepared) Address() net.Addr { return prepared.listener.Addr() }

// Close releases a prepared service after its running components have returned.
// It drains workers even when their request callers have already been cancelled.
func (prepared *Prepared) Close() error {
	prepared.closeOnce.Do(func() {
		if prepared.listener != nil {
			if err := prepared.listener.Close(); err != nil && !errors.Is(err, net.ErrClosed) {
				prepared.closeError = errors.Join(prepared.closeError, err)
			}
		}
		if prepared.handler != nil {
			prepared.closeError = errors.Join(prepared.closeError, prepared.handler.Close())
		}
		pools := []*executor.Pool{prepared.services.StoragePool, prepared.services.UpstreamPool, prepared.services.KdfPool}
		for _, pool := range pools {
			if pool != nil {
				pool.Close()
			}
		}
		for _, pool := range pools {
			if pool != nil {
				pool.Wait()
			}
		}
		if prepared.services.Scheduler != nil {
			prepared.closeError = errors.Join(prepared.closeError, prepared.services.Scheduler.Close())
		}
		if prepared.services.Delivery != nil {
			prepared.closeError = errors.Join(prepared.closeError, prepared.services.Delivery.Close())
		}
		if prepared.services.Cfp != nil {
			prepared.closeError = errors.Join(prepared.closeError, prepared.services.Cfp.Close())
		}
		if prepared.services.Auth != nil {
			prepared.closeError = errors.Join(prepared.closeError, prepared.services.Auth.Close())
		}
		if prepared.services.Codec != nil {
			prepared.services.Codec.Close()
		}
	})
	return prepared.closeError
}

// ReportManagedMeta emits bounded preparation counts without paths or catalog content.
func ReportManagedMeta(ctx context.Context, report meta.Report, label string) {
	counts := map[string]int{}
	for _, catalog := range report.Catalogs {
		counts[catalog.Action]++
	}
	slog.InfoContext(ctx, "storage.managed_meta.prepared", "event", "storage.managed_meta.prepared", "component", "storage", "context", label, "bundle_version", report.BundleVersion, "catalog_count", len(report.Catalogs), "created", counts["created"], "adopted", counts["adopted"], "updated", counts["updated"], "customized", counts["customized"], "unchanged", counts["unchanged"])
}
