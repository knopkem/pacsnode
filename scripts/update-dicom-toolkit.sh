#!/usr/bin/env sh

set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
LOCKFILE="${ROOT_DIR}/Cargo.lock"

usage() {
    cat <<'EOF'
Usage: sh scripts/update-dicom-toolkit.sh [option]

Refresh the dicom-toolkit-rs git dependencies pinned in Cargo.lock.

Options:
  --check       Print the currently locked dicom-toolkit-rs revision and exit.
  -h, --help    Show this help text.

Examples:
  sh scripts/update-dicom-toolkit.sh
  sh scripts/update-dicom-toolkit.sh --check
EOF
}

require_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        printf 'error: required command not found: %s\n' "$1" >&2
        exit 1
    fi
}

current_revision() {
    if [ ! -f "$LOCKFILE" ]; then
        return 0
    fi

    sed -n 's/.*git+https:\/\/github.com\/knopkem\/dicom-toolkit-rs?branch=main#\([0-9a-f][0-9a-f]*\)".*/\1/p' "$LOCKFILE" | head -n 1
}

print_current_revision() {
    revision=$(current_revision)
    if [ -n "$revision" ]; then
        printf 'Current dicom-toolkit-rs revision: %s\n' "$revision"
    else
        printf 'Current dicom-toolkit-rs revision: not locked yet\n'
    fi
}

case "${1:-}" in
    --check)
        print_current_revision
        exit 0
        ;;
    -h|--help)
        usage
        exit 0
        ;;
    "")
        ;;
    *)
        printf 'error: unknown option: %s\n\n' "$1" >&2
        usage >&2
        exit 1
        ;;
esac

require_cmd cargo

printf 'Updating dicom-toolkit-rs dependencies in %s\n' "$ROOT_DIR"
before_revision=$(current_revision)

cd "$ROOT_DIR"
cargo update \
    -p dicom-toolkit-core \
    -p dicom-toolkit-dict \
    -p dicom-toolkit-data \
    -p dicom-toolkit-net \
    -p dicom-toolkit-image \
    -p dicom-toolkit-codec

after_revision=$(current_revision)

if [ -n "$before_revision" ] && [ "$before_revision" != "$after_revision" ]; then
    printf 'Updated dicom-toolkit-rs revision: %s -> %s\n' "$before_revision" "$after_revision"
elif [ -n "$after_revision" ]; then
    printf 'dicom-toolkit-rs remains at revision: %s\n' "$after_revision"
else
    printf 'dicom-toolkit-rs dependencies updated.\n'
fi
