// The Android side of tauri-plugin-headstate-refresh.
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

/** One window Rust is refreshing in: released by `complete`. */
class RefreshWindow {
    val done = CountDownLatch(1)
    val success = AtomicBoolean(false)
}

@TauriPlugin
class HeadstateRefreshPlugin(private val activity: Activity) : Plugin(activity) {
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
}
