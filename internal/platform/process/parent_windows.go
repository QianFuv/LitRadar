package process

// ParentEnvironment carries the expected parent on platforms using an explicit watcher.
const ParentEnvironment = "LITRADAR_WORKER_PARENT_PID"

// StartParentGuard relies on the non-inherited, kill-on-close Job owned by the parent on Windows.
func StartParentGuard() (func(), error) { return func() {}, nil }
