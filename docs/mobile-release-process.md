# Mobile companion: release process

How a `mobile-v*` tag becomes a TestFlight build and a Play internal-testing
build, and what a person still has to do around it. Design:
[Mobile Companion Design](superpowers/specs/2026-09-05-mobile-companion-design.md),
section *Release*. Workflow: `.github/workflows/mobile-release.yml`.

The desktop's release (`CONTRIBUTING.md`, "Cutting a release") is a
different pipeline with a different tag prefix. Nothing here touches it.

## Versioning

- The companion's version is its own, tagged `mobile-vX.Y.Z`, independent of
  the desktop's `vX.Y.Z`. Compatibility between the two is the wire
  protocol's integer version (returned by `/v1/hello`), not either app
  version.
- The tag is the single source of truth. The workflow stamps it into
  `src-mobile/tauri.conf.json` and `src-mobile/Cargo.toml` for the build
  (`scripts/stamp-mobile-version.py`); nothing is committed. The `0.1.0` in
  those files and in `gen/apple` is a placeholder that Tauri overwrites at
  build time.
- The **build number** is the workflow run number, and it is the same on
  both stores: `CFBundleVersion` on iOS, `versionCode` on Android. It is
  what the pairing walkthrough's run record asks for and what the job
  prints in its summary. Both stores require it to increase; a run number
  only ever increases for a given workflow file, so do not rename the file.
- Any tag on the same commit produces a new build number, so a re-tag after
  a failed upload is fine.

## Cutting a release

1. Make sure `main` is green and the mobile checks pass
   (`make lint-mobile test-mobile check-mobile-ios`; Android needs an NDK,
   see the Makefile).
2. Tag and push:

   ```
   git tag mobile-v0.2.0
   git push origin mobile-v0.2.0
   ```

3. Wait for the **Mobile release** workflow. Both jobs run the mobile checks
   first, then build, then upload. Failures stop before anything reaches a
   store; the pre-release is created only after both uploads succeed.
   The build number is in the job log ("Version 0.2.0, build 57") and in
   the pre-release notes.
4. When the build appears in TestFlight and on the Play internal track
   (a few minutes of processing each), run
   [the pairing walkthrough](mobile-pairing-walkthrough.md) on a real iPhone
   and a real Android device, recording **that build number** in the run
   record.
5. Promote manually, from the App Store Connect and Play Console web UIs,
   only after a clean run against that exact build. Automation stops at the
   testing tracks on purpose.

The GitHub pre-release holds the `.ipa`, the `.aab`, and a `SHA256SUMS`
file for provenance. Users do not install from it.

## Rehearsing without a tag

The workflow has a **Run workflow** button with a `dry_run` input, default
on. A dry run builds both apps **unsigned**, uploads them as workflow
artifacts (`mobile-ios`, `mobile-android`), and skips signing, both store
uploads, and the pre-release. It needs no secrets and does not need the
release to be enabled, so it is how to prove the pipeline works before
enabling it.

GitHub only shows the button for a workflow file that is on the default
branch, so a dry run cannot be triggered from a PR that adds or changes the
file; merge first, then dispatch.

A dispatch with `dry_run` off is a real release and is refused unless it is
run from a `mobile-v*` tag with the release enabled.

## Enabling the release (the first release gate)

Both build jobs are gated on the repository variable
`MOBILE_RELEASE_ENABLED`. Until it is `true`, a `mobile-v*` tag does
nothing (the jobs show as skipped). The spec keeps store builds manual until
the pairing walkthrough has passed **twice with no findings**; when it has:

1. Settings > Secrets and variables > Actions > **Variables** > New
   repository variable.
2. Name `MOBILE_RELEASE_ENABLED`, value `true`.

Set it to anything else (or delete it) to disable again. No workflow edit is
needed in either direction.

## Secrets

Settings > Secrets and variables > Actions > **Secrets**. This workflow is
the only one that reads them; the CI workflow has no access to any of them.
The workflow checks that every secret it needs is present before building
and names the missing ones.

### iOS

| Secret | What it is |
|---|---|
| `IOS_DIST_CERT_P12_BASE64` | Base64 of a `.p12` export of an **Apple Distribution** certificate with its private key |
| `IOS_DIST_CERT_PASSWORD` | The password the `.p12` was exported with |
| `IOS_PROVISIONING_PROFILE_BASE64` | Base64 of an **App Store** provisioning profile for `com.pktstorm.headstate.companion` |
| `APPSTORE_API_KEY_ID` | The App Store Connect API key's Key ID |
| `APPSTORE_API_ISSUER_ID` | The Issuer ID shown above the keys table |
| `APPSTORE_API_PRIVATE_KEY` | The contents of the key's `.p8` file, including the BEGIN/END lines |

How to generate them:

- **Certificate.** In Keychain Access, Certificate Assistant > Request a
  Certificate From a Certificate Authority, saved to disk. At
  developer.apple.com > Certificates, create an *Apple Distribution*
  certificate from that request and download it; double-click to install.
  Back in Keychain Access, select the certificate **and** its private key,
  File > Export Items, format `.p12`, set a password. Then
  `base64 -i dist.p12 | pbcopy`.
  Not a *Developer ID Application* certificate (that is the desktop's, and
  cannot sign for the App Store) and not *Apple Development* (device
  testing only).
- **Provisioning profile.** developer.apple.com > Profiles > new, type
  *App Store Connect* (under Distribution), for the companion's App ID and
  the certificate above. Download, then
  `base64 -i profile.mobileprovision | pbcopy`. Regenerate it whenever the
  certificate is renewed: a profile is bound to the certificate it was made
  with.
- **API key.** App Store Connect > Users and Access > Integrations > App
  Store Connect API > Team Keys > generate, role *App Manager* (the least
  role that can upload builds). The Key ID and Issuer ID are on that page;
  the `.p8` downloads once and cannot be re-downloaded, so put it in the
  secret immediately: `pbcopy < AuthKey_XXXXXXXXXX.p8`.

The Apple team ID is not a secret: Tauri reads it from the certificate.

### Android

| Secret | What it is |
|---|---|
| `ANDROID_UPLOAD_KEYSTORE_BASE64` | Base64 of the **upload** keystore (`.jks`) |
| `ANDROID_KEYSTORE_PASSWORD` | The keystore's password |
| `ANDROID_KEY_ALIAS` | The alias of the upload key inside it (`upload` below) |
| `ANDROID_KEY_PASSWORD` | The key's password (a PKCS12 keystore, the default, requires this to equal the keystore password) |
| `PLAY_SERVICE_ACCOUNT_JSON` | The contents of a Google Cloud service account's JSON key |

How to generate them:

- **Upload keystore.** Per Tauri's Android signing guide:

  ```
  keytool -genkey -v -keystore upload-keystore.jks -keyalg RSA -keysize 2048 \
    -validity 10000 -alias upload
  ```

  Then `base64 -i upload-keystore.jks | pbcopy`. Keep the file somewhere
  safe too: losing it means requesting an upload-key reset from Google.
- **Play App Signing.** In the Play Console the app must be enrolled in
  Play App Signing (the default for new apps), so Google holds the *app
  signing key* and this keystore is only the *upload key*. That is what
  lets the upload key be rotated without a new listing, as the spec
  requires. Register the upload certificate there
  (`keytool -export -rfc -keystore upload-keystore.jks -alias upload -file upload.pem`).
- **Service account.** Google Cloud Console > IAM > Service Accounts > create
  (no roles needed in Cloud), then Keys > Add key > JSON; that file is the
  secret (`pbcopy < service-account.json`). In the Play Console, Users and
  permissions > invite the service account's email, grant it the app with
  *Release to testing tracks*. It needs nothing broader: promotion is manual.

The signing block Gradle needs is added to the generated Android project by
`scripts/android-release-signing.py`; the workflow runs it on every build,
and it is a no-op once the block is present (it will be committed with
`gen/android`). The keystore is written to `gen/android/keystore.properties`
for the build and deleted afterwards; that path is in `.gitignore` twice.

## What the workflow does, per platform

**iOS** (`macos-latest`, Xcode 26): run the mobile checks; create a
throwaway keychain and import the certificate; install the provisioning
profile; `tauri ios build --ci --export-method app-store-connect` with the
certificate and profile also given to the Tauri CLI, which switches the
Xcode project to manual signing for that build; `xcrun altool
--upload-package` with the API key; delete the keychain, profile and key
(also on failure).

**Android** (`ubuntu-latest`, pinned NDK, the same toolchain steps as CI's
`mobile-android` job): run the mobile checks; generate `gen/android` if it
is not committed yet; write `keystore.properties`; `tauri android build
--ci --aab --target aarch64`; verify the bundle is signed; upload to the
`internal` track as a completed release through the Play Developer API
(`scripts/play-upload.py`); delete the keystore and credentials.

**Publish**: both artifacts and one `SHA256SUMS`, verified before upload,
attached to a GitHub pre-release for the tag with notes generated against
the previous `mobile-v*` tag.

## Rotating material

- Certificate expired or revoked: new certificate **and** new profile;
  update both secrets.
- Upload key compromised: Play Console > Setup > App signing > request
  upload key reset; new keystore; update the three Android secrets.
- API key or service account leaked: revoke it where it was created and
  replace the secret. Neither can promote a build, only upload one.
