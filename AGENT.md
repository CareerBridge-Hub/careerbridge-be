# CareerBridge engineering checklist

Updated: 2026-11-01. Checkmarks describe implemented work; live-service and deployment acceptance are tracked separately below.

## Initial scope

- [x] Implement a non-chatbot Rust API with Gemini through an OpenAI-compatible provider adapter, configurable for a future DeepSeek migration. DeepSeek compatibility has not been tested live.
- [x] Use Supabase PostgreSQL with Rust/sqlx migrations, pgvector, RLS, and restricted API/worker roles.
- [x] Implement Rust unit tests, PDF subprocess tests, and backend journey tests with disposable PostgreSQL and provider fixtures.
- [x] Connect the frontend to the backend API and Supabase authentication. Fixture-based browser evidence is in `docs/VALIDATION.md`.
- [x] Add frontend logic and API/authentication/polling/component tests. Evidence is recorded in `docs/VALIDATION.md`; this update does not rerun frontend checks.
- [x] Implement private temporary CV storage through Supabase Storage and document bucket setup, encryption, cleanup, and credentials in `docs/SETUP.md`.

## Backend and data

- [x] Generate OpenAPI in Rust with `utoipa`; serve interactive Swagger UI at `/swagger-ui/` and export the contract with the `export-openapi` binary.
- [x] Run local E5 embedding inference in Rust, with bounded queues, versioned catalog vectors, and worker refresh of missing/stale embeddings.
- [x] Provision the approved model snapshot in Docker with HTTPS curl downloads, size/SHA-256 checks, and a network-disabled inference build check.
- [x] Apply the catalog migrations to the configured Supabase database and import 376 LPK/BLK institutions, 2,083 program/training entries, and 4,330 separate historical realization records.
- [x] Preserve unknown fees/modes, provenance, stable import IDs, existing samples, and foreign-key integrity. Verify committed import counts and API SELECT grants.
- [x] Keep one-time Python import/fixture tools local under Git- and Docker-ignored `scripts/`; remove their CI dependency.

- [x] Configure `.env`/`.env.example` with one `SUPABASE_DB_URL`, session pooling, bundled public CA/verified TLS, generated stable encryption key, and explicit missing provider fields.
- [x] Apply the sixth migration granting runtime-role membership and verify API/worker role selection plus API RLS using a read-only live connection test. Document the privileged-login credential tradeoff.

- [x] Provision the private 2 MiB PDF-only Storage bucket and two ordinary plus one admin synthetic test accounts through local Python automation. Verify password login, ES256 sessions, application roles, RLS isolation, private Storage roundtrip/deletion, and rerun preservation of IDs/passwords.
- [x] Correct Storage requests for modern secret-key headers and bucket-level deletion with a prefixes body; add regression coverage. Admin management remains disabled.

- [x] Replace frontend Supabase authentication with backend-managed opaque HttpOnly sessions, encrypted server-side tokens, session RLS, serialized refresh, logout revocation, CSRF/Origin checks and bounded login throttling. Apply the session migration and verify two live synthetic users, encrypted storage, concurrent refresh and revoked-cookie replay denial.
- [x] Remove frontend Supabase SDK/env keys; update cookie transport, signup OTP UI, password minimum, Swagger and operating instructions. Rust tests/Clippy and frontend build/integration checks pass. Trivy HIGH/CRITICAL scanning returned no findings; Cargo audit reported no known vulnerabilities and the two existing unmaintained warnings.

- [x] Add backend `AUTH_SIGNUP_OTP_ENABLED` (secure default true), set false in local/example env for temporary testing, and skip OTP through create-only Auth Admin registration followed by password login. Reject duplicates without modifying existing accounts, disable OTP verification while false, and cover both modes plus frontend behavior with provider-fixture tests.

- [x] Remove full training/institution pagination from the login wait, skip redundant provider-session lookup for existing profiles, reuse occupations in match responses, and load the large catalog in bounded background batches with cache/retry/loading guards. Verify login completes with a deliberately stalled training response and same-user refresh does not re-download catalogs.

- [x] Replace background full-catalog downloads with feature-scoped server pagination: login/profile only, dashboard summary, visible job/training pages, direct item/ID lookups, and lazy skill/job search. Rank/filter/page IDs in SQL before hydration, bound analysis output, and apply supporting indexes.
- [x] Add query cancellation, debounced URL search, offset reset, loading/error/retry states, nullable fee/mode display and bounded metadata caching. Verify live paging/filter/ranking and concurrent cookie refresh. The earlier full background catalog loader is superseded; live read-only browser pagination acceptance passed with login navigation in 1.922 s (one sample). See `docs/PAGINATION.md`.

- [x] Infer one occupation per processed CV with the LLM (closed-set catalog indices, evidence quotes verified against the CV, confidence gate, one validation retry, failure never fails the CV). Catalog matches are reused; new roles are saved on CV confirm as the user's own private occupation (`occupations.owner_id`, RLS-scoped, no KBJI, no job-search quota) that gets scoring, training recommendations, analysis, and vacancies via its closest catalog occupation or required-skill tags. Unit tests cover validation/prompt payload/retries; `tests/user_occupation_integration.rs` covers RLS isolation and vacancy mapping but has not been run (no local database).

- [x] Make CV skills field-agnostic: CV terms the catalog lacks go to the LLM (with nearest catalog skills as candidates), which maps them onto existing skills or names a standard skill; new names are stored as `skills.sumber='ai'` through the validated `add_ai_skill` SECURITY DEFINER function (worker has no direct catalog INSERT), with the CV wording as alias. Occupation inference may name required skills the catalog lacks (`wajibBaru`/`opsionalBaru`). Seed 59 catalog occupations across health, education, retail, hospitality, logistics, construction, manufacturing, finance, HR, legal, creative and agriculture with requirements, aliases and search keywords/title patterns.
- [x] Import vacancies of every field: employer boards classify titles with the search-queue title patterns instead of the software-only regex; all non-metered sources run every 2 hours (`JOB_SCRAPE_INTERVAL_SECONDS=7200`, Extract 12-hour floor removed); OpenWebNinja stays daily. `kitalulus` added to defaults (live preview 2026-11-14 returned 30 "perawat" listings; `karir` reported a schema change, `indeed`/`glassdoor` were blocked, so they stay off).

## Security and CI

- [x] Validate JWT signatures/issuer/audience/expiry; enforce per-user RLS and server-validated CV skill provenance.
- [x] Bound PDF extraction, upload size, embedding queues, provider responses/timeouts, and retries; encrypt stored CV extraction results.
- [x] Run Rust and tooling advisory reviews; retain the two unmaintained transitive dependency warnings without suppressing them. See `docs/SECURITY.md`.
- [x] Configure CI formatting, Clippy, tests, disposable database migrations/journeys, OpenAPI consistency, dependency audits, Docker checks, and Swagger UI integrity/advisory checks.
- [x] Add pinned Semgrep Community Edition source checks with safe/unsafe rule fixtures; verify six rules locally against 24 Rust source files with zero findings.
- [x] Configure opt-in manual CodeQL Rust security-extended analysis for public repositories; skip it on private repositories so GitHub Free does not require paid Code Security.
- [x] Default CI to Semgrep, checksum-pinned Trivy 0.76.1 HIGH/CRITICAL repository security, and Swagger integrity/advisory scans; move Rust/database verification and optional Docker builds to manual workflows. Cancel superseded runs, bound job times, and retain Cargo registry caching for manual verification.
- [x] Add a startup runbook, 53 manual cases covering current features, known-gap expectations, and a deferred Playwright/agent automation plan.

- [x] Set `LLM_MODEL=gemini-2.5-flash` in `.env` and `.env.example` after checking Google's official free-tier pricing; live provider acceptance remains pending.

## Pending acceptance and follow-up

- [ ] When re-enabling signup OTP, configure Supabase signup email template to show `{{ .Token }}` and verify live OTP delivery/registration. Test live password change/session revocation and full browser cookie/CV journey. See `docs/COOKIE_AUTH.md`.

- [ ] Execute and record the new manual feature cases; the checklist is a test plan, not completed acceptance.
- [ ] After manual smoke acceptance, install/configure Playwright in the frontend and implement the agreed cases with an agent-assisted workflow.

- [ ] Run the updated GitHub workflows on GitHub and record actual duration/quota usage. Workflow configuration and local scans are verified; hosted jobs have not been executed from this workspace.
- [ ] Build and scan the updated Docker image on a Docker-capable host, including OS packages/native ONNX libraries, and measure readiness/RSS under the intended 2 vCPU / 2 GiB container limits.
- [ ] Verify the full live browser/CV/Gemini journey using a separate test project. Live Auth account provisioning/password login and Storage API smoke checks passed; they do not establish end-to-end LLM/browser acceptance.
- [ ] Run catalog embedding backfill for the newly imported training data against the intended backend configuration and verify persisted model versions/counts.
- [ ] Review explicit training-to-skill mappings. The import found zero exact matches against the existing skill catalog; semantic embeddings alone do not establish confirmed skill coverage.
- [x] Update frontend display/filter handling for nullable imported fees and delivery modes with server-side filters; run frontend tests/build.
- [ ] Apply migration `202611110001_user_occupations.sql`, run `user_occupation_integration` against a disposable database, and evaluate live DeepSeek occupation inference on real CVs.
- [ ] Apply migration `202611140001_ai_skills_all_fields.sql` to Supabase, run `job-import --retag`, then upload real non-IT CVs (e.g. nurse, cashier, welder) and review the `sumber='ai'` skills it creates. Local evidence (2026-11-14, disposable PostgreSQL 18 + pgvector in WSL): all migrations apply on a fresh database (240 skills, 70 catalog occupations, every occupation with a required skill, all 70 search queries with title patterns); `add_ai_skill` as `careerbridge_worker` creates, deduplicates by name and refuses aliases on admin skills; CI database suites pass except `backend_http_journey`, whose PDF subprocess cannot start on Windows (`env_clear`); a 3-source import classified 667/667 vacancies across 12 fields; live Gemini mapped synthetic nurse/welder terms onto existing or new skills, rejected adjectives/licences and inferred the Perawat occupation.
- [ ] Evaluate real LLM CV extraction and broader labeled retrieval data; current recorded evaluation uses synthetic fixtures and reports its limitations.
- [ ] Replace unmaintained `paste` and `ttf-parser` through reviewed upstream migrations when feasible; advisory scans do not prove absence of vulnerabilities or supply-chain attacks.

## Engineering rules

- Preserve Rust layering and use reviewed, maintainable implementations.
- Enforce security controls and least-privilege credentials; never expose secrets in logs or browser variables.
- Review dependency provenance, advisories, lockfiles, native runtime/model artifacts, and license changes.
- Keep implementation completion separate from verified production acceptance; record limitations and test evidence.
