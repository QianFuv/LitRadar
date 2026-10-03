package weekly

import (
	"os"
	"syscall"
)

func creationTime(_ string, info os.FileInfo) (Timestamp, bool) {
	attributes, ok := info.Sys().(*syscall.Win32FileAttributeData)
	if !ok {
		return Timestamp{}, false
	}
	ticks := uint64(attributes.CreationTime.HighDateTime)<<32 | uint64(attributes.CreationTime.LowDateTime)
	return Timestamp{int64(ticks/10000000) - 11644473600, uint32(ticks%10000000) * 100}, true
}
