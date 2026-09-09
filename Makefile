.PHONY: dev build test test-rust test-ui lint lint-rust lint-ui lint-deps fmt icons \
	mobile-frontend lint-mobile test-mobile check-mobile-ios check-mobile-android \
	deny-mobile ios-init android-init icons-mobile ios-device android-device

# ---- Mobile companion (src-mobile) ---------------------------------------
#
# A separate crate with its own lockfile; see src-mobile/Cargo.toml for
# why. None of these targets is part of `lint` or `test` above: the
# desktop gates stay exactly what CI runs, and the mobile ones need the
# iOS or Android toolchain (#518 gives them their own CI job).
#
# `TAURI_APP_PATH` is load-bearing on every `yarn tauri` call. Yarn runs
# package scripts from the workspace root, and from there the Tauri CLI
# finds `src-tauri` first -- so without it `yarn tauri ios init` sets up
# the DESKTOP crate for iOS. This was observed, not inferred.
#
# The shared frontend, built for the phone. `tauri ios build` runs this
# itself through `beforeBuildCommand`; it is here for anyone driving
# xcodebuild directly. `cargo check` and `cargo test` do NOT need it:
# the crate was verified to compile with `dist/` absent.
mobile-frontend:
	VITE_TARGET=mobile yarn build

# `--workspace`: src-mobile is a small workspace whose members are the
# in-repo plugins under src-mobile/plugins; without the flag only the
# app crate is linted and tested.
lint-mobile:
	cd src-mobile && cargo fmt --check
	cd src-mobile && cargo clippy --workspace --all-targets -- -D warnings
	# No Kotlin is compiled anywhere -- not by this target, not in CI, which
	# generates the Android Studio project and never runs Gradle. Tauri
	# dispatches on the LITERAL @Command method name, so a name that does not
	# match what Rust invokes fails on a device and nowhere else (#698).
	# Not prefixed with `cd src-mobile`: each recipe line is its own shell,
	# so this one starts at the repo root like the rest.
	python3 scripts/check-plugin-commands.py

test-mobile:
	cd src-mobile && cargo test --workspace

# Proves the phone-only dependencies (reqwest on rustls/aws-lc-rs, rcgen)
# cross-compile: aws-lc-sys builds C and assembly for the target, which
# a host `cargo check` never exercises.
check-mobile-ios:
	rustup target add aarch64-apple-ios
	cd src-mobile && cargo check --target aarch64-apple-ios

# Needs an Android NDK: aws-lc-sys looks for `aarch64-linux-android-clang`
# and fails without one (observed). Run through `yarn tauri android`
# tooling or with NDK_HOME set.
check-mobile-android:
	rustup target add aarch64-linux-android
	cd src-mobile && cargo check --target aarch64-linux-android

deny-mobile:
	cd src-mobile && cargo deny check

# Install and run on a REAL device, for the pairing walkthrough
# (docs/mobile-pairing-walkthrough.md). The walkthrough refuses
# simulators and emulators, correctly: they have no Secure Enclave, no
# Keystore-backed biometric gate, and iOS does not show the
# local-network prompt in the simulator -- so three of the things the
# run exists to check cannot be checked there.
#
# `--open` hands off to Xcode rather than building headless. Signing a
# development build needs a team, and the committed project carries none
# (`DEVELOPMENT_TEAM` is absent by design -- it is personal to whoever
# builds, and the release workflow injects its own). Xcode's Signing &
# Capabilities tab is where a person selects theirs, once, and the
# setting stays in their local checkout.
#
# `--host` goes with `--open`, per Tauri's own help: a device cannot
# reach `localhost`, so the dev server has to be served on the public
# network address. Vite already listens on 0.0.0.0 for this to work.
#
# Not a release path. Store builds come from `mobile-release.yml` on a
# `mobile-v*` tag; see docs/mobile-release-process.md.
ios-device:
	TAURI_APP_PATH=src-mobile yarn tauri ios dev --open --host

# The Android equivalent. `tauri android dev` installs over adb, so a
# device with USB debugging on and `adb devices` listing it is all that
# is needed -- no signing team, and no Play Console.
android-device:
	TAURI_APP_PATH=src-mobile yarn tauri android dev

# Regenerates gen/apple. The generated project is committed; re-run only
# when Tauri's template changes, and review the diff.
ios-init:
	TAURI_APP_PATH=src-mobile yarn tauri ios init --ci

# Same for gen/android. Needs an Android SDK with ANDROID_HOME and
# NDK_HOME set; the `mobile-android` CI job is the machine that has one.
android-init:
	TAURI_APP_PATH=src-mobile yarn tauri android init --ci

# The companion's icons, from the same master as the desktop. `yarn tauri
# icon` emits every platform's variant; the phone keeps the iOS and
# Android sets plus the 1024px source, and the desktop's icons are not
# touched.
icons-mobile:
	yarn tauri icon src-tauri/icons/icon-master.png -o src-mobile/icons
	cd src-mobile/icons && rm -f 128x128.png 128x128@2x.png 32x32.png 64x64.png \
		icon.icns icon.ico Square*.png StoreLogo.png

dev:
	yarn tauri dev

build:
	yarn tauri build

test: test-rust test-ui

test-rust:
	cd src-tauri && cargo test

test-ui:
	yarn vitest run

lint: lint-rust lint-ui lint-deps

# Guards that answer a question in a second which would otherwise be
# answered by a job that takes minutes. `tauri build` refuses to bundle
# when an @tauri-apps/* package and its Rust crate disagree on
# major/minor, and that check lives inside the bundle -- so before this
# target existed, the mismatch passed lint and both test jobs and failed
# from the slowest one in CI (#555).
lint-deps:
	python3 scripts/check-tauri-versions.test.py
	python3 scripts/check-tauri-versions.py
	python3 scripts/android-release-signing.test.py

lint-rust:
	cd src-tauri && cargo fmt --check
	cd src-tauri && cargo clippy --all-targets -- -D warnings

lint-ui:
	yarn tsc -b --force
	yarn eslint .
	yarn knip
	# The focus ring is CSS the test suite structurally cannot see: jsdom
	# applies no stylesheets, `?raw` on a .css file returns empty because
	# @tailwindcss/vite claims it, and tests avoid node:fs. Deleting the
	# rule un-fixes every button in the app with a green suite (#694).
	./scripts/check-focus-css.sh

fmt:
	cd src-tauri && cargo fmt

# Requires Pillow: pip install -r scripts/requirements.txt
#
# `yarn tauri icon` also emits Windows/iOS/Android icon variants this
# macOS-only app never uses, and its ICNS encoder is non-deterministic --
# re-running against unchanged source art re-packs icon.icns with different
# compressed-stream bytes even though every image inside is pixel-identical.
# Restore the 1024 master over icon.png (as before), prune the unused
# variants, and restore the committed icon.icns bytes when its *content*
# (not raw bytes) matches what's already committed -- so a second run of
# this target leaves `git status` clean.
icons:
	python3 scripts/make-icons.py
	yarn tauri icon src-tauri/icons/icon.png
	cp src-tauri/icons/icon-master.png src-tauri/icons/icon.png
	rm -rf src-tauri/icons/android src-tauri/icons/ios
	# icon.ico is KEPT: tauri_build embeds it as a Windows resource, and
	# without it the build script fails before compiling any app code.
	# Deleting it was correct while this was macOS-only and is not now.
	rm -f src-tauri/icons/StoreLogo.png
	rm -f src-tauri/icons/Square*.png src-tauri/icons/64x64.png
	python3 scripts/make-icons.py --restore-icns-if-unchanged
