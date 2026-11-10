package api

import (
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"slices"
	"strconv"
	"strings"
	"sync"
	"sync/atomic"
	"testing"

	"github.com/danielgtaylor/huma/v2"
)

type responseProbe struct {
	api        huma.API
	router     *http.ServeMux
	operations map[string]*huma.Operation
	mutex      sync.Mutex
	observed   map[string]bool
	failures   []string
}

var activeResponseProbe atomic.Pointer[responseProbe]

func observeResponse(request *http.Request, response *httptest.ResponseRecorder) {
	probe := activeResponseProbe.Load()
	if probe == nil || response.Code < 200 || response.Code >= 400 || request.Method == "HEAD" {
		return
	}
	_, pattern := probe.router.Handler(request)
	operation := probe.operations[pattern]
	if operation == nil {
		return
	}
	probe.mutex.Lock()
	defer probe.mutex.Unlock()
	key := operation.OperationID + " " + strconv.Itoa(response.Code)
	contract := operation.Responses[strconv.Itoa(response.Code)]
	if contract == nil {
		probe.failures = append(probe.failures, key+": undocumented success status")
		return
	}
	probe.observed[operation.OperationID] = true
	if len(contract.Content) == 0 {
		return
	}
	media, _, _ := strings.Cut(response.Header().Get("Content-Type"), ";")
	content := contract.Content[media]
	if content == nil {
		probe.failures = append(probe.failures, key+": undocumented media "+media)
		return
	}
	if media != "application/json" {
		return
	}
	var value any
	if err := json.Unmarshal(response.Body.Bytes(), &value); err != nil {
		probe.failures = append(probe.failures, key+": "+err.Error())
		return
	}
	result := huma.ValidateResult{}
	huma.Validate(probe.api.OpenAPI().Components.Schemas, content.Schema, huma.NewPathBuffer(nil, 0), huma.ModeReadFromServer, value, &result)
	for _, failure := range result.Errors {
		probe.failures = append(probe.failures, key+": "+failure.Error())
	}
}

// TestOperationSuccessResponsesMatchGeneratedSchemas validates real workflows against every operation's generated response contract.
func TestOperationSuccessResponsesMatchGeneratedSchemas(t *testing.T) {
	api, err := newDocumentationAPI()
	if err != nil {
		t.Fatal(err)
	}
	document, err := GenerateOpenAPI()
	if err != nil {
		t.Fatal(err)
	}
	var serialized struct {
		Version    string `json:"openapi"`
		Components struct {
			Schemas map[string]json.RawMessage `json:"schemas"`
		} `json:"components"`
	}
	if err := json.Unmarshal(document, &serialized); err != nil {
		t.Fatal(err)
	}
	if serialized.Version != "3.1.0" || len(serialized.Components.Schemas) == 0 {
		t.Fatal("serialized OpenAPI schema registry is absent or has the wrong version")
	}
	for name := range serialized.Components.Schemas {
		if _, exists := api.OpenAPI().Components.Schemas.Map()[name]; !exists {
			t.Fatal("serialized schema has no Go contract", name)
		}
	}
	var encodedDocument any
	if err := json.Unmarshal(document, &encodedDocument); err != nil {
		t.Fatal(err)
	}
	var validateReferences func(any)
	validateReferences = func(value any) {
		switch value := value.(type) {
		case map[string]any:
			if reference, ok := value["$ref"].(string); ok && strings.HasPrefix(reference, "#/components/schemas/") {
				if _, exists := serialized.Components.Schemas[strings.TrimPrefix(reference, "#/components/schemas/")]; !exists {
					t.Fatal("serialized schema reference is missing", reference)
				}
			}
			for _, child := range value {
				validateReferences(child)
			}
		case []any:
			for _, child := range value {
				validateReferences(child)
			}
		}
	}
	validateReferences(encodedDocument)
	probe := &responseProbe{api: api, router: http.NewServeMux(), operations: map[string]*huma.Operation{}, observed: map[string]bool{}}
	for _, path := range api.OpenAPI().Paths {
		for _, operation := range []*huma.Operation{path.Get, path.Post, path.Put, path.Delete, path.Patch} {
			if operation == nil {
				continue
			}
			pattern := operation.Method + " " + operation.Path
			probe.router.HandleFunc(pattern, func(http.ResponseWriter, *http.Request) {})
			probe.operations[pattern] = operation
			for _, response := range operation.Responses {
				for _, media := range response.Content {
					if media.Schema != nil {
						media.Schema.PrecomputeMessages()
					}
				}
			}
		}
	}
	for _, schema := range api.OpenAPI().Components.Schemas.Map() {
		schema.PrecomputeMessages()
	}
	activeResponseProbe.Store(probe)
	t.Cleanup(func() { activeResponseProbe.Store(nil) })
	for _, workflow := range []struct {
		name string
		run  func(*testing.T)
	}{
		{"auth", TestAuthRouteLifecycleAndNumericUserIdentity},
		{"native_auth", TestNativeLoginPreservesWebSessionAndSupportsBearerLifecycle},
		{"email", TestEmailHttpContract},
		{"oidc", TestNativeOidcHttpFlow},
		{"admin_users", TestAdministratorRoutesEnforceRolesSelfProtectionAndNumericIds},
		{"admin_invites", TestAdministratorInviteOptionalBodyAndRevocation},
		{"admin_announcements", TestAdministratorAnnouncementsValidateBeforeLookupAndAuditChanges},
		{"admin_tasks", TestAdministratorScheduledTaskValidationAndLifecycle},
		{"admin_settings", TestAdministratorRuntimeSettingsValidateRawCapabilitiesAndMaskSecrets},
		{"admin_telemetry", TestAdminTelemetryAuthorizationAndShape},
		{"public", TestPublicHealthAndAnnouncementsRemainUnauthenticated},
		{"favorites", TestFavoriteLifecycleOwnerIsolationAndExport},
		{"graphs", TestRelationGraphFavoriteLifecycle},
		{"cfp", TestCfpRoutesReadCatalogWithoutIndexAndContinueAliasCursor},
		{"cnki", TestCnkiRoutesPreserveLifecycleAndClassifiedErrors},
		{"tracking_settings", TestTrackingSettingsPreserveSecretIntentAndMaskedProjection},
		{"manual_push", TestManualPushDurableAdmissionOwnershipAndCancellation},
		{"manual_acknowledgement", TestManualUnknownAcknowledgementRemainsOwnerOnlyAndAtomic},
		{"additional_auth", exerciseAdditionalAuthResponses},
		{"index", exerciseIndexResponses},
		{"additional_favorites", exerciseAdditionalFavoriteResponses},
		{"additional_tracking", exerciseAdditionalTrackingResponses},
		{"article_access", exerciseArticleResponses},
		{"oidc_link", exerciseOIDCLinkResponses},
	} {
		t.Run(workflow.name, workflow.run)
	}
	probe.mutex.Lock()
	defer probe.mutex.Unlock()
	for _, failure := range probe.failures {
		t.Error(failure)
	}
	missing := []string{}
	for _, operation := range probe.operations {
		if !probe.observed[operation.OperationID] {
			missing = append(missing, operation.OperationID)
		}
	}
	slices.Sort(missing)
	if len(missing) > 0 {
		t.Errorf("operations without an actual successful response: %s", strings.Join(missing, ", "))
	}
	t.Logf("validated successful responses for %d of %d operations", len(probe.observed), len(probe.operations))
}
