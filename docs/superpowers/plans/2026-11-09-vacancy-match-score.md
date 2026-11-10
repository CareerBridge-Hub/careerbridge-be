# Skor Kecocokan Lowongan — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Setiap lowongan di `/api/v1/me/vacancies` membawa skor kecocokan dengan skill user (`Cocok 60% · 3/5 skill`, plus daftar skill yang kurang), dan halaman Lowongan bisa diurutkan "Paling cocok".

**Architecture:** Importer menandai skill yang disebut di teks lowongan (judul + deskripsi + kualifikasi) dengan pencocokan alias katalog berbasis token — logika murni di `domain`, hasilnya disimpan di tabel baru `vacancy_skills`. Endpoint lowongan menghitung skor di SQL dari `vacancy_skills` ∩ `user_skills`, sehingga satu sumber skor dipakai untuk urutan dan tampilan. FE menampilkan badge skor, skill kurang, dan pilihan urutan.

**Tech Stack:** Rust (Actix Web, sqlx, PostgreSQL + RLS), React + TypeScript + Vitest (repo `careerbridge-FE`).

**Spec:** Dokumen ini sendiri (bagian "Desain" di bawah). Tidak ada spec terpisah.

## Desain

Keputusan yang sudah diambil — implementer tidak perlu memilih ulang:

1. **Tanpa embedding untuk lowongan.** Saran awal ("tambah kolom `embedding` di `job_vacancies`") dibatalkan: kemiripan vektor seluruh teks lowongan tidak bisa menjelaskan *skill mana* yang kurang. Pencocokan alias bisa dijelaskan, deterministik, dan murah.
2. **Penanda skill = alias katalog yang muncul sebagai urutan token utuh** pada `normalize(title + description + requirements)`. `normalize` sama dengan jalur CV (`domain::normalize`), sehingga `c++`, `c#`, huruf besar/kecil, dan tanda baca berperilaku sama. Tidak ada pencocokan substring: `sql` tidak cocok dengan `mysqldump`.
3. **Penanda dihitung saat import** (di `persist`, transaksi yang sama dengan upsert) dan disimpan di `vacancy_skills`. Penanda lama diganti, tidak ditambah.
4. **Skor** = `round(100 × |skill lowongan ∩ skill user| / |skill lowongan|)`. Tanpa bobot wajib/opsional — teks lowongan tidak membedakannya. Lowongan tanpa skill terdeteksi → `matchScore = null` (bukan 0).
5. **Hanya skill katalog** (`user_skills`). `user_custom_skills` tidak dihitung.
6. **Backfill** lewat CLI `job-import --retag` (sekali setelah migrasi, dan setelah admin mengubah alias). Tanpa ini, penanda menyusul sendiri saat lowongan di-upsert ulang (≤ 6 jam sumber perusahaan, ≤ 7 hari Google Jobs).
7. **Urutan** `sort=match`: skor menurun, `null` paling akhir, lalu `posted_at DESC NULLS LAST, id`. Default (`sort` kosong atau `recent`) = urutan sekarang.

## Global Constraints

- Layering: logika murni di `src/domain/mod.rs` (tanpa DB/HTTP); SQL di `src/infrastructure`; route di `src/presentation`.
- `cargo fmt --check` dan `cargo clippy --locked --all-targets -- -D warnings` wajib bersih.
- Nama file migrasi: `migrations/202611090001_vacancy_skills.sql`. Migrasi yang sudah ada tidak diubah.
- Role DB: importer = `careerbridge_importer`, API = `careerbridge_api`. Setiap tabel baru: `ENABLE` + `FORCE ROW LEVEL SECURITY`, `REVOKE ALL ... FROM PUBLIC,anon,authenticated`, grant minimum.
- Integration test hanya ke DB disposable yang namanya mengandung `_test` (pola `tests/admin_integration.rs`), dengan `#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]`.
- **Migrasi ke DB test:** `cargo run --bin migrate` membaca `.env` (Supabase live). Selalu set `SUPABASE_DB_URL` ke DB test di environment saat menjalankan perintah; `dotenvy` tidak menimpa variabel yang sudah ada.
- Teks UI dalam Bahasa Indonesia. Nama field JSON camelCase.
- FE: `npm run typecheck`, `npm run lint`, `npm run test:integration` wajib lulus.

## Review Focus

1. **Alias pendek/ambigu** (`api` = "api" dalam Bahasa Indonesia, `cs`, `sem`, `lan`) bisa memberi false positive. Batas yang diterima; penangkalnya adalah pencocokan token utuh (tes Task 1: `sql` ≠ `mysqldump`, `react` ≠ `reaktif`). Admin menghapus alias yang bermasalah lalu menjalankan `--retag`.
2. **Lowongan tanpa skill terdeteksi** harus tampil tanpa skor (bukan "0%") dan berada di urutan terakhir saat `sort=match`. Tes: Task 3 (SQL), Task 4 (UI).
3. **User tanpa skill**: semua lowongan bertanda akan 0%. UI menonaktifkan "Paling cocok" dan menampilkan petunjuk. Tes: Task 4.
4. **Skill dihapus admin padahal dipakai `vacancy_skills`**: penghapusan tidak boleh gagal karena FK (`ON DELETE CASCADE`). Tes: Task 2.
5. **Retag mengganti, bukan menambah**: alias yang dihapus menghilangkan penanda lama. Tes: Task 2.

---

### Task 1: Pencocokan skill di teks (domain)

**Files:**
- Modify: `src/domain/mod.rs` (fungsi baru setelah `aliases`, sekitar baris 159; tes di modul `tests` yang ada)

**Interfaces:**
- Consumes: `normalize(&str) -> String`, `aliases(&[Skill]) -> HashMap<String, Uuid>` (sudah ada).
- Produces: `pub fn skills_in_text(text: &str, index: &HashMap<String, Uuid>) -> HashSet<Uuid>`

- [ ] **Step 1: Tulis tes yang gagal** di `mod tests` (`src/domain/mod.rs`):

```rust
#[test]
fn finds_catalog_skills_as_whole_token_sequences() {
    let (sql, excel, team, cpp, react) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
    let skill = |id, nama: &str, alias: &[&str]| Skill { id, nama: nama.into(), kategori: "T".into(), alias: alias.iter().map(|a| a.to_string()).collect() };
    let index = aliases(&[
        skill(sql, "SQL", &[]),
        skill(excel, "Microsoft Excel", &["excel"]),
        skill(team, "Kerja Tim", &["kerja sama tim"]),
        skill(cpp, "C++", &[]),
        skill(react, "React", &["react.js"]),
    ]);
    let found = skills_in_text("Menguasai SQL, EXCEL.\nMampu Kerja-Sama Tim; pengalaman C++/Java.", &index);
    assert_eq!(found, HashSet::from([sql, excel, team, cpp]));
    assert!(skills_in_text("pakai mysqldump, sikap reaktif", &index).is_empty());
    assert_eq!(skills_in_text("React.js developer", &index), HashSet::from([react]));
    assert!(skills_in_text("", &index).is_empty());
    assert!(skills_in_text("SQL", &HashMap::new()).is_empty());
}
```

- [ ] **Step 2: Jalankan, pastikan gagal**

Run: `cargo test --locked --lib finds_catalog_skills_as_whole_token_sequences`
Expected: FAIL kompilasi, `cannot find function skills_in_text`.

- [ ] **Step 3: Implementasi `skills_in_text`**

Pendekatan: `let tokens: Vec<&str> = normalized.split(' ')` (setelah `normalize`, abaikan string kosong); `max_n` = jumlah token alias terpanjang di `index` (dihitung sekali per panggilan); untuk tiap posisi `i` dan `n` di `1..=max_n` (selama `i+n <= tokens.len()`), cek `index.get(&tokens[i..i+n].join(" "))`. Kompleksitas O(token × max_n).

- [ ] **Step 4: Jalankan, pastikan lulus**

Run: `cargo test --locked --lib finds_catalog_skills_as_whole_token_sequences`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/domain/mod.rs
git commit -m "feat(domain): detect catalog skills in vacancy text by alias tokens"
```

---

### Task 2: Tabel `vacancy_skills`, penanda saat import, dan `--retag`

**Files:**
- Create: `migrations/202611090001_vacancy_skills.sql`
- Modify: `src/infrastructure/jobs.rs` (`persist` sekitar baris 855–909; fungsi baru setelah `persist`)
- Modify: `src/bin/job-import.rs` (arm `--retag` baru; perbarui doc comment baris 1)
- Modify: `docs/JOB_SEARCH_QUEUE.md` (subbagian singkat "Penanda skill lowongan": cara kerja, `--retag`, batas alias pendek)
- Create: `tests/vacancy_match_integration.rs`

**Interfaces:**
- Consumes: `domain::{aliases, skills_in_text}` (Task 1), `db::skills(&PgPool)`.
- Produces:
  - Tabel `vacancy_skills(vacancy_id uuid, skill_id uuid)`.
  - `async fn tag_vacancy(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, vacancy: Uuid, text: &str, index: &HashMap<String, Uuid>) -> Result<(), AppError>` (privat, di `jobs.rs`).
  - `pub async fn retag_all(pool: &PgPool) -> Result<u64, AppError>` — mengembalikan jumlah lowongan yang ditandai ulang.

- [ ] **Step 1: Tulis migrasi** `migrations/202611090001_vacancy_skills.sql`:

```sql
-- Catalog skills mentioned in a vacancy's text; written by the importer, read by the API.
CREATE TABLE vacancy_skills (
 vacancy_id uuid NOT NULL REFERENCES job_vacancies ON DELETE CASCADE,
 skill_id uuid NOT NULL REFERENCES skills ON DELETE CASCADE,
 PRIMARY KEY(vacancy_id,skill_id)
);
CREATE INDEX vacancy_skills_skill ON vacancy_skills(skill_id);
ALTER TABLE vacancy_skills ENABLE ROW LEVEL SECURITY;
ALTER TABLE vacancy_skills FORCE ROW LEVEL SECURITY;
REVOKE ALL ON vacancy_skills FROM PUBLIC,anon,authenticated;
GRANT SELECT,INSERT,DELETE ON vacancy_skills TO careerbridge_importer;
GRANT SELECT ON vacancy_skills TO careerbridge_api;
CREATE POLICY importer ON vacancy_skills TO careerbridge_importer USING(true) WITH CHECK(true);
-- Visibility follows job_vacancies RLS through the join in the vacancy page query.
CREATE POLICY api_read ON vacancy_skills FOR SELECT TO careerbridge_api USING(true);
-- The importer reads the alias index to tag vacancies.
GRANT SELECT ON skills,skill_aliases TO careerbridge_importer;
CREATE POLICY importer_read ON skills FOR SELECT TO careerbridge_importer USING(true);
CREATE POLICY importer_read ON skill_aliases FOR SELECT TO careerbridge_importer USING(true);
```

- [ ] **Step 2: Tulis integration test yang gagal** di `tests/vacancy_match_integration.rs`.

Helper di file ini (dipakai juga oleh Task 3):

- `struct Fixture { a: Uuid, b: Uuid, user: Uuid, v_both: Uuid, v_a: Uuid, v_none: Uuid, prefix: String }`
- `async fn fixture(owner: &PgPool) -> Fixture` — sisipkan dengan pool owner (pola `tests/admin_integration.rs:21-28,139`):
  - skill `a` dan `b` bernama `format!("Fixture A {}", Uuid::new_v4())` / `"Fixture B …"` (token nama tidak muncul di deskripsi), masing-masing dengan satu alias unik `format!("fixa{}", Uuid::new_v4().simple())` / `fixb…`;
  - satu user (`auth.users` + `users`) dengan `user_skills = {a}` (sumber `manual`);
  - tiga lowongan `source='grab'`, judul `format!("{prefix} …")` dengan `prefix` unik per fixture, dan `posted_at` berbeda supaya urutan default = `v_none` (terbaru), `v_both`, `v_a` (terlama):
    - `v_both`: description `"Wajib <alias a> dan <alias b>"`
    - `v_a`: description `"Menguasai <alias a>"`
    - `v_none`: description `"Tanpa skill katalog"`
- `async fn cleanup(owner: &PgPool, f: &Fixture)` — hapus berurutan: lowongan, `auth.users` (cascade ke `users` → `user_skills`), lalu skill `a`/`b` (`DELETE … WHERE id=ANY($1)`, aman bila sudah terhapus).
- `async fn tags(owner: &PgPool, vacancy: Uuid) -> HashSet<Uuid>` = `SELECT skill_id FROM vacancy_skills WHERE vacancy_id=$1`.

```rust
#[tokio::test]
#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]
async fn retag_replaces_vacancy_skills_and_survives_skill_deletion() {
    // owner pool + assert current_database() contains "_test"; importer = connect_runtime_pool(Importer)
    let f = fixture(&owner).await;
    assert!(jobs::retag_all(&importer).await.unwrap() >= 3);
    assert_eq!(tags(&owner, f.v_both).await, HashSet::from([f.a, f.b]));
    assert_eq!(tags(&owner, f.v_a).await, HashSet::from([f.a]));
    assert!(tags(&owner, f.v_none).await.is_empty());
    // Deleting a tagged skill (b is not in user_skills) is not blocked by vacancy_skills.
    sqlx::query("DELETE FROM skills WHERE id=$1").bind(f.b).execute(&owner).await.unwrap();
    assert_eq!(tags(&owner, f.v_both).await, HashSet::from([f.a]));
    // Alias removed → stale tag disappears on retag (replace, not append).
    sqlx::query("DELETE FROM skill_aliases WHERE skill_id=$1").bind(f.a).execute(&owner).await.unwrap();
    jobs::retag_all(&importer).await.unwrap();
    assert!(tags(&owner, f.v_both).await.is_empty());
    assert!(tags(&owner, f.v_a).await.is_empty());
    cleanup(&owner, &f).await;
}
```

- [ ] **Step 3: Jalankan, pastikan gagal**

Run (SUPABASE_DB_URL dan TEST_DATABASE_URL = DB test):
`cargo run --bin migrate && cargo test --locked --test vacancy_match_integration -- --ignored retag_replaces`
Expected: FAIL kompilasi, `cannot find function retag_all in module jobs`.

- [ ] **Step 4: Implementasi**

- `tag_vacancy`: `DELETE FROM vacancy_skills WHERE vacancy_id=$1`, lalu `INSERT INTO vacancy_skills(vacancy_id,skill_id) SELECT $1,unnest($2::uuid[])` dengan id hasil `skills_in_text(text, index)` (lewati INSERT bila kosong).
- `persist`: sebelum `pool.begin()`, `let index = aliases(&db::skills(pool).await?);`. Di loop, setelah upsert tiap `job`, panggil `tag_vacancy(&mut tx, id, &format!("{}\n{}\n{}", job.title, job.description, job.requirements), &index)`.
- `retag_all`: muat index yang sama; satu transaksi; `SELECT id,title,description,requirements FROM job_vacancies WHERE status='active'`; `tag_vacancy` per baris; commit; kembalikan jumlah baris. Tambahkan komentar `// ponytail: one transaction over all active rows; batch by id ranges if active vacancies exceed ~50k.`
- `job-import.rs`: arm `Some("--retag") if args.len() == 1` → `let pool = jobs::importer_pool().await?;` lalu cetak `{"retagged": n}`. Tambahkan `--retag` ke doc comment baris 1.
- Docs: subbagian di `docs/JOB_SEARCH_QUEUE.md` sesuai Files di atas.

- [ ] **Step 5: Jalankan, pastikan lulus**

Run: `cargo test --locked --test vacancy_match_integration -- --ignored retag_replaces`
Expected: PASS. Lalu `cargo clippy --locked --all-targets -- -D warnings` bersih.

- [ ] **Step 6: Commit**

```bash
git add migrations/202611090001_vacancy_skills.sql src/infrastructure/jobs.rs src/bin/job-import.rs docs/JOB_SEARCH_QUEUE.md tests/vacancy_match_integration.rs
git commit -m "feat(jobs): tag vacancies with catalog skills on import and add --retag"
```

---

### Task 3: Skor kecocokan dan `sort=match` di `/me/vacancies`

**Files:**
- Modify: `src/infrastructure/jobs.rs` (`VacancyItem`, `VacancyPage`, `VacancyQuery`, `validate`, `page`, sekitar baris 946–1060)
- Modify: `src/presentation/http.rs:895-906` (handler `vacancies` dan summary utoipa)
- Modify: `tests/admin_integration.rs:141-150` (literal `VacancyQuery` + panggilan `jobs::page`)
- Modify: `tests/vacancy_match_integration.rs`

**Interfaces:**
- Consumes: tabel `vacancy_skills` (Task 2), `browsing::by_ids(&PgPool, "skills", &[Uuid])`.
- Produces:
  - `VacancyQuery.sort: String` (`#[serde(default)]`), nilai valid: `""`, `"recent"`, `"match"`; selain itu `AppError::Invalid("Invalid vacancy page filters")`.
  - `pub async fn page(pool: &PgPool, have: &[Uuid], p: &VacancyQuery) -> Result<Value, AppError>`
  - Field baru per item: `matchScore: number|null`, `ownedSkillIds: uuid[]`, `missingSkillIds: uuid[]` (masing-masing urut menaik, `[]` bila kosong). Field baru di tingkat halaman: `skills` = label (`by_ids`) untuk semua id di kedua daftar pada halaman ini.

- [ ] **Step 1: Tulis integration test yang gagal** di `tests/vacancy_match_integration.rs`:

```rust
#[tokio::test]
#[ignore = "requires a disposable migrated TEST_DATABASE_URL"]
async fn vacancy_page_scores_and_sorts_by_user_skill_match() {
    // owner, api pool (RuntimeRole::Api), importer; f = fixture(&owner); jobs::retag_all(&importer)
    let mut q = jobs::VacancyQuery { offset: 0, limit: 10, q: f.prefix.clone(), city: String::new(), source: String::new(), occupation: String::new(), sort: "match".into() };
    let page = jobs::page(&api, &[f.a], &q).await.unwrap();
    let items = page["items"].as_array().unwrap();
    let ids: Vec<String> = items.iter().map(|i| i["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(ids, [f.v_a.to_string(), f.v_both.to_string(), f.v_none.to_string()]);
    assert_eq!(items[0]["matchScore"], 100);
    assert_eq!(items[1]["matchScore"], 50);
    assert_eq!(items[1]["ownedSkillIds"], json!([f.a]));
    assert_eq!(items[1]["missingSkillIds"], json!([f.b]));
    assert!(items[2]["matchScore"].is_null());
    assert_eq!(items[2]["ownedSkillIds"], json!([]));
    let labels: HashSet<String> = page["skills"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap().to_owned()).collect();
    assert_eq!(labels, HashSet::from([f.a.to_string(), f.b.to_string()]));
    // No user skills: tagged vacancies score 0, untagged stay null.
    let none = jobs::page(&api, &[], &q).await.unwrap();
    assert_eq!(none["items"][0]["matchScore"], 0);
    q.sort = "popular".into();
    assert!(matches!(jobs::page(&api, &[f.a], &q).await, Err(AppError::Invalid(_))));
    cleanup(&owner, &f).await;
}
```

Urutan default fixture (`v_none`, `v_both`, `v_a`) kebalikan dari urutan yang diharapkan di atas, jadi tes ini hanya lulus bila `sort=match` benar-benar mengubah urutan.

- [ ] **Step 2: Jalankan, pastikan gagal**

Run: `cargo test --locked --test vacancy_match_integration -- --ignored vacancy_page_scores`
Expected: FAIL kompilasi (`struct VacancyQuery has no field named sort` / jumlah argumen `page` salah).

- [ ] **Step 3: Implementasi**

- `VacancyQuery`: tambah `sort`; `validate` menolak nilai di luar daftar.
- `page`: bind `have` sebagai `$8::uuid[]` dan `sort` sebagai `$9`. Tambahkan ke query item:

```sql
LEFT JOIN LATERAL (
 SELECT round(100.0*count(*) FILTER (WHERE skill_id=ANY($8))/nullif(count(*),0))::int AS score,
  coalesce(array_agg(skill_id ORDER BY skill_id) FILTER (WHERE skill_id=ANY($8)),'{}') AS owned,
  coalesce(array_agg(skill_id ORDER BY skill_id) FILTER (WHERE NOT skill_id=ANY($8)),'{}') AS missing
 FROM vacancy_skills WHERE vacancy_id=v.id) m ON true
...
ORDER BY CASE WHEN $9='match' THEN m.score END DESC NULLS LAST, v.posted_at DESC NULLS LAST, v.id
```

  dan `'matchScore',m.score,'ownedSkillIds',m.owned,'missingSkillIds',m.missing` di `jsonb_build_object`. Query `count(*)` tidak berubah. Setelah commit transaksi, kumpulkan id dari kedua array semua item → `by_ids(pool,"skills",&ids)` → `"skills"` di JSON hasil. Tambahkan `// ponytail: lateral score per filtered row; precompute per-user scores if active vacancies exceed ~20k.`
- `VacancyItem`: tambah `match_score: Option<i64>`, `owned_skill_ids: Vec<Uuid>`, `missing_skill_ids: Vec<Uuid>` (sudah camelCase lewat `rename_all`). `VacancyPage`: tambah `skills: Vec<crate::domain::Skill>`.
- Handler `vacancies` (`http.rs`): ganti `db::active_tx(...).commit()` dengan membaca `SELECT skill_id FROM user_skills WHERE user_id=$1` di dalam `active_tx` itu (pola `browsing::matches`, `browsing.rs:296-302`), commit, lalu `jobs::page(&s.pool, &have, &p)`. Ubah summary utoipa menjadi "One page of published Indonesia vacancies with application links and the caller's skill match".
- `tests/admin_integration.rs`: tambah `sort: String::new()` ke literal dan ubah panggilan menjadi `jobs::page(&pool, &[], &query)`.

- [ ] **Step 4: Jalankan, pastikan lulus**

Run: `cargo test --locked --test vacancy_match_integration --test admin_integration -- --ignored`
Expected: PASS semua. Lalu `cargo test --locked` (tes non-ignored, termasuk `rejects_stale_or_malformed_vacancy_params` di `jobs.rs` — tambahkan `sort=popular` ke kasus yang ditolak dan `sort=match` ke kasus yang diterima di tes itu) dan `cargo clippy --locked --all-targets -- -D warnings`.

- [ ] **Step 5: Commit**

```bash
git add src/infrastructure/jobs.rs src/presentation/http.rs tests/admin_integration.rs tests/vacancy_match_integration.rs
git commit -m "feat(api): score vacancies against user skills and sort by match"
```

---

### Task 4: Tampilan skor dan urutan di halaman Lowongan (repo `careerbridge-FE`)

**Files:**
- Modify: `../careerbridge-FE/src/app/lib/api.ts:74` (tipe `Vacancy`)
- Modify: `../careerbridge-FE/src/app/pages/Lowongan.tsx`
- Create: `../careerbridge-FE/src/app/pages/Lowongan.spec.tsx`

**Interfaces:**
- Consumes: respons Task 3 (`matchScore`, `ownedSkillIds`, `missingSkillIds`, `skills` di halaman); `useCareer().user.skills` dari `@/app/lib/hooks`.
- Produces: parameter URL `sort` (`match` | kosong) yang diteruskan ke `fetchVacanciesPage`.

- [ ] **Step 1: Tulis tes yang gagal** `Lowongan.spec.tsx`, pola mock sama dengan `Pekerjaan.spec.tsx` (mock `useServerPage` dan `useCareer`). Data mock: dua lowongan — `A` (`matchScore:60`, `ownedSkillIds:['s1','s2','s3']`, `missingSkillIds:['s4','s5']`) dan `B` (`matchScore:null`, kedua daftar `[]`); `skills` berisi label `s4:'SQL'`, `s5:'Docker'`.

```tsx
it('shows the backend match score and missing skills, and no score when none detected',()=>{
  render(<BrowserRouter><Lowongan/></BrowserRouter>)
  expect(screen.getByText('Cocok 60%')).toBeTruthy()
  expect(screen.getByText('3/5 skill')).toBeTruthy()
  expect(screen.getByText('Skill kurang: SQL, Docker')).toBeTruthy()
  expect(screen.getAllByText(/Cocok \d+%/)).toHaveLength(1)
})
it('passes the URL sort to the server page request',()=>{
  history.replaceState(null,'','/lowongan?sort=match&offset=12')
  render(<BrowserRouter><Lowongan/></BrowserRouter>)
  expect(mocks.page.mock.calls.at(-1)?.[1]).toMatchObject({sort:'match',offset:12})
  expect(screen.getByRole('combobox',{name:'Urutkan lowongan'})).toBeTruthy()
})
it('disables match sorting and explains why when the user has no skills',()=>{
  // useCareer mock: user.skills = []
  render(<BrowserRouter><Lowongan/></BrowserRouter>)
  expect(screen.getByText('Tambahkan skill di halaman CV agar kecocokan lowongan bisa dihitung.')).toBeTruthy()
})
```

(Untuk tes ketiga, ubah `useCareer` mock per tes lewat `vi.hoisted` state, sama seperti `mocks.page`.)

- [ ] **Step 2: Jalankan, pastikan gagal**

Run (di `careerbridge-FE`): `npx vitest run src/app/pages/Lowongan.spec.tsx`
Expected: FAIL, `Unable to find an element with the text: Cocok 60%`.

- [ ] **Step 3: Implementasi**

- `api.ts`: tambah `matchScore:number|null; ownedSkillIds:string[]; missingSkillIds:string[]` ke `Vacancy`. `PageResult.skills` sudah ada.
- `Lowongan.tsx`:
  - `const sort=params.get('sort') ?? ''`, diteruskan ke `useServerPage(... {..., sort})`.
  - Select kelima, `aria-label="Urutkan lowongan"`, opsi `__recent` "Terbaru" dan `match` "Paling cocok"; ganti lewat `change('sort', ...)` yang sudah ada (sekaligus reset `offset`). Grid filter menjadi `lg:grid-cols-5`.
  - `const {user}=useCareer()`; bila `user.skills.length===0`: opsi "Paling cocok" `disabled`, dan tampilkan `<p>` petunjuk dengan teks persis di tes.
  - `VacancyCard` menerima `skillNames: Map<string,string>` (dari `result.data?.skills`). Bila `matchScore!==null`: `<Badge>Cocok {matchScore}%</Badge>` di kelompok badge, dan baris `{owned}/{owned+missing} skill`. Bila `missingSkillIds.length>0`: teks `Skill kurang: ` + maksimal 5 nama dipisah `, `, lalu ` +N lainnya` bila lebih. Id tanpa label dilewati.

- [ ] **Step 4: Jalankan, pastikan lulus**

Run: `npx vitest run src/app/pages/Lowongan.spec.tsx && npm run typecheck && npm run lint && npm run test:integration`
Expected: semua PASS.

- [ ] **Step 5: Commit (repo FE)**

```bash
git -C ../careerbridge-FE add src/app/lib/api.ts src/app/pages/Lowongan.tsx src/app/pages/Lowongan.spec.tsx
git -C ../careerbridge-FE commit -m "feat(lowongan): show skill match score, missing skills and match sorting"
```

---

## Setelah semua task

- Terapkan migrasi ke Supabase live dan jalankan `cargo run --release --bin job-import -- --retag` sekali. Ini perubahan ke DB produksi: lakukan setelah review dan dengan persetujuan pemilik.
- Cek manual: login sebagai user test yang punya skill → `/lowongan` → pilih "Paling cocok" → lowongan dengan skor tertinggi di atas, lowongan tanpa skor di akhir.

Di luar lingkup (tambahkan bila diminta): bobot wajib/opsional per lowongan, `user_custom_skills`, filter skor minimum, notifikasi lowongan cocok, tombol "Tutup gap ini" ke pelatihan (saran #2 berikutnya — memakai `missingSkillIds` dari plan ini).
