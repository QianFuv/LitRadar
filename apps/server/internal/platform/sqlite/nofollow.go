package sqlite

import (
	"fmt"
	"net/url"
	"os"
	"path/filepath"
	"sync"
	"unsafe"

	"modernc.org/libc"
	"modernc.org/libc/sys/types"
	sqlite3 "modernc.org/sqlite/lib"
)

var noFollowVfs struct {
	sync.Once
	base uintptr
	err  error
}

// UseNoFollow selects the VFS that refuses symbolic links during path resolution and physical opens.
func UseNoFollow(query url.Values) error {
	if err := registerNoFollowVfs(); err != nil {
		return err
	}
	query.Set("vfs", "litradar-nofollow")
	return nil
}

func registerNoFollowVfs() error {
	noFollowVfs.Do(func() {
		tls := libc.NewTLS()
		defer tls.Close()
		noFollowVfs.base = sqlite3.Xsqlite3_vfs_find(tls, 0)
		if noFollowVfs.base == 0 {
			noFollowVfs.err = fmt.Errorf("SQLite default VFS is unavailable")
			return
		}
		name, err := libc.CString("litradar-nofollow")
		if err != nil {
			noFollowVfs.err = err
			return
		}
		address := libc.Xmalloc(tls, types.Size_t(unsafe.Sizeof(sqlite3.Tsqlite3_vfs{})))
		if address == 0 {
			libc.Xfree(tls, name)
			noFollowVfs.err = fmt.Errorf("allocate SQLite no-follow VFS")
			return
		}
		vfs := vfsAt(address)
		*vfs = *vfsAt(noFollowVfs.base)
		vfs.FpNext = 0
		vfs.FzName = name
		open, fullPathname := noFollowOpen, noFollowFullPathname
		vfs.FxOpen = *(*uintptr)(unsafe.Pointer(&open))
		vfs.FxFullPathname = *(*uintptr)(unsafe.Pointer(&fullPathname))
		if code := sqlite3.Xsqlite3_vfs_register(tls, address, 0); code != sqlite3.SQLITE_OK {
			libc.Xfree(tls, address)
			libc.Xfree(tls, name)
			noFollowVfs.err = fmt.Errorf("register SQLite no-follow VFS: %d", code)
		}
	})
	return noFollowVfs.err
}

func hasSymlink(filename string) bool {
	for filename != "." {
		metadata, err := os.Lstat(filename)
		if err == nil && metadata.Mode()&os.ModeSymlink != 0 {
			return true
		}
		parent := filepath.Dir(filename)
		if parent == filename {
			break
		}
		filename = parent
	}
	return false
}

func noFollowFullPathname(tls *libc.TLS, _ uintptr, filename uintptr, capacity int32, output uintptr) int32 {
	if hasSymlink(libc.GoString(filename)) {
		return sqlite3.SQLITE_CANTOPEN_SYMLINK
	}
	base := vfsAt(noFollowVfs.base)
	function := *(*func(*libc.TLS, uintptr, uintptr, int32, uintptr) int32)(unsafe.Pointer(&base.FxFullPathname))
	code := function(tls, noFollowVfs.base, filename, capacity, output)
	if code == sqlite3.SQLITE_OK_SYMLINK || code == sqlite3.SQLITE_OK && hasSymlink(libc.GoString(output)) {
		return sqlite3.SQLITE_CANTOPEN_SYMLINK
	}
	return code
}

func noFollowOpen(tls *libc.TLS, _ uintptr, filename uintptr, file uintptr, flags int32, outputFlags uintptr) int32 {
	if filename != 0 && hasSymlink(libc.GoString(filename)) {
		return sqlite3.SQLITE_CANTOPEN_SYMLINK
	}
	base := vfsAt(noFollowVfs.base)
	function := *(*func(*libc.TLS, uintptr, uintptr, uintptr, int32, uintptr) int32)(unsafe.Pointer(&base.FxOpen))
	return function(tls, noFollowVfs.base, filename, file, flags|sqlite3.SQLITE_OPEN_NOFOLLOW, outputFlags)
}

// vfsAt views a libc allocation through its byte slice without converting an integer into a Go pointer.
func vfsAt(address uintptr) *sqlite3.Tsqlite3_vfs {
	return (*sqlite3.Tsqlite3_vfs)(unsafe.Pointer(unsafe.SliceData(libc.GoBytes(address, int(unsafe.Sizeof(sqlite3.Tsqlite3_vfs{}))))))
}
