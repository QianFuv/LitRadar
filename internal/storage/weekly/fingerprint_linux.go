package weekly

import (
	"os"

	"golang.org/x/sys/unix"
)

func creationTime(filename string, _ os.FileInfo) (Timestamp, bool) {
	var information unix.Statx_t
	if unix.Statx(unix.AT_FDCWD, filename, unix.AT_STATX_SYNC_AS_STAT, unix.STATX_BTIME, &information) != nil || information.Mask&unix.STATX_BTIME == 0 {
		return Timestamp{}, false
	}
	return Timestamp{information.Btime.Sec, information.Btime.Nsec}, true
}
