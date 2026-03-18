Plan: User Management & Authentication System
Replace the single-user JWT auth plugin with a full user management system that scales from a standalone research box to an enterprise hospital deployment. Two auth modes ship in one binary: local auth (multi-user, ABAC-capable RBAC, admin-managed via UI) and OIDC auth (Keycloak/external IdP). Phase 1 delivers multi-user local auth; Phase 2 adds OIDC federation. OHIF supports all three landing UX patterns depending on auth mode.

Status legend: complete = implemented, partial = implemented in part or with meaningful deviations from this plan, pending = not implemented yet.

Architecture

Phase 1 — Multi-User Local Auth + RBAC
Phase 1A: Core User Store & Token Service
Step 1: Database schema (no deps, parallel with Step 2) [complete]
* New migration migrations/0006_users_and_roles.sql — users table (id UUID, username, display_name, email, password_hash, role, attributes JSONB, is_active, failed_login_attempts, locked_until, password_changed_at, timestamps), refresh_tokens table (id, user_id FK, token_hash UNIQUE, expires_at, revoked_at), password_policy singleton table (min_length, require_uppercase, require_digit, require_special, max_failed_attempts, lockout_duration, max_age_days)
* Equivalent SQLite migration in migrations
* Add query modules: queries user.rs + refresh_token.rs
Step 2: Domain types in pacs-core (no deps, parallel with Step 1) [complete]
* New crates/pacs-core/src/domain/user.rs — User, UserId newtype, UserRole enum (Admin, Radiologist, Technologist, Viewer, Uploader), UserQuery
* New auth domain types — PasswordPolicy, RefreshToken, TokenPair, AuthMode enum
* Extend MetadataStore trait in mod.rs with: store_user, get_user, get_user_by_username, query_users, delete_user, store_refresh_token, revoke_refresh_tokens, get_refresh_token, get_password_policy, upsert_password_policy
Step 3: Implement user queries (depends on 1, 2) [complete]
* PostgreSQL queries in queries using sqlx::query! macros
* SQLite queries in store.rs
Step 4: Token service (depends on 2) [partial]
* New crates/pacs-auth-plugin/src/token.rs — TokenService with issue_pair(), refresh() (with rotation + reuse detection), validate_access(), revoke_all()
* Access: short-lived JWT (15 min default), HS256, claims include sub, role, attrs
* Refresh: opaque 256-bit random, SHA-256 hashed in DB, 7-day default, single-use rotation. Reuse of old token → revoke ALL user tokens (compromise detection per RFC 6749 §10.4)
Current status: token issuance, validation, refresh rotation, and revocation exist in pacs-auth-plugin, but not as a dedicated token.rs service and without explicit refresh-token reuse-compromise detection.
Step 5: Password service (depends on 2, parallel with Step 4) [partial]
* New crates/pacs-auth-plugin/src/password.rs — Argon2id hashing (m=65536, t=3, p=4), policy validation, account lockout logic (increment failed attempts, lock after threshold, reset on success)
Current status: Argon2 hashing, policy validation, and lockout behavior are implemented, but not as a dedicated password.rs service module.
Step 6: CLI bootstrapping (depends on 3, 5) [partial]
* Add pacsnode create-admin --username admin subcommand to main.rs
* Reads password from --password-stdin or interactive prompt
* Validates against policy, hashes, inserts into users with Admin role
* Refuses if admin exists unless --force
* Generates random jwt_secret if not already in config
* On first startup with no users: log prominent warning
Current status: create-admin exists and bootstraps an admin with a generated compliant password when no users exist, but the planned password-input flags/force flow/jwt-secret generation are not all present.
Phase 1B: Auth Plugin Rewrite
Step 7: Rewrite pacs-auth-plugin (depends on 4, 5, 6) [partial]
* Config changes to [plugins.auth]: mode = "local", jwt_secret, access_token_ttl_secs = 900, refresh_token_ttl_secs = 604800, login_path, refresh_path, logout_path, public_paths
* New endpoints: POST /auth/login → TokenPair + httpOnly cookie, POST /auth/refresh → rotation, POST /auth/logout → revoke all, GET /auth/me → user profile
* Middleware: dual extraction (httpOnly cookie first, then Bearer header), check is_active, CSRF protection via double-submit cookie on mutation requests when auth is cookie-based
* Extend AuthenticatedUser in auth.rs with role: String, attributes: serde_json::Value
* Extract middleware into crates/pacs-auth-plugin/src/middleware.rs
* Plugin ID: auth (keep basic-auth as backwards-compat alias)
Current status: local auth mode, the auth endpoints, role/attribute-aware authenticated users, and backward-compatible plugin ID behavior are implemented. Cookie-first extraction, CSRF protection, and the middleware.rs split are not implemented.
Step 8: Login page (depends on 7) [pending]
* GET /auth/login serves Askama-rendered HTML login form (matches admin UI styling)
* Vanilla HTML form + fetch POST → on success: httpOnly cookie set, redirect to ?redirect= or /
* In public_paths by default
Phase 1C: RBAC / ABAC Engine
Step 9: Policy engine (parallel with Step 7) [complete]
* New crates/pacs-core/src/policy.rs — PolicyEngine struct
* Built-in roles with default permissions: admin (all), radiologist (read+query+measure+report), technologist (read+query+upload), viewer (read+query), uploader (upload only)
* Attribute-based filters from user attributes JSONB: department, modality_access (array), study_access ("all"/"department"/"assigned")
* check_permission(user, action, resource) -> bool
* apply_query_filters(user, &mut StudyQuery) — injects modality/department filters before store query
* Applied in: qido.rs, wado.rs, stow.rs, studies.rs
Step 10: Admin UI — User Management (depends on 7, 9) [complete]
* Add to web.rs: user list, create/edit/deactivate, password policy editor
* New Askama templates: users.html, user_form.html, password_policy.html
* All protected by auth middleware + admin role check
Phase 1D: OHIF Viewer Integration
Step 11: OHIF auth wiring (depends on 7, 8) [pending]
* mode: local → unauthenticated /viewer/ redirects to /auth/login?redirect=/viewer/ → login sets httpOnly cookie → browser sends cookie on all DICOMweb fetches automatically (no JS token handling needed)
* mode: none → no auth config in app-config.js
* mode: oidc → wire config shape now, implement in Phase 2
* Modify generated_app_config() in lib.rs based on active auth mode

Phase 2 — OIDC Federation
Step 12: OIDC token validation [partial] — New crates/pacs-auth-plugin/src/oidc.rs with JWKS cache, RS256 validation, claim-to-role mapping, JIT user provisioning
Current status: OIDC bearer validation is implemented in pacs-auth-plugin with static-key, JWKS, and issuer-discovery support plus claim mapping, but not as a separate oidc.rs module and without JIT user provisioning.
Step 13: OIDC login flow [pending] — GET /auth/login → IdP redirect, GET /auth/callback → code exchange → session cookie
Step 14: Keycloak Docker Compose [pending] — docker/docker-compose.oidc.yml with pre-configured realm, two clients (pacsnode confidential + ohif-viewer public SPA), default roles, LDAP federation template
Step 15: Hospital integration docs [pending] — LDAP/AD via Keycloak federation, IHE XUA (SAML/OIDC), IHE ATNA (existing audit logger), HL7v2 ADT context (future)

Verification
1. Unit tests in each new module: password hash/verify/policy, token issue/validate/rotate/reuse-detection, policy permission checks + attribute filtering, middleware public-path/expired/deactivated/cookie-vs-bearer
2. Integration tests (Pg testcontainers): user CRUD, login flow + lockout, refresh rotation + revocation, RBAC enforcement (viewer can't delete, admin can, radiologist filtered by modality)
3. OHIF manual: mode: local → create admin → login → viewer loads → DICOMweb requests carry cookie → viewer-role user blocked from admin
4. CLI: pacsnode create-admin fresh → created, re-run → refused, empty password → policy violation
5. CI: existing pipeline + new tests pass

Decisions
* DIMSE stays AE-whitelist only — DIMSE callers are machines, not humans
* ABAC over pure RBAC — attributes JSONB allows adding new policy dimensions (department, modality) without migrations
* Refresh token rotation with reuse detection — OWASP best practice, detects token theft
* Cookie + Bearer dual extraction — admin UI (SSR) uses httpOnly cookie; API clients use Bearer header
* JIT provisioning for OIDC — users auto-created on first login with role from IdP claims, local attribute overrides possible
* Keycloak as documented IdP — open-source, self-hosted (critical for air-gapped hospitals), native LDAP, healthcare community. Any OIDC-compliant IdP works via standard protocol.
* Password policy in DB — runtime-editable via admin UI without restart
Further Considerations
1. CSRF for cookie auth — double-submit cookie pattern needed on POST/PUT/DELETE when auth is via cookie. Include in Step 7.
2. httpOnly cookie for OHIF — browser sends cookie automatically on same-origin DICOMweb requests, no JS token exposure. Needs validation that OHIF's fetch calls don't strip cookies. If they do, fall back to Authorization header with token from sessionStorage.
3. Audit enrichment — add user_role TEXT and auth_method TEXT to audit_log in the same migration as users (Step 1).
