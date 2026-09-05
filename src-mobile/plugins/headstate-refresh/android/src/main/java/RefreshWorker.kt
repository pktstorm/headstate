// The worker WorkManager runs in each window. See HeadstateRefreshPlugin.kt
// for the lifecycle and the protocol with Rust.
//
// A plain `Worker` rather than a `CoroutineWorker`: `doWork` may block,
// and blocking on one latch is the whole job. WorkManager stops a worker
// after ten minutes; the wait here is far shorter, since the refresh is
// one `/v1/hello` and one `get_cached` against a desktop that is either
// there or not.

package com.pktstorm.headstate.refresh

import android.content.Context
import androidx.work.Worker
import androidx.work.WorkerParameters
import app.tauri.Logger
import java.util.concurrent.TimeUnit

class RefreshWorker(context: Context, params: WorkerParameters) : Worker(context, params) {
    companion object {
        /**
         * How long Rust gets. The client tries each stored address with
         * a three-second connect timeout, so a desktop that is away is
         * known to be away well inside this.
         */
        const val WINDOW_SECONDS = 60L
    }

    override fun doWork(): Result {
        val (id, window) = HeadstateRefreshPlugin.begin() ?: run {
            Logger.debug("headstate-refresh: window granted but the app is not running")
            return Result.success()
        }
        val finished = try {
            window.done.await(WINDOW_SECONDS, TimeUnit.SECONDS)
        } catch (e: InterruptedException) {
            // WorkManager is stopping us (onStopped): treat as the
            // deadline having passed.
            false
        }
        if (!finished) {
            HeadstateRefreshPlugin.expire(id)
            Logger.debug("headstate-refresh: window $id expired")
            return Result.retry()
        }
        return if (window.success.get()) {
            Logger.debug("headstate-refresh: window $id refreshed")
            Result.success()
        } else {
            // Rust gave up quietly (the desktop is unreachable, or
            // refused us). Retry with backoff rather than at the next
            // period: fewer attempts, not more, while it stays away.
            Logger.debug("headstate-refresh: window $id gave up")
            Result.retry()
        }
    }
}
