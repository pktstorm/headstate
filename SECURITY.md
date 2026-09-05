# Security Policy

## The token model

Headstate does not implement its own authentication or credential storage.
It delegates entirely to the [GitHub CLI](https://cli.github.com/):

- At startup, Headstate runs `gh auth token` and reads the token from its
  output.
- That token is held **in memory only**, for the lifetime of the running
  app. It is never written to Headstate's local SQLite database, never
  logged, and never included in an error message or a command's return
  value.
- Headstate stores no credentials of its own. If you revoke or rotate your
  `gh` session, Headstate has nothing left over to clean up — quitting the
  app is enough.
- The token is used only to talk to `api.github.com`. Headstate makes no
  other outbound network calls.

## Writes to GitHub

Headstate started strictly read-only. Since the write path landed it can
also act on a pull request — merge, close, mark as draft, enqueue, approve,
request changes, comment, resolve or reply to a review thread, re-run
checks, update the branch, toggle auto-merge, and open a package-update
PR — but **only when you click that action in the app**. Nothing runs on a
timer, nothing is batched, and each write goes through one module
(`src-tauri/src/github/mutate.rs`) so the full list is in one place. The
"nudge" feature is still clipboard-only: it composes a text block from data
already in memory and never posts anything anywhere on your behalf.

## Local storage

Headstate caches a snapshot of your open pull requests in a local SQLite
database (for fast startup and offline viewing between polls) and a small
merge-history table. This cache contains only pull request metadata
(titles, URLs, CI/review/merge state, labels) that Headstate already fetched
from `api.github.com` on your behalf — never your GitHub token or any other
credential.

## The remote listener and paired devices (Headstate 5.0)

Headstate 5.0 adds a companion phone app that pairs with a running desktop
and drives it remotely. **All of it is off until you enable it.** Nothing
in this section runs, listens, or exists on disk until you turn on
**Allow phone connections** in Settings, and everything above about GitHub
still holds: the GitHub token is still read from `gh auth token`, still
held in memory only, still never stored, and the desktop still talks to no
internet host other than `api.github.com`. The phone is not a second
GitHub client. It is a remote control for the one you already have.

### The desktop identity key

On first enable, the desktop generates its own ECDSA P-256 key pair and a
self-signed certificate (ten-year validity, not tied to a hostname). The
**private key is stored in the platform keychain** — the first and only
keychain entry Headstate makes. One exception, on Linux only: the
keychain there is the freedesktop Secret Service, which needs a daemon
(gnome-keyring, KWallet, KeePassXC) on the session bus, and a headless
box, a CI runner, or a bare window manager has none. When no Secret
Service is available, the key is kept instead in a file in Headstate's
own data directory, readable by the owning user only (mode 0600), and
the step down is logged at every start. macOS and Windows never fall
back. Wherever it lives, the key is the TLS server identity for the
listener, and nothing else: it is not a GitHub credential, it cannot be
used to talk to GitHub, and it never leaves the machine.

A phone does not trust this certificate by name or by any CA. At pairing
it pins the certificate's SHA256 fingerprint, delivered out of band in the
QR code, and refuses to connect to anything else. That is why **removing
the key from the keychain invalidates every pairing**: a new key means a
new fingerprint, every paired phone will refuse the new one, and each
phone must be paired again. Deleting the key is therefore also the blunt
way to revoke every phone at once.

### The listener

The listener is an HTTPS server on the desktop:

- **Off by default.** It starts when you enable **Allow phone connections**
  and stops when you disable it. It does not run at all otherwise.
- It binds to all interfaces on **port 41919**, a fixed port from the
  dynamic range so it never collides with a well-known service. It is
  reachable wherever the desktop is reachable — on the same network, or
  over an overlay such as Tailscale or WireGuard if you already run one.
  Headstate does not open any port on your router and does not integrate
  with any overlay.
- While it is on, the desktop announces itself on the local network over
  mDNS (`_headstate._tcp`) with its port and the first sixteen hex
  characters of its fingerprint, so a phone can find it after the LAN
  address changes. This is a local-network broadcast, not a call to any
  service.
- **TLS 1.3 only, mutual TLS.** Every connection must present a client
  certificate. The desktop accepts a certificate only when its SHA256
  fingerprint matches a row in `paired_devices` — no CA, no chain
  building. An unpaired or revoked certificate fails the TLS handshake,
  so an attacker on your network never reaches HTTP and no request body
  is ever read. The key exchange is hybrid X25519MLKEM768 (see below).

What it accepts, in full:

| Method | Path | Purpose |
|---|---|---|
| POST | `/v1/pair` | The only endpoint that admits an unpaired client, and only while a pairing token is live |
| POST | `/v1/call/{command}` | Invoke one allowlisted command; JSON in, JSON out |
| GET | `/v1/events` | Server-sent events, the same event names the desktop UI receives |
| GET | `/v1/hello` | Desktop version, protocol version, and the signed-in login, for the connection banner |

What it refuses:

- Any connection without a paired client certificate, at the handshake.
- Any `/v1/pair` request outside the two-minute window after you press
  **Pair a phone**. The pairing token is 32 random bytes, single use, and
  bound to both certificates' fingerprints, so a token seen in transit is
  useless without the matching phone key. Nothing is stored until you
  approve a confirmation dialog on the desktop naming the device and
  showing the fingerprint the phone also shows.
- Any command not on the remote allowlist. The allowlist lives in one Rust
  file (`src-tauri/src/remote/surface.rs`) and every command is classed as
  read, write, destructive, or local. **Local commands are never callable
  remotely** — anything that opens a window, reveals a file, changes
  autostart, restarts the app, or runs an editor or agent. A test asserts
  every registered command appears in exactly one class, so a new command
  is not remotely callable until someone deliberately adds it.
- Any replayed request. Every write carries a nonce and a timestamp; the
  desktop rejects a duplicate nonce and any timestamp more than sixty
  seconds off its clock.
- Any destructive command without a valid step-up signature (below).

Every command a phone issues runs on the desktop through the same code
path the desktop UI would use, under the same cleanup ledger and the same
logging. A phone cannot do anything the desktop could not.

### The `paired_devices` table

Paired phones are recorded in the same local SQLite database as the PR
cache, in a `paired_devices` table. **It holds public material only:** a
device name, the session certificate and its fingerprint, the phone's
ECDSA P-256 public key, its ML-DSA-65 public key if the phone has one, and
the pairing and last-seen timestamps. There is no private key, no pairing
token, and no GitHub token in it. Revoking a device in Settings, **Paired
devices**, deletes its row and closes any open connection from that
certificate; the phone finds out on its next handshake.

### The phone never holds the GitHub token

The desktop is the trust boundary. The phone never receives the GitHub
token, never talks to `api.github.com`, and has no poll loop of its own.
It sees only what the desktop sends it over `/v1/events` and `/v1/call`,
and it can act only by asking the desktop to act. A stolen phone whose
pairing has been revoked has nothing: no credential, no access, and a
cached PR snapshot it can no longer refresh. The phone's step-up signing
keys live in the Secure Enclave or Android Keystore and are not
exportable; its TLS session key is software-backed, because rustls must
hold the private key bytes to present a client certificate, and is kept
in the phone's keychain (an iOS Keychain item, or on Android a
preference encrypted under a Keystore-held AES key).

### Step-up for destructive commands

Commands that delete things — worktrees, local and remote branches, build
artifacts, virtualenvs, Docker images, volumes, and build cache, and
applying package updates — require more than a paired connection. Each
such request carries an `X-Headstate-Signature` header over the canonical
JSON of `{command, args, nonce, timestamp}`, produced by a signing key
that lives in the phone's secure hardware behind biometric or device
passcode access control. The gate is on the key itself, not on a prompt
the app shows, so a signature cannot be produced without the check.
Deleting a worktree from the phone costs one Face ID or fingerprint
prompt; reading or approving a PR does not.

The signing key is a **hybrid of ECDSA P-256 and ML-DSA-65**. P-256 is
always present and always in hardware; ML-DSA-65 is added whenever the
phone's keystore can generate one (iOS 26 and Android 17 on supporting
hardware). The pairing record says which keys the phone has, and the
desktop **verifies every signature that record says to expect** — a
pairing that registered an ML-DSA key is refused if a request carries
only the ECDSA signature, so a phone cannot be silently downgraded. The
desktop only ever holds the public halves.

Independently of the signature, the desktop posts a native notification
for every destructive command it executes on behalf of a phone, naming
the device.

### Post-quantum key exchange

The TLS key exchange between phone and desktop is **hybrid X25519MLKEM768**
by default, so traffic recorded off the wire today cannot be decrypted by
a future quantum computer. The TLS certificates themselves remain ECDSA
P-256 in 5.0 because ML-DSA in TLS 1.3 is still an IETF draft; since both
ends pin a fingerprint rather than validate a chain, migrating them later
is a matter of regenerating and re-pairing.

### Threat model

| Threat | Mitigation |
|---|---|
| Attacker on the same wifi connects to the listener | Client certificate required at the TLS layer; unpaired certs are refused before any request body is read |
| Attacker impersonates the desktop to the phone | Phone pins the desktop's certificate fingerprint, delivered out of band by QR code at pairing |
| Attacker scans the pairing QR over your shoulder | Pairing token is single use, expires in two minutes, and the desktop shows a confirmation dialog naming the device before the pairing is stored |
| Phone is stolen, unlocked | Destructive commands require a per-request signature from a biometric-gated key; the desktop can revoke the device |
| Phone is stolen, locked | The step-up signing keys live in the Secure Enclave or Android Keystore and are not exportable; the TLS session key is software-backed, kept in the phone's keychain, and is worthless once the desktop revokes the pairing |
| Desktop listener is reachable from the internet | Listener is off by default and binds only when the user enables it; even when on, unpaired clients are refused at the handshake |
| A paired phone issues a command the desktop UI could not | The remote surface is an explicit allowlist, enumerated in one Rust file, with local-only commands excluded |
| Request replay | Every write carries a nonce and timestamp; the desktop rejects duplicates and clock skew beyond sixty seconds |
| Traffic recorded today, decrypted by a future quantum computer | TLS key exchange is hybrid X25519 plus ML-KEM-768 by default |
| Step-up signatures forged by a future quantum computer | The step-up key is a hybrid of ECDSA P-256 and ML-DSA-65, held in the phone's secure hardware where the platform supports it |

The full design, including what is deliberately out of scope (no relay,
no push notifications, no hosted component of any kind), is in
`docs/superpowers/specs/2026-09-05-mobile-companion-design.md`.

## Reporting a vulnerability

If you find a security issue in Headstate, please report it privately
rather than opening a public issue, using GitHub's Security Advisory flow
for this repository:

**https://github.com/pktstorm/headstate/security/advisories/new**

Please include enough detail to reproduce the issue (steps, affected
version, and impact). We'll acknowledge reports and follow up as we
investigate.
