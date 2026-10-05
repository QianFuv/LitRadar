package api

import (
	"net/http/httptest"

	"strings"
	"testing"
)

func TestJsonBodyLimitAndOptionalAbsence(t *testing.T) {
	for _, size := range []int{2 * 1024 * 1024, 2*1024*1024 + 1} {
		body := "{}" + strings.Repeat(" ", size-2)
		request := httptest.NewRequest("POST", "/", strings.NewReader(body))
		request.Header.Set("Content-Type", "application/json")
		_, failure := extractBody(request, requestBodies["TokenCreateRequest"], false)
		if size == 2*1024*1024 && failure != nil {
			t.Fatal(failure)
		}
		if size > 2*1024*1024 && (failure == nil || failure.status != 413) {
			t.Fatal("oversize body not rejected")
		}
		request = httptest.NewRequest("POST", "/", strings.NewReader(body))
		_, failure = extractBody(request, requestBodies["AdminInviteCodeCreate"], true)
		if failure != nil {
			t.Fatal(failure)
		}
	}
}
