# Completed pairing walkthrough runs

One file per run, named `YYYY-MM-DD-<platform>.md`, where `<platform>`
is `ios` or `android`. Each is a filled-in copy of
[`../mobile-pairing-walkthrough.md`](../mobile-pairing-walkthrough.md):
the header completed, the boxes ticked, and the findings section either
empty or holding what went wrong.

Runs are evidence, not paperwork. A run with a finding is more useful
than one without, and a run that stopped halfway is still worth
committing — say where it stopped and why.

## What a run is for

Store builds stay manual until this walkthrough has passed **twice with
no findings**, once on a real iPhone and once on a real Android device.
Both platforms, both clean; an iOS-only pass does not open the gate. The
rule and its reasoning are in the walkthrough itself.

## Getting a build onto a device

Simulators and emulators do not count, and the reason is not
pedantry: they have no Secure Enclave and no Keystore-backed biometric
gate, so the step-up prompt cannot be exercised, and iOS does not show
the local-network permission prompt in the simulator. Three of the
things a run exists to check are unavailable there.

**iPhone.** `make ios-device` opens the Xcode project with the dev
server bound to this machine's network address. Xcode is where signing
happens: open Signing & Capabilities, pick your team, and run to the
attached device. The committed project deliberately carries no
`DEVELOPMENT_TEAM` — it is personal to whoever builds, and the release
workflow injects its own — so this is a one-time setting in your own
checkout.

**Android.** `make android-device` installs over adb. A device with USB
debugging enabled and showing up in `adb devices` is all that is needed:
no signing team, no Play Console.

Either way the phone and the desktop must be on the same network, or on
the same overlay. The walkthrough's discovery steps depend on it.

## What to record beyond the checklist

The walkthrough's boxes are the minimum. Two other things are worth
capturing while a device is in hand, because they cannot be answered
from a development machine:

- **Anything #512 asks about the Secure Enclave** — whether
  `SecureEnclave.MLDSA65.PrivateKey` accepts access control, which
  enclave generations support ML-DSA on the OS you are running, and
  whether `setUserAuthenticationRequired` behaves for ML-DSA keys as it
  does for EC keys. Those are open questions with no answer available
  off-device.
- **How the biometric prompt fails.** Cancel it, fail it enough times to
  trigger lockout, and — if you are willing to re-pair afterwards —
  change the device's enrolled biometric to invalidate the key. Each
  should produce a different, actionable message rather than one opaque
  error.
