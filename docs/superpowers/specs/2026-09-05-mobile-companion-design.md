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
| Traffic recorded today, decrypted by a future quantum computer | TLS key exchange is hybrid X25519 plus ML-KEM-768 by default |
| Step-up signatures forged by a future quantum computer | The step-up key is a hybrid of ECDSA P256 and ML-DSA-65, held in the phone's secure hardware where the platform supports it |

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
│   ├── keys.rs       session key + biometric-gated hybrid signing keys
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

The phone holds a session key and a signing key pair, all generated at
pairing and all non-exportable:

- A **session key**, ECDSA P256, whose certificate is the TLS client
  identity. Usable whenever the app is open.
- A **signing key pair** used only for step-up: an ECDSA P256 key and,
  where the platform can hold one, an ML-DSA-65 key. Both are generated
  in the platform's secure hardware with biometric or device passcode
  access control, through a small Tauri mobile plugin described under
  Post-quantum posture.

A destructive request carries a header, `X-Headstate-Signature`, over the
canonical JSON of `{command, args, nonce, timestamp}`. The header holds
two signatures when the ML-DSA key exists and one when it does not; the
desktop verifies every signature the pairing record says to expect and
refuses the request if any is missing or invalid. It also checks the
timestamp is within sixty seconds and records the nonce for that window.
Read and write requests carry no signature. The result is that deleting a
worktree from the phone costs one Face ID prompt, which matches the weight
of the action, while approving a PR does not.

The desktop also posts a native notification for every destructive command
it executes on behalf of a phone, naming the device. That is a second,
independent signal that something happened, and it costs nothing.

### Post-quantum posture

The question asked of this design was whether the step-up key can be
post-quantum. As of September 2026 it can, in hardware, on both platforms,
with a fallback that Android makes unavoidable.

**What the platforms offer.**

- iOS 26, released September 2025, added ML-DSA-65 and ML-DSA-87 to
  CryptoKit, with Secure Enclave variants: `SecureEnclave.MLDSA65` and
  `SecureEnclave.MLDSA87` each expose a `PrivateKey` on every Apple
  platform at version 26. Apple's own guidance is that the ML-DSA API is
  meant for building hybrid signatures at the application level.
- Android 17, released 16 June 2026, added ML-DSA-65 and ML-DSA-87 to
  Android Keystore through the standard `KeyPairGenerator`, `KeyFactory`,
  and `Signature` APIs, with keys generated in secure hardware on
  supported devices. The qualifier matters: the OS ships on Pixel first,
  other manufacturers follow over the next year or two, and a device on
  Android 16 or older has no ML-DSA in its keystore at all.

**What that means for the design.** A post-quantum-only signing key would
exclude most Android phones for years. A classical-only key would leave
the step-up signature forgeable by a future quantum computer. The hybrid
above takes both: ECDSA P256 is always present and always in hardware, and
ML-DSA-65 is added whenever the keystore can generate one. The pairing
record says which keys the phone has, so a device that pairs without
ML-DSA and later upgrades can re-pair to add it, and the desktop never
silently accepts a downgrade.

ML-DSA-65 rather than 87 because it is NIST security category 3, a
3,309-byte public key, and a 3,309-byte signature, which is already
larger than everything else in a request combined; category 5 buys
nothing for a signature that only has to hold until the device is
revoked.

**Why signatures are the second priority, not the first.** A quantum
adversary attacks the two halves of this protocol differently. Recorded
traffic can be stored today and decrypted once a large enough machine
exists, so key exchange has to be post-quantum now. A signature only has
to be unforgeable at the moment it is checked, so a classical signature
scheme is safe until such a machine actually exists, and can be swapped
later. This design therefore:

1. Makes the TLS key exchange hybrid now. rustls with the aws-lc-rs
   provider prefers X25519MLKEM768 by default, so PR titles and file
   listings recorded off the wire today stay private.
2. Makes the step-up signature hybrid now, because the platform keys are
   available and the cost is one extra signature on a rare request.
3. Leaves the TLS certificates themselves on ECDSA P256 for v1. ML-DSA in
   TLS 1.3 is still an IETF draft, and rustls is in the middle of turning
   it on by default for its 0.23 line; the pull request to do so was
   marked ready for review on 4 September 2026 and was open when this
   was written. Because both ends pin the peer's fingerprint rather than
   validating a chain, migrating the certificates later is a matter of
   regenerating them and re-pairing, and the plan is to do that once
   rustls ships it.

**Implementation.**

- A custom Tauri mobile plugin, `tauri-plugin-headstate-keys`, with a
  Swift side over CryptoKit and a Kotlin side over Android Keystore. It
  exposes `generate()`, `public_keys()`, and `sign(bytes)`; `sign`
  returns the ECDSA signature and, when present, the ML-DSA signature.
  Biometric gating is done by the platform's access control on the key
  itself, not by a separate prompt, so a signature cannot be produced
  without the check.
- The desktop verifies ML-DSA-65 with RustCrypto's `ml-dsa` crate,
  pinned at 0.1.1, alongside `p256` 0.13 for the ECDSA half. The other
  candidate was Cryspen's `libcrux-ml-dsa` 0.0.10, whose core is
  formally verified; both implement final FIPS 204, and a cross-check
  during implementation showed each verifies the other's signatures and
  derives the same public key from the same seed. `ml-dsa` won on the
  build: no build scripts, fifteen new lock entries all from RustCrypto,
  and the `signature` traits `p256` already uses, against three
  `build.rs` files, twenty-nine new entries including `hax-lib` proc
  macros and a `cfg`-gated `bindgen` chain, and a 0.0.x API. The audit
  gap is accepted because the desktop only verifies: formal verification
  mostly protects a signer's secrets, and the only ML-DSA signer is the
  phone's secure hardware. The header grammar, canonical bytes, and a
  byte-exact test vector for the phone to match live in
  `src-tauri/src/remote/stepup.rs`. aws-lc-rs also ships ML-DSA, but its
  API has moved between the unstable and stable modules across recent
  releases, so it was not considered.
- The desktop's own identity key stays P256 in v1, for the same reason as
  the certificates.

**Verified during the mobile crate spike** (#512, September 2026). These
are the claims this section rests on that could not be confirmed from
documentation alone when it was written. Each is now either resolved with
its source, or marked unconfirmed with what was tried.

- **Resolved: `SecureEnclave.MLDSA65.PrivateKey` is biometric-gatable the
  same way as the P256 key.** Apple's reference lists four initialisers:
  `init(accessControl:)`, `init(accessControl:authenticationContext:)`,
  `init(dataRepresentation:)`, and
  `init(dataRepresentation:authenticationContext:)`, with the full
  signature `init(accessControl: SecAccessControl = ..., authenticationContext:
  LAContext? = nil) throws`. That is the P256 key's shape minus the
  `compactRepresentable:` parameter, which has no meaning for a lattice
  key. Available on iOS 26.0+ and every other Apple platform at 26.0.
  Source: <https://developer.apple.com/documentation/cryptokit/secureenclave/mldsa65/privatekey>
  and its `init(accesscontrol:authenticationcontext:)` page, read on
  2026-09-04.
- **Unconfirmed: which Secure Enclave generations support ML-DSA.** No
  Apple source names a hardware floor. Tried: the `SecureEnclave.MLDSA65`
  reference (availability is by OS version only, 26.0 everywhere); the
  Platform Security guide's "Quantum-secure cryptography in Apple
  operating systems" and "The Secure Enclave" pages (neither ties ML-DSA
  to a chip); and the WWDC25 session 314 transcript, which says only that
  "the ML-DSA implementation has Secure Enclave support". What is known:
  iOS 26 itself requires A13 or later, so on iPhone the floor is at least
  A13 by OS support alone. The plugin therefore does not test for a chip;
  it calls `SecureEnclave.MLDSA65.PrivateKey(accessControl:)` at pairing
  and treats a throw as "no ML-DSA", which is the same rule Android gets
  below. Record the result per device in the pairing walkthrough.
- **Resolved, with one gap: Android's behaviour when KeyMint lacks
  ML-DSA.** The `KeyGenParameterSpec` reference states: "ML-DSA support
  is only available on devices with a Trusted Execution Environment
  running KeyMint version >= 5. Support can be determined by checking
  that `PackageManager.FEATURE_HARDWARE_KEYSTORE` has a value >= 500."
  So the plugin checks `hasSystemFeature(FEATURE_HARDWARE_KEYSTORE, 500)`
  before generating and pairs with ECDSA alone when it is false; there is
  no documented software-backed ML-DSA path. The gap: the documentation
  does not say which exception `generateKeyPair()` throws on a device
  that fails that check, so the plugin also treats any exception there as
  "no ML-DSA", and after a successful generation it confirms
  `KeyInfo.getSecurityLevel()` is TEE or StrongBox before advertising the
  key. `KeyProperties.KEY_ALGORITHM_ML_DSA_65` is "Added in API level
  37". Source: <https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec>
  ("Example: ML-DSA key pair for signing") and
  <https://developer.android.com/reference/android/security/keystore/KeyProperties>,
  read on 2026-09-04.
- **Resolved by documentation, not yet by a device: `setUserAuthenticationRequired`
  applies to ML-DSA keys.** The method's reference is algorithm-agnostic
  ("This authorization applies only to secret key and private key
  operations. Public key operations are not restricted.") and the same
  `KeyGenParameterSpec` page documents ML-DSA-specific constraints on its
  sibling builder methods (digest must be `DIGEST_NONE`; `setKeySize` is
  ignored), which shows ML-DSA keys go through the same spec and the
  same authorisations. No source says otherwise. Confirm on a real
  Android 17 device in the pairing walkthrough: one Face/fingerprint
  prompt per destructive command.
  Source: <https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec.Builder#setUserAuthenticationRequired(boolean)>.
- **Resolved for iOS, deferred to CI for Android: aws-lc-rs
  cross-compiles.** `cargo check --target aarch64-apple-ios` and
  `cargo build --target aarch64-apple-ios --lib` of `src-mobile` (reqwest
  0.13 on rustls 0.23/aws-lc-rs 1.18.1, rcgen 0.14 on aws-lc-rs) succeed
  with Xcode 26.6 and the stock `aarch64-apple-ios` Rust target;
  `aws-lc-sys` 0.45.0 compiled its C and assembly for the target,
  including the `mldsa_*_aarch64_asm.o` objects. Android is unverified on
  the spike machine: with no NDK, `cargo check --target
  aarch64-linux-android` fails inside `aws-lc-sys` with `failed to find
  tool "aarch64-linux-android-clang"`, which is the toolchain being
  absent, not a compile error. The `mobile` CI job (#518) runs that check
  with the NDK installed and is the verification of record.
- **Resolved: aws-lc-rs exposes ML-DSA in its stable module.** At 1.18.0,
  the version in `src-tauri/Cargo.lock`, `aws_lc_rs::signature`
  re-exports `PqdsaKeyPair`, `PqdsaPrivateKey`, `PqdsaPublicKey`, and the
  `ML_DSA_44`, `ML_DSA_65`, `ML_DSA_87` algorithms from `crate::pqdsa`
  (`src/signature.rs`, lines 309-310), and `aws_lc_rs::unstable::signature`
  is a module of deprecated aliases whose doc comment reads "The ML-DSA
  signature APIs have been stabilized; use `crate::signature` instead."
  The same is true of 1.18.1, which `src-mobile/Cargo.lock` resolved.
  So aws-lc-rs is a third candidate for the desktop's ML-DSA-65
  verifier, alongside RustCrypto `ml-dsa` and `libcrux-ml-dsa`, and it is
  already in the desktop's lock through octocrab. Source: the crate
  source in the local cargo registry, and
  <https://docs.rs/aws-lc-rs/1.18.0/aws_lc_rs/unstable/index.html>.
- **Still open: the desktop's two providers.** The desktop links octocrab
  on the ring provider and will add aws-lc-rs for the listener. rustls
  requires an explicit process-level default when both are compiled in,
  so this is a one-line `CryptoProvider::install_default` in the
  listener's start-up, but it has to be made and belongs to the listener
  work, not the mobile crate. The mobile crate avoids the question: it
  is a separate crate with its own lockfile, on aws-lc-rs only.

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
     "signing_keys": {
       "ecdsa_p256": "…base64…",
       "mldsa_65": "…base64… or absent"
     },
     "proof": "HMAC-SHA256(token, client_fp || server_fp)"
   }
   ```

   The proof binds the token to both certificates, so a token observed in
   transit is useless without the matching client key.

4. Desktop verifies the proof, invalidates the token, and shows a modal:
   "Pair *Octocat's phone*? Fingerprint `ab12 cd34 …`", with a line
   saying whether the phone offered a post-quantum signing key. The phone
   shows the same fingerprint so the user can compare. On approve, the
   desktop inserts the row and returns `200`; on deny or timeout it
   returns `403` and the phone discards its keys.

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
  ecdsa_pubkey    BLOB NOT NULL,          -- P256, SEC1 uncompressed, 65 bytes
  mldsa_pubkey    BLOB,                   -- ML-DSA-65, 1952 bytes, NULL if the phone has none
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
| `reqwest` with `rustls` on the aws-lc-rs provider, `native-tls` off | HTTPS client; `Identity` from the session key, custom verifier for the pinned server fingerprint, X25519MLKEM768 key exchange |
| `rcgen` | self-signed session certificate |
| `tauri-plugin-headstate-keys` (new, in-repo) | Swift and Kotlin sides that generate and use the hardware-backed session, ECDSA, and ML-DSA keys; see Post-quantum posture |
| `tauri-plugin-biometric` | prompt wording and availability checks; the actual gate is the keystore access control on the key |
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

### What the phone does when it is in the background

Also nothing, and this is a decision rather than an open question. iOS
suspends an app shortly after it leaves the foreground and a streaming
connection dies with it; background URL sessions exist for discrete
downloads and uploads, not for a held-open event stream. The only way to
be told about a change while suspended is a push notification, and a push
needs a server, which this design does not have. Android is more lenient
but its background limits point the same way.

So the phone catches up on resume: reopen the event stream, receive the
current snapshot, and refresh whatever view is showing. The one cheap
improvement worth taking is a background refresh task on each platform,
which the OS grants opportunistically for a few seconds at a time; it can
fetch `/v1/hello` and the cached list so the app opens fresh. It is not a
stream and must not be designed as one.

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
  stale timestamp refused, with a reused nonce refused, valid accepted;
  a pairing that recorded an ML-DSA key refuses a request carrying only
  the ECDSA signature.
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
  a macOS runner. It runs on every push and produces no installable
  artifact.
- A separate `mobile-release` job, described under Release, builds signed
  store artifacts and runs only on a tag.
- `scripts/check-privacy.sh` runs on the new paths as it does today; the
  QR example in this document uses documentation addresses and synthetic
  names for that reason.

## Release

The desktop ships through GitHub Releases with signed updater artifacts
and updates itself. Neither applies to the phone: the stores distribute
the app, and there is no self-update path. The mobile release process is
therefore separate from the desktop's and versioned separately.

### Versioning and compatibility

- `src-mobile` carries its own version, tagged `mobile-vX.Y.Z`, independent
  of the desktop's `vX.Y.Z`.
- The wire protocol has its own integer version, returned by `/v1/hello`
  and embedded in the pairing QR as `v`. The phone refuses to talk to a
  desktop whose protocol version is lower than it requires and shows an
  "update Headstate on your desktop" banner naming the minimum. The
  desktop accepts any phone at or below its own protocol version.
- A protocol bump is a deliberate change to `remote/surface.rs` or the
  pairing payload, recorded in the spec, never a side effect of a
  release.

### Signing

- **iOS**: an Apple Developer Program distribution certificate and an App
  Store provisioning profile, plus an App Store Connect API key for
  upload. Stored as repository secrets: the certificate as a base64 `.p12`
  with its password, the profile as base64, the API key as its three
  parts. The macOS runner installs them into a temporary keychain for the
  duration of the job and deletes it after.
- **Android**: an upload keystore, with Play App Signing holding the
  release key so the upload key can be rotated without a new listing.
  Stored as a base64 secret with its passwords.
- No signing material is ever committed, and the release job is the only
  workflow with access to these secrets.

### The release job

`mobile-release` runs on a `mobile-v*` tag:

1. Check out, install the mobile toolchains, run the `mobile` checks.
2. Build the signed IPA with `tauri ios build` and the signed AAB with
   `tauri android build`, injecting the version from the tag.
3. Upload the IPA to TestFlight and the AAB to the Play internal testing
   track, using the platform APIs directly. This is as far as automation
   goes.
4. Attach both artifacts and their checksums to a GitHub pre-release for
   the tag, for provenance, not for distribution.

Promotion from TestFlight and internal testing to the public stores is a
manual step, taken after the pairing walkthrough has passed on a real
device from that exact build. The walkthrough checklist records the build
number it was run against.

### Store listings

Both listings say plainly that the app requires a desktop running
Headstate and does nothing on its own, since a reviewer without one will
see an unpairable app. Provide a review note with a short video of the
pairing flow. The privacy declarations are: local network access, for
discovery and pairing; biometrics, for the step-up key; no data collected
by the developer, because there is no server. Screenshots use synthetic
repositories per `CONTRIBUTING.md`.

### First release gate

Store builds stay manual until the pairing walkthrough has been done twice
without findings. After that the release job is enabled and every tag
produces a TestFlight and internal-testing build.

## Open questions

- **Post-quantum TLS certificates.** Decided as a follow-up rather than
  v1; see Post-quantum posture. The trigger is rustls enabling ML-DSA
  signature schemes by default in a released version.
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
   against a desktop on the same wifi, and runs `get_cached`. Includes
   the keys plugin and the checks listed under Post-quantum posture.
4. **Allowlist and step-up.** Full read and write surface, destructive
   signature, desktop notification.
5. **Responsive pass.** PR list, detail, nudge, cleanup, connection banner.
6. **Security Policy update and pairing walkthrough.** Then a first
   manual TestFlight and Play internal build.
7. **Release pipeline.** Signing secrets, the `mobile-release` job, store
   listings, and the first public store submission.

## References

Consulted on 2026-09-05 for the post-quantum section.

- Apple, "Get ahead with quantum-secure cryptography", WWDC25 session
  314: <https://developer.apple.com/videos/play/wwdc2025/314/>
- Apple, `SecureEnclave.MLDSA65` reference:
  <https://developer.apple.com/documentation/cryptokit/secureenclave/mldsa65>
- Android Developers, "Android 17 is here", 16 June 2026:
  <https://developer.android.com/blog/posts/android-17-is-here>
- Android Developers, "The Fourth Beta of Android 17":
  <https://developer.android.com/blog/posts/the-fourth-beta-of-android-17>
- IETF, "Use of ML-DSA in TLS 1.3", draft-ietf-tls-mldsa.
- rustls, pull request "[0.23] Enable ML-DSA by default", open as of
  4 September 2026.
- RustCrypto `ml-dsa` and Cryspen `libcrux-ml-dsa` on crates.io.
