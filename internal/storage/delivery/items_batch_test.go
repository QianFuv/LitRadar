package delivery

import (
	"context"
	"errors"
	"fmt"
	"reflect"
	"testing"
)

// TestRunItemBatchPreservesProgressAndRollback retains stored identities in requested order.
func TestRunItemBatchPreservesProgressAndRollback(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	run, err := repository.EnqueueRun(ctx, scheduledRun("batch"))
	if err != nil {
		t.Fatal(err)
	}
	first := RunItemCreate{ItemKind: ItemKindArticle, ItemKey: "first", ArticleId: pointer(int64(1))}
	second := RunItemCreate{ItemKind: ItemKindSubscriber, ItemKey: "second", UserId: pointer(int64(1))}
	initial, err := repository.InsertRunItems(ctx, run.Id, []RunItemCreate{first, second}, 10)
	if err != nil || len(initial) != 2 {
		t.Fatal(initial, err)
	}
	claimed, err := repository.ClaimRun(ctx, run.Id, "owner", 0, 10, 100)
	if err != nil {
		t.Fatal(err)
	}
	item, err := repository.ClaimItem(ctx, run.Id, "owner", claimed.Run.Revision, initial[0].Id, "item-owner", 11, 30)
	if err != nil {
		t.Fatal(err)
	}
	finished, err := repository.FinalizeItem(ctx, item.Id, "item-owner", item.Revision, ItemStatusSucceeded, pointer(`{"delivered":true}`), nil, 12)
	if err != nil {
		t.Fatal(err)
	}
	third := RunItemCreate{ItemKind: ItemKindArticle, ItemKey: "third", ArticleId: pointer(int64(3))}
	records, err := repository.EnsureRunItems(ctx, run.Id, []RunItemCreate{second, third, first}, 20)
	if err != nil || len(records) != 3 {
		t.Fatal(records, err)
	}
	if !reflect.DeepEqual(records[0], initial[1]) || !reflect.DeepEqual(records[2], *finished) || records[1].ItemKey != third.ItemKey || records[1].CreatedAt != 20 || records[1].Status != ItemStatusPending {
		t.Fatal("ensure changed stored progress or requested order", records)
	}
	before, err := repository.ListRunItems(ctx, run.Id)
	if err != nil {
		t.Fatal(err)
	}
	fourth := RunItemCreate{ItemKind: ItemKindArticle, ItemKey: "fourth", ArticleId: pointer(int64(4))}
	changed := first
	changed.ArticleId = pointer(int64(99))
	if records, err := repository.EnsureRunItems(ctx, run.Id, []RunItemCreate{fourth, changed}, 21); !errors.Is(err, ErrConflict) || records != nil {
		t.Fatal("identity conflict was not rejected atomically", records, err)
	}
	if records, err := repository.InsertRunItems(ctx, run.Id, []RunItemCreate{fourth, second}, 21); err == nil || records != nil {
		t.Fatal("duplicate insert was not rejected atomically", records, err)
	}
	after, err := repository.ListRunItems(ctx, run.Id)
	if err != nil || !reflect.DeepEqual(before, after) {
		t.Fatal("failed batch changed durable progress", after, err)
	}
	if _, err := repository.FinalizeRun(ctx, run.Id, "owner", claimed.Run.Revision, RunStatusCompleted, nil, nil, 23); err != nil {
		t.Fatal(err)
	}
	if _, err := repository.EnsureRunItems(ctx, run.Id, nil, 24); !errors.Is(err, ErrConflict) {
		t.Fatal("empty ensure bypassed terminal run admission", err)
	}
	if records, err := repository.InsertRunItems(ctx, run.Id, nil, 24); err != nil || len(records) != 0 {
		t.Fatal("empty insert changed terminal run admission", records, err)
	}
}

// TestRunItemBatchReadbackFailure rolls back earlier inserts and releases statements for retry.
func TestRunItemBatchReadbackFailure(t *testing.T) {
	repository := testRepository(t)
	ctx := context.Background()
	run, err := repository.EnqueueRun(ctx, scheduledRun("readback-batch"))
	if err != nil {
		t.Fatal(err)
	}
	items := []RunItemCreate{{ItemKind: ItemKindArticle, ItemKey: "first"}, {ItemKind: ItemKindArticle, ItemKey: "missing"}}
	if _, err := repository.database.ExecContext(ctx, "CREATE TRIGGER remove_batch_item AFTER INSERT ON delivery_run_items WHEN NEW.item_key='missing' BEGIN DELETE FROM delivery_run_items WHERE id=NEW.id; END"); err != nil {
		t.Fatal(err)
	}
	if records, err := repository.EnsureRunItems(ctx, run.Id, items, 10); !errors.Is(err, ErrNotFound) || records != nil {
		t.Fatal("missing batch readback was ignored", records, err)
	}
	if records, err := repository.ListRunItems(ctx, run.Id); err != nil || len(records) != 0 {
		t.Fatal("readback failure published earlier inserts", records, err)
	}
	if _, err := repository.database.ExecContext(ctx, "DROP TRIGGER remove_batch_item"); err != nil {
		t.Fatal(err)
	}
	if records, err := repository.EnsureRunItems(ctx, run.Id, items, 10); err != nil || len(records) != 2 {
		t.Fatal("batch retry failed", records, err)
	}
}

// BenchmarkRunItemBatch measures new and replayed durable batches without growing the database.
func BenchmarkRunItemBatch(b *testing.B) {
	for _, count := range []int{1, 120, 1000} {
		for _, mode := range []string{"insert", "ensure-new", "ensure-replay"} {
			b.Run(fmt.Sprintf("items=%d/%s", count, mode), func(b *testing.B) {
				repository := testRepository(b)
				ctx := context.Background()
				run, err := repository.EnqueueRun(ctx, scheduledRun("batch"))
				if err != nil {
					b.Fatal(err)
				}
				items := make([]RunItemCreate, count)
				for index := range items {
					items[index] = RunItemCreate{ItemKind: ItemKindArticle, ItemKey: fmt.Sprint(index + 1), ArticleId: pointer(int64(index + 1))}
				}
				if mode == "ensure-replay" {
					if _, err := repository.InsertRunItems(ctx, run.Id, items, 10); err != nil {
						b.Fatal(err)
					}
				}
				b.ReportAllocs()
				for b.Loop() {
					var records []RunItemRecord
					var err error
					if mode == "insert" {
						records, err = repository.InsertRunItems(ctx, run.Id, items, 20)
					} else {
						records, err = repository.EnsureRunItems(ctx, run.Id, items, 20)
					}
					if err != nil || len(records) != count || records[0].ItemKey != items[0].ItemKey || records[count-1].ItemKey != items[count-1].ItemKey {
						b.Fatal("batch result changed", len(records), err)
					}
					if mode != "ensure-replay" {
						b.StopTimer()
						if _, err := repository.database.ExecContext(ctx, "DELETE FROM delivery_run_items WHERE delivery_run_id=?", run.Id); err != nil {
							b.Fatal(err)
						}
						b.StartTimer()
					}
				}
			})
		}
	}
}
