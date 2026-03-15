# smoke-test.ps1 — end-to-end smoke test for pacsnode (Windows PowerShell)
#
# Tests the full pipeline:
#   1. Health check (HTTP)
#   2. Register a test DICOM node (REST API)
#   3. C-ECHO  via DIMSE   (echoscu)
#   4. C-STORE via DIMSE   (storescu  — uploads testfiles\*.dcm)
#   5. Statistics check    (REST API  — confirms files were stored)
#   6. QIDO-RS query       (DICOMweb  — lists uploaded studies)
#   7. C-FIND  via DIMSE   (findscu   — queries by Study Root)
#   8. WADO-RS retrieve    (DICOMweb  — retrieves a single instance;
#                           equivalent to C-GET without a dedicated getscu
#                           binary, which dicom-toolkit-rs does not provide)
#   9. Cleanup             (removes the test node registration)
#
# Prerequisites:
#   - pacsnode running (docker compose up, or cargo run)
#   - cargo in PATH (used to install DICOM CLI tools if not already present)
#
# Usage:
#   .\scripts\smoke-test.ps1
#   .\scripts\smoke-test.ps1 -PacsHost 10.0.0.5 -DicomPort 4242

[CmdletBinding()]
param(
    [string]$PacsHost   = $env:PACS_HOST   ?? "localhost",
    [uint16]$HttpPort   = [uint16]($env:HTTP_PORT   ?? 8042),
    [uint16]$DicomPort  = [uint16]($env:DICOM_PORT  ?? 4242),
    [string]$PacsAe     = $env:PACS_AE     ?? "PACSNODE",
    [string]$ClientAe   = $env:CLIENT_AE   ?? "SMOKETEST"
)

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$HttpBase    = "http://${PacsHost}:${HttpPort}"
$ScriptRoot  = Split-Path -Parent $PSCommandPath
$TestfilesDir = Join-Path (Split-Path -Parent $ScriptRoot) "testfiles"

# ── Helpers ───────────────────────────────────────────────────────────────────

$PassCount = 0
$FailCount = 0

function Write-Ok([string]$Msg) {
    Write-Host "  " -NoNewline
    Write-Host "√" -ForegroundColor Green -NoNewline
    Write-Host " $Msg"
    $script:PassCount++
}

function Write-Fail([string]$Msg) {
    Write-Host "  " -NoNewline
    Write-Host "X" -ForegroundColor Red -NoNewline
    Write-Host " $Msg"
    $script:FailCount++
}

function Write-Step([int]$Num, [string]$Title) {
    Write-Host ""
    Write-Host "── Step ${Num}: ${Title}" -ForegroundColor Cyan
}

function Test-Command([string]$Name) {
    $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

# Invoke-RestMethod wrapper that does not throw on non-2xx (returns $null)
function Invoke-Api {
    param([string]$Uri, [string]$Method = "GET", $Body = $null, [string]$ContentType = "application/json")
    try {
        $params = @{ Uri = $Uri; Method = $Method; ErrorAction = "Stop" }
        if ($null -ne $Body) {
            $params.Body        = ($Body | ConvertTo-Json -Compress)
            $params.ContentType = $ContentType
        }
        return Invoke-RestMethod @params
    } catch {
        return $null
    }
}

# Return the HTTP status code without throwing
function Get-HttpStatus([string]$Uri, [string]$Method = "GET", $Body = $null) {
    try {
        $params = @{ Uri = $Uri; Method = $Method; ErrorAction = "Stop" }
        if ($null -ne $Body) { $params.Body = ($Body | ConvertTo-Json -Compress); $params.ContentType = "application/json" }
        $resp = Invoke-WebRequest @params -UseBasicParsing
        return $resp.StatusCode
    } catch [System.Net.WebException] {
        $r = $_.Exception.Response
        if ($null -ne $r) { return [int]$r.StatusCode }
        return 0
    } catch {
        return 0
    }
}

# ── Banner ────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "╔══════════════════════════════════════════╗" -ForegroundColor White
Write-Host "║       pacsnode smoke test                ║" -ForegroundColor White
Write-Host "╚══════════════════════════════════════════╝" -ForegroundColor White
Write-Host "  PACS:   $HttpBase  (AE: $PacsAe)"
Write-Host "  DIMSE:  ${PacsHost}:${DicomPort}"
Write-Host "  Client: $ClientAe"
Write-Host "  Files:  $TestfilesDir"

# ── Step 0: Prerequisites ─────────────────────────────────────────────────────

Write-Step 0 "Prerequisites"

if (-not (Test-Command "cargo")) {
    Write-Fail "cargo not found — install Rust from https://rustup.rs"
    exit 1
}
Write-Ok "cargo found"

$DcmFiles = Get-ChildItem -Path $TestfilesDir -Filter "*.dcm" -ErrorAction SilentlyContinue |
            Sort-Object Name
if ($DcmFiles.Count -eq 0) {
    Write-Fail "No .dcm files found in $TestfilesDir"
    exit 1
}
Write-Ok "$($DcmFiles.Count) DICOM test file(s) found"

# Install dicom-toolkit-rs CLI tools if not present
$toolsMissing = -not (Test-Command "echoscu") -or
                -not (Test-Command "storescu") -or
                -not (Test-Command "findscu")

if ($toolsMissing) {
    Write-Host ""
    Write-Host "  DICOM CLI tools not found — installing via cargo..." -ForegroundColor Yellow
    Write-Host "  (This may take a few minutes on the first run.)"
    & cargo install `
        --git https://github.com/knopkem/dicom-toolkit-rs `
        --branch main `
        dicom-toolkit-tools `
        --quiet
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Failed to install dicom-toolkit-tools — check cargo output above"
        exit 1
    }
}
Write-Ok "echoscu / storescu / findscu available"

# ── Step 1: Health check ──────────────────────────────────────────────────────

Write-Step 1 "Health check  GET /health"

$health = Invoke-Api "$HttpBase/health"
if ($health -and $health.status -eq "ok") {
    Write-Ok "Server is healthy"
} else {
    Write-Fail "Health check failed — is pacsnode running on $HttpBase ?"
    exit 1
}

# ── Step 2: Register test node ────────────────────────────────────────────────

Write-Step 2 "Register test node  POST /api/nodes"

$nodeBody = @{
    ae_title    = $ClientAe
    host        = $PacsHost
    port        = [int]$DicomPort
    description = "Smoke test node"
    tls_enabled = $false
}
$code = Get-HttpStatus "$HttpBase/api/nodes" "POST" $nodeBody
if ($code -eq 201) {
    Write-Ok "Node '$ClientAe' registered (201 Created)"
} else {
    Write-Fail "Node registration returned HTTP $code"
}

$nodes = Invoke-Api "$HttpBase/api/nodes"
if ($nodes -and ($nodes | Where-Object { $_.ae_title -eq $ClientAe })) {
    Write-Ok "Node '$ClientAe' confirmed in GET /api/nodes"
} else {
    Write-Fail "Node '$ClientAe' not visible in GET /api/nodes"
}

# ── Step 3: C-ECHO ────────────────────────────────────────────────────────────

Write-Step 3 "C-ECHO  (DIMSE)"

$echoOut = & echoscu $PacsHost $DicomPort --aetitle $ClientAe --called-ae $PacsAe --verbose 2>&1
if ($LASTEXITCODE -eq 0) {
    Write-Ok "C-ECHO succeeded (exit 0)"
} else {
    Write-Fail "C-ECHO failed — check DIMSE port $DicomPort and AE title '$PacsAe'"
}

# ── Step 4: C-STORE (upload test files) ───────────────────────────────────────

Write-Step 4 "C-STORE  — uploading $($DcmFiles.Count) file(s)"

$filePaths = $DcmFiles | ForEach-Object { $_.FullName }
& storescu $PacsHost $DicomPort @filePaths --aetitle $ClientAe --called-ae $PacsAe --verbose 2>&1 |
    Out-Null
if ($LASTEXITCODE -eq 0) {
    Write-Ok "C-STORE completed (exit 0)"
} else {
    Write-Fail "C-STORE failed — check DIMSE port and server logs"
}

# ── Step 5: Statistics ────────────────────────────────────────────────────────

Write-Step 5 "Statistics check  GET /statistics"

$stats = Invoke-Api "$HttpBase/statistics"
if ($stats -and $stats.studies -gt 0) {
    Write-Ok "Database has $($stats.studies) study/studies, $($stats.instances) instance(s)"
} else {
    Write-Fail "No studies found after upload (studies=$($stats?.studies))"
}

# ── Step 6: QIDO-RS query ─────────────────────────────────────────────────────

Write-Step 6 "QIDO-RS  GET /wado/studies"

$qidoCode = Get-HttpStatus "$HttpBase/wado/studies"
if ($qidoCode -eq 200) {
    Write-Ok "QIDO-RS endpoint reachable (HTTP 200)"
} else {
    Write-Fail "QIDO-RS returned HTTP $qidoCode"
}

# Resolve study UID via REST API (reliable structured response)
$studyUid = ""
$studyList = Invoke-Api "$HttpBase/api/studies"
if ($studyList -and $studyList.Count -gt 0) {
    $studyUid = $studyList[0].study_uid
    Write-Ok "StudyInstanceUID resolved via REST: $studyUid"
} else {
    Write-Fail "Could not resolve a study UID — was C-STORE successful?"
}

# ── Step 7: C-FIND (DIMSE) ────────────────────────────────────────────────────

Write-Step 7 "C-FIND  (DIMSE Study Root)"

$findOut = & findscu $PacsHost $DicomPort `
    --aetitle $ClientAe --called-ae $PacsAe `
    --level STUDY `
    --key "0008,0052=STUDY" `
    --verbose 2>&1

if ($LASTEXITCODE -eq 0) {
    Write-Ok "C-FIND completed (exit 0)"
} else {
    Write-Fail "C-FIND failed or returned no results"
}

# ── Step 8: WADO-RS retrieve (C-GET equivalent) ───────────────────────────────

Write-Step 8 "WADO-RS retrieve  (DICOMweb C-GET equivalent)"
Write-Host "  (dicom-toolkit-rs has no getscu binary; WADO-RS is the" -ForegroundColor DarkGray
Write-Host "   standard DICOMweb equivalent for instance retrieval)" -ForegroundColor DarkGray

if ($studyUid -ne "") {
    # Resolve series and instance UIDs via REST API
    $seriesUid = ""
    $instanceUid = ""

    $seriesList = Invoke-Api "$HttpBase/api/studies/$studyUid/series"
    if ($seriesList -and $seriesList.Count -gt 0) {
        $seriesUid = $seriesList[0].series_uid
    }

    if ($seriesUid -ne "") {
        $instanceList = Invoke-Api "$HttpBase/api/series/$seriesUid/instances"
        if ($instanceList -and $instanceList.Count -gt 0) {
            $instanceUid = $instanceList[0].instance_uid
        }
    }

    if ($instanceUid -ne "") {
        $retrieveUrl = "$HttpBase/wado/studies/$studyUid/series/$seriesUid/instances/$instanceUid"
        $code = Get-HttpStatus $retrieveUrl
        if ($code -eq 200) {
            Write-Ok "WADO-RS retrieve returned HTTP 200"
            Write-Ok "SeriesInstanceUID: $seriesUid"
            Write-Ok "SOPInstanceUID:    $instanceUid"
        } else {
            Write-Fail "WADO-RS retrieve returned HTTP $code"
        }
    } else {
        Write-Fail "Could not resolve series/instance UIDs for WADO-RS retrieve"
    }
} else {
    Write-Fail "Skipping WADO-RS retrieve — no study UID available"
}

# ── Step 9: System info ───────────────────────────────────────────────────────

Write-Step 9 "System info  GET /system"

$sysInfo = Invoke-Api "$HttpBase/system"
if ($sysInfo) {
    Write-Ok "AE title: $($sysInfo.ae_title), registered nodes: $($sysInfo.nodes.Count)"
}

# ── Step 10: Cleanup ──────────────────────────────────────────────────────────

Write-Step 10 "Cleanup  DELETE /api/nodes/$ClientAe"

$code = Get-HttpStatus "$HttpBase/api/nodes/$ClientAe" "DELETE"
if ($code -eq 204) {
    Write-Ok "Test node '$ClientAe' removed"
} else {
    Write-Fail "Node removal returned HTTP $code (non-fatal)"
}

# ── Summary ───────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host "──────────────────────────────────────────" -ForegroundColor White
$total = $PassCount + $FailCount
if ($FailCount -eq 0) {
    Write-Host "  All $total checks passed √" -ForegroundColor Green
    exit 0
} else {
    Write-Host "  $FailCount/$total checks failed X" -ForegroundColor Red
    exit 1
}
