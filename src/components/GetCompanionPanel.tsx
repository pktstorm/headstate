import { QRCodeSVG } from "qrcode.react";
import { useState } from "react";

/// The public TestFlight join link for the companion beta (#736).
///
/// Empty until the external TestFlight group exists and its public link
/// is enabled -- see `docs/mobile-release-process.md`. A placeholder URL
/// would be worse than none: a QR code that resolves to nothing sends
/// someone to a dead page having already installed TestFlight, and they
/// have no way to tell whether they did something wrong.
///
/// Not a secret. A public link is public by design, so it belongs in the
/// repo rather than in a build secret -- and putting it here means the
/// panel's behaviour is testable without one.
///
/// Module-private: the panel is the only thing that needs it, and it
/// reaches it as `joinUrl`'s default. Exporting it would be an unused
/// export, which `knip` fails the build over.
const TESTFLIGHT_JOIN_URL = "";

/// How to get the companion app onto a phone.
///
/// Desktop-only by construction: it is rendered inside the desktop's
/// half of the Phone settings. Someone reading this on their Mac cannot
/// follow the link on the device that needs it, which is the whole
/// problem this panel solves -- hence the QR code as the primary route
/// and the copyable URL as the fallback.
/// `joinUrl` is a prop defaulting to the constant so both states are
/// testable. Without it the not-yet-configured branch would be the only
/// one any test could reach until the beta group exists, which is
/// exactly the branch that stops mattering once it does.
export function GetCompanionPanel({
  joinUrl = TESTFLIGHT_JOIN_URL,
}: {
  joinUrl?: string;
} = {}) {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);
  const configured = joinUrl !== "";

  return (
    <div className="mt-5 flex flex-col gap-2">
      <span className="text-sm font-medium">Get the mobile companion</span>
      <p className="text-xs text-[#8b949e]">
        The companion app pairs with this desktop to show your pull requests on
        your phone. It is in beta and distributed through TestFlight.
      </p>
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="self-start rounded border border-[#30363d] px-3 py-1 text-xs text-[#e6edf3] hover:bg-[#21262d]"
      >
        {open ? "Hide instructions" : "Get the mobile companion app"}
      </button>

      {open ? (
        <div className="mt-2 flex flex-col gap-3">
          <span className="text-xs font-medium text-[#8b949e]">Beta version</span>

          {/* Numbered, because the order matters: TestFlight must exist
              on the phone before the invitation can do anything with
              it. Someone who opens the link first gets an App Store
              page for TestFlight and loses the invitation. */}
          <ol className="flex list-decimal flex-col gap-2 pl-5 text-xs text-[#8b949e]">
            <li>
              On your iPhone, install Apple&rsquo;s TestFlight app from the App
              Store, if you don&rsquo;t already have it.
            </li>
            <li>
              Open the Headstate companion beta invitation on your iPhone
              {configured ? (
                <>
                  {" "}
                  by scanning the code below. (It has to be opened on your
                  iPhone, not on this Mac.)
                </>
              ) : (
                <> — see below.</>
              )}
            </li>
            <li>
              In TestFlight, tap <span className="text-[#e6edf3]">Accept</span>,
              then <span className="text-[#e6edf3]">Install</span>, to put the
              Headstate companion on your phone.
            </li>
          </ol>

          {configured ? (
            <div className="flex flex-wrap items-start gap-4">
              {/* The point of the panel. The invitation has to open on
                  the phone while the user is reading this on a Mac, and
                  typing a TestFlight URL by hand is exactly the friction
                  worth removing. White ground and a quiet zone: a QR
                  drawn straight onto the dark panel scans unreliably. */}
              <div
                role="img"
                aria-label="QR code linking to the TestFlight beta invitation"
                className="shrink-0 rounded bg-white p-2"
              >
                <QRCodeSVG value={joinUrl} size={176} level="M" />
              </div>
              <div className="flex min-w-0 flex-col gap-2">
                <p className="text-xs text-[#8b949e]">
                  Point your iPhone camera at this code, then tap the
                  notification that appears.
                </p>
                {/* Kept visible and copyable. Not everyone can scan --
                    a camera may be unavailable, and some people would
                    rather send the link to themselves. */}
                <span className="text-xs font-medium text-[#8b949e]">
                  Or open this link on your iPhone
                </span>
                <code className="break-all font-mono text-xs text-[#e6edf3]">
                  {joinUrl}
                </code>
                <button
                  type="button"
                  onClick={() => {
                    void navigator.clipboard
                      .writeText(joinUrl)
                      .then(() => setCopied(true))
                      .catch(() => setCopied(false));
                  }}
                  className="self-start rounded border border-[#30363d] px-2 py-1 text-xs text-[#e6edf3] hover:bg-[#21262d]"
                >
                  {copied ? "Copied" : "Copy link"}
                </button>
              </div>
            </div>
          ) : (
            /* Says plainly that there is nothing to scan yet, rather
               than rendering a code that goes nowhere. The beta group
               is a console task on Apple's side (#736), so this state
               is real and will be seen. */
            <p className="text-xs text-[#d29922]">
              The public beta invitation is not available yet. Once the
              TestFlight beta group is open, a link and a QR code will appear
              here.
            </p>
          )}
        </div>
      ) : null}
    </div>
  );
}
