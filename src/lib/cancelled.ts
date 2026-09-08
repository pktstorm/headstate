/// The marker the companion rejects with when the user dismisses the
/// biometric prompt. Must match `CANCELLED` in
/// `src-mobile/src/companion.rs`.
const CANCELLED = "headstate:cancelled";

/// Whether this rejection is the user declining a confirmation prompt,
/// rather than anything going wrong.
///
/// Every destructive command on the phone is gated by Face ID or the
/// Android biometric prompt, and dismissing that sheet is a decision --
/// the action simply does not happen. Reporting it back as a failed
/// action tells someone their own choice was an error.
///
/// The native sides already classify this precisely (Swift maps
/// `LAError.userCancel` and friends; Kotlin maps its own cancel codes),
/// and the classification now survives all the way across: a Tauri
/// command's error crosses the IPC boundary as a string, so the marker
/// is how the variant is carried.
export function isCancelled(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error);
  return message === CANCELLED;
}

/// The message to show for a rejection, or null when it should be
/// silent.
///
/// The one place callers need to think about this: `toast.error(...)`
/// becomes `const m = errorMessage(e); if (m) toast.error(m)`.
export function errorMessage(error: unknown): string | null {
  if (isCancelled(error)) return null;
  return error instanceof Error ? error.message : String(error);
}
