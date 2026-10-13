package ai.storyteller.photocraft;

import android.app.NativeActivity;
import android.content.Intent;
import android.net.Uri;
import android.os.Bundle;

/**
 * PhotoCraft's Activity: {@code android.app.NativeActivity} plus the two things a purely
 * native Activity has no way to see.
 *
 * <p><b>Why this class exists.</b> {@code NativeActivity} forwards the lifecycle it declares,
 * and that list has no {@code onActivityResult}: the framework declares no such native method,
 * so the VM never looks for {@code ANativeActivity_onActivityResult} (the NDK's C glue defines
 * that symbol, and nothing ever calls it). An app packaged as a NativeActivity which starts an
 * activity for a result — which is every Storage Access Framework picker — is never told the
 * answer. This subclass is the whole bridge: it hands the result to {@code libphotocraft.so},
 * which reads it through JNI (see {@code apps/photocraft-android/src/activity.rs}).
 *
 * <p>It also reports pause and resume, and hands over a file the user opened us with from a file
 * manager or the gallery, so PhotoCraft appears in *Open with*. None of that is possible from a
 * purely native Activity: those callbacks are not in the framework's list either.
 *
 * <p>Nothing else here changes: the native library is still loaded from
 * {@code android.app.lib_name} and {@code android_main} still runs on its own thread.
 */
public class PickerActivity extends NativeActivity {

    /**
     * Loads this app's own library, and it has to happen here rather than leaving it to
     * {@code NativeActivity.onCreate}.
     *
     * <p>{@code NativeActivity} loads the library with {@code System.load(path)}, and
     * {@code System.load} registers a native library against the class loader of the class that
     * called it — which for {@code NativeActivity} is the <em>boot</em> loader. The VM resolves a
     * native method against the loader of the class that declares it, and this class comes from
     * the app's own {@code classes.dex}, so it would look in the app's loader, find nothing there
     * and throw {@code UnsatisfiedLinkError: No implementation found for void
     * ai.storyteller.photocraft.PickerActivity.onPickerResume()}.
     *
     * <p>Loading it from a class of ours registers it with the app's loader as well, and the same
     * library file is only {@code dlopen}ed once either way. The block runs before the system
     * constructs this Activity, so the library is in place before anything calls into it.
     *
     * <p>The name is the {@code [lib] name} in Cargo.toml, which is also what the
     * {@code android.app.lib_name} meta-data points at.
     */
    static {
        System.loadLibrary("photocraft");
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);
        // The launch may *be* a file: "Open with PhotoCraft" from a file manager, or the gallery's
        // share/open. `onNewIntent` is not involved in that case, only in the ones below.
        offer(getIntent());
    }

    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        // `launchMode` is `singleTask`, so a second file opens *this* Activity instead of making a
        // second one: the intent arrives here, on the Java main thread, while the window is already
        // up. `setIntent` keeps `getIntent()` pointing at the newest one.
        setIntent(intent);
        offer(intent);
    }

    /**
     * Hands a file the user opened us with to the native side.
     *
     * <p>Only {@code ACTION_VIEW} with data, and only once: the native side queues the URI and
     * reads it on a worker, because a large PSD must not be read on the UI thread. Anything else
     * the launcher sends — {@code ACTION_MAIN}, a share target with no data — is ignored.
     */
    private void offer(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction())) {
            return;
        }
        Uri data = intent.getData();
        if (data == null) {
            return;
        }
        onOpenWith(data.toString());
    }

    @Override
    protected void onActivityResult(int requestCode, int resultCode, Intent data) {
        super.onActivityResult(requestCode, resultCode, data);
        // `data` is null when the user backed out; the native side reads that as "no file".
        onPickerResult(requestCode, resultCode, data);
    }

    @Override
    protected void onPause() {
        super.onPause();
        onPickerPause();
    }

    @Override
    protected void onResume() {
        super.onResume();
        onPickerResume();
    }

    /** A file to open, as its {@code content://} (or {@code file://}) URI. See {@code activity.rs}. */
    private static native void onOpenWith(String uri);

    /** A picker result. See {@code ANativeActivity_onActivityResult} in {@code activity.rs}. */
    private static native void onPickerResult(int requestCode, int resultCode, Intent data);

    /** The Activity is no longer in front: a picker may have just been opened. */
    private static native void onPickerPause();

    /** The Activity is in front again. See the class comment. */
    private static native void onPickerResume();
}