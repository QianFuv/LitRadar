package scholarly

import "testing"

// BenchmarkCrossrefAcceptPage measures durable full-page validation and insertion independently of fixture resets.
func BenchmarkCrossrefAcceptPage(b *testing.B) {
	initial := worksetTestState()
	initial.FrozenAt = 1
	initial.CreatedFrom = new(int64(1))
	initial.RootTotal = new(uint64(226))
	initial.Phase = CrossrefPhase{Kind: "collect", Partition: 1, From: 1, Until: 1, Cursor: new("*"), Expected: new(uint64(226))}
	workset := createTestWorkset(b, initial)
	page := worksetTestPage(0, 225, 1, 226)
	b.ReportAllocs()
	for b.Loop() {
		b.StopTimer()
		if err := workset.exec("DELETE FROM works; UPDATE partitions SET received=0,cursor='*',status='cursor' WHERE id=1"); err != nil {
			b.Fatal(err)
		}
		workset.state = initial.Clone()
		b.StartTimer()
		checkpoint, err := workset.Accept(page)
		if err != nil || checkpoint.Phase.Received != 225 || checkpoint.Sequence != initial.Sequence+1 {
			b.Fatal("page admission changed", checkpoint, err)
		}
	}
}
