// Command litradar-fixture executes isolated migration experiments with synthetic inputs.
package main

import (
	"flag"
	"fmt"
	"os"

	"github.com/QianFuv/LitRadar/internal/testkit/primitives"
)

func main() {
	mode := flag.String("mode", "", "certificate, ledger, probe, or helpers")
	directory := flag.String("directory", "/fixtures", "synthetic fixture directory")
	flag.Parse()
	if err := primitives.Run(*mode, *directory); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
