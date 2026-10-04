// Command litradar-fixture executes isolated migration experiments with synthetic inputs.
package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"slices"

	"github.com/QianFuv/LitRadar/internal/testkit/fullstack"
	"github.com/QianFuv/LitRadar/internal/testkit/primitives"
)

func main() {
	if slices.Contains(os.Args[1:], "--project-root") || slices.Contains(os.Args[1:], "--refresh-cfp") {
		if err := fullstack.Run(context.Background(), os.Args[1:], os.Stdout); err != nil {
			fmt.Fprintln(os.Stderr, "full-stack fixture seeding failed:", err)
			os.Exit(1)
		}
		return
	}
	mode := flag.String("mode", "", "certificate, ledger, probe, or helpers")
	directory := flag.String("directory", "/fixtures", "synthetic fixture directory")
	flag.Parse()
	if err := primitives.Run(*mode, *directory); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
