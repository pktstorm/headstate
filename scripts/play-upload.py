#!/usr/bin/env python3
"""Upload an Android App Bundle to a Google Play track.

Talks to the Google Play Developer API directly rather than through a
third-party action: the whole exchange is four HTTP calls, and this keeps
the Play service account credential out of any code the project does not
own. The sequence is the one the API requires for any change to a listing
(https://developers.google.com/android-publisher/api-ref/rest/v3/edits):

  1. POST  edits                       -- open an edit (a transaction)
  2. POST  edits/{id}/bundles          -- upload the AAB into it
  3. PUT   edits/{id}/tracks/{track}   -- put that versionCode on a track
  4. POST  edits/{id}:commit           -- make it real

Nothing is visible to anyone until step 4, so a failure part-way leaves
the listing untouched; the abandoned edit expires on its own.

Requires `google-auth` and `requests`; the workflow installs pinned
versions into a throwaway virtualenv. The service account needs the
"Release to testing tracks" permission on the app in the Play Console.
"""

import argparse
import hashlib
import sys

import requests
from google.auth.transport.requests import AuthorizedSession
from google.oauth2 import service_account

SCOPE = "https://www.googleapis.com/auth/androidpublisher"
API = "https://androidpublisher.googleapis.com/androidpublisher/v3/applications"
UPLOAD_API = "https://androidpublisher.googleapis.com/upload/androidpublisher/v3/applications"


def checked(response: requests.Response, what: str) -> dict:
    """Fail loudly with the API's own error body, not a bare status code."""
    if not response.ok:
        print(f"{what} failed: HTTP {response.status_code}", file=sys.stderr)
        print(response.text, file=sys.stderr)
        sys.exit(1)
    return response.json() if response.text else {}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--package", required=True, help="applicationId, e.g. com.example.app")
    parser.add_argument("--aab", required=True, help="path to the signed .aab")
    parser.add_argument("--service-account", required=True, help="path to the service account JSON key")
    parser.add_argument("--track", default="internal", help="Play track (default: internal)")
    parser.add_argument("--release-name", default=None, help="name shown in the Play Console")
    parser.add_argument(
        "--status",
        default="completed",
        choices=["draft", "completed"],
        help="`completed` makes the build available to the track's testers; `draft` only stages it",
    )
    args = parser.parse_args()

    credentials = service_account.Credentials.from_service_account_file(
        args.service_account, scopes=[SCOPE]
    )
    session = AuthorizedSession(credentials)
    base = f"{API}/{args.package}"

    edit_id = checked(session.post(f"{base}/edits", json={}, timeout=60), "create edit")["id"]

    with open(args.aab, "rb") as f:
        aab = f.read()
    local_sha256 = hashlib.sha256(aab).hexdigest()
    bundle = checked(
        session.post(
            f"{UPLOAD_API}/{args.package}/edits/{edit_id}/bundles",
            params={"uploadType": "media"},
            headers={"Content-Type": "application/octet-stream"},
            data=aab,
            timeout=600,
        ),
        "upload bundle",
    )
    version_code = bundle["versionCode"]
    # Play echoes the digest of what it stored; a mismatch means the bytes
    # that reached Google are not the bytes whose checksum the release
    # publishes, which is precisely what the checksums exist to catch.
    remote_sha256 = bundle.get("sha256")
    if remote_sha256 and remote_sha256 != local_sha256:
        print(f"sha256 mismatch: local {local_sha256}, Play {remote_sha256}", file=sys.stderr)
        sys.exit(1)

    release = {"versionCodes": [str(version_code)], "status": args.status}
    if args.release_name:
        release["name"] = args.release_name
    checked(
        session.put(
            f"{base}/edits/{edit_id}/tracks/{args.track}",
            json={"track": args.track, "releases": [release]},
            timeout=60,
        ),
        f"assign to {args.track} track",
    )
    checked(session.post(f"{base}/edits/{edit_id}:commit", timeout=60), "commit edit")

    print(f"uploaded versionCode {version_code} (sha256 {local_sha256}) to the {args.track} track as {args.status}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
