package delivery

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"errors"
	"math"
	"reflect"
	"time"

	auth "github.com/QianFuv/LitRadar/internal/domain/auth"
	"github.com/QianFuv/LitRadar/internal/recommend"
	store "github.com/QianFuv/LitRadar/internal/storage/delivery"
)

const deliveryLeaseSeconds = 3600.0

type durableRun struct {
	run                     *store.RunRecord
	lease                   *store.LeaseRecord
	checkpoint              *store.CheckpointRecord
	owner                   string
	workflow                store.Workflow
	dbName                  string
	didTakeOverCompetingRun bool
}

func unixNow() float64          { return math.Max(0, float64(time.Now().UnixNano())/1e9) }
func pointer[T any](value T) *T { return &value }

func (run *durableRun) renew(ctx context.Context, repository *store.Repository, now float64) error {
	updated, err := repository.RenewRun(ctx, run.run.Id, run.owner, run.run.Revision, now, deliveryLeaseSeconds)
	if err != nil {
		return err
	}
	run.run = updated
	lease, err := repository.RenewLease(ctx, run.workflow, run.dbName, run.run.Id, run.owner, run.lease.Revision, now, deliveryLeaseSeconds)
	if err != nil {
		return err
	}
	run.lease = lease
	return nil
}

func (run *durableRun) finalizeWithCheckpoint(ctx context.Context, repository *store.Repository, status store.RunStatus, checkpointStatus store.CheckpointStatus, snapshot recommend.Snapshot, completedAt, result, errorCode *string, now float64) error {
	if snapshot.IssueArticleCounts == nil {
		snapshot.IssueArticleCounts = map[string]int64{}
	}
	if snapshot.InpressArticleCounts == nil {
		snapshot.InpressArticleCounts = map[string]int64{}
	}
	encoded, err := auth.EncodeJson(snapshot)
	if err != nil {
		return errors.New("Delivery checkpoint serialization failed")
	}
	return run.finalizeCheckpoint(ctx, repository, status, store.CheckpointUpdate{Status: checkpointStatus, SnapshotJson: string(encoded), LastCompletedRunAt: completedAt, UpdatedAt: now}, result, errorCode)
}

func (run *durableRun) finalizeCheckpoint(ctx context.Context, repository *store.Repository, status store.RunStatus, update store.CheckpointUpdate, result, errorCode *string) error {
	var revision *int64
	if run.checkpoint != nil {
		revision = &run.checkpoint.Revision
	}
	finalized, err := repository.FinalizeRunWithCheckpoint(ctx, run.run.Id, run.owner, run.run.Revision, status, result, errorCode, run.workflow, run.dbName, revision, update, run.lease.Revision)
	if err != nil {
		return err
	}
	run.run, run.checkpoint, run.lease = finalized.Run, finalized.Checkpoint, finalized.Lease
	return nil
}

func (run *durableRun) failBestEffort(ctx context.Context, repository *store.Repository, errorCode string, now float64) {
	recovery, err := repository.ReconcileAfterTakeover(ctx, run.run.Id, run.owner, run.run.Revision, now)
	isAmbiguous := err == nil && (recovery.UnknownItemCount > 0 || recovery.UnknownDedupeCount > 0)
	if run.run.Status.IsActive() {
		status, checkpointStatus := store.RunStatusFailed, store.CheckpointStatusFailed
		if isAmbiguous {
			status, checkpointStatus, errorCode = store.RunStatusUnknown, store.CheckpointStatusUnknown, "ambiguous_delivery"
		}
		update := store.CheckpointUpdate{Status: checkpointStatus, SnapshotJson: "{}", UpdatedAt: now}
		if run.checkpoint != nil {
			update.SnapshotJson = run.checkpoint.SnapshotJson
			update.LastCompletedRunAt = run.checkpoint.LastCompletedRunAt
		}
		if run.finalizeCheckpoint(ctx, repository, status, update, nil, &errorCode) == nil {
			return
		}
		if updated, err := repository.FinalizeRun(ctx, run.run.Id, run.owner, run.run.Revision, status, nil, &errorCode, now); err == nil {
			run.run = updated
		}
	}
	repository.ReleaseLease(ctx, run.workflow, run.dbName, run.run.Id, run.owner, run.lease.Revision, now)
}

func admitDurableRun(ctx context.Context, repository *store.Repository, config RunConfig, userId *int64, externalId string, now float64) (*durableRun, *store.RunRecord, error) {
	var random [16]byte
	if _, err := rand.Read(random[:]); err != nil {
		return nil, nil, err
	}
	owner := "worker-" + hex.EncodeToString(random[:])
	var deadline *float64
	if config.ExecutionControl != nil {
		deadline = pointer(config.ExecutionControl.Deadline())
	}
	admission, err := repository.AdmitRun(ctx, store.RunCreate{ExternalId: externalId, Workflow: config.Workflow, ScopeKey: config.DbName, DbName: &config.DbName, TriggerKind: config.Trigger, Mode: config.Mode, UserId: userId, DeadlineAt: deadline, CreatedAt: now})
	if err != nil {
		return nil, nil, err
	}
	if admission.Kind == "busy" {
		return nil, nil, ErrBusy
	}
	if admission.Run.Status.IsTerminal() {
		return nil, admission.Run, nil
	}
	claimed, didTakeOver, err := claimCandidate(ctx, repository, admission.Run, owner, now)
	if err != nil {
		return nil, nil, err
	}
	if claimed.Status.IsTerminal() {
		return nil, claimed, nil
	}
	if claimed.Mode != config.Mode || claimed.TriggerKind != config.Trigger || !reflect.DeepEqual(claimed.UserId, userId) {
		repository.FinalizeRun(ctx, claimed.Id, owner, claimed.Revision, store.RunStatusFailed, nil, pointer("recovery_context_mismatch"), now)
		return nil, nil, errors.New("Recovered delivery run does not match the current invocation")
	}
	lease, err := repository.AcquireLease(ctx, config.Workflow, config.DbName, claimed.Id, owner, now, deliveryLeaseSeconds)
	if err != nil {
		return nil, nil, err
	}
	if lease.Kind == "busy" {
		repository.FinalizeRun(ctx, claimed.Id, owner, claimed.Revision, store.RunStatusSkipped, nil, pointer("workflow_lease_busy"), now)
		return nil, nil, ErrBusy
	}
	run := &durableRun{run: claimed, lease: lease.Lease, owner: owner, workflow: config.Workflow, dbName: config.DbName, didTakeOverCompetingRun: didTakeOver}
	run.checkpoint, err = repository.LoadCheckpoint(ctx, config.Workflow, config.DbName)
	if err != nil {
		run.failBestEffort(ctx, repository, "checkpoint_load_failed", now)
		return nil, nil, err
	}
	if _, err = repository.ReconcileAfterTakeover(ctx, run.run.Id, owner, run.run.Revision, now); err != nil {
		run.failBestEffort(ctx, repository, "recovery_failed", now)
		return nil, nil, err
	}
	started, err := repository.StartRun(ctx, run.run.Id, owner, run.run.Revision, now)
	if err != nil {
		run.failBestEffort(ctx, repository, "run_start_failed", now)
		return nil, nil, err
	}
	run.run = started
	if config.DedupeRetentionDays > 0 {
		if _, err := repository.CleanupConfirmedDedupe(ctx, config.Workflow, config.DbName, math.Max(0, now-float64(config.DedupeRetentionDays)*86400)); err != nil {
			run.failBestEffort(ctx, repository, "dedupe_cleanup_failed", now)
			return nil, nil, err
		}
	}
	return run, nil, nil
}

func claimCandidate(ctx context.Context, repository *store.Repository, candidate *store.RunRecord, owner string, now float64) (*store.RunRecord, bool, error) {
	claim, err := repository.ClaimRun(ctx, candidate.Id, owner, candidate.Revision, now, deliveryLeaseSeconds)
	if err != nil {
		return nil, false, err
	}
	if claim.Kind == "claimed" || claim.Kind == "unavailable" && claim.Run.Status.IsTerminal() {
		return claim.Run, false, nil
	}
	if claim.Kind != "busy" {
		return nil, false, ErrBusy
	}
	if candidate.Status == store.RunStatusQueued {
		repository.CancelRun(ctx, candidate.Id, candidate.Revision, now)
	}
	active := claim.Run
	if active.LeaseExpiresAt == nil || *active.LeaseExpiresAt > now {
		return nil, false, ErrBusy
	}
	claim, err = repository.ClaimRun(ctx, active.Id, owner, active.Revision, now, deliveryLeaseSeconds)
	if err != nil {
		return nil, false, err
	}
	if claim.Kind == "claimed" || claim.Kind == "unavailable" && claim.Run.Status.IsTerminal() {
		return claim.Run, true, nil
	}
	return nil, false, ErrBusy
}
