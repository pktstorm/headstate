// The Android side of tauri-plugin-headstate-refresh.
//
// Two jobs, and the second one is only here because of the first: the
// periodic background window (below), and the Wi-Fi multicast lock that
// mDNS discovery needs (#610, at the bottom of this file). The lock
// lives in this plugin because this plugin's AndroidManifest.xml is
// where CHANGE_WIFI_MULTICAST_STATE is declared, and that is where it
// is declared because this is the plugin that already owns Android
// lifecycle. src/multicast.rs records why it is not a plugin of its own.
//
// One unique PeriodicWorkRequest, name `com.pktstorm.headstate.companion.refresh`,
// at WorkManager's fifteen-minute floor, constrained to a connected
// network. It is NOT a stream and must not become one: the worker gets
// a bounded window, hands Rust one window id, waits for Rust to say it
// is done, and returns. The Rust side (src/lib.rs) documents the
// protocol; this file and RefreshWorker.kt match it.
//
// # Lifecycle
//
// - Construction (Rust's `register_android_plugin`, at app start):
//   enqueue the periodic work with ExistingPeriodicWorkPolicy.KEEP, so
//   a second launch neither duplicates nor resets the schedule.
// - `register` (from Rust, right after): keep the channel windows are
//   announced on. It is process-global (a companion-object field)
//   because WorkManager runs the worker in this process but not through
//   this plugin instance.
// - A window: RefreshWorker sends `begin`, waits on a latch that
//   `complete` releases, and turns Rust's answer into the work result.
//
// # When the app is not running
//
// WorkManager may start the process to run the worker, but that does
// not start the Activity, and the Rust side only starts with the
// Activity. The worker then finds no channel and returns success --
// there is nothing to refresh, and the next open reconnects anyway.
//
// This file was written without an Android SDK on the machine and has
// not been compiled; see the note in the PR that added it.

package com.pktstorm.headstate.refresh

import android.app.Activity
import android.content.Context
import android.net.wifi.WifiManager
import androidx.work.BackoffPolicy
import androidx.work.Constraints
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import app.tauri.Logger
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Channel
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CountDownLatch
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong
import java.util.concurrent.TimeUnit

@InvokeArg
class RegisterArgs {
    lateinit var channel: Channel
}

@InvokeArg
class CompleteArgs {
    var id: Long = 0
    var success: Boolean = false
}

@InvokeArg
class AcquireMulticastArgs {
    /** Diagnostic only: what `dumpsys wifi` shows for this lock. */
    lateinit var tag: String
}

/** One window Rust is refreshing in: released by `complete`. */
class RefreshWindow {
    val done = CountDownLatch(1)
    val success = AtomicBoolean(false)
}

@TauriPlugin
class HeadstateRefreshPlugin(private val activity: Activity) : Plugin(activity) {
    /**
     * The Wi-Fi multicast lock held across an mDNS browse (#610), or
     * null when no browse is running.
     *
     * Instance state, not a companion-object field like `channel`:
     * `channel` is process-global because WorkManager runs the worker
     * outside this plugin instance, but the lock is only ever taken by
     * a Rust call that arrives THROUGH this instance, so there is
     * nothing to share.
     */
    private var multicastLock: WifiManager.MulticastLock? = null

    /**
     * Guards `multicastLock`. Rust holds one guard at a time today, but
     * acquire and release arrive as two separate bridge calls and
     * nothing in Tauri promises they land on the same thread.
     */
    private val multicastGate = Any()

    companion object {
        /** Also `TASK_IDENTIFIER` in src/lib.rs and the iOS task identifier. */
        const val WORK_NAME = "com.pktstorm.headstate.companion.refresh"
        /** WorkManager's minimum period. */
        const val PERIOD_MINUTES = 15L

        @Volatile
        var channel: Channel? = null
        val windows = ConcurrentHashMap<Long, RefreshWindow>()
        private val ids = AtomicLong(0)

        /** Announce a window to Rust. Null when Rust is not up. */
        fun begin(): Pair<Long, RefreshWindow>? {
            val channel = channel ?: return null
            val id = ids.incrementAndGet()
            val window = RefreshWindow()
            windows[id] = window
            channel.send(message("begin", id))
            return Pair(id, window)
        }

        /** The OS deadline passed: tell Rust to stop, forget the window. */
        fun expire(id: Long) {
            windows.remove(id)
            channel?.send(message("expire", id))
        }

        private fun message(kind: String, id: Long): JSObject {
            return JSObject().put("kind", kind).put("id", id)
        }

        /**
         * Enqueue the periodic refresh. KEEP: an existing schedule is left
         * alone, so this is safe to call on every launch. The backoff
         * applies after a `retry()`: exponential from the period itself,
         * so a desktop that is off for the night is asked about less and
         * less often, up to WorkManager's five-hour ceiling.
         */
        fun enqueue(context: Context) {
            val constraints = Constraints.Builder()
                .setRequiredNetworkType(NetworkType.CONNECTED)
                .build()
            val request = PeriodicWorkRequestBuilder<RefreshWorker>(PERIOD_MINUTES, TimeUnit.MINUTES)
                .setConstraints(constraints)
                .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, PERIOD_MINUTES, TimeUnit.MINUTES)
                .build()
            WorkManager.getInstance(context.applicationContext)
                .enqueueUniquePeriodicWork(WORK_NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }
    }

    init {
        try {
            enqueue(activity)
            Logger.info("headstate-refresh: periodic refresh enqueued")
        } catch (e: Exception) {
            // The app must start regardless: a phone without background
            // refresh still catches up on resume.
            Logger.error("headstate-refresh: could not enqueue the periodic refresh", e)
        }
    }

    // ---- Commands (called from Rust) ---------------------------------

    /** `{"channel": <Channel>}`: the channel windows are announced on. */
    @Command
    fun register(invoke: Invoke) {
        val args = invoke.parseArgs(RegisterArgs::class.java)
        channel = args.channel
        invoke.resolve()
    }

    /** `{"id": N, "success": bool}`: Rust finished the window's refresh. */
    @Command
    fun complete(invoke: Invoke) {
        val args = invoke.parseArgs(CompleteArgs::class.java)
        windows.remove(args.id)?.let { window ->
            window.success.set(args.success)
            window.done.countDown()
        }
        invoke.resolve()
    }

    // ---- The Wi-Fi multicast lock (#610) -----------------------------
    //
    // Rust calls `acquireMulticast` before an mDNS browse and
    // `releaseMulticast` after, from a `Drop` that runs on every exit
    // path (plugins/headstate-refresh/src/multicast.rs). Without a held
    // lock the Wi-Fi driver filters inbound multicast and the browse
    // finds nothing, silently.
    //
    // The names Rust sends are these method names verbatim: Tauri's
    // Android PluginHandle looks a command up by `method.name` with no
    // case conversion, so renaming either half of a pair silently
    // breaks the call at runtime. (The snake_case spellings in
    // build.rs are ACL permission names, a different namespace.)
    //
    // # setReferenceCounted(false), deliberately
    //
    // The default is reference counting: N acquires need N releases,
    // and an extra release throws RuntimeException ("MulticastLock
    // under-locked"). Not counted here, for two reasons.
    //
    // 1. There is exactly one holder. `discovery.rs`'s browse is
    //    reached only from `Client::rediscover`, which runs one browse
    //    at a time on a blocking task. So a count would never exceed
    //    one and would buy nothing.
    // 2. Counting turns a lost release into a permanent leak. The
    //    release travels over the Tauri bridge, and if that call is
    //    ever lost -- a webview torn down mid-browse, a Rust panic
    //    after the acquire but before the guard is created -- a counted
    //    lock stays held for the life of the process and drains the
    //    battery invisibly. Uncounted, ANY release fully clears it, so
    //    the worst case is one leaked browse's worth rather than
    //    forever, and the next release cleans it up.
    //
    // The cost of not counting is that a hypothetical second concurrent
    // browse would have its lock dropped by the first one's release.
    // That is a browse that finds nothing -- the failure we already
    // tolerate -- not a leak, and it is the direction to fail in.
    //
    // # Why the lock object is not reused across browses
    //
    // A fresh MulticastLock per acquire, released and discarded: a
    // released lock can be re-acquired, but keeping one around means
    // keeping a field that could be left in either state, and this
    // whole issue is about state nobody can see. One object, one
    // acquire, one release.

    /** `{"tag": "..."}`: hold the Wi-Fi multicast lock for a browse. */
    @Command
    fun acquireMulticast(invoke: Invoke) {
        val args = invoke.parseArgs(AcquireMulticastArgs::class.java)
        synchronized(multicastGate) {
            if (multicastLock != null) {
                // Rust holds one guard at a time, so this means a
                // release was lost. Say so: a silently doubled acquire
                // is how a lock ends up held forever.
                Logger.warn("headstate-refresh: multicast lock re-acquired while already held")
                releaseMulticastLocked()
            }
            try {
                val wifi = activity.applicationContext
                    .getSystemService(Context.WIFI_SERVICE) as WifiManager
                val lock = wifi.createMulticastLock(args.tag)
                // See the block comment above.
                lock.setReferenceCounted(false)
                lock.acquire()
                multicastLock = lock
                Logger.debug("headstate-refresh: multicast lock acquired")
                invoke.resolve()
            } catch (e: Exception) {
                // The REJECT is what carries this, not the Logger line:
                // Tauri's Logger is gated on BuildConfig.DEBUG, so
                // everything it prints is invisible in a release build.
                // A reject reaches Rust as an Err, and multicast.rs
                // logs it at warn through the app's own logger, which
                // does write to the log file a user can send us. That
                // is the entire point of #610 -- a browse that finds
                // nothing must never again be indistinguishable from a
                // LAN with no desktop on it -- so do not "simplify"
                // this into a resolve.
                Logger.error("headstate-refresh: could not take the multicast lock", e)
                invoke.reject("could not take the Wi-Fi multicast lock: ${e.message}")
            }
        }
    }

    /** Release the multicast lock. Safe to call when nothing is held. */
    @Command
    fun releaseMulticast(invoke: Invoke) {
        synchronized(multicastGate) {
            releaseMulticastLocked()
        }
        invoke.resolve()
    }

    /** Caller holds `multicastGate`. */
    private fun releaseMulticastLocked() {
        val lock = multicastLock ?: return
        // Cleared FIRST: if `release()` throws, the field must not keep
        // pointing at a lock nobody will ever release again.
        multicastLock = null
        try {
            // Belt and braces. AOSP reaches its "under-locked" throw
            // only on the ref-counted branch, so releasing an unheld
            // uncounted lock is already a no-op -- but that is one
            // framework detail away from being wrong, and a fresh lock
            // is created per acquire, so `isHeld` costs nothing.
            if (lock.isHeld) {
                lock.release()
            }
            Logger.debug("headstate-refresh: multicast lock released")
        } catch (e: Exception) {
            Logger.error("headstate-refresh: could not release the multicast lock", e)
        }
    }
}
