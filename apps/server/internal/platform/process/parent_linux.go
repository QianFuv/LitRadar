package process

import (
	"fmt"
	"os"
	"strconv"
	"sync"
	"syscall"
	"time"
)

// ParentEnvironment carries the expected immediate parent for private same-binary workers.
const ParentEnvironment = "LITRADAR_WORKER_PARENT_PID"

// StartParentGuard validates ownership before input and independently watches for parent death.
// The returned stop function joins the watcher without killing a normally stopping worker.
func StartParentGuard() (func(), error) {
	value, exists := os.LookupEnv(ParentEnvironment)
	if !exists {
		return nil, fmt.Errorf("worker parent ownership is missing")
	}
	parent, err := strconv.Atoi(value)
	group, groupError := syscall.Getpgid(0)
	if err != nil || parent <= 0 || parent > 2147483647 || groupError != nil || group != os.Getpid() {
		return nil, fmt.Errorf("worker parent ownership is invalid")
	}
	if os.Getppid() != parent {
		syscall.Kill(-os.Getpid(), syscall.SIGKILL)
		os.Exit(1)
	}
	stop, done := make(chan struct{}), make(chan struct{})
	var once sync.Once
	go func() {
		defer close(done)
		ticker := time.NewTicker(100 * time.Millisecond)
		defer ticker.Stop()
		for {
			select {
			case <-stop:
				return
			case <-ticker.C:
				if os.Getppid() != parent {
					syscall.Kill(-os.Getpid(), syscall.SIGKILL)
					os.Exit(1)
				}
			}
		}
	}()
	return func() { once.Do(func() { close(stop) }); <-done }, nil
}
