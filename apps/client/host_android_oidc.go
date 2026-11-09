//go:build android && !server

package main

/*
#include <jni.h>
#include <stdlib.h>

static const char* litradarOidcCopyString(JNIEnv* env, jstring value) {
	if (value == NULL) {
		return NULL;
	}
	return (*env)->GetStringUTFChars(env, value, NULL);
}

static void litradarOidcReleaseString(JNIEnv* env, jstring value, const char* chars) {
	if (chars != NULL) {
		(*env)->ReleaseStringUTFChars(env, value, chars);
	}
}
*/
import "C"

// Java_com_wails_app_LitRadarOidc_nativeActivityCreated tells the sign-in
// callbacks that MainActivity was created anew, so sign-ins of the previous
// page count as interrupted.
//
//export Java_com_wails_app_LitRadarOidc_nativeActivityCreated
func Java_com_wails_app_LitRadarOidc_nativeActivityCreated(env *C.JNIEnv, class C.jclass) {
	appLinkOidcCallbacks.Restart()
}

// Java_com_wails_app_LitRadarOidc_nativeDeliverCallback receives a
// litradar://oidc/callback link from MainActivity and hands it to the waiting
// sign-in. It returns JNI_TRUE when a sign-in accepted the link.
//
//export Java_com_wails_app_LitRadarOidc_nativeDeliverCallback
func Java_com_wails_app_LitRadarOidc_nativeDeliverCallback(env *C.JNIEnv, class C.jclass, uri C.jstring) C.jboolean {
	chars := C.litradarOidcCopyString(env, uri)
	if chars == nil {
		return C.JNI_FALSE
	}
	link := C.GoString(chars)
	C.litradarOidcReleaseString(env, uri, chars)
	if appLinkOidcCallbacks.Deliver(link) {
		return C.JNI_TRUE
	}
	return C.JNI_FALSE
}
