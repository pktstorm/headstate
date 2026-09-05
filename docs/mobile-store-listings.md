# Mobile companion: store listings and the first public release

Everything that goes into App Store Connect and the Play Console for
Headstate Companion, written down once so a submission is a copy-paste
job and every answer to a store questionnaire is on record. The release
mechanics (tag, release job, walkthrough, promotion) are in
[mobile-release-process.md](mobile-release-process.md); this file is the
listing and the gate.

Design: [Mobile Companion Design](superpowers/specs/2026-09-05-mobile-companion-design.md)
(see *Release*, *Store listings* and *First release gate*).

Every value here is synthetic or public. Nothing in this file, and
nothing in a committed screenshot, may name a private repository,
hostname, or address: see `CONTRIBUTING.md`, *The privacy rule*.

## Identity

| Field | Value |
|---|---|
| Bundle identifier / application ID | `com.pktstorm.headstate.companion` |
| App name (both stores) | Headstate Companion |
| Developer | pktstorm |
| Source | <https://github.com/pktstorm/headstate> |
| Support URL | <https://github.com/pktstorm/headstate/issues> |
| Marketing URL (optional) | <https://github.com/pktstorm/headstate> |
| Privacy policy URL | <https://github.com/pktstorm/headstate/blob/main/docs/mobile-store-listings.md#privacy-policy> |
| Licence | Apache-2.0, as `src-mobile/Cargo.toml` says |
| Price | Free, no in-app purchases, no ads |
| Version | From the `mobile-vX.Y.Z` tag; the desktop's `vX.Y.Z` is separate |

The identifier is fixed in `src-mobile/tauri.conf.json` and in
`src-mobile/gen/apple/project.yml`; both stores make it permanent on
first upload. The Play developer contact email is the account's, entered
in the Console, not written here.

## App Store Connect

Character limits are Apple's: name 30, subtitle 30, promotional text
170, keywords 100 (comma separated, no spaces after commas), description
4000.

**Name** (30): `Headstate Companion`

**Subtitle** (30): `Your desktop's pull requests`

**Promotional text** (170):

> Requires Headstate on a Mac on the same network. Pair once by QR code,
> then read, review, and clean up from your phone. Nothing leaves your
> own machines.

**Description**:

> Headstate Companion is a remote control for Headstate, the open-source
> macOS app that watches your open pull requests. It does nothing on its
> own: it needs a desktop running Headstate, on the same wifi or the same
> overlay network, and it shows an unpairable screen until it has one.
>
> Pair once by scanning a QR code shown on the desktop and confirming the
> fingerprint on both screens. From then on the phone talks only to that
> desktop, over a mutually authenticated TLS connection that no other
> device can join.
>
> WHAT YOU CAN DO FROM THE PHONE
> • See the same pull request list the desktop shows, with CI and
>   mergeability at a glance
> • Open a pull request: checks, reviews, and threads
> • Approve, comment, and reply, exactly as you would at the desk
> • Build a nudge list of the pull requests waiting on someone
> • Clean up: worktrees, build artifacts, virtual environments, and Docker
>   images, with size and age, selected and removed from the phone
>
> Anything that deletes something asks for Face ID or Touch ID first.
> The key that authorises it lives in the Secure Enclave and never leaves
> the phone.
>
> WHAT IT DOES NOT DO
> • It has no GitHub login of its own. The desktop keeps the token; the
>   phone only asks the desktop.
> • It talks to no server. There is no account, no relay, no analytics,
>   and no data collected by the developer.
> • It does nothing when the desktop is away: it shows the last snapshot
>   it received, marked stale, with every action disabled.
>
> Headstate Companion is open source under the Apache 2.0 licence. The
> desktop app, the source, and the security policy are at
> github.com/pktstorm/headstate.

**Keywords** (100):

`pull request,code review,github,developer,git,worktree,remote,companion,ci,merge`

**Primary category**: Developer Tools. **Secondary**: Productivity.

**Age rating**: answer every content question "None"; the result is 4+.

**Copyright**: `© <year> pktstorm`.

**Version release**: manual. The build is promoted from TestFlight by
hand after the walkthrough passes on it; see *First public release*
below.

**Device family**: the generated Xcode project (`src-mobile/gen/apple/
project.yml`) does not set `TARGETED_DEVICE_FAMILY`, and Xcode's default
is iPhone and iPad. Tablet layouts are out of scope for v1 (design spec,
*Out of scope for v1*), and App Store Connect requires iPad screenshots
for any build that runs on iPad. Before the first upload, either set the
target to iPhone only or accept the iPad screenshot requirement; this
document assumes iPhone only.

### App Review notes

Paste into *App Review Information*, *Notes*. Attach the pairing video
(see *The review video*) as the demo, and leave *Sign-in required*
unchecked: there is no account.

> Headstate Companion is a remote control for a macOS application,
> Headstate (open source: github.com/pktstorm/headstate). It requires a
> Mac on the same local network running that application, and it does
> nothing on its own. Without a desktop to pair with, the app shows only
> its pairing screen, which is the expected and documented behaviour.
>
> Pairing works by scanning a QR code that the desktop displays and
> confirming a certificate fingerprint on both screens. The attached
> video shows the complete flow on a real iPhone and a real Mac: enabling
> the listener on the desktop, scanning the code, approving on the
> desktop, the pull request list appearing on the phone, and a
> destructive action (removing a git worktree) prompting for Face ID
> before it runs.
>
> Local network permission is requested to find and connect to the
> desktop (Bonjour, `_headstate._tcp`). Face ID is used to unlock a
> signing key held in the Secure Enclave; that key authorises
> destructive commands and nothing else. The app has no server, no
> account, and collects no data. All pull request content shown in the
> video comes from public demonstration repositories (octocat/hello-world
> and octocat/spoon-knife).
>
> The app cannot be exercised past the pairing screen without a Mac
> running Headstate. If the reviewer needs one, the desktop app is a free
> download from the GitHub releases page linked above.

### Privacy nutrition label

*App Privacy*, *Data Collection*: **Data Not Collected**. Every category
below is answered "No" to "Do you or your third-party partners collect
this data?", which is what produces that label.

| Category | Collected? | Why not |
|---|---|---|
| Contact Info (name, email, phone, address, other) | No | No account. The device name typed at pairing is sent to the user's own desktop and stored there, nowhere else. |
| Health & Fitness | No | Not used. |
| Financial Info | No | Not used; no purchases. |
| Location (precise, coarse) | No | Not used. Local network discovery is by Bonjour, not location. |
| Sensitive Info | No | Not used. |
| Contacts | No | Not used. |
| User Content (emails, messages, photos, audio, gameplay, customer support, other) | No | Review comments typed on the phone go to the user's own desktop, which posts them to GitHub with the user's own token. The developer never sees them. |
| Browsing History | No | Not used. |
| Search History | No | Not used. |
| Identifiers (user ID, device ID) | No | The certificate fingerprint identifies the phone to the user's own desktop only. It is never sent to the developer. |
| Purchases | No | None. |
| Usage Data (product interaction, advertising data, other) | No | No analytics SDK, no telemetry. |
| Diagnostics (crash data, performance data, other) | No | The app writes a log file on the device (`tauri-plugin-log`). It is never transmitted; a user may attach it to a bug report by hand. |
| Other Data | No | None. |

**Tracking**: No, the app does not track. There is no advertising, no
third-party SDK, and no data leaves the user's own devices.

**Camera** (the QR scanner) and **Face ID** are permissions, not data
collection; they are declared in `Info.plist`, not on the label.

## Play Console

Character limits are Google's: app name 30, short description 80, full
description 4000.

**App name** (30): `Headstate Companion`

**Short description** (80):

`Drive your desktop Headstate from your phone. Needs Headstate on a Mac.`

**Full description**: the App Store description above, verbatim, with
"Face ID or Touch ID" replaced by "your fingerprint or face", "Secure
Enclave" by "the Android Keystore", and the WHAT YOU CAN DO / WHAT IT
DOES NOT DO headings kept. Play shows the description as plain text;
the `•` characters are fine.

**Category**: Tools. **Tags**: Developer tools, Productivity.

**Contact details**: the developer account's email; the support URL
above as the website.

**Content rating** (IARC questionnaire): Utility, Productivity,
Communication, or Other; answer every content question "No". Result:
Everyone / PEGI 3.

**Target audience**: 18 and over. Not designed for children.

**Ads**: No. **News app**: No. **Government app**: No. **Financial
features**: None.

**Release**: internal testing track from the release job; promotion to
production is manual (see *First public release*).

### App access

*Policy*, *App access*: **All or some functionality is restricted**.
Instructions:

> The app is a remote control for a macOS application (Headstate,
> github.com/pktstorm/headstate) and shows only a pairing screen until
> it is paired with a Mac on the same local network running that
> application. There is no login and no credentials to supply. A video of
> the full pairing flow and every screen behind it is linked from the
> app's listing notes; the same video is attached to the iOS review. The
> desktop application is a free download from the GitHub page above.

### Data safety

*Policy*, *Data safety*.

**Does your app collect or share any of the required user data types?**
No.

That answer skips the per-type questions, but the reasoning per type is
recorded here so the answer can be defended:

| Data type | Collected | Shared | Why not |
|---|---|---|---|
| Location (approximate, precise) | No | No | Not used. |
| Personal info (name, email, user IDs, address, phone, race, political or religious beliefs, sexual orientation, other) | No | No | No account. The device name goes to the user's own desktop only. |
| Financial info | No | No | Not used. |
| Health and fitness | No | No | Not used. |
| Messages (emails, SMS, other in-app messages) | No | No | Review comments go to the user's own desktop, which posts them to GitHub under the user's own token. |
| Photos and videos | No | No | The camera is used to scan a QR code; no image is stored or sent. |
| Audio files | No | No | Not used. |
| Files and docs | No | No | Not used. |
| Calendar | No | No | Not used. |
| Contacts | No | No | Not used. |
| App activity (interactions, in-app search, installed apps, other user-generated content, other actions) | No | No | No analytics. |
| Web browsing | No | No | Not used. |
| App info and performance (crash logs, diagnostics, other) | No | No | The on-device log file is never transmitted. |
| Device or other IDs | No | No | The certificate fingerprint identifies the phone to the user's own desktop only. |

**Security practices**

- *Is all of the user data collected by your app encrypted in transit?*
  Yes. Everything the app sends goes to the user's own desktop over TLS
  1.3 with mutual certificate authentication. (Asked even when nothing
  is collected; answer it truthfully.)
- *Do you provide a way for users to request that their data is
  deleted?* Not applicable: no data is collected. If the form insists,
  the answer is that *Forget desktop* on the phone and *Revoke* on the
  desktop delete every record either side holds about the other.
- *Independent security review*: No.

### Permissions

Play's *Permissions declaration* form covers only sensitive permissions
(SMS, call log, accessibility, and the like). None apply. The manifest
Tauri generates for the companion asks for `INTERNET`, and the barcode
scanner and biometric plugins add `CAMERA` and `USE_BIOMETRIC` when they
land. Nothing to declare.

## Permission strings (iOS)

What `src-mobile/Info.ios.plist` declares today; the Tauri CLI merges it
into the generated project's `Info.plist`. The strings appear verbatim in
the iOS permission prompt and are quoted in the App Review note above,
so they must not drift from the plist.

| Key | Value |
|---|---|
| `NSLocalNetworkUsageDescription` | Headstate Companion looks for your desktop Headstate on the local network to pair with it and send it commands. |
| `NSBonjourServiceTypes` | `_headstate._tcp` |

Two more are needed before a store build and are **not in the plist
yet**; they belong with the plugins that use them (`capabilities/
default.json` says as much) and are recorded here so the wording is
settled before they are added:

| Key | Proposed value |
|---|---|
| `NSCameraUsageDescription` | Headstate Companion uses the camera to scan the pairing code shown on your desktop. |
| `NSFaceIDUsageDescription` | Headstate Companion uses Face ID to unlock the key that authorises deleting things from your desktop. |

iOS terminates an app that touches the camera or Face ID without the
matching string, and App Review rejects a binary whose strings do not
say what the permission is for. Check both are present in the built
`Info.plist` (`gen/apple/headstate-companion_iOS/Info.plist` after
`tauri ios build`) as part of the pre-submission checklist below.

## Privacy policy

Both stores require a URL; the URL above points at this section.

> **Headstate Companion privacy policy**
>
> Headstate Companion is a remote control for the Headstate desktop
> application. It is developed by pktstorm and published as open source
> at github.com/pktstorm/headstate.
>
> **No data is collected by the developer.** The app has no server, no
> account, no analytics, no advertising, and no third-party SDK that
> reports anywhere. The developer receives nothing from the app, ever.
>
> **Where the app sends data.** The app communicates with exactly one
> party: the desktop computer you paired it with, which is your own. It
> sends that desktop the commands you issue (for example "show me this
> pull request", "approve", "remove this worktree") and the name you gave
> the phone at pairing. The desktop, not the phone, talks to GitHub,
> using a token that only the desktop holds. The connection is encrypted
> with TLS 1.3 and authenticated in both directions by certificates that
> the two devices exchanged when you paired them.
>
> **Local network.** The app uses local network access to find your
> desktop (Bonjour, service type `_headstate._tcp`) and to connect to
> it. It does not scan for, connect to, or record any other device.
>
> **Camera.** The camera is used only to scan the pairing QR code shown
> on your desktop. No image is stored or sent anywhere.
>
> **Biometrics.** Face ID, Touch ID, or your Android biometric protect a
> signing key that lives in the phone's secure hardware (Secure Enclave
> or Android Keystore). Actions that delete something on the desktop
> require a signature from that key, which is why the prompt appears.
> The app never sees your biometric data; the operating system checks it
> and unlocks the key.
>
> **What is stored on the phone.** The paired desktop's name, address,
> and certificate fingerprint; the phone's own certificate and keys; and
> the most recent snapshot of pull requests the desktop sent, so the app
> can show something while the desktop is unreachable. *Forget desktop*
> in the app deletes all of it. A diagnostic log is written to the app's
> own storage and is never transmitted.
>
> **Revocation.** The desktop can revoke a paired phone at any time, after
> which the phone is refused at the connection level and returns to its
> pairing screen.
>
> **Changes.** This policy is versioned with the source code; its history
> is the file's git history.
>
> **Contact.** Open an issue at github.com/pktstorm/headstate/issues.

## The review video

Both review notes point at one video of the pairing flow. It is recorded
by the maintainer on a real iPhone and a real Mac, never committed to the
repository (size, and a video cannot be privacy-scanned), and attached
directly in App Store Connect; the Play listing note links to the same
file wherever it is hosted.

What it shows, in order, and nothing else: desktop Settings, *Pair a
phone*, the QR and countdown; the phone scanning it and showing the
fingerprint; the desktop's approval modal with the same fingerprint;
approval; the PR list on the phone with the connection banner; one PR
detail; Cleanup, one worktree selected, *Remove*, the Face ID prompt,
and the worktree gone on the desktop. Under two minutes.

Same rule as screenshots: the desktop is signed in to a throwaway
account whose only checkouts are `octocat/hello-world` and
`octocat/spoon-knife`, the desktop is named `octocat's laptop`, and the
phone `Octocat's phone`. Nothing else may be on screen.

## Screenshots

Five screens, in listing order. Each one is a real capture on a real
device (a simulator has no local-network prompt and no biometric, and a
mocked screen is not the product), with the phone paired to a desktop
that holds only synthetic repositories. The captions are the store's
optional text over each image.

| # | Screen | How to get there | Caption |
|---|---|---|---|
| 1 | PR list with the connection banner | Paired, connected, list view, banner reading `octocat's laptop · reachable · last poll …` | Your desktop's pull requests, on your phone |
| 2 | PR detail | Open the `octocat/hello-world` PR "Add retry to the fetch client" | Checks, reviews, and threads, ready to act on |
| 3 | Nudge wizard | From the list, *Nudge*, with two PRs selected | Who is waiting on whom, paste-ready |
| 4 | Cleanup: worktrees | Cleanup, Worktrees, one throwaway worktree selected, *Remove* visible | Reclaim disk from anywhere; deletion asks for Face ID |
| 5 | Pairing | Fresh install or *Forget desktop*: the scan screen with the fingerprint from a just-scanned QR | Pair once by QR code and compare the fingerprint |

The banner's exact wording comes from `src/components/ConnectionBanner.tsx`;
if the copy changes, retake screenshot 1 rather than editing the image.

### Synthetic data, and only synthetic data

- The desktop is signed in to a throwaway GitHub account. Its only
  checkouts are `octocat/hello-world` and `octocat/spoon-knife`; the
  pull requests visible are the ones the fixtures in `src/fixtures/prs.ts`
  model ("Add retry to the fetch client", "Fix flaky timezone test",
  "Bump the parser dependency"), opened on those throwaway checkouts for
  the purpose. No other repository, organisation, avatar, or login may
  appear.
- Desktop name `octocat's laptop`, phone name `Octocat's phone`. Any
  address on screen is from the RFC 5737 range (`192.0.2.10`) or the
  CGNAT range (`100.64.0.7`).
- Status bar: use the platform's clean-status-bar tooling (Xcode's
  `simctl status_bar` does not apply to a device; on iOS use the
  Developer settings or crop; on Android use the demo mode). No carrier
  name, no notification icons.
- Before committing, look at every image at full size and read every
  string in it against the rule above. `scripts/check-privacy.sh` is
  run over the commit too, but `git grep` skips binary files: the script
  guards the file names, the captions, and this document, not the pixels.
  The eyes are the gate for the images.

### Sizes

Apple, iPhone only (see *Device family* above):

| Display | Pixels (portrait) | Required |
|---|---|---|
| 6.9" (iPhone 16 Pro Max class) | 1320 × 2868 | Yes |
| 6.5" (iPhone 11 Pro Max / XS Max class) | 1284 × 2778 or 1242 × 2688 | Optional: App Store Connect scales the 6.9" set down when this one is absent. Upload it when a device is at hand |

App Store Connect accepts up to ten per size; ship five. Apple's
screenshot specifications page is the authority on sizes and changes
with each hardware generation; check it on the day.

Google Play, phone:

| Asset | Size | Required |
|---|---|---|
| Phone screenshots | 16:9 or 9:16, each side 320–3840 px; a 1080 × 2400 capture is fine | At least 2, ship the same 5 |
| Feature graphic | 1024 × 500 PNG or JPEG, no alpha | Yes |
| App icon | 512 × 512 PNG, 32-bit | Yes; export from `src-mobile/icons/icon.png` |

The feature graphic is the icon on the app's background colour
(`#0d1117`, from `tauri.conf.json`) with the name beside it, no
screenshot and no device frame. Seven-inch and ten-inch tablet
screenshots are not required while the Play listing does not opt into
tablets.

### Where they live

Committed under `docs/store/` as `ios-<n>-<slug>.png` and
`android-<n>-<slug>.png`, matching the table order, plus
`android-feature.png`. Stage new files with `git add -N` before running
`scripts/check-privacy.sh`, which refuses to run with untracked files
present. Retake rather than edit: an image whose text no longer matches
the app is worse than a missing one.

## Pre-submission checklist

Run through in order for every store submission, not only the first.

- [ ] The build under review is the exact build number the pairing
      walkthrough passed on, on a real device, and the run record is
      committed under `docs/walkthroughs/`.
- [ ] `src-mobile/Info.ios.plist` carries the four permission strings
      above with the wording above, and the built `Info.plist` shows
      them.
- [ ] The banner wording in screenshot 1 matches
      `src/components/ConnectionBanner.tsx` on the tagged commit.
- [ ] Every screenshot has been read at full size against the privacy
      rule, and `scripts/check-privacy.sh` printed `privacy check: clean`
      on the commit that added them.
- [ ] The privacy policy URL above resolves to this section on `main`.
- [ ] Apple: the nutrition label reads *Data Not Collected*; the review
      note is pasted and the video attached; *Sign-in required* is
      unchecked; the version release is set to manual.
- [ ] Google: *Data safety* answers "No" to collection; *App access* has
      the restricted-functionality note; the content rating is
      Everyone; the target audience is 18+.
- [ ] Both descriptions still say, in the first paragraph, that the app
      requires a desktop running Headstate and does nothing on its own.

## First public release

The first promotion from TestFlight and the Play internal-testing track
to the public stores is a manual step, and it stays manual for every
release after it. The mechanics of producing the build, waiting for the
release job, and running the walkthrough against that build number are
in [mobile-release-process.md](mobile-release-process.md); this section
is only the gate.

1. Store builds are produced by hand until the pairing walkthrough
   (`docs/mobile-pairing-walkthrough.md`) has passed **twice with no
   findings**. Only then is the `mobile-release` job enabled.
2. For the release under consideration, the walkthrough has been run on a
   real iPhone and a real Android device from the **exact build number**
   sitting in TestFlight and internal testing, and the run records are
   committed.
3. The pre-submission checklist above is clean.
4. Promote by hand: App Store Connect, the build's *Submit for Review*
   with manual release; Play Console, *Promote release* from internal
   testing to production. Neither is automated, by design.
5. After the stores approve, release. The tag, the GitHub pre-release,
   and the store builds all carry the same version.

What remains manual for the maintainer, every time: the store accounts
and their forms, the review video, real-device screenshots, the
walkthrough runs, and the promotion itself. This document exists so that
none of those steps involves composing text under time pressure.
