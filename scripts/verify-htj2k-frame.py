#!/usr/bin/env python3
"""Prove that /frames returns the stored HTJ2K fragment bytes unchanged.

This script fetches:
  1. `/wado/.../frames/<n>` with `Accept: multipart/related; type="image/jphc"`
  2. `/wado/.../bulkdata/7FE00010`

It then compares the returned frame payload against the matching encapsulated
Pixel Data fragment from the bulk-data response.

If the bytes match exactly and the viewer renders the image, pacsnode is
serving stored HTJ2K and OHIF/Cornerstone is decoding it client-side.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from dataclasses import dataclass
from email import policy
from email.parser import BytesParser
from typing import Iterable
from urllib.error import HTTPError, URLError
from urllib.parse import quote
from urllib.request import Request, urlopen

HTJ2K_TRANSFER_SYNTAXES = {
    "3.2.840.10008.1.2.4.96",
    "1.2.840.10008.1.2.4.201",
    "1.2.840.10008.1.2.4.202",
    "1.2.840.10008.1.2.4.203",
}


class VerificationError(RuntimeError):
    """Raised when runtime verification fails."""


@dataclass(frozen=True)
class HttpResponse:
    url: str
    status: int
    headers: dict[str, str]
    body: bytes


@dataclass(frozen=True)
class MultipartPart:
    content_type: str
    content_location: str | None
    payload: bytes


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description=(
            "Verify that pacsnode returns stored HTJ2K codestream bytes on "
            "/wado/.../frames by comparing them to the stored encapsulated "
            "Pixel Data fragments."
        )
    )
    parser.add_argument(
        "--base-url",
        default="http://localhost:8042",
        help="Base pacsnode URL (default: %(default)s)",
    )
    parser.add_argument("--study", help="StudyInstanceUID to verify")
    parser.add_argument("--series", help="SeriesInstanceUID to verify")
    parser.add_argument("--instance", help="SOPInstanceUID to verify")
    parser.add_argument(
        "--frame",
        type=int,
        default=1,
        help="1-based frame number to compare (default: %(default)s)",
    )
    return parser.parse_args()


def fetch(url: str, headers: dict[str, str] | None = None) -> HttpResponse:
    request = Request(url, headers=headers or {})
    try:
        with urlopen(request) as response:
            return HttpResponse(
                url=response.geturl(),
                status=response.status,
                headers={key.lower(): value for key, value in response.headers.items()},
                body=response.read(),
            )
    except HTTPError as exc:
        body = exc.read().decode("utf-8", errors="replace")
        raise VerificationError(f"{url} returned HTTP {exc.code}: {body[:300]}") from exc
    except URLError as exc:
        raise VerificationError(f"failed to reach {url}: {exc.reason}") from exc


def fetch_json(url: str) -> object:
    response = fetch(url, headers={"Accept": "application/json"})
    try:
        return json.loads(response.body)
    except json.JSONDecodeError as exc:
        raise VerificationError(f"{url} did not return valid JSON") from exc


def quote_uid(value: str) -> str:
    return quote(value, safe="")


def discover_first_htj2k_instance(base_url: str) -> tuple[str, str, str, str | None]:
    studies = fetch_json(f"{base_url}/api/studies")
    if not isinstance(studies, list):
        raise VerificationError("/api/studies did not return a JSON array")

    for study in studies:
        if not isinstance(study, dict):
            continue
        study_uid = study.get("study_uid")
        if not isinstance(study_uid, str) or not study_uid:
            continue

        series_items = fetch_json(f"{base_url}/api/studies/{quote_uid(study_uid)}/series")
        if not isinstance(series_items, list):
            continue

        for series in series_items:
            if not isinstance(series, dict):
                continue
            series_uid = series.get("series_uid")
            if not isinstance(series_uid, str) or not series_uid:
                continue

            instances = fetch_json(f"{base_url}/api/series/{quote_uid(series_uid)}/instances")
            if not isinstance(instances, list):
                continue

            for instance in instances:
                if not isinstance(instance, dict):
                    continue
                instance_uid = instance.get("instance_uid")
                transfer_syntax = instance.get("transfer_syntax")
                if (
                    isinstance(instance_uid, str)
                    and instance_uid
                    and isinstance(transfer_syntax, str)
                    and transfer_syntax in HTJ2K_TRANSFER_SYNTAXES
                ):
                    return study_uid, series_uid, instance_uid, transfer_syntax

    raise VerificationError(
        "no HTJ2K instance found via /api; pass --study/--series/--instance explicitly"
    )


def lookup_instance_transfer_syntax(
    base_url: str,
    series_uid: str,
    instance_uid: str,
) -> str | None:
    instances = fetch_json(f"{base_url}/api/series/{quote_uid(series_uid)}/instances")
    if not isinstance(instances, list):
        return None

    for instance in instances:
        if not isinstance(instance, dict):
            continue
        if instance.get("instance_uid") == instance_uid:
            transfer_syntax = instance.get("transfer_syntax")
            return transfer_syntax if isinstance(transfer_syntax, str) else None

    return None


def parse_content_type(value: str) -> tuple[str, dict[str, str]]:
    parts = [part.strip() for part in value.split(";")]
    media_type = parts[0].lower()
    params: dict[str, str] = {}
    for part in parts[1:]:
        if "=" not in part:
            continue
        key, raw_value = part.split("=", 1)
        params[key.strip().lower()] = raw_value.strip().strip('"')
    return media_type, params


def parse_multipart(response: HttpResponse) -> list[MultipartPart]:
    content_type = response.headers.get("content-type")
    if not content_type:
        raise VerificationError(f"{response.url} did not return a Content-Type header")

    parser_input = (
        f"Content-Type: {content_type}\r\nMIME-Version: 1.0\r\n\r\n".encode("ascii")
        + response.body
    )
    message = BytesParser(policy=policy.default).parsebytes(parser_input)
    if not message.is_multipart():
        payload = message.get_payload(decode=True)
        return [
            MultipartPart(
                content_type=message.get_content_type(),
                content_location=message.get("Content-Location"),
                payload=payload if payload is not None else response.body,
            )
        ]

    parts: list[MultipartPart] = []
    for part in message.iter_parts():
        payload = part.get_payload(decode=True)
        parts.append(
            MultipartPart(
                content_type=part.get_content_type(),
                content_location=part.get("Content-Location"),
                payload=payload if payload is not None else b"",
            )
        )
    return parts


def require_no_content_encoding(response: HttpResponse) -> None:
    content_encoding = response.headers.get("content-encoding")
    if content_encoding and content_encoding.lower() != "identity":
        raise VerificationError(
            f"{response.url} returned Content-Encoding={content_encoding!r}; "
            "byte-for-byte verification requires an uncompressed HTTP response"
        )


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def hex_prefix(data: bytes, length: int = 16) -> str:
    return data[:length].hex(" ")


def ensure_all_or_none(values: Iterable[str | None]) -> bool:
    present = [value is not None for value in values]
    return all(present) or not any(present)


def main() -> int:
    args = parse_args()

    if args.frame < 1:
        raise VerificationError("--frame must be >= 1")

    if not ensure_all_or_none((args.study, args.series, args.instance)):
        raise VerificationError("pass either all of --study/--series/--instance or none of them")

    if args.study and args.series and args.instance:
        study_uid = args.study
        series_uid = args.series
        instance_uid = args.instance
        instance_transfer_syntax = lookup_instance_transfer_syntax(
            args.base_url, series_uid, instance_uid
        )
    else:
        study_uid, series_uid, instance_uid, instance_transfer_syntax = discover_first_htj2k_instance(
            args.base_url
        )

    frame_url = (
        f"{args.base_url}/wado/studies/{quote_uid(study_uid)}"
        f"/series/{quote_uid(series_uid)}"
        f"/instances/{quote_uid(instance_uid)}"
        f"/frames/{args.frame}"
    )
    bulk_url = (
        f"{args.base_url}/wado/studies/{quote_uid(study_uid)}"
        f"/series/{quote_uid(series_uid)}"
        f"/instances/{quote_uid(instance_uid)}"
        f"/bulkdata/7FE00010"
    )

    frame_response = fetch(
        frame_url,
        headers={
            "Accept": 'multipart/related; type="image/jphc"; transfer-syntax="*"',
            "Accept-Encoding": "identity",
        },
    )
    bulk_response = fetch(
        bulk_url,
        headers={"Accept-Encoding": "identity"},
    )

    require_no_content_encoding(frame_response)
    require_no_content_encoding(bulk_response)

    frame_content_type = frame_response.headers.get("content-type", "")
    frame_media_type, frame_params = parse_content_type(frame_content_type)
    if frame_media_type != "multipart/related":
        raise VerificationError(
            f"/frames did not return multipart/related (got {frame_content_type!r})"
        )
    if frame_params.get("type", "").lower() != "image/jphc":
        raise VerificationError(
            f"/frames did not return image/jphc parts (got {frame_content_type!r})"
        )

    frame_transfer_syntax = frame_params.get("transfer-syntax")
    if frame_transfer_syntax and frame_transfer_syntax not in HTJ2K_TRANSFER_SYNTAXES:
        raise VerificationError(
            f"/frames returned non-HTJ2K transfer syntax {frame_transfer_syntax!r}"
        )

    if (
        instance_transfer_syntax
        and frame_transfer_syntax
        and frame_transfer_syntax != instance_transfer_syntax
    ):
        raise VerificationError(
            "frame response transfer-syntax does not match stored instance transfer syntax "
            f"({frame_transfer_syntax} != {instance_transfer_syntax})"
        )

    frame_parts = parse_multipart(frame_response)
    if len(frame_parts) != 1:
        raise VerificationError(
            f"/frames/{args.frame} returned {len(frame_parts)} parts; expected exactly 1"
        )
    frame_payload = frame_parts[0].payload

    bulk_parts = parse_multipart(bulk_response)
    if len(bulk_parts) < args.frame:
        raise VerificationError(
            f"/bulkdata/7FE00010 returned {len(bulk_parts)} parts; frame {args.frame} is unavailable"
        )
    stored_payload = bulk_parts[args.frame - 1].payload

    if frame_payload != stored_payload:
        raise VerificationError(
            "frame payload does not match the stored encapsulated Pixel Data fragment:\n"
            f"  /frames sha256 : {sha256(frame_payload)}\n"
            f"  /bulkdata sha256: {sha256(stored_payload)}"
        )

    print(f"Study UID            : {study_uid}")
    print(f"Series UID           : {series_uid}")
    print(f"Instance UID         : {instance_uid}")
    print(f"Stored transfer syntax: {instance_transfer_syntax or frame_transfer_syntax or 'unknown'}")
    print(f"/frames Content-Type : {frame_content_type}")
    print(f"/bulkdata parts      : {len(bulk_parts)}")
    print(f"Frame bytes          : {len(frame_payload)}")
    print(f"SHA-256              : {sha256(frame_payload)}")
    print(f"Leading bytes        : {hex_prefix(frame_payload)}")
    print()
    print("OK: /frames returned the exact stored encapsulated HTJ2K fragment bytes.")
    print(
        "If OHIF renders the image at the same time, decoding is happening client-side rather than in pacsnode."
    )
    print(
        "Note: browser Network size can still differ because multipart framing and HTTP content-encoding affect the displayed transfer size."
    )
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except VerificationError as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)
