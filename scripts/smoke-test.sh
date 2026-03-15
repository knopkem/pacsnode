#!/usr/bin/env sh
# smoke-test.sh — end-to-end smoke test for pacsnode
#
# Tests the full pipeline:
#   1. Health check (HTTP)
#   2. Register a test DICOM node (REST API)
#   3. C-ECHO  via DIMSE   (echoscu)
#   4. C-STORE via DIMSE   (storescu  — uploads testfiles/*.dcm)
#   5. Statistics check    (REST API  — confirms files were stored)
#   6. QIDO-RS query       (DICOMweb  — lists uploaded studies)
#   7. C-FIND  via DIMSE   (findscu   — queries by Study Root)
#   8. WADO-RS retrieve    (DICOMweb  — retrieves a single instance;
#                           equivalent to C-GET without a dedicated getscu
#                           binary, which dicom-toolkit-rs does not provide)
#   9. Cleanup             (removes the test node registration)
#
# Prerequisites:
#   - pacsnode running (docker compose up -d, or cargo run)
#   - cargo in PATH (used to install DICOM CLI tools if not already present)
#
# Usage:
#   sh scripts/smoke-test.sh
#   PACS_HOST=10.0.0.5 DICOM_PORT=4242 sh scripts/smoke-test.sh

set -eu

# ── Configuration ─────────────────────────────────────────────────────────────

PACS_HOST="${PACS_HOST:-localhost}"
HTTP_PORT="${HTTP_PORT:-8042}"
DICOM_PORT="${DICOM_PORT:-4242}"
PACS_AE="${PACS_AE:-PACSNODE}"
CLIENT_AE="${CLIENT_AE:-SMOKETEST}"

HTTP_BASE="http://${PACS_HOST}:${HTTP_PORT}"
TESTFILES_DIR="$(cd "$(dirname "$0")/.." && pwd)/testfiles"

# ── Helpers ───────────────────────────────────────────────────────────────────

PASS=0
FAIL=0
STUDY_UID=""
SERIES_UID=""
INSTANCE_UID=""

ok() {
    printf "  \033[32m✓\033[0m %s\n" "$1"
    PASS=$((PASS + 1))
}

fail() {
    printf "  \033[31m✗\033[0m %s\n" "$1"
    FAIL=$((FAIL + 1))
}

step() {
    printf "\n\033[1;34m── Step %s: %s\033[0m\n" "$1" "$2"
}

require_cmd() {
    command -v "$1" >/dev/null 2>&1
}

# Extract first StudyInstanceUID from the REST /api/studies response.
extract_study_uid_rest() {
    curl -sf "${HTTP_BASE}/api/studies" 2>/dev/null | python3 -c \
        "import sys,json; d=json.load(sys.stdin); print(d[0]['study_uid']) if d else print('')" 2>/dev/null
}

# Resolve series_uid and instance_uid for a study via REST API.
# Prints "series_uid instance_uid" on one line.
extract_instance_uids() {
    # $1 = study_uid
    series_uid=$(curl -sf "${HTTP_BASE}/api/studies/$1/series" 2>/dev/null | python3 -c \
        "import sys,json; d=json.load(sys.stdin); print(d[0]['series_uid']) if d else print('')" 2>/dev/null) || return 1

    instance_uid=$(curl -sf "${HTTP_BASE}/api/series/${series_uid}/instances" 2>/dev/null | python3 -c \
        "import sys,json; d=json.load(sys.stdin); print(d[0]['instance_uid']) if d else print('')" 2>/dev/null) || return 1

    printf '%s %s' "$series_uid" "$instance_uid"
}

# Returns success if a QIDO-RS JSON array contains the expected UID in the given tag.
qido_contains_uid() {
    # $1 = JSON file path, $2 = DICOM tag, $3 = expected UID
    python3 - "$1" "$2" "$3" <<'PY'
import json
import sys

path, tag, expected = sys.argv[1:4]
with open(path, encoding="utf-8") as f:
    payload = json.load(f)

if not isinstance(payload, list):
    raise SystemExit(1)

for item in payload:
    if not isinstance(item, dict) or not item:
        continue
    tag_value = item.get(tag)
    if not isinstance(tag_value, dict):
        continue
    values = tag_value.get("Value")
    if isinstance(values, list) and expected in [str(v) for v in values]:
        raise SystemExit(0)

raise SystemExit(1)
PY
}

# ── Tool installation ──────────────────────────────────────────────────────────

install_dicom_tools() {
    printf "\n\033[1;33mDICOM CLI tools not found in PATH — installing via cargo...\033[0m\n"
    printf "(This may take a few minutes on the first run.)\n\n"
    cargo install \
        --git https://github.com/knopkem/dicom-toolkit-rs \
        --branch main \
        dicom-toolkit-tools \
        --quiet 2>&1 || {
        printf "\033[31mFailed to install dicom-toolkit-tools. Ensure cargo is in PATH.\033[0m\n"
        exit 1
    }
    printf "\033[32mInstalled: echoscu, storescu, findscu\033[0m\n"
}

printf "\n\033[1;37m╔══════════════════════════════════════════╗\033[0m\n"
printf "\033[1;37m║       pacsnode smoke test                ║\033[0m\n"
printf "\033[1;37m╚══════════════════════════════════════════╝\033[0m\n"
printf "  PACS:   %s  (AE: %s)\n" "$HTTP_BASE" "$PACS_AE"
printf "  DIMSE:  %s:%s\n" "$PACS_HOST" "$DICOM_PORT"
printf "  Client: %s\n" "$CLIENT_AE"
printf "  Files:  %s\n" "$TESTFILES_DIR"

# ── Check prerequisites ───────────────────────────────────────────────────────

step 0 "Prerequisites"

if ! require_cmd curl; then
    fail "curl not found — please install curl"
    exit 1
fi
ok "curl found"

if ! require_cmd python3; then
    fail "python3 not found — needed to parse JSON responses"
    exit 1
fi
ok "python3 found"

if ! require_cmd echoscu || ! require_cmd storescu || ! require_cmd findscu; then
    install_dicom_tools
fi
ok "echoscu / storescu / findscu available"

DCM_FILES=$(find "$TESTFILES_DIR" -maxdepth 1 -name "*.dcm" 2>/dev/null | sort)
if [ -z "$DCM_FILES" ]; then
    fail "No .dcm files found in $TESTFILES_DIR"
    exit 1
fi
ok "$(printf '%s' "$DCM_FILES" | wc -l | tr -d ' ') DICOM test file(s) found"

# ── Step 1: Health check ──────────────────────────────────────────────────────

step 1 "Health check  GET /health"

STATUS=$(curl -sf "${HTTP_BASE}/health" 2>/dev/null | python3 -c \
    "import sys,json; print(json.load(sys.stdin).get('status','?'))" 2>/dev/null) || STATUS=""

if [ "$STATUS" = "ok" ]; then
    ok "Server is healthy"
else
    fail "Health check failed (status='$STATUS') — is pacsnode running on ${HTTP_BASE}?"
    exit 1
fi

# ── Step 2: Register test node ────────────────────────────────────────────────

step 2 "Register test node  POST /api/nodes"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -X POST "${HTTP_BASE}/api/nodes" \
    -H "Content-Type: application/json" \
    -d "{\"ae_title\":\"${CLIENT_AE}\",\"host\":\"${PACS_HOST}\",\"port\":${DICOM_PORT},\"description\":\"Smoke test node\",\"tls_enabled\":false}")

if [ "$HTTP_CODE" = "201" ]; then
    ok "Node '${CLIENT_AE}' registered (201 Created)"
else
    fail "Node registration returned HTTP ${HTTP_CODE}"
fi

# Confirm via GET
NODES=$(curl -sf "${HTTP_BASE}/api/nodes" 2>/dev/null | python3 -c \
    "import sys,json; nodes=json.load(sys.stdin); aes=[n['ae_title'] for n in nodes]; print(' '.join(aes))" 2>/dev/null) || NODES=""

if printf '%s' "$NODES" | grep -q "$CLIENT_AE"; then
    ok "Node '${CLIENT_AE}' confirmed in GET /api/nodes"
else
    fail "Node '${CLIENT_AE}' not visible in GET /api/nodes (got: ${NODES})"
fi

# ── Step 3: C-ECHO ────────────────────────────────────────────────────────────

step 3 "C-ECHO  (DIMSE)"

if echoscu "$PACS_HOST" "$DICOM_PORT" \
    --aetitle "$CLIENT_AE" --called-ae "$PACS_AE" \
    --verbose 2>&1 | grep -qi "success\|verified\|C-ECHO"; then
    ok "C-ECHO succeeded"
elif echoscu "$PACS_HOST" "$DICOM_PORT" \
    --aetitle "$CLIENT_AE" --called-ae "$PACS_AE" >/dev/null 2>&1; then
    ok "C-ECHO succeeded (exit 0)"
else
    fail "C-ECHO failed — check DIMSE port ${DICOM_PORT} and AE title '${PACS_AE}'"
fi

# ── Step 4: C-STORE (upload test files) ───────────────────────────────────────

step 4 "C-STORE  — uploading $(printf '%s' "$DCM_FILES" | wc -l | tr -d ' ') file(s)"

# Pass all .dcm files as arguments (xargs handles spaces in paths)
# shellcheck disable=SC2086
if printf '%s\n' $DCM_FILES | xargs storescu "$PACS_HOST" "$DICOM_PORT" \
    --aetitle "$CLIENT_AE" --called-ae "$PACS_AE" \
    --verbose 2>&1 | grep -qi "store\|sent\|success"; then
    ok "C-STORE completed (verbose output confirmed)"
elif printf '%s\n' $DCM_FILES | xargs storescu "$PACS_HOST" "$DICOM_PORT" \
    --aetitle "$CLIENT_AE" --called-ae "$PACS_AE" >/dev/null 2>&1; then
    ok "C-STORE completed (exit 0)"
else
    fail "C-STORE failed — check DIMSE port and server logs"
fi

# ── Step 5: Statistics ────────────────────────────────────────────────────────

step 5 "Statistics check  GET /statistics"

STATS=$(curl -sf "${HTTP_BASE}/statistics" 2>/dev/null)
STUDIES=$(printf '%s' "$STATS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('studies',0))" 2>/dev/null) || STUDIES=0
INSTANCES=$(printf '%s' "$STATS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('instances',0))" 2>/dev/null) || INSTANCES=0

if [ "${STUDIES:-0}" -gt 0 ]; then
    ok "Database has ${STUDIES} study/studies, ${INSTANCES} instance(s)"
else
    fail "No studies found after upload (studies=${STUDIES})"
fi

# ── Step 6: QIDO-RS query ─────────────────────────────────────────────────────

step 6 "QIDO-RS  GET /wado/studies"

STUDY_UID=$(extract_study_uid_rest)
if [ -n "$STUDY_UID" ]; then
    ok "StudyInstanceUID resolved via REST: ${STUDY_UID}"
else
    fail "Could not resolve a study UID — was C-STORE successful?"
    STUDY_UID=""
fi

QIDO_STUDIES_BODY=$(mktemp)
QIDO_CODE=$(curl -s -o "$QIDO_STUDIES_BODY" -w "%{http_code}" "${HTTP_BASE}/wado/studies" 2>/dev/null) || QIDO_CODE=0
if [ "$QIDO_CODE" = "200" ]; then
    if [ -n "$STUDY_UID" ] && qido_contains_uid "$QIDO_STUDIES_BODY" "0020000D" "$STUDY_UID"; then
        ok "QIDO-RS studies response contains StudyInstanceUID ${STUDY_UID}"
    else
        fail "QIDO-RS studies response missing StudyInstanceUID ${STUDY_UID:-<unknown>}"
    fi
else
    fail "QIDO-RS studies returned HTTP ${QIDO_CODE}"
fi
rm -f "$QIDO_STUDIES_BODY"

if [ -n "$STUDY_UID" ]; then
    UIDS=$(extract_instance_uids "$STUDY_UID" 2>/dev/null) || UIDS=""
    SERIES_UID=$(printf '%s' "$UIDS" | cut -d' ' -f1)
    INSTANCE_UID=$(printf '%s' "$UIDS" | cut -d' ' -f2)

    if [ -n "$SERIES_UID" ]; then
        QIDO_SERIES_BODY=$(mktemp)
        QIDO_SERIES_CODE=$(curl -s -o "$QIDO_SERIES_BODY" -w "%{http_code}" \
            "${HTTP_BASE}/wado/studies/${STUDY_UID}/series" 2>/dev/null) || QIDO_SERIES_CODE=0
        if [ "$QIDO_SERIES_CODE" = "200" ] && qido_contains_uid "$QIDO_SERIES_BODY" "0020000E" "$SERIES_UID"; then
            ok "QIDO-RS series response contains SeriesInstanceUID ${SERIES_UID}"
        else
            fail "QIDO-RS series response missing SeriesInstanceUID ${SERIES_UID}"
        fi
        rm -f "$QIDO_SERIES_BODY"
    else
        fail "Could not resolve a series UID for QIDO-RS series validation"
    fi

    if [ -n "$SERIES_UID" ] && [ -n "$INSTANCE_UID" ]; then
        QIDO_INSTANCES_BODY=$(mktemp)
        QIDO_INSTANCES_CODE=$(curl -s -o "$QIDO_INSTANCES_BODY" -w "%{http_code}" \
            "${HTTP_BASE}/wado/studies/${STUDY_UID}/series/${SERIES_UID}/instances" 2>/dev/null) || QIDO_INSTANCES_CODE=0
        if [ "$QIDO_INSTANCES_CODE" = "200" ] && qido_contains_uid "$QIDO_INSTANCES_BODY" "00080018" "$INSTANCE_UID"; then
            ok "QIDO-RS instances response contains SOPInstanceUID ${INSTANCE_UID}"
        else
            fail "QIDO-RS instances response missing SOPInstanceUID ${INSTANCE_UID}"
        fi
        rm -f "$QIDO_INSTANCES_BODY"
    else
        fail "Could not resolve an instance UID for QIDO-RS instance validation"
    fi
fi

# ── Step 7: C-FIND (DIMSE) ────────────────────────────────────────────────────

step 7 "C-FIND  (DIMSE Study Root)"

CFIND_OUT=$(findscu "$PACS_HOST" "$DICOM_PORT" \
    --aetitle "$CLIENT_AE" --called-ae "$PACS_AE" \
    --level STUDY \
    --key "0008,0052=STUDY" \
    --verbose 2>&1) || CFIND_OUT=""

# findscu exits 0 on success; result count may be in output
if printf '%s' "$CFIND_OUT" | grep -qiE "response|result|match|study|0020,000d"; then
    ok "C-FIND returned responses"
elif echo "$?" | grep -q "^0$"; then
    ok "C-FIND completed (exit 0)"
else
    fail "C-FIND failed or returned no results"
fi

# ── Step 8: WADO-RS retrieve (equivalent to C-GET) ───────────────────────────

step 8 "WADO-RS retrieve  (DICOMweb C-GET equivalent)"
printf "  \033[2m(dicom-toolkit-rs has no getscu binary; WADO-RS is the standard\033[0m\n"
printf "  \033[2m DICOMweb equivalent for instance retrieval)\033[0m\n"

if [ -n "$STUDY_UID" ]; then
    # Resolve a series/instance UID within the study if step 6 did not already.
    if [ -z "$SERIES_UID" ] || [ -z "$INSTANCE_UID" ]; then
        UIDS=$(extract_instance_uids "$STUDY_UID" 2>/dev/null) || UIDS=""
        SERIES_UID=$(printf '%s' "$UIDS" | cut -d' ' -f1)
        INSTANCE_UID=$(printf '%s' "$UIDS" | cut -d' ' -f2)
    fi

    if [ -n "$SERIES_UID" ] && [ -n "$INSTANCE_UID" ]; then
        HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
            -H "Accept: multipart/related; type=\"application/dicom\"" \
            "${HTTP_BASE}/wado/studies/${STUDY_UID}/series/${SERIES_UID}/instances/${INSTANCE_UID}")

        if [ "$HTTP_CODE" = "200" ]; then
            ok "WADO-RS retrieve returned HTTP 200"
            ok "SeriesInstanceUID:  ${SERIES_UID}"
            ok "SOPInstanceUID:     ${INSTANCE_UID}"
        else
            fail "WADO-RS retrieve returned HTTP ${HTTP_CODE}"
        fi
    else
        fail "Could not resolve series/instance UIDs for WADO-RS retrieve"
    fi
else
    fail "Skipping WADO-RS retrieve — no study UID available"
fi

# ── Step 9: System info ───────────────────────────────────────────────────────

step 9 "System info  GET /system"

SYS=$(curl -sf "${HTTP_BASE}/system" 2>/dev/null)
SYS_AE=$(printf '%s' "$SYS" | python3 -c "import sys,json; print(json.load(sys.stdin).get('ae_title','?'))" 2>/dev/null) || SYS_AE="?"
NODE_COUNT=$(printf '%s' "$SYS" | python3 -c "import sys,json; print(len(json.load(sys.stdin).get('nodes',[])))" 2>/dev/null) || NODE_COUNT=0

ok "AE title: ${SYS_AE}, registered nodes: ${NODE_COUNT}"

# ── Step 10: Cleanup ─────────────────────────────────────────────────────────

step 10 "Cleanup  DELETE /api/nodes/${CLIENT_AE}"

HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" \
    -X DELETE "${HTTP_BASE}/api/nodes/${CLIENT_AE}")

if [ "$HTTP_CODE" = "204" ]; then
    ok "Test node '${CLIENT_AE}' removed"
else
    fail "Node removal returned HTTP ${HTTP_CODE} (non-fatal)"
fi

# ── Summary ───────────────────────────────────────────────────────────────────

printf "\n\033[1;37m──────────────────────────────────────────\033[0m\n"
TOTAL=$((PASS + FAIL))
if [ "$FAIL" -eq 0 ]; then
    printf "\033[1;32m  All %d checks passed ✓\033[0m\n\n" "$TOTAL"
    exit 0
else
    printf "\033[1;31m  %d/%d checks failed ✗\033[0m\n\n" "$FAIL" "$TOTAL"
    exit 1
fi
