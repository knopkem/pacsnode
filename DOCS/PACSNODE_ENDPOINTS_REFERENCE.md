# pacsnode API Endpoints Quick Reference

## DICOMweb Endpoints

### QIDO-RS (Query)
```
GET /wado/studies
  ?PatientID=...
  &PatientName=...
  &StudyDate=YYYYMMDD
  &StudyDateRange=YYYYMMDD-YYYYMMDD
  &Modality=...
  &Accession=...
  &limit=50&offset=0

GET /wado/studies/{study_uid}/series
  ?SeriesUID=...&Modality=...&SeriesNumber=...&limit=50&offset=0

GET /wado/studies/{study_uid}/series/{series_uid}/instances
  ?SOPInstanceUID=...&InstanceNumber=...&limit=50&offset=0
```

### STOW-RS (Store)
```
POST /wado/studies
Content-Type: multipart/related; type=application/dicom; boundary=...

[DICOM part 1]
--boundary
Content-Type: application/dicom
...DICOM bytes...
--boundary--

Response: application/dicom+json (PS3.18 format with stored UIDs)
```

### WADO-RS (Retrieve DICOM)
```
GET /wado/studies/{study_uid}
GET /wado/studies/{study_uid}/series/{series_uid}
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_list}
  ?Accept=multipart/related;type=application/dicom
  ?Accept=multipart/related;type=application/dicom;transfer-syntax=1.2.840.10008.1.2.1

Response: multipart/related; type=application/dicom (all matching instances)
```

### WADO-RS (Retrieve Rendered)
```
GET /wado/studies/{study_uid}/rendered
GET /wado/studies/{study_uid}/series/{series_uid}/rendered
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/rendered
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/frames/{frame_list}/rendered
  ?Accept=image/png or image/jpeg
  ?windowCenter=40&windowWidth=400
  ?rows=512&columns=512
  ?region=x,y,width,height

Response: multipart/related; type=image/png or image/jpeg
```

### WADO-RS (Metadata)
```
GET /wado/studies/{study_uid}/metadata
GET /wado/studies/{study_uid}/series/{series_uid}/metadata
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/metadata

Response: application/dicom+json (full DICOM tag set)
```

### WADO-RS (Bulk Data)
```
GET /wado/studies/{study_uid}/series/{series_uid}/instances/{instance_uid}/bulkdata/{tag_path}

Example:
  /bulkdata/00281010/0  → first overlay
  /bulkdata/7FE0/0010   → pixel data frame

Response: multipart/related with binary chunks
```

### WADO-URI (Legacy)
```
GET /wado
  ?requestType=WADO
  &studyUID=1.2.3.4
  &seriesUID=1.2.3.4.1
  &objectUID=1.2.3.4.1.1
  &contentType=application/dicom  (or image/png, image/jpeg)
  &transferSyntax=1.2.840.10008.1.2.1  (DICOM only)
  &frameNumber=1  (multi-frame only)
  &windowCenter=40&windowWidth=400  (rendered only)

Response: application/dicom or image/png or image/jpeg
```

---

## REST API Endpoints

### Studies
```
GET    /api/studies
GET    /api/studies/{study_uid}
DELETE /api/studies/{study_uid}
```

### Series
```
GET    /api/studies/{study_uid}/series
GET    /api/series/{series_uid}
DELETE /api/series/{series_uid}
```

### Instances
```
GET    /api/series/{series_uid}/instances
GET    /api/instances/{instance_uid}
DELETE /api/instances/{instance_uid}
```

### Nodes (DICOM Remote Systems)
```
GET    /api/nodes
POST   /api/nodes          (body: {"ae_title": "REMOTE", "hostname": "...", "port": 104})
DELETE /api/nodes/{ae_title}
```

### Audit Logs
```
GET    /api/audit/logs
GET    /api/audit/logs/{id}
```

### System
```
GET /health
GET /statistics
GET /system
GET /metrics              (if pacs-metrics-plugin enabled)
```

---

## Authentication Endpoints

```
POST /auth/login          (body: {"username": "admin", "password": "..."})
  Response: {"access_token": "jwt...", "token_type": "Bearer", "expires_in": 3600}

POST /auth/refresh        (body: {"refresh_token": "..."})
  Response: {"access_token": "jwt...", "expires_in": 3600}

Authorization: Bearer <jwt>  (add to all protected endpoints)
```

---

## Admin Dashboard

```
GET /admin                (admin dashboard HTML)
  (if pacs-admin-plugin enabled)
```

---

## OHIF Viewer

```
GET /viewer               (SPA root)
GET /viewer/              (with trailing slash)
GET /viewer/app-config.js (auto-generated OHIF config)
  (if pacs-viewer-plugin enabled)
```

---

## Middleware & Behavior

### Request Limits
- **Timeout**: 30 seconds
- **Body size**: 500 MiB max
- **Compression**: gzip/deflate (automatic)

### Response Codes
- **200 OK**: Success
- **201 Created**: Resource created (STOW-RS)
- **204 No Content**: Success, no body
- **400 Bad Request**: Invalid parameters/content
- **401 Unauthorized**: Authentication required (if auth enabled)
- **403 Forbidden**: Access denied (if RBAC enabled)
- **404 Not Found**: Resource doesn't exist
- **408 Request Timeout**: Operation took >30s
- **413 Payload Too Large**: Body > 500 MiB
- **415 Unsupported Media Type**: Wrong Content-Type
- **500 Internal Server Error**: Server error

### CORS
- **Current**: Permissive (unsafe for production)
- **Production**: Configure allowed origins in auth plugin

---

## Content Types

### DICOMweb Standard
```
application/dicom                      DICOM Part 10 file
application/dicom+json                 DICOM JSON (PS3.18)
multipart/related; type=application/dicom
multipart/related; type=image/png
multipart/related; type=image/jpeg
```

### Transfer Syntax in Accept Header
```
Accept: multipart/related;type=application/dicom;transfer-syntax=1.2.840.10008.1.2.1
```

Supported transfer syntaxes:
- `1.2.840.10008.1.2` — Implicit VR LE
- `1.2.840.10008.1.2.1` — Explicit VR LE (default)
- `1.2.840.10008.1.2.2` — Explicit VR BE
- `1.2.840.10008.1.2.1.99` — Deflated Explicit VR LE
- `1.2.840.10008.1.2.4.50` — JPEG Baseline
- `1.2.840.10008.1.2.4.57` — JPEG Lossless (Process 14)
- `1.2.840.10008.1.2.4.70` — JPEG Lossless (Process 14, SV1)
- `1.2.840.10008.1.2.4.90` — JPEG 2000 Lossless
- `1.2.840.10008.1.2.4.91` — JPEG 2000 Lossy
- `1.2.840.10008.1.2.5` — RLE Lossless

---

## Environment Variables

### Database
```
PACS_DATABASE__URL=postgresql://user:pass@host:5432/pacs
PACS_DATABASE__MAX_CONNECTIONS=20
```

### DIMSE
```
PACS_DIMSE__PORT=4242
PACS_DIMSE__AE_TITLE=PACSNODE
PACS_DIMSE__MAX_ASSOCIATIONS=64
PACS_DIMSE__DIMSE_TIMEOUT_SECS=30
PACS_DIMSE__ACCEPT_ALL_TRANSFER_SYNTAXES=true
```

### Storage (S3)
```
PACS_STORAGE__ENDPOINT=http://localhost:9000
PACS_STORAGE__BUCKET=dicom
PACS_STORAGE__ACCESS_KEY=minioadmin
PACS_STORAGE__SECRET_KEY=minioadmin
```

### HTTP
```
PACS_HTTP__PORT=8042
PACS_HTTP__BIND_ADDRESS=0.0.0.0
```

### Plugins
```
PACS_PLUGINS__ENABLED=pacs-audit-plugin,pacs-auth-plugin,pacs-metrics-plugin,pacs-viewer-plugin
PACS_PLUGINS__PACS_AUTH_PLUGIN__USERNAME=admin
PACS_PLUGINS__PACS_AUTH_PLUGIN__PASSWORD_HASH=...
PACS_PLUGINS__PACS_AUTH_PLUGIN__JWT_SECRET=secret
PACS_PLUGINS__PACS_VIEWER_PLUGIN__STATIC_DIR=./web/viewer
```

### Logging
```
RUST_LOG=pacs_api=debug,pacs_dimse=info
```

---

## Error Response Format

```json
{
  "error": "description of what went wrong"
}
```

---

## Notes

- **NOT for clinical use** — not validated for diagnostic/therapeutic purposes
- **No TLS support** — use reverse proxy (Nginx/Caddy) for HTTPS
- **No RBAC** — single hardcoded user if auth enabled
- **Server-side transcode** — automatic on WADO-RS/C-MOVE/C-GET retrieve
- **Frame numbers** — 1-indexed in WADO-URI, 0-indexed in WADO-RS
- **Fuzzy matching** — ILIKE-based prefix/suffix, not full-text search
- **Audit events** — auto-emitted for store/delete/query (if audit plugin enabled)

