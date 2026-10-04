package cli

var indexUsage = map[string]any{
	"usage":             "litradar index --secret-key-file PATH [--project-root PATH] [--auth-db PATH] [--file FILE] [--stop-after FILE] [--workers N] [--processes N] [--issue-batch N] [--timeout N] [--resume|--no-resume] [--update|--no-update] [--full-rescan|--no-full-rescan] [--notify] [--notify-dry-run] [--acknowledge-unknown-notify]",
	"defaults":          map[string]any{"workers": 6, "processes": 3, "issue_batch": 8, "resume": true, "update": false, "full_rescan": false},
	"provider_defaults": map[string]any{"scholarly": map[string]int{"workers": 6, "processes": 3}, "cnki": map[string]int{"workers": 6, "processes": 1}, "generic": map[string]int{"workers": 6, "processes": 1}},
	"modes": map[string]string{
		"concurrency":                "each omitted count is resolved per selected provider; provider_defaults is authoritative; explicit invalid counts are rejected",
		"update":                     "incremental synchronization that publishes a change manifest",
		"full_rescan":                "complete historical synchronization without a change manifest; mutually exclusive with --update",
		"resume":                     "continue only a compatible active project batch; completed batches always start a new independent update",
		"no_resume":                  "abandon the active batch and its owned traversal checkpoints, then start a new batch from committed anchors",
		"file":                       "select and freeze exactly one CSV; it cannot adopt an active all-CSV batch",
		"stop_after":                 "pause after finalizing the named selected CSV; keep the original batch resumable without starting later catalogs",
		"issue_batch":                "legacy active-batch resume metadata; explicit use warns and does not control current Provider concurrency or memory",
		"acknowledge_unknown_notify": "after review, acknowledge an ambiguous notify attempt and resume with a new delivery attempt",
	},
	"limits": map[string]int{"workers_min": 1, "workers_max": 32, "processes_min": 1, "processes_max": 32, "aggregate_max": 32, "scholarly_workers_max": 32, "scholarly_processes_max": 3, "scholarly_aggregate_max": 96},
}
