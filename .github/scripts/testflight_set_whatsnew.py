#!/usr/bin/env python3
"""Set TestFlight "What to Test" notes for a freshly uploaded build.

Driven by `.github/workflows/testflight.yml` right after `altool
--upload-app`. The build was just enqueued at App Store Connect and
may still be processing; this script waits for it to materialize in
the API and attaches the release notes.

Env vars (all required unless noted):

    APP_STORE_CONNECT_KEY_ID        e.g. ABCD1234EF
    APP_STORE_CONNECT_ISSUER_ID     UUID
    APP_STORE_CONNECT_KEY_PATH      path to AuthKey_<KID>.p8
    BUNDLE_ID                       e.g. app.outl.mobile-app
    SHORT_VERSION                   e.g. 0.5.3 (CFBundleShortVersionString)
    BUILD_NUMBER                    e.g. 47    (CFBundleVersion)
    NOTES_PATH                      path to release-notes.md
    LOCALE                          optional, defaults to en-US

Flags:

    --dry-run    Skip the POST/PATCH; print the payload and exit 0.

Exit codes:

    0  success, or notes empty (best-effort skip), or build not found
       within the poll window (best-effort skip — altool already
       succeeded, so the build will land in App Store Connect later
       and the CI run shouldn't fail).
    1  unrecoverable error (bad credentials, app not found, 4xx/5xx
       from App Store Connect on the write call).
"""

from __future__ import annotations

import json
import os
import sys
import time
from pathlib import Path
from typing import Any

import jwt  # type: ignore[import-untyped]
import requests

API_BASE = "https://api.appstoreconnect.apple.com"
WHATS_NEW_MAX = 4000  # Apple hard limit on `whatsNew`.
POLL_TIMEOUT_S = 5 * 60
POLL_INTERVAL_S = 15


def env(name: str, *, optional: bool = False, default: str | None = None) -> str:
    value = os.environ.get(name, default)
    if not value and not optional:
        sys.stderr.write(f"::error::missing env var {name}\n")
        sys.exit(1)
    return value or ""


def make_jwt(key_id: str, issuer_id: str, key_path: str) -> str:
    private_key = Path(key_path).read_text()
    now = int(time.time())
    payload = {
        "iss": issuer_id,
        "iat": now,
        "exp": now + 20 * 60,  # 20 min, App Store Connect max
        "aud": "appstoreconnect-v1",
    }
    return jwt.encode(
        payload,
        private_key,
        algorithm="ES256",
        headers={"kid": key_id, "typ": "JWT"},
    )


def api(token: str) -> requests.Session:
    s = requests.Session()
    s.headers.update(
        {
            "Authorization": f"Bearer {token}",
            "Content-Type": "application/json",
        }
    )
    return s


def resolve_app_id(session: requests.Session, bundle_id: str) -> str:
    url = f"{API_BASE}/v1/apps"
    r = session.get(url, params={"filter[bundleId]": bundle_id})
    if r.status_code != 200:
        sys.stderr.write(f"::error::GET /v1/apps -> {r.status_code}: {r.text}\n")
        sys.exit(1)
    data = r.json().get("data", [])
    if not data:
        sys.stderr.write(f"::error::no app for bundleId={bundle_id}\n")
        sys.exit(1)
    app_id = data[0]["id"]
    print(f"::notice::app id {app_id} for bundleId {bundle_id}")
    return app_id


def find_build(
    session: requests.Session, app_id: str, short_version: str, build_number: str
) -> str | None:
    """Poll until the build shows up in App Store Connect (or timeout).

    Filters by build version (CFBundleVersion) and pre-release version
    (CFBundleShortVersionString) to disambiguate when many builds share
    the same short version.
    """
    deadline = time.monotonic() + POLL_TIMEOUT_S
    attempt = 0
    while time.monotonic() < deadline:
        attempt += 1
        r = session.get(
            f"{API_BASE}/v1/builds",
            params={
                "filter[app]": app_id,
                "filter[version]": build_number,
                "filter[preReleaseVersion.version]": short_version,
                "limit": 1,
            },
        )
        if r.status_code == 200:
            builds = r.json().get("data", [])
            if builds:
                build_id = builds[0]["id"]
                print(
                    f"::notice::found build {build_id} "
                    f"({short_version} build {build_number}) "
                    f"after {attempt} poll(s)"
                )
                return build_id
        elif r.status_code >= 500:
            sys.stderr.write(
                f"::warning::transient {r.status_code} from /v1/builds; will retry\n"
            )
        else:
            sys.stderr.write(f"::error::GET /v1/builds -> {r.status_code}: {r.text}\n")
            sys.exit(1)
        time.sleep(POLL_INTERVAL_S)
    sys.stderr.write(
        f"::warning::build {short_version} ({build_number}) did not appear in "
        f"{POLL_TIMEOUT_S}s; skipping whatsNew (best-effort).\n"
    )
    return None


def find_existing_localization(
    session: requests.Session, build_id: str, locale: str
) -> str | None:
    # The nested relationship endpoint
    # `GET /v1/builds/{id}/betaBuildLocalizations` does NOT accept
    # `filter[locale]` (returns 400 PARAMETER_ERROR.ILLEGAL). The
    # top-level `/v1/betaBuildLocalizations` collection does — and
    # supports `filter[build]` to scope to a single build.
    r = session.get(
        f"{API_BASE}/v1/betaBuildLocalizations",
        params={
            "filter[build]": build_id,
            "filter[locale]": locale,
            "limit": 1,
        },
    )
    if r.status_code != 200:
        sys.stderr.write(
            f"::error::GET /v1/betaBuildLocalizations -> {r.status_code}: {r.text}\n"
        )
        sys.exit(1)
    data = r.json().get("data", [])
    return data[0]["id"] if data else None


def upsert_localization(
    session: requests.Session,
    build_id: str,
    locale: str,
    whats_new: str,
    *,
    dry_run: bool,
) -> None:
    existing_id = (
        None if dry_run else find_existing_localization(session, build_id, locale)
    )
    if existing_id:
        url = f"{API_BASE}/v1/betaBuildLocalizations/{existing_id}"
        body: dict[str, Any] = {
            "data": {
                "type": "betaBuildLocalizations",
                "id": existing_id,
                "attributes": {"whatsNew": whats_new},
            }
        }
        method = "PATCH"
    else:
        url = f"{API_BASE}/v1/betaBuildLocalizations"
        body = {
            "data": {
                "type": "betaBuildLocalizations",
                "attributes": {"locale": locale, "whatsNew": whats_new},
                "relationships": {
                    "build": {
                        "data": {"type": "builds", "id": build_id},
                    }
                },
            }
        }
        method = "POST"

    if dry_run:
        print(f"::notice::[dry-run] {method} {url}")
        print(json.dumps(body, indent=2))
        return

    r = session.request(method, url, json=body)
    if r.status_code not in (200, 201):
        sys.stderr.write(f"::error::{method} {url} -> {r.status_code}: {r.text}\n")
        sys.exit(1)
    print(f"::notice::{method} whatsNew ({len(whats_new)} chars) succeeded")


def main() -> int:
    dry_run = "--dry-run" in sys.argv

    key_id = env("APP_STORE_CONNECT_KEY_ID")
    issuer_id = env("APP_STORE_CONNECT_ISSUER_ID")
    key_path = env("APP_STORE_CONNECT_KEY_PATH")
    bundle_id = env("BUNDLE_ID")
    short_version = env("SHORT_VERSION")
    build_number = env("BUILD_NUMBER")
    notes_path = env("NOTES_PATH")
    locale = env("LOCALE", optional=True, default="en-US")

    notes = Path(notes_path).read_text().strip() if Path(notes_path).is_file() else ""
    if not notes:
        print("::notice::release notes empty; skipping whatsNew.")
        return 0
    if len(notes) > WHATS_NEW_MAX:
        print(
            f"::warning::notes ({len(notes)} chars) over Apple's "
            f"{WHATS_NEW_MAX}-char limit; truncating."
        )
        notes = notes[: WHATS_NEW_MAX - 3].rstrip() + "..."

    token = make_jwt(key_id, issuer_id, key_path)
    session = api(token)

    app_id = resolve_app_id(session, bundle_id)
    build_id = find_build(session, app_id, short_version, build_number)
    if not build_id:
        return 0  # best-effort: altool already succeeded.

    upsert_localization(session, build_id, locale, notes, dry_run=dry_run)
    return 0


if __name__ == "__main__":
    sys.exit(main())
