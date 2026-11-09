package com.wails.app;

import android.content.Intent;
import android.net.Uri;

/**
 * Forwards single sign-on callback links ({@code litradar://oidc/callback}) from the activity into
 * the Go host, which validates the state, exchanges the one-time code, and stores the token. The
 * link never reaches the WebView.
 */
final class LitRadarOidc {
    private static final String CALLBACK_SCHEME = "litradar";
    private static final String CALLBACK_HOST = "oidc";
    private static final String CALLBACK_PATH = "/callback";

    private LitRadarOidc() {
    }

    /**
     * Tells the Go host that the activity was created anew, so any sign-in started by the previous
     * page is treated as interrupted. Call it after the Go library is loaded and before delivering
     * the intents of the new activity.
     */
    static void onActivityCreated() {
        nativeActivityCreated();
    }

    /**
     * Hands the link of a {@code VIEW} intent to the Go host when it is a sign-in callback.
     *
     * @param intent intent delivered to the activity, or {@code null}
     * @return whether the intent carried a sign-in callback link
     */
    static boolean deliver(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction())) {
            return false;
        }
        Uri link = intent.getData();
        if (link == null
                || !CALLBACK_SCHEME.equals(link.getScheme())
                || !CALLBACK_HOST.equals(link.getHost())
                || !CALLBACK_PATH.equals(link.getPath())) {
            return false;
        }
        nativeDeliverCallback(link.toString());
        return true;
    }

    private static native void nativeActivityCreated();

    private static native boolean nativeDeliverCallback(String uri);
}
