package delivery

import (
	"math"
	"runtime"
	"strings"
	"unicode/utf8"

	"github.com/QianFuv/LitRadar/internal/transport"
)

func validateDbName(value string) error {
	hasDrivePrefix := len(value) >= 2 && value[1] == ':' && (value[0] >= 'a' && value[0] <= 'z' || value[0] >= 'A' && value[0] <= 'Z')
	if value == "" || len(value) > 255 || !utf8.ValidString(value) || !strings.HasSuffix(value, ".sqlite") || strings.Contains(value, "/") || runtime.GOOS == "windows" && (strings.Contains(value, `\`) || hasDrivePrefix) || hasControl(value) {
		return invalid("Delivery database name is invalid")
	}
	return nil
}

func hasControl(value string) bool {
	for _, character := range []byte(value) {
		if character < 32 || character == 127 {
			return true
		}
	}
	return false
}

func validateIdentifier(value, detail string) error {
	if value == "" || len(value) > 128 {
		return invalid(detail)
	}
	for _, character := range []byte(value) {
		if !(character >= 'a' && character <= 'z' || character >= 'A' && character <= 'Z' || character >= '0' && character <= '9' || strings.ContainsRune("._-:+", rune(character))) {
			return invalid(detail)
		}
	}
	return nil
}

func validateText(value string, maximum int, detail string) error {
	if value == "" || len(value) > maximum || hasControl(value) || !utf8.ValidString(value) {
		return invalid(detail)
	}
	return nil
}

func validateOptionalText(value *string, maximum int, detail string) error {
	if value != nil {
		return validateText(*value, maximum, detail)
	}
	return nil
}

func validateSymbol(value *string, detail string) error {
	if value == nil {
		return nil
	}
	if *value == "" || len(*value) > 64 {
		return invalid(detail)
	}
	for _, character := range []byte(*value) {
		if !(character >= 'a' && character <= 'z' || character >= '0' && character <= '9' || character == '_') {
			return invalid(detail)
		}
	}
	return nil
}

func validateJson(value string) error {
	if len(value) > 4*1024*1024 {
		return invalid("Delivery JSON exceeds its size limit")
	}
	if _, err := transport.ParseJson([]byte(value)); err != nil {
		return &Error{Kind: "json", cause: err}
	}
	return nil
}

func validateOptionalJson(value *string) error {
	if value != nil {
		return validateJson(*value)
	}
	return nil
}
func validatePositiveId(value int64, detail string) error {
	if value <= 0 {
		return invalid(detail)
	}
	return nil
}
func validateTime(value float64, detail string) error {
	if math.IsNaN(value) || math.IsInf(value, 0) || value < 0 {
		return invalid(detail)
	}
	return nil
}
func validateRevisionTime(revision int64, now float64) error {
	if revision < 0 {
		return invalid("Delivery revision is invalid")
	}
	return validateTime(now, "Delivery update time is invalid")
}
func validateLease(now, seconds float64) error {
	if err := validateTime(now, "Delivery lease time is invalid"); err != nil {
		return err
	}
	if math.IsNaN(seconds) || math.IsInf(seconds, 0) || seconds <= 0 || seconds > 86400 {
		return invalid("Delivery lease duration is invalid")
	}
	return nil
}
func validateRevisionLease(revision int64, now, seconds float64) error {
	if err := validateRevisionTime(revision, now); err != nil {
		return err
	}
	return validateLease(now, seconds)
}

func validateRunCreate(run RunCreate) error {
	if err := validateIdentifier(run.ExternalId, "Delivery external run id is invalid"); err != nil {
		return err
	}
	if err := validateText(run.ScopeKey, 255, "Delivery run scope is invalid"); err != nil {
		return err
	}
	if run.DbName != nil {
		if err := validateDbName(*run.DbName); err != nil {
			return err
		}
	}
	if run.TriggerKind != TriggerKindManual && run.DbName == nil {
		return invalid("Non-manual delivery runs require a database")
	}
	if run.TriggerKind == TriggerKindManual && run.UserId == nil {
		return invalid("Manual delivery runs require a user")
	}
	if run.UserId != nil {
		if err := validatePositiveId(*run.UserId, "Delivery run user id is invalid"); err != nil {
			return err
		}
	}
	if err := validateTime(run.CreatedAt, "Delivery run creation time is invalid"); err != nil {
		return err
	}
	if run.DeadlineAt != nil {
		if err := validateTime(*run.DeadlineAt, "Delivery run deadline is invalid"); err != nil {
			return err
		}
		if *run.DeadlineAt <= run.CreatedAt {
			return invalid("Delivery run deadline must follow creation")
		}
	}
	if !run.Workflow.valid() || !run.TriggerKind.valid() || !run.Mode.valid() {
		return invalid("Delivery run classification is invalid")
	}
	return nil
}

func validateItemCreate(item RunItemCreate) error {
	if err := validateText(item.ItemKey, 512, "Delivery item key is invalid"); err != nil {
		return err
	}
	if item.UserId != nil {
		if err := validatePositiveId(*item.UserId, "Delivery item user id is invalid"); err != nil {
			return err
		}
	}
	if item.ArticleId != nil {
		if err := validatePositiveId(*item.ArticleId, "Delivery item article id is invalid"); err != nil {
			return err
		}
	}
	if !item.ItemKind.valid() {
		return invalid("Delivery item kind is invalid")
	}
	return nil
}

func validateResolutions(reservations []DedupeResolution) error {
	seen := map[int64]bool{}
	for _, reservation := range reservations {
		if err := validatePositiveId(reservation.Id, "Delivery dedupe id is invalid"); err != nil {
			return err
		}
		if reservation.ExpectedRevision < 0 || seen[reservation.Id] {
			return invalid("Delivery dedupe resolutions are invalid")
		}
		seen[reservation.Id] = true
	}
	return nil
}
