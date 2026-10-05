package index

import (
	"errors"

	"testing"
)

func TestOrderedCommitErrorRetainsPhase(t *testing.T) {
	for _, phase := range []string{"content", "control"} {
		cause := errors.New("failure")
		err := &ContentCheckpointError{Phase: phase, Cause: cause}
		expected := "content commit failed: failure"
		if phase == "control" {
			expected = "sync progress commit failed: failure"
		}
		if err.Error() != expected || !errors.Is(err, cause) {
			t.Fatalf("phase=%s got=%s", phase, err)
		}
	}
}
