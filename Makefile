.PHONY: dev build test test-rust test-ui lint lint-rust lint-ui fmt icons \
	mobile-frontend lint-mobile test-mobile check-mobile-ios check-mobile-android \
	deny-mobile ios-init icons-mobile

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

lint-mobile:
	cd src-mobile && cargo fmt --check
	cd src-mobile && cargo clippy --all-targets -- -D warnings

test-mobile:
	cd src-mobile && cargo test

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

# Regenerates gen/apple. The generated project is committed; re-run only
# when Tauri's template changes, and review the diff.
ios-init:
	TAURI_APP_PATH=src-mobile yarn tauri ios init --ci

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

lint: lint-rust lint-ui

lint-rust:
	cd src-tauri && cargo fmt --check
	cd src-tauri && cargo clippy --all-targets -- -D warnings

lint-ui:
	yarn tsc -b --force
	yarn eslint .
	yarn knip

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
