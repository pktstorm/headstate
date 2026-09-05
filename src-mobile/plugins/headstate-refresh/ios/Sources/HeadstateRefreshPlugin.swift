// The iOS side of tauri-plugin-headstate-refresh.
//
// One BGAppRefreshTask, identifier `com.pktstorm.headstate.companion.refresh`
// (listed in the app's BGTaskSchedulerPermittedIdentifiers, with `fetch`
// in UIBackgroundModes -- src-mobile/Info.ios.plist). It is NOT a stream
// and must not become one: iOS grants a refresh task a few seconds,
// opportunistically, at most every fifteen minutes or so, and only for
// an app the user has not force-quit. The Rust side (src/lib.rs)
// documents the protocol; this file matches it.
//
// # Lifecycle
//
// - `init`: Tauri calls it from `start_app()`, before `UIApplicationMain`,
//   which is earlier than Apple's "before the app finishes launching"
//   rule for `BGTaskScheduler.register`. The identifier is registered
//   and `UIApplication.didEnterBackgroundNotification` is observed.
// - Entering the background: submit a request with an earliest begin
//   date fifteen minutes out. Submitting again replaces the pending one,
//   so bouncing in and out of the app does not stack requests.
// - The launch handler: submit the next request first (the way Apple's
//   sample does; a task is one-shot), then hand Rust a window id on the
//   channel `register` gave us and wait for `complete`. The expiration
//   handler cancels: it tells Rust the window is gone, then ends the
//   task as unsuccessful. Rust's later `complete` for an id that is no
//   longer here is ignored.
//
// # Trying it
//
// The simulator never grants a refresh task on its own. With the app
// running under Xcode, pause it and run
//   e -l objc -- (void)[[BGTaskScheduler sharedScheduler] _simulateLaunchForTaskWithIdentifier:@"com.pktstorm.headstate.companion.refresh"]
// in the debugger, then resume. `_simulateExpirationForTaskWithIdentifier:`
// exercises the expiration path.

import BackgroundTasks
import Foundation
import Tauri
import UIKit

struct RegisterArgs: Decodable {
  let channel: Channel
}

struct CompleteArgs: Decodable {
  let id: UInt64
  let success: Bool
}

/// What goes to Rust on the channel: `{"kind":"begin","id":N}` or
/// `{"kind":"expire","id":N}`.
struct WindowMessage: Encodable {
  let kind: String
  let id: UInt64
}

class HeadstateRefreshPlugin: Plugin {
  /// Also `TASK_IDENTIFIER` in src/lib.rs and the Android work name.
  static let taskIdentifier = "com.pktstorm.headstate.companion.refresh"
  /// The earliest the next window may open. iOS treats it as a floor,
  /// not a schedule.
  static let interval: TimeInterval = 15 * 60

  private let lock = NSLock()
  private var channel: Channel?
  private var tasks: [UInt64: BGAppRefreshTask] = [:]
  private var nextId: UInt64 = 0

  override init() {
    super.init()
    let registered = BGTaskScheduler.shared.register(
      forTaskWithIdentifier: Self.taskIdentifier, using: nil
    ) { [weak self] task in
      guard let self = self, let task = task as? BGAppRefreshTask else {
        task.setTaskCompleted(success: false)
        return
      }
      self.handle(task)
    }
    if !registered {
      // Identifier missing from Info.plist, or registered twice: the
      // app still runs, it just never refreshes in the background.
      Logger.error(
        "headstate-refresh: could not register \(Self.taskIdentifier); check BGTaskSchedulerPermittedIdentifiers"
      )
    }
    NotificationCenter.default.addObserver(
      self,
      selector: #selector(didEnterBackground),
      name: UIApplication.didEnterBackgroundNotification,
      object: nil
    )
  }

  deinit {
    NotificationCenter.default.removeObserver(self)
  }

  @objc private func didEnterBackground() {
    Self.schedule()
  }

  /// Ask for a window no sooner than `interval` from now.
  static func schedule() {
    let request = BGAppRefreshTaskRequest(identifier: taskIdentifier)
    request.earliestBeginDate = Date(timeIntervalSinceNow: interval)
    do {
      try BGTaskScheduler.shared.submit(request)
    } catch {
      // `.notPermitted` when the identifier is not in Info.plist;
      // `.unavailable` in an app extension or when Background App
      // Refresh is off in Settings. Neither is worth more than a log.
      Logger.info("headstate-refresh: could not schedule a refresh: \(error)")
    }
  }

  /// The OS granted a window. Runs on BGTaskScheduler's own queue.
  private func handle(_ task: BGAppRefreshTask) {
    Self.schedule()

    lock.lock()
    guard let channel = channel else {
      lock.unlock()
      // Rust has not registered yet; nothing can use the window.
      task.setTaskCompleted(success: false)
      return
    }
    nextId += 1
    let id = nextId
    tasks[id] = task
    lock.unlock()

    task.expirationHandler = { [weak self] in
      guard let self = self, let task = self.take(id) else { return }
      Self.send(channel, kind: "expire", id: id)
      task.setTaskCompleted(success: false)
    }
    Self.send(channel, kind: "begin", id: id)
  }

  private func take(_ id: UInt64) -> BGAppRefreshTask? {
    lock.lock()
    defer { lock.unlock() }
    return tasks.removeValue(forKey: id)
  }

  private static func send(_ channel: Channel, kind: String, id: UInt64) {
    do {
      try channel.send(WindowMessage(kind: kind, id: id))
    } catch {
      Logger.error("headstate-refresh: could not send \(kind) for window \(id): \(error)")
    }
  }

  // MARK: Commands (called from Rust)

  /// `{"channel": <Channel>}`: the channel windows are announced on.
  @objc public func register(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(RegisterArgs.self)
    lock.lock()
    channel = args.channel
    lock.unlock()
    invoke.resolve()
  }

  /// `{"id": N, "success": bool}`: Rust finished the window's refresh.
  @objc public func complete(_ invoke: Invoke) throws {
    let args = try invoke.parseArgs(CompleteArgs.self)
    if let task = take(args.id) {
      task.setTaskCompleted(success: args.success)
    }
    invoke.resolve()
  }
}

@_cdecl("init_plugin_headstate_refresh")
func initPlugin() -> Plugin {
  return HeadstateRefreshPlugin()
}
