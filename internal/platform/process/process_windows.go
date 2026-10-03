package process

import (
	"context"
	"errors"
	"fmt"
	"os/exec"
	"syscall"
	"time"
	"unsafe"

	"golang.org/x/sys/windows"
)

type nativeTree struct {
	job       windows.Handle
	processes []windows.Handle
}

func prepareTree(command *exec.Cmd) (*nativeTree, error) {
	job, err := windows.CreateJobObject(nil, nil)
	if err != nil {
		return nil, err
	}
	limits := windows.JOBOBJECT_EXTENDED_LIMIT_INFORMATION{}
	limits.BasicLimitInformation.LimitFlags = windows.JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
	if _, err = windows.SetInformationJobObject(job, windows.JobObjectExtendedLimitInformation, uintptr(unsafe.Pointer(&limits)), uint32(unsafe.Sizeof(limits))); err != nil {
		windows.CloseHandle(job)
		return nil, err
	}
	command.SysProcAttr = &syscall.SysProcAttr{CreationFlags: windows.CREATE_SUSPENDED | windows.CREATE_NO_WINDOW, HideWindow: true}
	return &nativeTree{job: job}, nil
}

func (tree *nativeTree) attach(command *exec.Cmd, hook startHook) error {
	if hook != nil {
		if err := hook("before_assignment", command); err != nil {
			return err
		}
	}
	process, err := windows.OpenProcess(windows.PROCESS_SET_QUOTA|windows.PROCESS_TERMINATE, false, uint32(command.Process.Pid))
	if err != nil {
		return err
	}
	defer windows.CloseHandle(process)
	if err = windows.AssignProcessToJobObject(tree.job, process); err != nil {
		return err
	}
	if hook != nil {
		if err := hook("before_resume", command); err != nil {
			return err
		}
	}
	snapshot, err := windows.CreateToolhelp32Snapshot(windows.TH32CS_SNAPTHREAD, 0)
	if err != nil {
		return err
	}
	defer windows.CloseHandle(snapshot)
	entry := windows.ThreadEntry32{Size: uint32(unsafe.Sizeof(windows.ThreadEntry32{}))}
	err = windows.Thread32First(snapshot, &entry)
	hasResumed := false
	for err == nil {
		if entry.OwnerProcessID == uint32(command.Process.Pid) {
			thread, openError := windows.OpenThread(windows.THREAD_SUSPEND_RESUME, false, entry.ThreadID)
			if openError != nil {
				return openError
			}
			previous, resumeError := windows.ResumeThread(thread)
			windows.CloseHandle(thread)
			if resumeError != nil {
				return resumeError
			}
			if previous == 1 {
				hasResumed = true
			}
		}
		err = windows.Thread32Next(snapshot, &entry)
	}
	if !errors.Is(err, windows.ERROR_NO_MORE_FILES) {
		return err
	}
	if !hasResumed {
		return fmt.Errorf("no suspended primary thread")
	}
	if hook != nil {
		return hook("after_resume", command)
	}
	return nil
}

func (tree *nativeTree) retainProcesses() error {
	buffer := make([]uintptr, 65)
	deadline := time.Now().Add(5 * time.Second)
	for {
		err := windows.QueryInformationJobObject(tree.job, windows.JobObjectBasicProcessIdList, uintptr(unsafe.Pointer(&buffer[0])), uint32(len(buffer))*uint32(unsafe.Sizeof(buffer[0])), nil)
		if errors.Is(err, windows.ERROR_MORE_DATA) {
			if time.Now().After(deadline) {
				return context.DeadlineExceeded
			}
			buffer = make([]uintptr, len(buffer)*2)
			continue
		}
		if err != nil {
			return err
		}
		count := (*[2]uint32)(unsafe.Pointer(&buffer[0]))[1]
		for _, pid := range buffer[1 : 1+int(count)] {
			handle, err := windows.OpenProcess(windows.SYNCHRONIZE, false, uint32(pid))
			if errors.Is(err, windows.ERROR_INVALID_PARAMETER) {
				continue
			}
			if err != nil {
				return err
			}
			tree.processes = append(tree.processes, handle)
		}
		return nil
	}
}

func (tree *nativeTree) kill() error {
	err := tree.retainProcesses()
	return errors.Join(err, windows.TerminateJobObject(tree.job, 1))
}

func (tree *nativeTree) close() error {
	var result error
	for _, handle := range tree.processes {
		result = errors.Join(result, windows.CloseHandle(handle))
	}
	tree.processes = nil
	return errors.Join(result, windows.CloseHandle(tree.job))
}

func (tree *nativeTree) terminate(time.Duration) (Termination, error) {
	information := jobAccountingInformation{}
	err := windows.QueryInformationJobObject(tree.job, windows.JobObjectBasicAccountingInformation, uintptr(unsafe.Pointer(&information)), uint32(unsafe.Sizeof(information)), nil)
	if err == nil && information.ActiveProcesses == 0 {
		return AlreadyExited, nil
	}
	return Forced, errors.Join(err, tree.kill())
}

type jobAccountingInformation struct {
	TotalUserTime             int64
	TotalKernelTime           int64
	ThisPeriodTotalUserTime   int64
	ThisPeriodTotalKernelTime int64
	TotalPageFaultCount       uint32
	TotalProcesses            uint32
	ActiveProcesses           uint32
	TotalTerminatedProcesses  uint32
}

func (tree *nativeTree) waitEmpty(ctx context.Context) error {
	ticker := time.NewTicker(10 * time.Millisecond)
	defer ticker.Stop()
	for {
		information := jobAccountingInformation{}
		if err := windows.QueryInformationJobObject(tree.job, windows.JobObjectBasicAccountingInformation, uintptr(unsafe.Pointer(&information)), uint32(unsafe.Sizeof(information)), nil); err != nil {
			return err
		}
		if information.ActiveProcesses == 0 {
			hasPending := false
			for _, handle := range tree.processes {
				state, err := windows.WaitForSingleObject(handle, 0)
				if err != nil {
					return err
				}
				if state != windows.WAIT_OBJECT_0 {
					hasPending = true
				}
			}
			if !hasPending {
				return nil
			}
		}
		select {
		case <-ctx.Done():
			return ctx.Err()
		case <-ticker.C:
		}
	}
}
