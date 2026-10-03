// Package maintenance performs confirmed offline index rebuilds with durable recovery markers.
package maintenance

import (
	"errors"
	"fmt"
	"os"

	"github.com/QianFuv/LitRadar/internal/storage/config"
	native "github.com/mattn/go-sqlite3"
)

const markerName = ".litradar-index-maintenance.json"
const stagingName = ".litradar-index-staging"
const rollbackName = ".litradar-index-rollback"

// RecoveryPaths identifies the durable marker and candidate/original directories.
type RecoveryPaths struct {
	Marker   string `json:"marker"`
	Staging  string `json:"staging"`
	Rollback string `json:"rollback"`
}

// Measurement reports physical SQLite allocation without exposing stored content.
type Measurement struct {
	FileBytes        uint64 `json:"file_bytes"`
	PageSize         uint64 `json:"page_size"`
	PageCount        uint64 `json:"page_count"`
	FreelistCount    uint64 `json:"freelist_count"`
	FreelistBytes    uint64 `json:"freelist_bytes"`
	FtsBytes         uint64 `json:"fts_bytes"`
	HasContentShadow bool   `json:"has_content_shadow"`
}

// DatabaseReport records version, allocation and canonical membership after optimization.
type DatabaseReport struct {
	Database            string            `json:"database"`
	SourceSchemaVersion int64             `json:"source_schema_version"`
	TargetSchemaVersion int64             `json:"target_schema_version"`
	Before              Measurement       `json:"before"`
	After               Measurement       `json:"after"`
	RowCounts           map[string]uint64 `json:"row_counts"`
}

// Report summarizes a confirmed no-op or completed directory replacement.
type Report struct {
	Outcome                string           `json:"outcome"`
	DatabaseCount          int              `json:"database_count"`
	SourceBytes            uint64           `json:"source_bytes"`
	TemporaryBytesRequired uint64           `json:"temporary_bytes_required"`
	OptimizedBytes         uint64           `json:"optimized_bytes"`
	ReclaimedBytes         uint64           `json:"reclaimed_bytes"`
	Databases              []DatabaseReport `json:"databases"`
}

// Options requires explicit confirmation before any maintenance writes.
type Options struct {
	Config    config.Config
	Confirmed bool
}

// Failure retains stable CLI classification and recovery paths without serializing database content.
type Failure struct {
	Code, Message string
	Recovery      *RecoveryPaths
	Cause         error
}

// Error returns the operation diagnostic with any required recovery instructions.
func (failure Failure) Error() string { return failure.Message }

// Unwrap preserves the underlying filesystem or SQLite error for classification.
func (failure Failure) Unwrap() error { return failure.Cause }
func invalid(message string) error {
	return Failure{Code: "invalid_layout", Message: "unsafe index directory layout: " + message}
}
func validation(database, check string) error {
	return Failure{Code: "validation_failed", Message: fmt.Sprintf("index database %s failed %s", database, check)}
}
func interrupted(paths RecoveryPaths) error {
	return Failure{Code: "interrupted_state", Recovery: &paths, Message: fmt.Sprintf("interrupted index maintenance requires recovery; marker=%s, staging=%s, rollback=%s", paths.Marker, paths.Staging, paths.Rollback)}
}
func withRecovery(err error, paths RecoveryPaths) error {
	var known Failure
	if errors.As(err, &known) && (known.Code == "rollback_failed" || known.Code == "interrupted_state") {
		return err
	}
	code := "io"
	if errors.As(err, &known) {
		code = known.Code
	} else {
		var sqlite native.Error
		if errors.As(err, &sqlite) {
			code = "sqlite"
		}
	}
	return Failure{Code: "operation_failed", Recovery: &paths, Message: fmt.Sprintf("index maintenance failed during %s: %v; marker=%s, staging=%s, rollback=%s", code, err, paths.Marker, paths.Staging, paths.Rollback)}
}
func rollbackFailure(detail string, paths RecoveryPaths) error {
	return Failure{Code: "rollback_failed", Recovery: &paths, Message: fmt.Sprintf("index maintenance rollback failed: %s; marker=%s, staging=%s, rollback=%s", detail, paths.Marker, paths.Staging, paths.Rollback)}
}
func exists(filename string) (bool, error) {
	_, err := os.Lstat(filename)
	if errors.Is(err, os.ErrNotExist) {
		return false, nil
	}
	return err == nil, err
}

var searchCorpus = []string{`"Genome sequencing"`, `genom*`, `genome NOT preview`, `genome OR clinical`, `title:Clinical`, `journal_title:Alpha`, `authors:Alice`, `doi:"10.1000/genome"`, `pmid:1001`, `"resume"`}

type tableSpec struct{ name, columns, keys string }

var tables = []tableSpec{
	{"journals", "journal_id,catalog_id,title,title_aliases_json,issns_json,issn,eissn,area,utd_rank,utd_rating,abs_rank,abs_rating,fms_rank,fms_rating,fmscn_rank,fmscn_rating", "journal_id"},
	{"journal_identity_keys", "identity_kind,identity_value,canonical_catalog_id", "identity_kind,identity_value"},
	{"issues", "issue_id,journal_id,publication_year,title,volume,number,date", "issue_id"},
	{"articles", "article_id,journal_id,issue_id,title,publication_year,date,authors_json,start_page,end_page,abstract_text,doi,pmid,open_access,in_press", "article_id"},
	{"article_retraction_dois", "article_id,retraction_doi", "article_id,retraction_doi"},
	{"article_identity_keys", "identity_kind,identity_value,article_id", "identity_kind,identity_value"},
	{"article_listing", "article_id,journal_id,issue_id,publication_year,date,open_access,in_press,doi,pmid,area", "article_id"},
	{"article_change_events", "event_id,content_revision,article_id,change_kind,journal_id,issue_id,in_press,created_at", "event_id"},
}
