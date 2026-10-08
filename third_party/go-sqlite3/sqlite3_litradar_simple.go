//go:build cgo

package sqlite3

/*
#cgo linux LDFLAGS: ${SRCDIR}/../../target/simple-tokenizer/libsimple.a -lstdc++ -lm
#cgo windows LDFLAGS: ${SRCDIR}/../../target/simple-tokenizer/libsimple.a -lstdc++ -lwinpthread
#ifndef USE_LIBSQLITE3
#include "sqlite3-binding.h"
#else
#include <sqlite3.h>
#endif
#include <stdlib.h>
extern int sqlite3_simple_init(sqlite3*, char**, const void*);
*/
import "C"

import "unsafe"

// RegisterSimple initializes the statically linked tokenizer on this connection without enabling extension loading.
func (connection *SQLiteConn) RegisterSimple() error {
	connection.mu.Lock()
	defer connection.mu.Unlock()
	if connection.db == nil {
		return Error{Code: ErrMisuse, ExtendedCode: ErrNoExtended(ErrMisuse), err: "Simple registration requires an open connection"}
	}
	var message *C.char
	status := C.sqlite3_simple_init(connection.db, &message, nil)
	if message != nil {
		defer C.sqlite3_free(unsafe.Pointer(message))
	}
	if status == C.SQLITE_OK {
		return nil
	}
	description := C.GoString(message)
	if description == "" {
		description = C.GoString(C.sqlite3_errstr(status))
	}
	return Error{Code: ErrNo(status & 0xff), ExtendedCode: ErrNoExtended(status), err: description}
}
