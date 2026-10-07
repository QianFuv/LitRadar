package cli

import (
	"errors"
	"fmt"
	"slices"
	"strconv"
	"strings"

	"github.com/QianFuv/LitRadar/internal/runtime"
	"github.com/QianFuv/LitRadar/internal/storage/meta"
)

const serveUsage = "Usage: litradar serve --secret-key-file PATH [--host HOST] [--port PORT] [--project-root PATH] [--scheduler-interval-seconds N] [--require-secure-cookies] [--development]"

func parseServe(values []string, executable string) (runtime.Config, error) {
	bundle, err := meta.DiscoverPackagedDirectory()
	if err != nil {
		return runtime.Config{}, err
	}
	return parseServeWithBundle(values, executable, bundle)
}

func parseServeWithBundle(values []string, executable, bundle string) (runtime.Config, error) {
	args := arguments(slices.Clone(values))
	var empty runtime.Config
	host, port, err := parseServeAddress(&args)
	if err != nil {
		return empty, err
	}
	root, err := args.projectRoot()
	if err != nil {
		return empty, err
	}
	key, err := args.take("--secret-key-file")
	if err != nil {
		return empty, err
	}
	if key == nil {
		return empty, errors.New("--secret-key-file is required")
	}
	interval, err := parseServeInterval(&args)
	if err != nil {
		return empty, err
	}
	isHardened, isDevelopment := args.flag("--require-secure-cookies"), args.flag("--development")
	if len(args) > 0 {
		return empty, fmt.Errorf("unexpected serve arguments: %s", strings.Join(args, " "))
	}
	configuration, err := runtime.NewConfig(root, host, uint16(port), *key)
	if err != nil {
		return empty, err
	}
	configuration.Executable = executable
	configuration.BundledMetaDir = bundle
	configuration.SchedulerIntervalSeconds = interval
	configuration.IsDevelopment = isDevelopment
	configuration.AreSecureCookiesRequired = isHardened
	return configuration, configuration.ValidateDevelopment()
}

// parseServeAddress retains host spelling and port parsing before deployment arguments.
func parseServeAddress(args *arguments) (string, uint64, error) {
	host := "127.0.0.1"
	value, err := args.take("--host")
	if err != nil {
		return "", 0, err
	}
	if value != nil {
		host = *value
	}
	port := uint64(8000)
	value, err = args.take("--port")
	if err != nil {
		return "", 0, err
	}
	if value != nil {
		port, err = strconv.ParseUint(strings.TrimPrefix(*value, "+"), 10, 16)
		if err != nil {
			return "", 0, fmt.Errorf("invalid serve port: %s", *value)
		}
	}
	return host, port, nil
}

// parseServeInterval preserves the explicit plus-prefixed uint64 grammar and zero rejection.
func parseServeInterval(args *arguments) (uint64, error) {
	interval := uint64(30)
	value, err := args.take("--scheduler-interval-seconds")
	if err != nil {
		return 0, err
	}
	if value != nil {
		interval, err = strconv.ParseUint(strings.TrimPrefix(*value, "+"), 10, 64)
		if err != nil {
			return 0, fmt.Errorf("invalid scheduler interval: %s", *value)
		}
	}
	if interval == 0 {
		return 0, errors.New("--scheduler-interval-seconds must be greater than zero")
	}
	return interval, nil
}
