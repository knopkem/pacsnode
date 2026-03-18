# OHIF Viewer — pacsnode Server Requirements & Gap Analysis

This document maps every OHIF Viewer feature and extension against pacsnode's
current API surface.  The goal is to identify what pacsnode must implement,
improve, or expose so it can serve as a full-featured backend for the OHIF v3
viewer.

OHIF version surveyed: **3.12.0**  
pacsnode commit surveyed: **55eb062**

---

## Table of Contents

1. [OHIF Architecture Overview](#1-ohif-architecture-overview)
2. [Core DICOMweb Endpoints](#2-core-dicomweb-endpoints)
3. [Measurement Storage (Structured Reports)](#3-measurement-storage-structured-reports)
4. [Segmentation (SEG)](#4-segmentation-seg)
5. [Radiotherapy Structures (RTSTRUCT)](#5-radiotherapy-structures-rtstruct)
6. [Whole-Slide Microscopy (WSI)](#6-whole-slide-microscopy-wsi)
7. [Rendered / Thumbnail Endpoints](#7-rendered--thumbnail-endpoints)
8. [Authentication & Token Forwarding](#8-authentication--token-forwarding)
9. [Multi-Frame & Video](#9-multi-frame--video)
10. [Metadata Field Coverage (QIDO-RS)](#10-metadata-field-coverage-qido-rs)
11. [BulkDataURI Support](#11-bulkdatauri-support)
12. [Transfer Syntax Negotiation](#12-transfer-syntax-negotiation)
13. [STOW-RS (Store Instances)](#13-stow-rs-store-instances)
14. [Worklist / MWL / UPS-RS](#14-worklist--mwl--ups-rs)
15. [Hanging Protocols & Presentation State](#15-hanging-protocols--presentation-state)
16. [Performance & Concurrency](#16-performance--concurrency)
17. [Prioritised Implementation Roadmap](#17-prioritised-implementation-roadmap)

---

## 1. OHIF Architecture Overview

OHIF uses a layered extension/mode system:

```
Modes (workflow bundles)
  └── Extensions (feature packages)
        ├── cornerstone          — 2D/3D GPU rendering (WebGL)
        ├── cornerstone-dicom-sr — Structured Report read/write
        ├── cornerstone-dicom-seg— Segmentation read/write
        ├── cornerstone-dicom-rt — RTSTRUCT overlay
        ├── cornerstone-dicom-pmap — Parametric maps
        ├── dicom-microscopy     — Whole-slide tiled imaging
        ├── dicom-video          — DICOM video playback
        ├── dicom-pdf            — Encapsulated PDF
        ├── measurement-tracking — Measurement panel, SR save
        └── tmtv                 — PET/CT SUV / TMTV quantification
```

All server communication is via the `dicomweb-client` npm library, which issues
standard QIDO-RS, WADO-RS, and STOW-RS HTTP requests.  There is no proprietary
protocol; everything is PS3.18 with optional extensions.

---

## 2. Core DICOMweb Endpoints

These endpoints are required for any study to load at all.

### 2.1 QIDO-RS — Study/Series/Instance Search

| Endpoint | Method | Required params | pacsnode status |
|----------|--------|-----------------|-----------------|
| `/studies` | GET | `PatientName`, `PatientID`, `StudyDate`, `Modality`, `limit`, `offset` | ✅ Implemented |
| `/studies/{uid}/series` | GET | — | ✅ Implemented |
| `/studies/{uid}/series/{uid}/instances` | GET | — | ✅ Implemented |

**Additional query parameters OHIF uses:**

| Parameter | Notes | pacsnode status |
|-----------|-------|-----------------|
| `fuzzymatching=true` | Fuzzy patient-name matching | ✅ Implemented for study-level QIDO and passed through to the metadata store |
| `includeField=00080016` | SOP Class UID, Rows, Columns, etc. | ✅ Implemented for study/series QIDO shaping; instance QIDO already returns stored metadata |
| `AccessionNumber` | Required for worklist / order matching | ✅ Stored in JSONB |
| `StudyDescription` | Display field | ✅ Stored |
| `ModalitiesInStudy` | Multi-modality filter | ✅ Stored |

**OHIF reads these fields from QIDO responses:**

```
StudyInstanceUID    0020000D
SeriesInstanceUID   0020000E
SOPInstanceUID      00080018
StudyDate           00080020
StudyTime           00080030
AccessionNumber     00080050
PatientID           00100020
PatientName         00100010
StudyDescription    00081030
Modality            00080060
ModalitiesInStudy   00080061
NumberOfStudyRelatedInstances  00201208
NumberOfSeriesRelatedInstances 00201209
SeriesNumber        00200011
SeriesDescription   0008103E
SeriesDate          00080021
SeriesTime          00080031
```

All of these must be present in QIDO-RS responses for the study browser and
hanging protocols to work correctly.

### 2.2 WADO-RS — Metadata & Instance Retrieval

| Endpoint | Accept | Purpose | pacsnode status |
|----------|--------|---------|-----------------|
| `GET /studies/{uid}/series/{uid}/instances` | `application/dicom+json` | Retrieve all instance metadata for a series | ✅ Implemented |
| `GET /studies/{uid}/series/{uid}/instances/{uid}` | `multipart/related; type="application/octet-stream"` | Full DICOM P10 instance | ✅ Implemented |
| `GET /studies/{uid}/series/{uid}/instances/{uid}/frames/{n}` | `image/jpeg`, `image/png`, or multipart | Individual frame retrieval | ✅ Implemented, including comma-separated frame lists and rendered variants |
| `GET /studies/{uid}/series/{uid}/instances/{uid}/bulkdata/{tag}` | varies | BulkDataURI target | ✅ Implemented; encapsulated documents preserve their declared MIME type, video pixel data is inferred from transfer syntax, and other pixel data defaults to `application/octet-stream` |
| `GET /studies/{uid}/series/{uid}/instances/{uid}/metadata` | `application/dicom+json` | Single-instance metadata | ✅ Implemented via wado.rs |

---

## 3. Measurement Storage (Structured Reports)

### 3.1 What OHIF does

When the user saves measurements, OHIF:

1. Collects all tracked annotations from Cornerstone3D tools.
2. Converts them to a DICOM SR dataset via `@cornerstonejs/adapters`
   `MeasurementReport.generateReport()`.
3. Calls `dataSource.store.dicom(dataset)` → `dicomweb-client.storeInstances()`
   → `POST /studies` with `Content-Type: application/dicom`.
4. On load, OHIF queries for SR instances via QIDO-RS with
   `SOPClassUID=1.2.840.10008.5.1.4.1.1.88.*` and retrieves them via WADO-RS.

**SR SOP Classes generated by OHIF:**

| SOP Class UID | Name |
|---------------|------|
| `1.2.840.10008.5.1.4.1.1.88.11` | Basic Text SR |
| `1.2.840.10008.5.1.4.1.1.88.22` | Enhanced SR |
| `1.2.840.10008.5.1.4.1.1.88.33` | Comprehensive SR |
| `1.2.840.10008.5.1.4.1.1.88.34` | Comprehensive 3D SR (also used by microscopy) |

### 3.2 STOW-RS endpoint required

```
POST /wado/studies
Content-Type: multipart/related; type="application/dicom"; boundary=...

--boundary
Content-Type: application/dicom
<DICOM Part-10 bytes>
--boundary--
```

**pacsnode status:** ✅ STOW-RS exists at `POST /wado/studies` and accepts SR
and SEG SOP classes without SOP-class whitelisting.

### 3.3 QIDO-RS query for SR retrieval

OHIF queries:
```
GET /studies/{uid}/series?Modality=SR
GET /studies/{uid}/series/{uid}/instances?SOPClassUID=1.2.840.10008.5.1.4.1.1.88.11
```

SOPClassUID filtering must work in pacsnode's QIDO implementation.

**pacsnode status:** ✅ Implemented in instance-level QIDO query handling.

### 3.4 Per-user measurement storage

OHIF does **not** embed user identity into SR DICOM metadata.  User identity is
tracked server-side by the Bearer token.  To support per-user measurement
isolation, pacsnode would need:

- Multi-user authentication with user-scoped study/series ownership, **or**
- A naming convention where SR Series are tagged with a user-specific attribute
  set at STOW-RS ingest time.

**Current pacsnode status:** ❌ Single hardcoded user; no per-user scoping.

---

## 4. Segmentation (SEG)

### 4.1 What OHIF does

OHIF creates and displays DICOM SEG objects.

**SEG SOP Classes:**

| SOP Class UID | Name |
|---------------|------|
| `1.2.840.10008.5.1.4.1.1.66.4` | Segmentation Storage |
| `1.2.840.10008.5.1.4.1.1.66.7` | Labelmap Segmentation Storage |

Storage path is identical to SR: `POST /wado/studies` via STOW-RS.

### 4.2 SEG retrieval

```
GET /studies/{uid}/series?Modality=SEG
GET /studies/{uid}/series/{uid}/instances/{uid}
Accept: application/dicom; transfer-syntax=1.2.840.10008.1.2.1
```

SEG instances can be large (one frame per segmentation mask per source frame).
pacsnode must not time out on large multiframe object retrieval.

**pacsnode status:** ✅ STOW-RS for storage; ✅ WADO-RS for retrieval.
Recommend verifying behaviour for large (>100 MB) SEG objects.

---

## 5. Radiotherapy Structures (RTSTRUCT)

OHIF renders RTSTRUCT overlays but treats them as read-only.

**RTSTRUCT SOP Class:** `1.2.840.10008.5.1.4.1.1.481.3`

Retrieval:
```
GET /studies/{uid}/series?Modality=RTSTRUCT
GET /studies/{uid}/series/{uid}/instances/{uid}
```

No additional server capability is required beyond standard WADO-RS.

**pacsnode status:** ✅ No gaps expected; RTSTRUCT stored and retrieved like any
other SOP class.

---

## 6. Whole-Slide Microscopy (WSI)

### 6.1 SOP classes

| SOP Class UID | Name |
|---------------|------|
| `1.2.840.10008.5.1.4.1.1.77.1.6` | VL Whole Slide Microscopy Image Storage |
| `1.2.840.10008.5.1.4.1.1.91.1` | Microscopy Bulk Simple Annotations |
| `1.2.840.10008.5.1.4.1.1.88.34` | Comprehensive 3D SR (for microscopy measurements) |

### 6.2 Tiled frame retrieval

The `dicom-microscopy-viewer` library requests individual tiles as:
```
GET /studies/{uid}/series/{uid}/instances/{uid}/frames/{frameNumber}
Accept: application/dicom   (or multipart/related)
```

For a typical 40x slide this can be thousands of tiles.  The server must:

- Respond promptly (< 200 ms per tile) to allow smooth pan/zoom.
- Support concurrent requests (OHIF fires up to 100 in parallel for interaction
  and 75 for thumbnails).
- Return correct `Content-Type` multipart framing.

**pacsnode status:** ⚠️ Frame-level endpoint exists but performance under high
tile-request concurrency has not been tested.  S3 pre-signed URLs via
BulkDataURI would be the most scalable path for large WSI objects.

### 6.3 Pyramid / multi-resolution support

WSI objects are stored as multiple series at different resolutions (DICOM pyramid).
OHIF relies on QIDO-RS to discover all series in a study and the `dicom-microscopy-viewer`
library selects the appropriate resolution level based on viewport zoom.  No
special server endpoint is needed beyond standard QIDO-RS series enumeration.

### 6.4 Annotation storage

Microscopy annotations are stored as DICOM SR or Bulk Annotation instances via
STOW-RS (same path as SR above).  `constructSR.ts` calls
`wadoDicomWebClient.storeInstances()` with the constructed dataset.

**pacsnode status:** ✅ Same STOW-RS path as SR — no new endpoint needed.

---

## 7. Rendered / Thumbnail Endpoints

OHIF supports four thumbnail strategies, configurable per deployment.

### 7.1 Strategy comparison

| Strategy | Endpoint | Auth | Server requirement |
|----------|----------|------|--------------------|
| `wadors` | WADO-RS full instance, client renders | Bearer header | No extra endpoint |
| `thumbnail` | `GET .../instances/{uid}/thumbnail?accept=image/jpeg` | Bearer header | ✅ Implemented |
| `rendered` | `GET .../instances/{uid}/rendered?accept=image/jpeg` | Bearer header | ✅ Implemented |
| `thumbnailDirect` | Direct URL, no auth | None | Not recommended — bypasses auth |

### 7.2 `/thumbnail` endpoint specification

```
GET /wado/studies/{studyUID}/series/{seriesUID}/instances/{sopUID}/thumbnail
    ?accept=image/jpeg&rows=128&columns=128

Response: 200 OK
Content-Type: image/jpeg
Body: JPEG bytes
```

pacsnode renders the first frame of the requested instance through the existing
rendered-image pipeline, defaults thumbnails to JPEG, and defaults missing size
parameters to 128×128.

**pacsnode status:** ✅ Implemented at
`GET /wado/studies/{studyUID}/series/{seriesUID}/instances/{sopUID}/thumbnail`.

### 7.3 `/rendered` endpoint specification

```
GET /wado/studies/{studyUID}/series/{seriesUID}/instances/{sopUID}/rendered
    ?accept=image/jpeg&rows=512&columns=512

Response: 200 OK
Content-Type: image/jpeg | image/png
Body: rendered image bytes
```

Used by OHIF when `thumbnailRendering: 'rendered'` is configured.  Similar to
`/thumbnail` but full-resolution or at the requested `rows`/`columns`.

**pacsnode status:** ✅ Implemented for study, series, instance, and frame
render routes. Supports `Accept: image/jpeg|image/png` and `?accept=image/jpeg|image/png`.

---

## 8. Authentication & Token Forwarding

### 8.1 OHIF token flow

1. OHIF receives a Bearer token (via OIDC redirect, URL parameter, or manual
   config).
2. The token is stored in `UserAuthenticationService`.
3. Every QIDO/WADO/STOW request includes `Authorization: Bearer <token>`.

### 8.2 pacsnode auth model

- JWT auth plugin (`pacs-auth-plugin`) issues and validates Bearer tokens.
- Current model: single user; no RBAC.

### 8.3 What is needed for multi-user OHIF

| Requirement | pacsnode status |
|-------------|-----------------|
| Accept `Authorization: Bearer` on all DICOMweb routes | ✅ Implemented |
| Return `401 Unauthorized` on missing/expired token | ✅ Via auth plugin |
| Per-user study access control | ❌ Not implemented |
| OIDC provider integration (Keycloak, Auth0) | ❌ Not implemented |
| User identity surfaced in SR `ObserverContext` | ❌ OHIF does not embed this; server-side tracking only |

For a research/development deployment without multi-user requirements, the
current JWT plugin is sufficient.  For clinical use, per-user RBAC is required.

---

## 9. Multi-Frame & Video

### 9.1 Multi-frame DICOM

OHIF loads multi-frame instances by requesting individual frames:
```
GET /studies/{uid}/series/{uid}/instances/{uid}/frames/{1,2,3,...}
Accept: multipart/related; type="application/octet-stream"; transfer-syntax=1.2.840.10008.1.2.1
```

Multiple frame indices may be comma-separated in one request.  The server must
return a `multipart/related` response with one part per frame.

**pacsnode status:** ✅ Implemented; comma-separated frame lists such as
`/frames/1,2,3` are accepted for WADO-RS frame retrieval.

### 9.2 DICOM video (MPEG-4, H.264)

OHIF plays DICOM video objects via the `dicom-video` extension.  The video
payload is expected to be available as a BulkDataURI pointing to a
`video/mp4` or `video/mpeg` resource.

```json
{
  "7FE00010": {
    "BulkDataURI": "/wado/studies/{uid}/series/{uid}/instances/{uid}/bulkdata/7FE00010"
  }
}
```

OHIF's `default.js` config contains:
```js
bulkDataURI: {
  transform: url => url.replace('/pixeldata.mp4', '/rendered')
}
```

pacsnode must serve pixel data via BulkDataURI and set the correct `Content-Type`
(`video/mp4` or `application/octet-stream`) for video objects.

**pacsnode status:** ✅ BulkDataURI is emitted in WADO-RS JSON responses.
Video pixel-data responses infer `video/mp4` or `video/mpeg` from the stored
transfer syntax, while other pixel data still defaults to
`application/octet-stream`.

### 9.3 Encapsulated PDF

Similar pattern: PDF objects must have a BulkDataURI resolvable with
`Content-Type: application/pdf` (or `application/octet-stream`).

**pacsnode status:** ✅ Encapsulated documents now return the declared
`MIMETypeOfEncapsulatedDocument` for BulkDataURI responses, including
`application/pdf` when present.

---

## 10. Metadata Field Coverage (QIDO-RS)

OHIF expects the following fields to be present (or absent gracefully) in
QIDO-RS JSON responses.  Missing fields cause silent rendering failures or
broken hanging protocols.

### 10.1 Required study-level fields

| Tag | Name | pacsnode |
|-----|------|---------|
| `0020000D` | StudyInstanceUID | ✅ |
| `00080020` | StudyDate | ✅ |
| `00080030` | StudyTime | ✅ |
| `00080050` | AccessionNumber | ✅ |
| `00100020` | PatientID | ✅ |
| `00100010` | PatientName (PN JSON format) | ✅ |
| `00081030` | StudyDescription | ✅ |
| `00080061` | ModalitiesInStudy | ✅ |
| `00201208` | NumberOfStudyRelatedInstances | ✅ |

### 10.2 Required series-level fields

| Tag | Name | pacsnode |
|-----|------|---------|
| `0020000E` | SeriesInstanceUID | ✅ |
| `00200011` | SeriesNumber | ✅ |
| `00080060` | Modality | ✅ |
| `0008103E` | SeriesDescription | ✅ |
| `00201209` | NumberOfSeriesRelatedInstances | ✅ |
| `00080021` | SeriesDate | ✅ |
| `00080031` | SeriesTime | ✅ |

### 10.3 Required instance-level fields

| Tag | Name | pacsnode |
|-----|------|---------|
| `00080018` | SOPInstanceUID | ✅ |
| `00080016` | SOPClassUID | ✅ |
| `00200013` | InstanceNumber | ✅ |
| `00280010` | Rows | ✅ |
| `00280011` | Columns | ✅ |
| `00280008` | NumberOfFrames | ✅ |
| `00080008` | ImageType | ✅ |

### 10.4 PatientName JSON format

OHIF expects PN values in DICOM JSON format:
```json
{
  "00100010": {
    "vr": "PN",
    "Value": [{ "Alphabetic": "Smith^John" }]
  }
}
```

A flat string like `"Value": ["Smith^John"]` will fail OHIF's PN parser.

**pacsnode status:** ✅ JSON serialization uses `dicom-toolkit-data` which
follows PS3.18 PN encoding.

---

## 11. BulkDataURI Support

OHIF metadata requests (`application/dicom+json`) expect pixel data and other
large binary attributes to be returned as `BulkDataURI` references rather than
inline base64 — this is critical for performance.

```json
{
  "7FE00010": {
    "vr": "OB",
    "BulkDataURI": "http://pacsnode/wado/studies/1.2.3/series/4.5.6/instances/7.8.9/bulkdata/7FE00010"
  }
}
```

### 11.1 BulkDataURI endpoint

```
GET /wado/studies/{uid}/series/{uid}/instances/{uid}/bulkdata/{tag}
```

For encapsulated Pixel Data the response is the raw encapsulated byte stream
(all fragments concatenated, no DICOM framing).

**pacsnode status:** ✅ BulkDataURI is emitted; endpoint exists.  Encapsulated
documents preserve their declared MIME type, video pixel data infers
`video/mp4` or `video/mpeg` from transfer syntax, and other pixel data uses
`application/octet-stream`.  BulkDataURI generation also honors
`X-Forwarded-Prefix` so reverse-proxied deployments can emit prefixed relative
URIs directly.

### 11.2 Reverse-proxy path correction

If pacsnode sits behind a reverse proxy at `/pacs`, the BulkDataURI must be
`/pacs/wado/...` not `/wado/...`.  OHIF config supports a `transform` callback
to correct this.  pacsnode now also honors `X-Forwarded-Prefix` when generating
BulkDataURI values, so the server can emit `/pacs/wado/...` directly.

---

## 12. Transfer Syntax Negotiation

OHIF requests a specific transfer syntax via the WADO-RS Accept header:

```
Accept: multipart/related; type="application/octet-stream"; transfer-syntax=1.2.840.10008.1.2.1
```

or via a URL parameter:
```
?transferSyntax=1.2.840.10008.1.2.1
```

pacsnode currently transcodes on retrieve to the requested transfer syntax using
`dicom-toolkit-codec`.

### 12.1 Transfer syntaxes OHIF benefits from

| UID | Name | pacsnode |
|-----|------|---------|
| `1.2.840.10008.1.2.1` | Explicit VR Little Endian | ✅ |
| `1.2.840.10008.1.2.5` | RLE Lossless | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.50` | JPEG Baseline | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.57` | JPEG Lossless | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.70` | JPEG Lossless SV1 | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.80` | JPEG-LS Lossless | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.81` | JPEG-LS Near-Lossless | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.90` | JPEG 2000 Lossless | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.91` | JPEG 2000 Lossy | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.201` | HTJ2K Lossless | ✅ decode; ✅ encode |
| `1.2.840.10008.1.2.4.203` | HTJ2K | ✅ decode; ✅ encode |

OHIF's Cornerstone3D image loader supports client-side decoding of most of the
above so the server can return compressed data.  Returning the stored transfer
syntax (no server-side decode/re-encode) is optimal for performance.

---

## 13. STOW-RS (Store Instances)

OHIF uses STOW-RS to persist:
- Structured Reports (SR) — measurements saved from the measurement panel
- Segmentations (SEG) — masks created or modified in the segmentation mode
- Microscopy annotations (SR / Bulk Simple Annotations)

### 13.1 Required endpoint

```
POST /wado/studies
Content-Type: multipart/related; type="application/dicom"; boundary=<b>
Authorization: Bearer <token>

--<b>
Content-Type: application/dicom
<DICOM P10 bytes>
--<b>--
```

Response:
```
HTTP/1.1 200 OK
Content-Type: application/dicom+json
{
  "00081190": { "vr": "UR", "Value": ["http://pacsnode/wado/studies/{uid}"] }
}
```

**pacsnode status:** ✅ STOW-RS implemented at `POST /wado/studies`.

### 13.2 Validation considerations

pacsnode should accept any SOP class on STOW-RS (no SOP class whitelist) unless
a policy decision is made.  Blocking SR or SEG classes will silently prevent
measurement saving.

---

## 14. Worklist / MWL / UPS-RS

OHIF currently does **not** implement UPS-RS (Unified Procedure Step) or DICOM
MWL.  Its worklist UI is the QIDO-RS study browser only.

There is no server-side worklist endpoint needed to support the current OHIF
feature set.  This section documents what would be needed if a future OHIF
extension adds UPS support.

| Feature | Standard | pacsnode status |
|---------|----------|-----------------|
| Study browser via QIDO-RS | PS3.18 | ✅ |
| DICOM Modality Worklist (MWL) | PS3.4 C-FIND | ❌ Not implemented |
| UPS-RS (Worklist REST) | PS3.18 | ❌ Not implemented |

---

## 15. Hanging Protocols & Presentation State

OHIF hanging protocols are defined in extension code (TypeScript objects).  They
are not retrieved from the server.  No server endpoint is required.

DICOM Grayscale Softcopy Presentation State (GSPS) is **not** currently supported
by OHIF.  If it were added, it would require:
```
GET /studies/{uid}/series?Modality=PR
GET /studies/{uid}/series/{uid}/instances/{uid}
```
Standard WADO-RS — no new pacsnode capability needed.

---

## 16. Performance & Concurrency

OHIF fires concurrent requests at these levels (from `default.js`):

```js
maxNumRequests: {
  interaction: 100,   // active user panning/scrolling
  thumbnail:   75,    // study browser thumbnail loading
  prefetch:    25     // background prefetch
}
```

pacsnode's Tokio async runtime should handle this concurrency well, but the
following areas warrant load testing:

| Area | Risk | Mitigation |
|------|------|-----------|
| WADO-RS metadata for large series (1000+ instances) | High | Stream response, avoid buffering full JSON in memory |
| Frame retrieval for WSI (1000s of tiles) | High | Use S3 presigned URL redirect; avoid server-side re-encoding |
| STOW-RS of large SEG objects (100 MB+) | Medium | Increase upload timeout; stream to S3 without full memory buffer |
| QIDO-RS with `includeField=all` | Medium | Ensure GIN indexes cover the queried tag paths |

---

## 17. Prioritised Implementation Roadmap

### Priority 1 — Required for basic OHIF functionality (already mostly done)

| Item | Status | Action |
|------|--------|--------|
| QIDO-RS studies/series/instances | ✅ | Verify all required metadata fields are returned |
| WADO-RS metadata (`application/dicom+json`) | ✅ | Verify PN format, BulkDataURI presence |
| WADO-RS full instance retrieval | ✅ | Verify multipart framing |
| STOW-RS `POST /studies` | ✅ | SR and SEG SOP classes verified |
| BulkDataURI emission | ✅ | Relative BulkDataURI values verified, including `X-Forwarded-Prefix` handling |
| Bearer token auth on all DICOMweb routes | ✅ | — |

### Priority 2 — High value, moderate effort

| Item | Effort | Description |
|------|--------|-------------|
| `/thumbnail` endpoint | Done | Implemented as an instance-level JPEG-first alias over the existing rendered-image pipeline |
| `/rendered` endpoint | Done | Already implemented; now also honors `?accept=` in addition to the `Accept` header |
| Multi-frame `/frames/{n1,n2,...}` | Done | Comma-separated frame lists are accepted by WADO-RS frame retrieval and rendered-frame routes |
| SOPClassUID filtering in QIDO-RS | Done | Instance-level QIDO query passes `SOPClassUID` through to the metadata store |
| `includeField` support | Done | Study and series QIDO responses now merge requested metadata tags from stored DICOM JSON |

### Priority 3 — Enables advanced OHIF features

| Item | Effort | Description |
|------|--------|-------------|
| Per-user measurement isolation | High | Multi-user auth with ownership scoping on stored SR/SEG instances |
| WSI tile performance | High | Benchmark frame endpoint under 100 concurrent requests; add S3 redirect for large objects |
| OIDC integration | High | Replace JWT-only auth with OIDC provider support (Keycloak, Auth0) |

### Priority 4 — Future / out of scope for now

| Item | Notes |
|------|-------|
| UPS-RS worklist | OHIF does not use it yet |
| GSPS (Presentation State) | OHIF does not parse it yet |
| Server-side hanging protocols | Embedded in OHIF extensions; no server API |
| DICOM Conformance Statement | Needed for clinical procurement |
| HIPAA audit trail | `pacs-audit-plugin` exists; extend for PHI access events |

---

## Appendix A — OHIF Configuration Snippet for pacsnode

```js
// /Users/macair/projects/dicom/Viewers/platform/app/public/config/pacsnode.js
window.config = {
  routerBasename: '/',
  showStudyList: true,

  dataSources: [{
    namespace: '@ohif/extension-default.dataSourcesModule.dicomweb',
    sourceName: 'pacsnode',
    configuration: {
      friendlyName: 'pacsnode',
      name: 'pacsnode',

      // Base URLs — adjust to your deployment
      qidoRoot: 'http://localhost:4000/wado',
      wadoRoot: 'http://localhost:4000/wado',

      // Feature flags (match to pacsnode capabilities)
      qidoSupportsIncludeField: true,
      imageRendering: 'wadors',
      thumbnailRendering: 'thumbnail',
      enableStudyLazyLoad: true,
      supportsFuzzyMatching: true,
      supportsWildcard: true,

      bulkDataURI: {
        enabled: true,
        relativeResolution: 'studies',
        // Uncomment if behind a reverse proxy at /pacs:
        // startsWith: '/wado',
        // prefixWith: '/pacs/wado',
      },

      singlepart: 'bulkdata',
    },
  }],

  defaultDataSourceName: 'pacsnode',

  // Concurrent request limits
  maxNumRequests: {
    interaction: 100,
    thumbnail: 75,
    prefetch: 25,
  },
};
```

---

## Appendix B — Quick Reference: Endpoint Status

```
QIDO-RS
  GET /wado/studies                              ✅
  GET /wado/studies/{uid}/series                 ✅
  GET /wado/studies/{uid}/series/{uid}/instances ✅

WADO-RS
  GET .../instances (metadata, dicom+json)        ✅
  GET .../instances/{uid} (full P10)              ✅
  GET .../instances/{uid}/metadata                ✅
  GET .../instances/{uid}/frames/{n}              ✅ comma-list supported
  GET .../instances/{uid}/bulkdata/{tag}          ✅ document MIME types; ✅ video MIME types
  GET .../instances/{uid}/thumbnail               ✅
  GET .../instances/{uid}/rendered                ✅

STOW-RS
  POST /wado/studies                             ✅

Auth
  Authorization: Bearer <token> on all routes    ✅ (via pacs-auth-plugin)
```
