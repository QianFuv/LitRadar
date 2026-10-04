package cfp

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"time"

	domain "github.com/QianFuv/LitRadar/internal/domain/cfp"
)

type preparedNotice struct {
	source domain.Source
	notice domain.Notice
}
type preparedJournal struct {
	title, checkedOn string
	aliases          []string
	notices          []preparedNotice
	empty            *domain.EmptyJournal
}

// PreparedSeed owns a validated immutable copy and the exact input-byte digest.
type PreparedSeed struct {
	journals map[string]*preparedJournal
	notices  uint64
	digest   string
}

func (seed *PreparedSeed) ContentHash() string { return seed.digest }
func digest(data []byte) string                { sum := sha256.Sum256(data); return hex.EncodeToString(sum[:]) }

// PrepareSeed validates and normalizes before any write transaction is acquired.
func PrepareSeed(input []byte) (*PreparedSeed, error) {
	var seed domain.Seed
	if err := json.Unmarshal(input, &seed); err != nil {
		return nil, &PayloadError{Cause: err}
	}
	journals, err := prepare(seed)
	if err != nil {
		return nil, err
	}
	result := &PreparedSeed{journals: journals, digest: digest(input)}
	for _, journal := range journals {
		result.notices += uint64(len(journal.notices))
	}
	return result, nil
}
func sortedKeys[V any](values map[string]V) []string {
	keys := make([]string, 0, len(values))
	for key := range values {
		keys = append(keys, key)
	}
	slices.Sort(keys)
	return keys
}

func prepare(seed domain.Seed) (map[string]*preparedJournal, error) {
	if seed.FormatVersion != 1 {
		return nil, invalid("Unsupported CFP input format")
	}
	journals := map[string]*preparedJournal{}
	for _, source := range seed.Sources {
		notice := domain.ParseSource(source)
		if notice == nil {
			return nil, invalid(fmt.Sprintf("Invalid CFP source: %s / %s", source.JournalTitle, source.Title))
		}
		key := source.CatalogIds[0]
		journal := journals[key]
		if journal == nil {
			journal = &preparedJournal{title: source.JournalTitle, checkedOn: source.CheckedOn}
			journals[key] = journal
		}
		journal.aliases = append(journal.aliases, source.CatalogIds...)
		for _, existing := range journal.notices {
			if existing.notice.Id == notice.Id {
				return nil, invalid("Duplicate CFP notice within " + key)
			}
		}
		journal.checkedOn = max(journal.checkedOn, source.CheckedOn)
		journal.notices = append(journal.notices, preparedNotice{source: source, notice: *notice})
	}
	for _, empty := range seed.EmptyJournals {
		isValidDate := validEmptyDate(empty.CheckedOn)
		if len(empty.CatalogIds) == 0 || slices.ContainsFunc(empty.CatalogIds, func(id string) bool { return strings.TrimSpace(id) == "" }) || strings.TrimSpace(empty.JournalTitle) == "" || strings.TrimSpace(empty.SourceStatement) == "" || len(empty.Notices) != 0 || !domain.IsSourceUrl(empty.SourceUrl) || !isValidDate {
			return nil, invalid("Invalid verified-empty CFP statement")
		}
		key := empty.CatalogIds[0]
		if journals[key] != nil {
			return nil, invalid("A CFP journal cannot be both empty and populated in one source snapshot")
		}
		copy := empty
		journals[key] = &preparedJournal{title: empty.JournalTitle, checkedOn: empty.CheckedOn, aliases: append([]string{}, empty.CatalogIds...), empty: &copy}
	}
	owners := map[string]string{}
	count := uint64(0)
	for _, key := range sortedKeys(journals) {
		journal := journals[key]
		slices.Sort(journal.aliases)
		journal.aliases = slices.Compact(journal.aliases)
		for _, alias := range journal.aliases {
			if owner, exists := owners[alias]; exists && owner != key {
				return nil, invalid("Conflicting CFP alias ownership: " + alias)
			}
			owners[alias] = key
		}
		count += uint64(len(journal.notices))
	}
	if seed.ExpectedJournals != nil && *seed.ExpectedJournals != uint64(len(journals)) || seed.ExpectedNotices != nil && *seed.ExpectedNotices != count {
		return nil, invalid("CFP seed counts do not match the reviewed export")
	}
	return journals, nil
}

var emptyDatePattern = regexp.MustCompile(`^[\s\p{Z}\x{85}]*([+-]?[0-9]+)-[\s\p{Z}\x{85}]*([0-9]{1,2})-[\s\p{Z}\x{85}]*([0-9]{1,2})$`)

// validEmptyDate preserves chrono percent-Y parsing without widening populated-source validation.
func validEmptyDate(value string) bool {
	fields := emptyDatePattern.FindStringSubmatch(value)
	if fields == nil {
		return false
	}
	yearText := fields[1]
	if yearText[0] != '+' && yearText[0] != '-' && len(yearText) > 4 {
		return false
	}
	year, err := strconv.Atoi(yearText)
	if err != nil || year < -262143 || year > 262142 {
		return false
	}
	month, _ := strconv.Atoi(fields[2])
	day, _ := strconv.Atoi(fields[3])
	if month < 1 || month > 12 || day < 1 || day > 31 {
		return false
	}
	date := time.Date(year, time.Month(month), day, 0, 0, 0, 0, time.UTC)
	return date.Year() == year && int(date.Month()) == month && date.Day() == day
}
