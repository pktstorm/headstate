/// A certificate fingerprint as the pairing screens show it: lowercase
/// hex in groups of four, the way the phone shows it, so the two can be
/// compared block by block.
///
/// Accepts either form the Rust side produces -- the QR payload's `fp`
/// carries a `sha256:` prefix, the `pairing-request` event and the
/// paired-devices rows do not -- and renders both the same, because
/// a differently grouped string is exactly what a user comparing two
/// screens must never see.
export function formatFingerprint(fp: string): string {
  const hex = fp.trim().toLowerCase().replace(/^sha256:/, "");
  return hex.match(/.{1,4}/g)?.join(" ") ?? "";
}
