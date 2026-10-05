package index

import (
	"fmt"

	"strings"
	"testing"
)

func TestWorkerDiagnosticsExcludeOpaqueState(t *testing.T) {
	secret := "private-token-and-cursor"
	values := []any{WorkerBootstrap{CnkiCaptchaToken: &secret, ProviderProxyUrl: &secret, ScholarlyWorksetDir: &secret}, WorkerAssignment{CommittedAnchor: &secret, TraversalCheckpoint: &secret}, WorkerRequest{Assignments: []WorkerAssignment{{CommittedAnchor: &secret}}}}
	for _, value := range values {
		for _, format := range []string{"%v", "%+v", "%#v"} {
			if strings.Contains(fmt.Sprintf(format, value), secret) {
				t.Fatalf("leaked with %s", format)
			}
		}
	}
}
