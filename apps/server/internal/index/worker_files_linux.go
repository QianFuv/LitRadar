package index

import "syscall"

func removeWorkerFile(path string) error { return syscall.Unlink(path) }
