package scholarly

import "testing"

// BenchmarkCrossrefEmitPage measures read-only payload pagination after complete durable collection.
func BenchmarkCrossrefEmitPage(b *testing.B) {
	for _, scenario := range []struct {
		name  string
		count int
	}{{"single", 1}, {"full", 226}} {
		b.Run(scenario.name, func(b *testing.B) {
			initial := worksetTestState()
			initial.FrozenAt = 1
			initial.CreatedFrom = new(int64(1))
			initial.RootTotal = new(uint64(scenario.count))
			initial.Phase = CrossrefPhase{Kind: "collect", Partition: 1, From: 1, Until: 1, Cursor: new("*"), Expected: new(uint64(scenario.count))}
			if scenario.count == 1 {
				initial.Phase.Cursor = nil
			}
			workset := createTestWorkset(b, initial)
			for start := 0; start < scenario.count; start += 225 {
				if _, err := workset.Accept(worksetTestPage(start, min(225, scenario.count-start), 1, uint64(scenario.count))); err != nil {
					b.Fatal(err)
				}
			}
			if workset.state.Phase.Kind != "ready" {
				b.Fatal("collection never reached ready state")
			}
			b.ReportAllocs()
			for b.Loop() {
				page, err := workset.Emit("9999-12-31", nil, nil)
				if err != nil || len(page.Works) != min(225, scenario.count) || page.After == nil || page.HasMore != (scenario.count > 225) {
					b.Fatal("emission page changed", page, err)
				}
			}
		})
	}
}
