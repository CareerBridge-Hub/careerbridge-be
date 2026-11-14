# Antrian Pencarian Lowongan (Google Jobs via OpenWeb Ninja)

Worker harian yang mengisi halaman **Lowongan** dengan lowongan di Indonesia untuk **setiap kategori pekerjaan** di katalog (tabel `occupations`). Sumbernya Google for Jobs lewat API OpenWeb Ninja (`source = openwebninja`).

Sumber lain (situs lowongan seperti LinkedIn, JobStreet, Glints, Kalibrr, KitaLulus, serta feed perusahaan) berjalan tiap 2 jam (`JOB_SCRAPE_INTERVAL_SECONDS=7200`) untuk semua bidang pekerjaan dan tidak memakai kuota API. Feed perusahaan tidak lagi dibatasi lowongan IT: judul diklasifikasikan ke occupation katalog lewat `title_pattern` antrian pencarian; judul yang tidak cocok dengan occupation mana pun dilewati.

---

## 1. Konsep

Konsepnya **antrian bergilir (round-robin)**. Tiap kategori pekerjaan punya keyword pencarian. Setiap hari worker mengambil beberapa keyword yang paling lama tidak dijalankan, memanggil API, lalu menyimpan hasilnya.

```
occupations (11 kategori + yang ditambah admin)
      │  1 kategori = ≥1 keyword
      ▼
job_search_queries  ── antrian, urut last_run_at (yang belum pernah jalan duluan)
      │  tiap 24 jam ambil N baris (OPENWEBNINJA_QUERIES_PER_RUN, default 2)
      ▼
OpenWeb Ninja  GET /google-jobs/search?query=<keyword>&country=id&language=id
      │  1 keyword = 1 request (kena kuota)
      ▼
Filter: lokasi harus Indonesia  →  judul cocok title_pattern  →  link https publik
      ▼
job_vacancies (occupation_id = kategori dari keyword)  →  API /me/vacancies  →  halaman Lowongan
```

Ciri penting:

- **Bergilir, bukan sekaligus.** Kuota terbatas, jadi tiap hari hanya N keyword. Dengan 11 kategori dan N=2, semua kategori kebagian dalam 6 hari.
- **Tidak menutup lowongan.** Hasil pencarian hanyalah sampel, bukan daftar lengkap. Lowongan yang tidak muncul lagi tidak ditandai `closed`, tapi otomatis disembunyikan RLS kalau tidak terlihat lagi dalam **7 hari**.
- **Hemat kuota saat gagal.** Kalau API error, worker menunggu 24 jam (bukan 15 menit), jadi gangguan di API tidak menghabiskan kuota. Posisi antrian tidak maju, sehingga keyword yang gagal dicoba lagi besok.
- **Kategori baru otomatis masuk antrian.** Trigger `queue_search` pada `occupations` menambah keyword default `"<nama kecil> jobs in Jakarta"` untuk setiap occupation baru.
- **Tanpa duplikat.** Lowongan diidentifikasi dengan docid Google (`job_uid`), jadi lowongan yang ditemukan dua keyword hanya tersimpan sekali.

### Prioritas permintaan pengguna & tombol admin

- Kolom `job_search_queries.demand` = jumlah pengguna aktif yang menargetkan occupation itu (target, atau occupation AI miliknya lewat `base_occupation_id`). Dihitung ulang tiap pass importer oleh `public.refresh_search_demand()` (SECURITY DEFINER, hanya agregat).
- Urutan antrian: keyword dengan `demand>0` yang belum dicari 2 jam terakhir didahulukan, sisanya round-robin biasa. Gate 2 jam mencegah kategori lain kelaparan.
- Admin **Lowongan → "Cari lowongan sekarang"** (`POST /api/v1/admin/vacancies/scrape`) membuat semua sumber non-metered (`managed_by='rust'`) jatuh tempo sekarang dan membangunkan scheduler. Sumber yang sedang jalan, baru sukses, atau diminta < 10 menit lalu dilewati (`job_sources.requested_at`). `openwebninja` tidak ikut (kuota). Sumber tetap dijalankan berurutan, bukan paralel.

## 2. Komponen

| Bagian | Lokasi |
|---|---|
| Logika worker, filter, persist | `src/infrastructure/jobs.rs` (`Provider::GoogleJobs`, `next_queries`, `google_jobs`, `google_record`) |
| CLI | `src/bin/job-import.rs` |
| Migration (tabel antrian, seed keyword, trigger, kolom `occupation_id`) | `migrations/202611080004_openwebninja_source.sql` |
| Filter kategori di FE | `careerbridge-FE/src/app/pages/Lowongan.tsx`, `PekerjaanDetail.tsx` |

Tabel `job_search_queries`:

| Kolom | Arti |
|---|---|
| `occupation_id` | Kategori pekerjaan. Lowongan hasil keyword ini diberi kategori ini. |
| `keyword` | Teks `query` yang dikirim apa adanya ke API. |
| `title_pattern` | Regex Rust (opsional) yang **wajib** cocok dengan judul lowongan. `NULL` = terima semua judul. |
| `enabled` | `false` = keluar dari antrian. |
| `last_run_at` | Terakhir dijalankan. Penentu urutan antrian. |
| `last_count` | Jumlah lowongan yang diterima pada run terakhir. |
| `zero_streak` | Berapa run berturut-turut hasilnya 0. Tanda keyword perlu diperbaiki. |

API `GET /api/v1/me/vacancies`:

- Parameter `category` diganti `occupation` (UUID occupation).
- Tiap item sekarang membawa `occupationId` dan `occupationName`.
- Respons berisi `occupationOptions` (semua kategori katalog), dipakai dropdown filter.

## 3. Keyword yang sudah diverifikasi

Diuji live pada **8 November 2026** dengan `country=id&language=id`. Semua keyword di bawah mengembalikan 10 lowongan Indonesia, dan semua judulnya cocok dengan `title_pattern` masing-masing.

| Kategori | Keyword |
|---|---|
| Data Analyst | `data analyst jobs in Jakarta` |
| Web Developer | `web developer jobs in Jakarta` |
| UI/UX Designer | `ui ux designer jobs in Jakarta` |
| IT Support | `it support jobs in Jakarta` |
| Staf Administrasi | `staff administrasi jobs in Jakarta` |
| Staf Akuntansi | `accounting staff jobs in Jakarta` |
| Customer Service | `customer service jobs in Jakarta` |
| Digital Marketing | `digital marketing jobs in Jakarta` |
| Operator Produksi | `operator produksi jobs in Bekasi` |
| Software Engineer | `software engineer jobs in Jakarta` |
| DevOps Engineer | `devops engineer jobs in Jakarta` |

Temuan riset, penting saat membuat keyword baru:

- **Pola yang berhasil:** `<posisi> jobs in <kota>`.
- **`... jobs in Indonesia` → kosong** (dua kali dicoba).
- **Tanpa kota** (misal `lowongan data analyst`), lokasi hasilnya acak (Tasikmalaya, Semarang, Bali, mengikuti lokasi server API), dan 6 dari 10 keyword kosong. Jangan dipakai.
- **Keyword Inggris polos tanpa kota** (misal `data analyst`) → kosong.
- **Pakai kota yang memang punya banyak lowongan kategori itu.** Contoh: operator produksi → Bekasi (kawasan industri), bukan Jakarta.
- API sesekali membalas **HTTP 504**. Itu gangguan sementara; run berikutnya mengulang.

Field respons yang perlu diketahui: `job_city`, `job_country`, dan `job_posted_at_datetime_utc` selalu `null`. Karena itu kota/provinsi diambil dari `job_location` (`"Jakarta Barat, Kota Jakarta Barat, Daerah Khusus Ibukota Jakarta • melalui LinkedIn"`), dan tanggal dari `job_posted_at` (`"4 hari yang lalu"`).

## 4. Kuota

- Plan Basic: **100 request/bulan**, 1000/jam. Sisa kuota terlihat di dashboard https://app.openwebninja.com/api/realtime-jobs-data/overview.
- 1 keyword = 1 request. `OPENWEBNINJA_QUERIES_PER_RUN=2` → sekitar 60 request/bulan, menyisakan sekitar 40 untuk riset keyword (`--test-query`) dan `--once --force`.

Rumus aman: `PER_RUN × 30 + riset ≤ kuota`. Agar lowongan tetap tampil, setiap kategori perlu dijalankan ulang dalam 7 hari:

```
jumlah_keyword_aktif ÷ PER_RUN ≤ 7
```

| Keyword aktif | PER_RUN minimal | Request/bulan |
|---|---|---|
| 11 (sekarang) | 2 | ~60 |
| 14 | 2 | ~60 |
| 20 | 3 | ~90 (perlu plan lebih besar atau tanpa riset) |

Kalau keyword ditambah sampai rumus itu tidak terpenuhi, naikkan `PER_RUN` dan upgrade plan.

## 5. Cara menjalankan (pertama kali)

### 5.1 Isi `.env`

```env
JOB_SCRAPE_ENABLED=true
JOB_SCRAPE_INTERVAL_SECONDS=7200
# JOB_SCRAPE_SOURCES dikosongkan: semua sumber produksi + openwebninja (selama key terisi)
OPENWEBNINJA_API_KEY=<API key dari dashboard OpenWeb Ninja>
OPENWEBNINJA_QUERIES_PER_RUN=2
```

- Kalau `openwebninja` ada di `JOB_SCRAPE_SOURCES` tapi key kosong, server menolak start dengan error `OPENWEBNINJA_API_KEY is required for the openwebninja source`.
- Kalau `JOB_SCRAPE_SOURCES` tidak diisi, `openwebninja` otomatis ikut selama key terisi.
- **Jangan commit API key.** `.env` sudah di-`.gitignore`.

### 5.2 Jalankan migration

`SUPABASE_DB_URL` harus memakai kredensial pemilik schema.

```bash
cargo run --bin migrate
```

Docker: target `migrate`, lihat `docs/SETUP.md`.

Migration ini membuat tabel antrian, mengisi 11 keyword di atas, memasang trigger untuk occupation baru, menambah kolom `job_vacancies.occupation_id`, dan mengisi kategori lowongan lama (judul devops → DevOps Engineer, data analyst → Data Analyst, sisanya Software Engineer).

> Build di Windows tanpa NASM: set `AWS_LC_SYS_PREBUILT_NASM=1` sebelum `cargo`. Contoh Git Bash: `export AWS_LC_SYS_PREBUILT_NASM=1`.

### 5.3 Cek antrian

```bash
cargo run --bin job-import -- --status
```

Hasilnya berisi `sources` (status tiap sumber) dan `searchQueue` (11 keyword, `last_run_at` masih `null`).

### 5.4 Isi data sekarang (opsional)

Perintah ini memakai `PER_RUN` request dan menjalankan semua sumber, termasuk feed perusahaan:

```bash
cargo run --bin job-import -- --once --force
```

Ulangi `--status`. Dua keyword teratas sekarang punya `last_run_at` dan `last_count`.

### 5.5 Jalankan otomatis

Pilih **salah satu**. Keduanya aman berjalan bersamaan karena ada lease di DB, tapi tidak perlu.

- **Bersama server API** (`JOB_SCRAPE_ENABLED=true`). Server mengecek tiap 60 detik dan menjalankan sumber yang jatuh tempo.
  ```bash
  cargo run --bin careerbridge-be
  ```
- **Proses terpisah**:
  ```bash
  cargo run --bin job-import -- --daemon
  ```

Run pertama langsung jalan; berikutnya tiap 24 jam untuk `openwebninja`, tiap 2 jam untuk semua sumber lain. Jadwal tersimpan di `job_sources.next_run_at`, jadi tetap berlanjut setelah restart.

## 6. Perintah CLI `job-import`

| Perintah | Fungsi | Kuota | Tulis DB |
|---|---|---|---|
| `--status` | Status sumber + isi antrian | 0 | tidak |
| `--test-query "<keyword>" ["<regex>"]` | Uji satu keyword: berapa hasil, mana yang lolos filter | 1 | tidak |
| `--once` | Jalankan sumber yang jatuh tempo | PER_RUN bila `openwebninja` jatuh tempo | ya |
| `--once --force` | Jalankan semua sumber sekarang | PER_RUN | ya |
| `--daemon` | Loop terus (cek tiap 60 detik) | PER_RUN/hari | ya |
| `--preview` | CSV feed perusahaan ke `.cache/software-jobs/`. Melewati `openwebninja`. | 0 | tidak |
| `--retag` | Tandai ulang skill semua lowongan aktif dengan alias katalog saat ini | 0 | ya (`vacancy_skills`) |

### Penanda skill lowongan

Setiap import mencari nama dan alias skill katalog di judul + deskripsi + kualifikasi lowongan, lalu menyimpan hasilnya di `vacancy_skills` (penanda lama diganti). Pencocokan memakai urutan token utuh setelah normalisasi yang sama dengan jalur CV: `sql` tidak cocok dengan `mysqldump`, `react` tidak cocok dengan `reaktif`. Skor kecocokan di `/api/v1/me/vacancies` dihitung dari tabel ini.

Jalankan `job-import --retag` sekali setelah migrasi `202611090001_vacancy_skills.sql`, dan setiap kali admin mengubah alias. Tanpa itu, penanda menyusul sendiri saat lowongan di-upsert ulang (≤ 6 jam feed perusahaan, ≤ 7 hari Google Jobs).

Batas: alias pendek atau bermakna umum (`api`, `cs`, `sem`, `lan`) bisa memberi false positive pada teks Bahasa Indonesia. Hapus alias yang bermasalah lewat admin, lalu jalankan `--retag`.

## 7. Mengelola keyword

Keyword dikelola lewat SQL di Supabase SQL editor, sebagai pemilik schema.

**Riset dulu sebelum menyimpan** (1 request):

```bash
cargo run --bin job-import -- --test-query "barista jobs in Jakarta" "(?i)\b(barista|coffee)\b"
```

Lihat `results` (jumlah dari API) dan `accepted` (lolos filter Indonesia + regex). Kalau `results` 0, ganti kata atau kota. Kalau banyak judul yang relevan ditolak, longgarkan regex.

**Ganti keyword default occupation baru** (trigger membuat `"<nama> jobs in Jakarta"` tanpa regex):

```sql
UPDATE job_search_queries
SET keyword = 'barista jobs in Jakarta', title_pattern = '(?i)\b(barista|coffee)\b', zero_streak = 0
WHERE occupation_id = (SELECT id FROM occupations WHERE nama = 'Barista');
```

**Tambah keyword kedua untuk satu kategori** (misal kota lain):

```sql
INSERT INTO job_search_queries(occupation_id, keyword, title_pattern)
SELECT id, 'web developer jobs in Bandung', title_pattern
FROM occupations o JOIN job_search_queries q ON q.occupation_id = o.id
WHERE o.nama = 'Web Developer' AND q.keyword = 'web developer jobs in Jakarta';
```

Perhatikan rumus kuota di bagian 4.

**Nonaktifkan / aktifkan:**

```sql
UPDATE job_search_queries SET enabled = false WHERE keyword = 'web developer jobs in Bandung';
```

**Jalankan keyword tertentu duluan besok:**

```sql
UPDATE job_search_queries SET last_run_at = NULL WHERE keyword = 'ui ux designer jobs in Jakarta';
```

**Cari keyword bermasalah:**

```sql
SELECT o.nama, q.keyword, q.last_run_at, q.last_count, q.zero_streak
FROM job_search_queries q JOIN occupations o ON o.id = q.occupation_id
WHERE q.enabled AND q.zero_streak >= 2
ORDER BY q.zero_streak DESC;
```

Aturan `title_pattern`:

- Sintaks regex **Rust**. Awali dengan `(?i)` agar tidak peka huruf besar/kecil.
- Kalau regex tidak valid, keyword dilewati **tanpa** memakai kuota, `zero_streak` naik, dan log berisi `invalid title_pattern; query skipped`.

## 8. Aturan filter lowongan

Lowongan disimpan hanya jika **semua** syarat ini terpenuhi:

1. Judul tidak kosong dan cocok `title_pattern` (jika diisi).
2. `job_location` menyebut Indonesia atau nama provinsi/pulau Indonesia (Jakarta, Jawa, Banten, Bali, Sumatera, Kalimantan, Sulawesi, Papua, dst.). Papua Nugini, Singapura, dan `"Di mana saja"` ditolak.
3. Link lamaran `https`, port 443, tanpa user/password, dan host berupa nama domain (bukan IP / `localhost`).
4. ID, panjang teks, dan perusahaan lolos validasi `Vacancy::validate`.

Lowongan yang gagal dilewati saja; run tetap berlanjut.

## 9. Troubleshooting

| Gejala | Penyebab / tindakan |
|---|---|
| `--status`: `openwebninja.last_error` terisi, `next_run_at` +24 jam | API error (504, kuota habis, key salah). Cek dashboard. Run berikutnya mengulang otomatis. Perlu sekarang: `--once --force`. |
| `next_run_at` terus maju tapi tidak ada request di dashboard | Antrian kosong / semua `enabled=false`. Cek `searchQueue`. |
| Kategori kosong di halaman Lowongan | Keyword kategori itu belum gilirannya, `zero_streak` tinggi, atau lowongannya >7 hari tidak terlihat lagi. Cek `--status` dan tuning keyword. |
| Server gagal start: `OPENWEBNINJA_API_KEY is required...` | Isi key atau hapus `openwebninja` dari `JOB_SCRAPE_SOURCES`. |
| `OPENWEBNINJA_QUERIES_PER_RUN must be 1 to 10` | Nilai env di luar rentang. |
| Kuota habis sebelum reset | Turunkan `PER_RUN` ke 1, kurangi riset, atau upgrade plan. Kuota reset tiap 30 hari (lihat dashboard). |

## 10. Batasan

- Hanya halaman pertama per keyword (`num_pages=1`, sekitar 10 lowongan). Biaya `num_pages > 1` belum diverifikasi.
- `posted_at` diperkirakan dari teks relatif ("4 hari yang lalu"), presisinya per menit/jam/hari, dan bisa kosong.
- Kategori lowongan feed perusahaan ditentukan dari judul (DevOps / Data Analyst / Software Engineer). Kategori lowongan Google ditentukan dari keyword yang menemukannya.
- Belum ada UI admin untuk keyword; kelola lewat SQL (bagian 7).
