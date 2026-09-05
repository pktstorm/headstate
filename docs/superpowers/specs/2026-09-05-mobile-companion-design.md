# Mobile Companion Design

**Date:** 2026-09-05
**Status:** Draft

## Problem

Headstate's two jobs, seeing the real state of every open pull request and
cleaning up the machine that produced them, both happen at a desk. The
moments that want them most often do not: a review request lands while you
are away from the laptop, or you remember a stack of stale worktrees and
Docker images on the train home. Today there is no way to act on either
without sitting down at the desktop.

A mobile app cannot simply be a second copy of Headstate. The cleanup half
reads and deletes files on the desktop, so a phone has nothing to clean.
The PR half could talk to GitHub on its own, but then the phone would hold
a second GitHub token, a second poll loop, and a second cleanup ledger, and
the two copies would disagree.

This document designs a **companion app**: one iOS and Android build that
pairs with a running desktop Headstate and drives it remotely. The desktop
remains the single place that holds the GitHub token, runs the poll loop,
and touches the filesystem.

## Decisions

The three questions that shaped this design, and their answers.

**No VPN is required for security. mTLS is.** Mutual TLS with a pinned
self-signed certificate on each end gives mutual authentication and
encryption of the same strength a WireGuard tunnel would, for this use.
The VPN question is about reachability, not security, and is answered
separately below. The app never trusts the network it is on.

**The desktop is the trust boundary.** The phone never holds the GitHub
token and never talks to `api.github.com`. Every command the phone issues
is executed by the desktop under the desktop's existing code paths, its
existing cleanup ledger, and its existing logging. A stolen phone with a
revoked pairing has nothing.

**One codebase for both platforms, on the stack already in use.** Tauri 2,
which the desktop pins at 2.11, builds for iOS and Android from the same
React frontend in `src/`. The mobile Rust crate is a thin client that owns
the TLS session and the device key; the webview never sees a certificate.

## Non-goals

- A hosted relay. Headstate has no server and this design does not add one.
- Push notifications through Apple or Google. That requires a server.
- Running any part of cleanup on the phone.
- A tablet or desktop-width layout for the mobile build.
- Replacing the desktop app. The desktop must be running for the companion
  to do anything.

## Threat model

| Threat | Mitigation |
|---|---|
| Attacker on the same wifi connects to the listener | Client certificate required at the TLS layer; unpaired certs are refused before any request body is read |
| Attacker impersonates the desktop to the phone | Phone pins the desktop's certificate fingerprint, delivered out of band by QR code at pairing |
| Attacker scans the pairing QR over your shoulder | Pairing token is single use, expires in two minutes, and the desktop shows a confirmation dialog naming the device before the pairing is stored |
| Phone is stolen, unlocked | Destructive commands require a per-request signature from a biometric-gated key; the desktop can revoke the device |
| Phone is stolen, locked | Device keys live in the Secure Enclave or Android Keystore and are not exportable |
| Desktop listener is reachable from the internet | Listener is off by default and binds only when the user enables it; even when on, unpaired clients are refused at the handshake |
| A paired phone issues a command the desktop UI could not | The remote surface is an explicit allowlist, enumerated in one Rust file, with local-only commands excluded |
| Request replay | Every write carries a nonce and timestamp; the desktop rejects duplicates and clock skew beyond sixty seconds |

## Architecture

```
src-tauri/src (desktop, existing)
├── remote/
│   ├── mod.rs        feature gate; start/stop the listener from settings
│   ├── identity.rs   desktop key pair and self-signed cert, generated once
│   ├── listener.rs   axum over rustls; requires and verifies client certs
│   ├── pairing.rs    QR payload, pairing token, approve/revoke
│   ├── surface.rs    the remote allowlist: command name -> handler + class
│   ├── events.rs     fan-out of Tauri events to SSE subscribers
│   └── discovery.rs  mDNS advertisement of _headstate._tcp
└── store/
    └── devices.rs    paired_devices table

src-mobile/ (new Tauri mobile crate)
├── src/
│   ├── lib.rs        mobile_entry_point; registers the client commands
│   ├── keys.rs       session key + biometric-gated signing key
│   ├── client.rs     reqwest with client identity and server pinning
│   ├── pairing.rs    scan QR, connect, prove token, store fingerprint
│   └── events.rs     SSE subscriber; re-emits as Tauri events
├── tauri.conf.json   identifier com.pktstorm.headstate.companion
└── gen/              Xcode and Android Studio projects (generated)

src (shared frontend)
├── api/
│   ├── transport.ts  the seam: call(name, args) and listen(event, cb)
│   ├── local.ts      transport backed by @tauri-apps/api invoke
│   ├── remote.ts     transport backed by the mobile crate's client
│   └── tauri.ts      unchanged signatures; now built on transport.ts
└── components/       responsive pass; see Layout
```

### The frontend seam

Every screen in Headstate already reaches Rust through the typed wrappers
in `src/api/tauri.ts`; nothing else in the tree imports `invoke`. That file
keeps its exported signatures and gains one indirection: each wrapper calls
`transport.call(name, args)` instead of `invoke(name, args)`. The build
selects the transport with a Vite define, `import.meta.env.VITE_TARGET`,
which is `desktop` or `mobile`. The desktop build is byte-for-byte the same
behaviour it has today.

Events go through the same seam. The desktop poll loop emits
`prs-updated`, `poll-state`, `poll-error`, `prs-truncated`,
`prs-incomplete`, `store-error`, `worktree-removal-progress`,
`reviewing-short`, and `update-run-done`. The remote transport subscribes
to the desktop's event stream and re-emits these under the same names, so
the TanStack Query hooks in `src/api/hooks.ts` do not change.

### Desktop listener

An axum server on a tokio task, started when the user enables **Allow
phone connections** in Settings and stopped when they disable it. Off by
default. It binds to all interfaces on a fixed port, 41919, chosen from the
dynamic range so it never collides with a well-known service.

TLS is rustls with:

- TLS 1.3 only.
- The desktop's own self-signed certificate, generated on first enable
  with `rcgen`, ten-year validity, private key stored in the platform keychain.
  Headstate uses no keychain today; this is the first entry. The
  certificate is not tied to a hostname; the phone pins the fingerprint,
  not the name.
- A client certificate verifier that accepts a connection only when the
  presented certificate's SHA256 fingerprint matches a row in
  `paired_devices`. No CA, no chain building. Unpaired certificates fail
  the handshake, so an attacker never reaches HTTP.

The HTTP surface is deliberately tiny:

| Method | Path | Purpose |
|---|---|---|
| POST | `/v1/pair` | Only endpoint that accepts an unpaired client; see Pairing |
| POST | `/v1/call/{command}` | Invoke one allowlisted command; JSON body in, JSON body out |
| GET | `/v1/events` | Server-sent events; replays the Tauri event names above |
| GET | `/v1/hello` | Desktop version, protocol version, viewer login; used for the connection banner |

Why HTTP and SSE rather than WebSocket or a bespoke framing: the phone's
Rust client is reqwest, which already carries rustls and client identity
support; SSE reconnects for free; and every command maps one-to-one onto
a request, which keeps the desktop's existing per-command logging intact.

Command bodies are the same JSON shapes Tauri's IPC serialises today, so
the desktop handler for `/v1/call/{command}` is a match over the allowlist
that deserialises the arguments and calls the same `commands::*` function
the webview would have called.

### The remote allowlist

`remote/surface.rs` is the single place that decides what a phone can do.
Each entry is a command name and a class:

- **read**: no side effects on GitHub or disk.
- **write**: changes GitHub state through the existing write module.
- **destructive**: deletes files, branches, images, or volumes.
- **local**: not exposed remotely. Anything that opens a window, reveals a
  file, changes autostart, restarts the app, or runs an editor.

| Class | Commands |
|---|---|
| read | `get_auth_state`, `get_cached`, `get_cached_reviewing`, `refresh_now`, `get_stats`, `get_history`, `get_periods`, `get_cycle_trend`, `get_merged_detail`, `get_reviewing`, `count_reviewing`, `get_pr_detail`, `get_viewer`, `build_target`, `latest_release`, `list_worktrees`, `classify_worktrees`, `size_worktrees`, `list_branches`, `scan_artifacts`, `size_artifacts`, `scan_venvs`, `size_venvs`, `docker_state`, `docker_builds`, `docker_images`, `docker_disk_usage`, `docker_dangling_volumes`, `docker_running_containers`, `preview_cleanup`, `cleanup_log`, `get_cleanup_prefs`, `assessed_worktrees`, `check_packages`, `packages_markdown`, `scan_claude_md`, `read_claude_md`, `get_poll_interval`, `get_worktree_dirs` |
| write | `act_on_pr`, `act_on_prs`, `review_pr`, `comment_on_pr`, `resolve_thread`, `unresolve_thread`, `reply_to_thread`, `rerun_checks`, `update_pr_branch`, `set_auto_merge`, `mark_assessed`, `clear_assessed`, `set_cleanup_prefs`, `set_poll_interval`, `open_update_pr` |
| destructive | `delete_head_branch`, `delete_branches`, `delete_remote_branches`, `remove_worktree`, `remove_worktrees`, `remove_worktree_forced`, `remove_artifacts`, `remove_venvs`, `remove_orphan`, `docker_remove_images`, `docker_remove_volume`, `docker_prune_cache`, `apply_package_updates` |
| local | `diag_log`, `reveal_log`, `pull_checkout`, `get_ui_prefs`, `set_ui_prefs`, `get_autostart`, `set_autostart`, `get_notify_prefs`, `set_notify_prefs`, `set_worktree_dirs`, `assess_worktree`, `claudify_command`, `apply_updates_in_background`, `docker_restart`, `docker_start`, `set_view_needs_github` |

The table is the contract. A command added to `generate_handler!` on the
desktop is not remotely callable until someone adds it here and picks a
class, and a unit test asserts that every registered command appears in
exactly one class so the omission is loud rather than silent.

`assess_worktree` is local because it launches an agent on the desktop and
streams its output to a window; that belongs to a later milestone.
`pull_checkout` is local because it modifies a checkout the user is
presumably sitting in front of.

### Step-up for destructive commands

The phone holds two keys, both generated at pairing and both
non-exportable:

- A **session key**, whose certificate is the TLS client identity. Usable
  whenever the app is open.
- A **signing key**, an Ed25519 key whose use requires biometric or device
  passcode confirmation through `tauri-plugin-biometric` and the platform
  keystore's access control.

A destructive request carries a header, `X-Headstate-Signature`, over the
canonical JSON of `{command, args, nonce, timestamp}` signed with the
signing key. The desktop verifies against the signing public key stored at
pairing, checks the timestamp is within sixty seconds, and records the
nonce for that window. Read and write requests carry no signature. The
result is that deleting a worktree from the phone costs one Face ID
prompt, which matches the weight of the action, while approving a PR does
not.

The desktop also posts a native notification for every destructive command
it executes on behalf of a phone, naming the device. That is a second,
independent signal that something happened, and it costs nothing.

## Pairing

Pairing is a QR code shown on the desktop and scanned by the phone. It
transfers the desktop's fingerprint out of band, which is what makes the
pin trustworthy, and a short-lived token that lets the desktop tell an
intended pairing from an opportunistic one.

1. Desktop, Settings, **Pair a phone**. The desktop enables the listener if
   it is not already on, generates a 32-byte random pairing token with a
   two-minute expiry, and renders a QR code containing:

   ```json
   {
     "v": 1,
     "name": "octocat's laptop",
     "addrs": ["192.0.2.10", "100.64.0.7"],
     "port": 41919,
     "fp": "sha256:…desktop cert fingerprint…",
     "token": "…base64url…",
     "exp": 1757068800
   }
   ```

   `addrs` is every non-loopback address the desktop has, including an
   overlay address if one is present, so the phone can try each in order.

2. Phone scans the QR with `tauri-plugin-barcode-scanner`, generates the
   session and signing keys, and opens a TLS connection presenting the
   session certificate. The desktop's verifier admits it because the
   pairing token is unexpired and the request path is `/v1/pair`; every
   other path refuses unpaired certificates at the handshake. The phone
   checks the presented server certificate's fingerprint against `fp` and
   aborts on mismatch.

3. Phone posts to `/v1/pair`:

   ```json
   {
     "token": "…",
     "device_name": "Octocat's phone",
     "signing_pubkey": "…base64…",
     "proof": "HMAC-SHA256(token, client_fp || server_fp)"
   }
   ```

   The proof binds the token to both certificates, so a token observed in
   transit is useless without the matching client key.

4. Desktop verifies the proof, invalidates the token, and shows a modal:
   "Pair *Octocat's phone*? Fingerprint `ab12 cd34 …`". The phone shows the
   same fingerprint so the user can compare. On approve, the desktop
   inserts the row and returns `200`; on deny or timeout it returns `403`
   and the phone discards its keys.

5. Phone stores `{name, addrs, port, fp}` in its own settings store. From
   here on, every connection is ordinary mTLS.

**Revocation.** Settings, **Paired devices**, lists rows with name, last
seen, and a Revoke button. Revoking deletes the row and closes any open
connection from that certificate. The phone learns it is revoked on its
next handshake failure and returns to the pairing screen.

**Re-pairing.** Repeats the flow. The desktop replaces the old row for the
same device name only if the user confirms; otherwise both coexist.

### Storage

```sql
CREATE TABLE paired_devices (
  id              INTEGER PRIMARY KEY,
  name            TEXT NOT NULL,
  cert_fp         TEXT NOT NULL UNIQUE,   -- sha256 of the session cert, hex
  cert_der        BLOB NOT NULL,          -- for the verifier
  signing_pubkey  BLOB NOT NULL,          -- ed25519, 32 bytes
  paired_at       TEXT NOT NULL,
  last_seen       TEXT
);
```

Added as a versioned migration in `store/schema.rs`. The desktop private
key is not in SQLite; it lives in the platform keychain, which is a
Security Policy change and is called out below.

## Reachability

The listener is reachable wherever the desktop is reachable. Three cases:

**Same network.** The desktop advertises `_headstate._tcp` over mDNS
with the port and the first sixteen hex characters of its fingerprint as
a TXT record, so the phone can find it when the LAN address changed since
pairing. On iOS this requires `NSLocalNetworkUsageDescription` and
`NSBonjourServiceTypes` in the app's plist; the OS shows a one-time prompt.

**Different network.** The recommended path is an overlay network the user
already runs, such as Tailscale or plain WireGuard. The desktop puts the
overlay address in `addrs` at pairing, and the phone tries it after the LAN
addresses fail. Headstate does not integrate with any overlay; it only has
to be told an address, and mTLS still applies on top of it. This is the
answer to "does it need a VPN": for remote use, an overlay is the honest
way to reach a laptop behind NAT without hosting a relay, and it is
documented rather than built.

**Port forwarding.** Supported in the sense that nothing prevents it, and
mTLS makes it survivable, but not documented as a recommended setup. A
laptop that moves between networks makes it fragile anyway.

## Mobile app

### Why Tauri mobile

Three alternatives were considered.

- **Capacitor** wraps the same React app but leaves networking to the
  platform webview, and WKWebView cannot present a client certificate from
  the Secure Enclave without native code. That native code is the whole
  point of the mobile crate, so Capacitor saves nothing.
- **React Native or Expo** reuses the hooks and store but every component
  is rewritten. Headstate's screens are the product; rewriting them doubles
  the maintenance surface for the same pixels.
- **Flutter** is a full rewrite in a second language.

Tauri 2 mobile reuses `src/` unchanged, reuses the Rust ecosystem the
desktop already depends on for TLS, and the desktop's `lib.rs` already
carries `#[cfg_attr(mobile, tauri::mobile_entry_point)]`. The mobile crate
is separate rather than a feature of `src-tauri` because it must not link
octocrab, rusqlite, or any Docker or git code, and because the two apps
have different bundle identifiers, icons, and store listings.

### Mobile Rust dependencies

| Crate | Purpose |
|---|---|
| `tauri` 2 | app shell, mobile entry point |
| `reqwest` with `rustls` and `native-tls` off | HTTPS client; `Identity` from the session key, custom verifier for the pinned server fingerprint |
| `rcgen` | self-signed session certificate |
| `ed25519-dalek` | signing key operations when the platform keystore cannot hold an Ed25519 key directly; see Open questions |
| `tauri-plugin-biometric` | gate the signing key |
| `tauri-plugin-stronghold` | encrypted local settings: desktop fingerprint, addresses |
| `tauri-plugin-barcode-scanner` | scan the pairing QR |
| `mdns-sd` | LAN discovery |

### Client commands

The mobile crate exposes a handful of Tauri commands to the shared
frontend; these are what `src/api/remote.ts` calls.

| Command | Purpose |
|---|---|
| `pair_from_qr(payload)` | runs the pairing flow above; returns the desktop name |
| `unpair()` | forgets the desktop and destroys the device keys |
| `connection_state()` | `unpaired`, `connecting`, `connected`, `unreachable`, `revoked` |
| `remote_call(command, args)` | posts to `/v1/call/{command}`; signs destructive commands after biometric |
| `subscribe_events()` | opens the SSE stream; re-emits each event by name |

`remote_call` refuses commands not in the allowlist client-side too, so a
mistake in the frontend fails locally with a clear error rather than with
a 404 from the desktop.

### Layout

The desktop window enforces a minimum width of 1000 pixels and the
components assume it. The mobile build needs a responsive pass, scoped to
the views that ship in v1:

- **PR list** as a single column of rows; the repo sidebar becomes a sheet.
- **PR detail** with the same review, comment, thread, and merge actions.
- **Nudge wizard**, since a paste-ready list is more useful from a phone
  than from a desk.
- **Cleanup**: worktrees, artifacts, venvs, and Docker, each as a list with
  size, age, and a select-and-remove flow. The desktop's preview and ledger
  views are reused as-is.
- **Connection banner** at the top of every screen showing the desktop
  name, reachability, and last poll time; tapping it opens pairing
  settings.

Stats and the dashboard stay desktop-only in v1. A `useIsMobile` hook
switches layouts; components do not fork.

### What the phone does when the desktop is away

Nothing, except say so. The connection banner reads "Octocat's laptop is
unreachable" with the last-seen time. The most recent PR snapshot received
over `/v1/events` is cached in the phone's settings store so the list
renders while unreachable, marked stale, with actions disabled. There is
no local poll loop and no GitHub token to run one with.

## Security Policy changes

`SECURITY.md` currently states that Headstate stores no credentials of its
own and makes no outbound network calls beyond `api.github.com`. Both
remain true for the desktop's GitHub behaviour, and the document must gain
a section covering:

- The desktop identity key in the platform keychain, what it is for, and
  that removing it invalidates every pairing.
- The listener: off by default, port, what it accepts, what it refuses.
- The paired-devices table and that it holds public keys only.
- That the phone never holds the GitHub token.

## Testing

- **Verifier unit tests**: paired cert accepted, unpaired refused, revoked
  refused on the next handshake, expired pairing token refused, replayed
  token refused, wrong proof refused.
- **Allowlist test**: every command in `generate_handler!` appears in
  exactly one class in `surface.rs`.
- **Signature tests**: destructive call without signature refused, with a
  stale timestamp refused, with a reused nonce refused, valid accepted.
- **Transport tests** in `src/api`: `tauri.ts` wrappers produce identical
  calls through the local and remote transports; events arrive under the
  same names.
- **Loopback integration test**: start the listener in-process, pair a
  synthetic client, run one command from each class, revoke, confirm
  refusal. Runs in CI on Linux with no phone.
- **Manual pairing walkthrough** on a real iPhone and a real Android device
  before each release, recorded as a checklist in `docs/`.

## CI

- Desktop CI is unchanged; the `remote` module is compiled and tested on
  the existing Linux, macOS, and Windows jobs.
- A new `mobile` job runs `cargo check` and `cargo test` for `src-mobile`
  on Linux with the Android target, and `cargo check` for the iOS target on
  a macOS runner. Store builds are manual until the pairing walkthrough has
  been done twice without findings.
- `scripts/check-privacy.sh` runs on the new paths as it does today; the
  QR example in this document uses documentation addresses and synthetic
  names for that reason.

## Open questions

- **Ed25519 in the platform keystore.** iOS Secure Enclave and Android
  StrongBox hold NIST P256 keys natively; Ed25519 is not universal. The
  safe choice is ECDSA over P256 for the signing key on both platforms and
  drop `ed25519-dalek`. Decide during the mobile crate spike.
- **Background execution.** iOS suspends the SSE stream within seconds of
  the app leaving the foreground. v1 accepts that the phone catches up on
  resume. Background refresh is out of scope because there is no push path.
- **Multiple desktops.** The phone's settings store is designed as a list
  of desktops even though the UI shows one, so adding a switcher later is
  a frontend change only.

## Out of scope for v1

- Stats and the dashboard on mobile.
- The worktree assessment agent from the phone.
- Any relay, push, or hosted component.
- Tablet layouts.
- Windows and Linux desktops advertising over mDNS if the platform makes it
  awkward; pairing still works by address.

## Milestones

1. **Transport seam.** `src/api/transport.ts` with the local backend only.
   Desktop behaviour unchanged, tests green. Small and mergeable alone.
2. **Desktop listener and pairing.** `remote/` module, `paired_devices`
   migration, Settings UI for enable, pair, and revoke, loopback
   integration test. Ships behind the off-by-default toggle.
3. **Mobile crate spike.** `src-mobile` builds for both targets, pairs
   against a desktop on the same wifi, and runs `get_cached`.
4. **Allowlist and step-up.** Full read and write surface, destructive
   signature, desktop notification.
5. **Responsive pass.** PR list, detail, nudge, cleanup, connection banner.
6. **Security Policy update and pairing walkthrough.** Then a TestFlight
   and Play internal build.
