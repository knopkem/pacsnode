# smoke-test.ps1 — end-to-end smoke test for pacsnode (Windows PowerShell)
#
# Tests the full pipeline:
#   1. Health check (HTTP)
#   2. Register a test DICOM node (REST API)
#   3. C-ECHO  via DIMSE   (echoscu)
#   4. C-STORE via DIMSE   (storescu  — uploads testfiles\*.dcm)
#   5. Statistics check    (REST API  — confirms files were stored)
#   6. QIDO-RS query       (DICOMweb  — lists uploaded studies)
#   7. C-FIND  via DIMSE   (findscu   — validates patient/study/series/image levels)
#   8. WADO-RS / WADO-URI  (DICOMweb  — retrieves an instance, frame bytes,
#                           rendered PNG preview, bulk pixel data, and
#                           legacy WADO-URI retrieval)
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

function Get-HttpResponse([string]$Uri, [string]$Method = "GET") {
    try {
        return Invoke-WebRequest -Uri $Uri -Method $Method -ErrorAction Stop -UseBasicParsing
    } catch {
        return $null
    }
}

function Test-QidoContainsUid {
    param(
        [Parameter(Mandatory = $true)]$Payload,
        [Parameter(Mandatory = $true)][string]$Tag,
        [Parameter(Mandatory = $true)][string]$ExpectedUid
    )

    foreach ($item in @($Payload)) {
        if ($null -eq $item) { continue }

        $tagProp = $item.PSObject.Properties[$Tag]
        if ($null -eq $tagProp) { continue }

        $valueProp = $tagProp.Value.PSObject.Properties["Value"]
        if ($null -eq $valueProp) { continue }

        foreach ($value in @($valueProp.Value)) {
            if ([string]$value -eq $ExpectedUid) {
                return $true
            }
        }
    }

    return $false
}

function Test-CFindHasResults {
    param([Parameter(Mandatory = $true)][string]$Output)
    return $Output -match 'Found [1-9][0-9]* result\(s\):'
}

function Test-CFindContainsValue {
    param(
        [Parameter(Mandatory = $true)][string]$Output,
        [Parameter(Mandatory = $true)][string]$ExpectedValue
    )

    return $Output.Contains($ExpectedValue)
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

$studyUid = ""
$patientId = ""
$seriesUid = ""
$instanceUid = ""

$studyList = Invoke-Api "$HttpBase/api/studies"
if ($studyList -and @($studyList).Count -gt 0) {
    $studyUid = @($studyList)[0].study_uid
    Write-Ok "StudyInstanceUID resolved via REST: $studyUid"
} else {
    Write-Fail "Could not resolve a study UID — was C-STORE successful?"
}

if ($studyUid -ne "") {
    $studyDetails = Invoke-Api "$HttpBase/api/studies/$studyUid"
    if ($studyDetails -and $studyDetails.patient_id) {
        $patientId = [string]$studyDetails.patient_id
        Write-Ok "PatientID resolved via REST: $patientId"
    } else {
        Write-Fail "Could not resolve a patient ID for C-FIND validation"
    }
}

$qidoCode = Get-HttpStatus "$HttpBase/wado/studies"
if ($qidoCode -eq 200) {
    $qidoStudies = Invoke-Api "$HttpBase/wado/studies"
    if ($studyUid -ne "" -and (Test-QidoContainsUid $qidoStudies "0020000D" $studyUid)) {
        Write-Ok "QIDO-RS studies response contains StudyInstanceUID $studyUid"
    } else {
        Write-Fail "QIDO-RS studies response missing StudyInstanceUID $studyUid"
    }
} else {
    Write-Fail "QIDO-RS studies returned HTTP $qidoCode"
}

if ($studyUid -ne "") {
    $seriesList = Invoke-Api "$HttpBase/api/studies/$studyUid/series"
    if ($seriesList -and @($seriesList).Count -gt 0) {
        $seriesUid = @($seriesList)[0].series_uid

        $qidoSeriesCode = Get-HttpStatus "$HttpBase/wado/studies/$studyUid/series"
        if ($qidoSeriesCode -eq 200) {
            $qidoSeries = Invoke-Api "$HttpBase/wado/studies/$studyUid/series"
            if (Test-QidoContainsUid $qidoSeries "0020000E" $seriesUid) {
                Write-Ok "QIDO-RS series response contains SeriesInstanceUID $seriesUid"
            } else {
                Write-Fail "QIDO-RS series response missing SeriesInstanceUID $seriesUid"
            }
        } else {
            Write-Fail "QIDO-RS series returned HTTP $qidoSeriesCode"
        }
    } else {
        Write-Fail "Could not resolve a series UID for QIDO-RS series validation"
    }

    if ($seriesUid -ne "") {
        $instanceList = Invoke-Api "$HttpBase/api/series/$seriesUid/instances"
        if ($instanceList -and @($instanceList).Count -gt 0) {
            $instanceUid = @($instanceList)[0].instance_uid

            $qidoInstancesCode = Get-HttpStatus "$HttpBase/wado/studies/$studyUid/series/$seriesUid/instances"
            if ($qidoInstancesCode -eq 200) {
                $qidoInstances = Invoke-Api "$HttpBase/wado/studies/$studyUid/series/$seriesUid/instances"
                if (Test-QidoContainsUid $qidoInstances "00080018" $instanceUid) {
                    Write-Ok "QIDO-RS instances response contains SOPInstanceUID $instanceUid"
                } else {
                    Write-Fail "QIDO-RS instances response missing SOPInstanceUID $instanceUid"
                }
            } else {
                Write-Fail "QIDO-RS instances returned HTTP $qidoInstancesCode"
            }
        } else {
            Write-Fail "Could not resolve an instance UID for QIDO-RS instance validation"
        }
    }
}

# ── Step 7: C-FIND (DIMSE) ────────────────────────────────────────────────────

Write-Step 7 "C-FIND  (DIMSE patient/study/series/image)"

if ($patientId -ne "") {
    $findPatientOut = (& findscu $PacsHost $DicomPort `
        --aetitle $ClientAe --called-ae $PacsAe `
        --level PATIENT `
        --key "0010,0020=$patientId" `
        --verbose 2>&1 | Out-String)
    if ($LASTEXITCODE -eq 0 -and (Test-CFindHasResults $findPatientOut) -and (Test-CFindContainsValue $findPatientOut $patientId)) {
        Write-Ok "C-FIND PATIENT returned PatientID $patientId"
    } else {
        Write-Fail "C-FIND PATIENT failed or did not contain PatientID $patientId"
    }
} else {
    Write-Fail "Skipping C-FIND PATIENT validation — no patient ID available"
}

if ($studyUid -ne "") {
    $findStudyOut = (& findscu $PacsHost $DicomPort `
        --aetitle $ClientAe --called-ae $PacsAe `
        --level STUDY `
        --key "0020,000D=$studyUid" `
        --verbose 2>&1 | Out-String)
    if ($LASTEXITCODE -eq 0 -and (Test-CFindHasResults $findStudyOut) -and (Test-CFindContainsValue $findStudyOut $studyUid)) {
        Write-Ok "C-FIND STUDY returned StudyInstanceUID $studyUid"
    } else {
        Write-Fail "C-FIND STUDY failed or did not contain StudyInstanceUID $studyUid"
    }
} else {
    Write-Fail "Skipping C-FIND STUDY validation — no study UID available"
}

if ($studyUid -ne "" -and $seriesUid -ne "") {
    $findSeriesOut = (& findscu $PacsHost $DicomPort `
        --aetitle $ClientAe --called-ae $PacsAe `
        --level SERIES `
        --key "0020,000D=$studyUid" `
        --key "0020,000E=" `
        --verbose 2>&1 | Out-String)
    if ($LASTEXITCODE -eq 0 -and (Test-CFindHasResults $findSeriesOut) -and (Test-CFindContainsValue $findSeriesOut $seriesUid)) {
        Write-Ok "C-FIND SERIES returned SeriesInstanceUID $seriesUid"
    } else {
        Write-Fail "C-FIND SERIES failed or did not contain SeriesInstanceUID $seriesUid"
    }
} else {
    Write-Fail "Skipping C-FIND SERIES validation — no series UID available"
}

if ($studyUid -ne "" -and $seriesUid -ne "" -and $instanceUid -ne "") {
    $findImageOut = (& findscu $PacsHost $DicomPort `
        --aetitle $ClientAe --called-ae $PacsAe `
        --level IMAGE `
        --key "0020,000D=$studyUid" `
        --key "0020,000E=$seriesUid" `
        --key "0008,0018=" `
        --verbose 2>&1 | Out-String)
    if ($LASTEXITCODE -eq 0 -and (Test-CFindHasResults $findImageOut) -and (Test-CFindContainsValue $findImageOut $instanceUid)) {
        Write-Ok "C-FIND IMAGE returned SOPInstanceUID $instanceUid"
    } else {
        Write-Fail "C-FIND IMAGE failed or did not contain SOPInstanceUID $instanceUid"
    }
} else {
    Write-Fail "Skipping C-FIND IMAGE validation — no instance UID available"
}

# ── Step 8: WADO-RS / WADO-URI retrieve ───────────────────────────────────────

Write-Step 8 "WADO-RS / WADO-URI retrieve"
Write-Host "  (dicom-toolkit-rs has no getscu binary; WADO-RS is the" -ForegroundColor DarkGray
Write-Host "   standard DICOMweb equivalent for instance retrieval)" -ForegroundColor DarkGray

if ($studyUid -ne "") {
    # Resolve series and instance UIDs via REST API if step 6 did not already.
    if ($seriesUid -eq "") {
        $seriesList = Invoke-Api "$HttpBase/api/studies/$studyUid/series"
        if ($seriesList -and @($seriesList).Count -gt 0) {
            $seriesUid = @($seriesList)[0].series_uid
        }
    }

    if ($seriesUid -ne "" -and $instanceUid -eq "") {
        $instanceList = Invoke-Api "$HttpBase/api/series/$seriesUid/instances"
        if ($instanceList -and @($instanceList).Count -gt 0) {
            $instanceUid = @($instanceList)[0].instance_uid
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

        $frameResp = Get-HttpResponse "$HttpBase/wado/studies/$studyUid/series/$seriesUid/instances/$instanceUid/frames/1"
        if ($frameResp -and $frameResp.StatusCode -eq 200 -and [string]$frameResp.Headers["Content-Type"] -like "*application/octet-stream*") {
            Write-Ok "WADO-RS frame retrieval returned octet-stream multipart data"
        } else {
            Write-Fail "WADO-RS frame retrieval failed"
        }

        $renderResp = Get-HttpResponse "$HttpBase/wado/studies/$studyUid/series/$seriesUid/instances/$instanceUid/rendered"
        if ($renderResp -and $renderResp.StatusCode -eq 200 -and [string]$renderResp.Headers["Content-Type"] -like "*image/png*") {
            Write-Ok "WADO-RS rendered instance returned PNG"
        } else {
            Write-Fail "WADO-RS rendered instance failed"
        }

        $bulkResp = Get-HttpResponse "$HttpBase/wado/studies/$studyUid/series/$seriesUid/instances/$instanceUid/bulkdata/7FE00010"
        if ($bulkResp -and $bulkResp.StatusCode -eq 200 -and [string]$bulkResp.Headers["Content-Type"] -like "*application/octet-stream*") {
            Write-Ok "WADO-RS bulk data returned application/octet-stream"
        } else {
            Write-Fail "WADO-RS bulk data failed"
        }

        $wadoUriResp = Get-HttpResponse "$HttpBase/wado?requestType=WADO&studyUID=$studyUid&seriesUID=$seriesUid&objectUID=$instanceUid"
        if ($wadoUriResp -and $wadoUriResp.StatusCode -eq 200 -and [string]$wadoUriResp.Headers["Content-Type"] -like "*application/dicom*") {
            Write-Ok "WADO-URI returned application/dicom"
        } else {
            Write-Fail "WADO-URI failed"
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
