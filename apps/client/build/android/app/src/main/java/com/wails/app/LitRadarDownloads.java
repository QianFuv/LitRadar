package com.wails.app;

import android.content.ContentResolver;
import android.content.ContentValues;
import android.content.Context;
import android.net.Uri;
import android.os.Environment;
import android.provider.MediaStore;
import android.util.Base64;
import android.webkit.JavascriptInterface;

import org.json.JSONObject;

import java.io.OutputStream;

/**
 * Saves files handed over by the LitRadar frontend into the public Downloads collection.
 *
 * Exposed to the WebView as {@code window.litradarDownloads}. MediaStore downloads need no
 * storage permission on API 29+, which is the minimum SDK of this app.
 */
public final class LitRadarDownloads {
    private static final int MAX_FILENAME_LENGTH = 200;

    private final Context context;

    /**
     * Creates the interface for one activity.
     *
     * @param context Context whose content resolver receives the downloads.
     */
    public LitRadarDownloads(Context context) {
        this.context = context.getApplicationContext();
    }

    /**
     * Writes one file into Downloads.
     *
     * @param filename Suggested file name; directories are stripped.
     * @param mimeType MIME type of the content.
     * @param base64Data File contents encoded as base64.
     * @return JSON {@code {"ok":true,"name":...}} or {@code {"ok":false,"error":...}}.
     */
    @JavascriptInterface
    public String save(String filename, String mimeType, String base64Data) {
        Uri item = null;
        ContentResolver resolver = context.getContentResolver();
        try {
            byte[] data = Base64.decode(base64Data == null ? "" : base64Data, Base64.DEFAULT);
            String name = sanitizeFilename(filename);
            ContentValues values = new ContentValues();
            values.put(MediaStore.MediaColumns.DISPLAY_NAME, name);
            values.put(MediaStore.MediaColumns.MIME_TYPE,
                    mimeType == null || mimeType.isEmpty() ? "application/octet-stream" : mimeType);
            values.put(MediaStore.MediaColumns.RELATIVE_PATH, Environment.DIRECTORY_DOWNLOADS);
            values.put(MediaStore.MediaColumns.IS_PENDING, 1);
            item = resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values);
            if (item == null) {
                return result(false, "insert failed");
            }
            try (OutputStream output = resolver.openOutputStream(item)) {
                if (output == null) {
                    throw new IllegalStateException("no output stream");
                }
                output.write(data);
            }
            ContentValues published = new ContentValues();
            published.put(MediaStore.MediaColumns.IS_PENDING, 0);
            resolver.update(item, published, null, null);
            return new JSONObject().put("ok", true).put("name", name).toString();
        } catch (Exception error) {
            if (item != null) {
                resolver.delete(item, null, null);
            }
            return result(false, error.getClass().getSimpleName());
        }
    }

    private static String sanitizeFilename(String filename) {
        String name = filename == null ? "" : filename.trim();
        int separator = Math.max(name.lastIndexOf('/'), name.lastIndexOf('\\'));
        if (separator >= 0) {
            name = name.substring(separator + 1);
        }
        name = name.replaceAll("[\\p{Cntrl}:*?\"<>|]", "_");
        if (name.isEmpty() || name.equals(".") || name.equals("..")) {
            name = "download";
        }
        return name.length() > MAX_FILENAME_LENGTH ? name.substring(0, MAX_FILENAME_LENGTH) : name;
    }

    private static String result(boolean ok, String error) {
        try {
            return new JSONObject().put("ok", ok).put("error", error).toString();
        } catch (Exception jsonError) {
            return "{\"ok\":false}";
        }
    }
}
