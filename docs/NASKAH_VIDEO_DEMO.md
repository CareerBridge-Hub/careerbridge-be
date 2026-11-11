# Naskah Video Demo CareerBridge

Panduan rekaman untuk 5 orang. Target durasi total **8 sampai 10 menit**.

Setiap bagian berisi:

- **Layar**: apa yang ditampilkan atau diklik.
- **Ucapan**: kalimat yang dibacakan. Tidak harus hafal kata per kata, yang penting poinnya tersampaikan dengan bahasa sendiri.

Ganti "Pembicara 1" sampai "Pembicara 5" dengan nama masing-masing.

---

## Ringkasan pembagian

| Bagian | Pembicara | Topik | Durasi |
| --- | --- | --- | --- |
| 1 | Pembicara 1 | Pembukaan, masalah, dan solusi | 1,5 menit |
| 2 | Pembicara 2 | Daftar akun dan upload CV | 2 menit |
| 3 | Pembicara 3 | Skor kecocokan dan penjelasan AI | 2 menit |
| 4 | Pembicara 4 | Rekomendasi pelatihan dan lowongan kerja | 2 menit |
| 5 | Pembicara 5 | Halaman admin, teknologi, dan penutup | 2 menit |

---

## Persiapan sebelum rekam

Kerjakan ini dulu supaya rekaman tidak terputus di tengah jalan.

1. **Server jalan.** Backend dan frontend sudah hidup, halaman bisa dibuka tanpa error. Cek `/health` dan `/ready` di backend.
2. **Siapkan 2 akun:**
   - Akun pengguna biasa yang **belum** punya CV (dipakai Pembicara 2 dan 3). Kalau mau memperlihatkan pendaftaran dari nol, siapkan email baru.
   - Akun admin (dipakai Pembicara 5).
3. **Siapkan 1 file CV contoh** dalam format PDF, ukuran di bawah 2 MB. Pakai CV buatan/fiktif, **jangan CV asli** milik siapa pun, karena isinya akan tampil di video.
4. **Coba upload CV itu sekali** sebelum rekam, untuk memastikan AI berhasil membaca dan skill-nya terisi. Setelah itu hapus CV-nya lagi supaya bisa direkam dari awal.
5. **Pilih satu pekerjaan contoh** yang hasilnya bagus untuk ditunjukkan (skornya tidak 0% dan tidak 100%, ada skill yang kurang, ada pelatihan yang disarankan). Catat namanya.
6. **Browser bersih:** tutup tab lain, matikan notifikasi, sembunyikan bookmark bar, zoom 100% sampai 110% supaya teks terbaca di video.
7. **Jangan tampilkan rahasia:** file `.env`, password, API key, dan dashboard Supabase jangan sampai ikut terekam.
8. Rekam layar dengan resolusi minimal 1080p. Suara direkam di ruangan yang tenang.

---

## Bagian 1. Pembukaan (Pembicara 1)

**Layar:** Wajah pembicara, lalu pindah ke halaman landing CareerBridge.

**Ucapan:**

> Halo, kami tim CareerBridge. Saya [nama], dan bersama saya ada [nama 2], [nama 3], [nama 4], dan [nama 5].
>
> Banyak pencari kerja, terutama lulusan baru dan lulusan SMK, bingung harus mulai dari mana. Mereka tahu pekerjaan yang diinginkan, tapi tidak tahu skill apa yang masih kurang, dan tidak tahu pelatihan mana yang bisa menutup kekurangan itu.
>
> CareerBridge menjawab masalah itu dalam tiga langkah. Pertama, upload CV. Kedua, sistem menunjukkan seberapa cocok skill kamu dengan sebuah pekerjaan dan skill apa yang masih kurang. Ketiga, sistem menyarankan pelatihan dari LPK dan BLK untuk menutup kekurangan itu, lengkap dengan lowongan kerja yang asli.
>
> Sekarang kita lihat langsung aplikasinya.

**Layar:** Scroll pelan di landing page, lalu klik tombol **Daftar gratis**.

---

## Bagian 2. Daftar akun dan upload CV (Pembicara 2)

**Layar:** Halaman **Daftar**.

**Ucapan:**

> Pertama, pengguna membuat akun. Cukup isi nama, email, dan password. Setelah itu ada pertanyaan singkat tentang pendidikan terakhir, jurusan, provinsi, dan kota. Data lokasi ini dipakai untuk menyaring pelatihan yang dekat dengan tempat tinggal pengguna.

**Layar:** Isi form, klik lanjut sampai masuk ke **Beranda**. Tunjukkan sapaan "Halo, [nama]".

> Setelah masuk, pengguna langsung diarahkan ke beranda. Karena belum ada skill tersimpan, langkah berikutnya adalah upload CV.

**Layar:** Buka menu **CV & Skill**. Tarik file PDF ke kotak upload.

> Di halaman CV, pengguna cukup menarik file PDF ke sini, maksimal 2 MB. CV ini bersifat pribadi, hanya pemiliknya yang bisa melihat, dan bisa dihapus kapan saja.

**Layar:** Biarkan progres berjalan. Tunjukkan 4 langkah prosesnya.

> Bisa dilihat ada empat tahap. File diunggah, teks diambil dari PDF, lalu AI memisahkan bagian pendidikan, pengalaman, dan skill. Terakhir, nama skill disamakan dengan kamus skill kami. Contohnya, kalau di CV tertulis "JS", sistem akan mengenalinya sebagai JavaScript.

**Layar:** Hasil muncul. Tunjukkan bagian Pendidikan, Pengalaman, dan daftar Skill. Hapus satu skill atau tambah satu skill, lalu klik simpan.

> Hasil bacaan AI tidak langsung disimpan. Pengguna bisa memeriksa dan membetulkannya dulu: menghapus skill yang salah, atau menambah skill yang terlewat. Kalau ada skill yang tidak ada di kamus, pengguna bisa mencocokkannya ke skill yang ada atau mengabaikannya. Setelah yakin, baru disimpan.
>
> Untuk yang belum punya CV, skill juga bisa dipilih langsung dari daftar.

**Tips:** Kalau proses AI lama, potong saja bagian menunggunya saat editing.

---

## Bagian 3. Skor kecocokan dan penjelasan AI (Pembicara 3)

**Layar:** Kembali ke **Beranda**. Tunjukkan kartu ringkasan: Skill dimiliki, Paling cocok, Skill wajib kurang, Pelatihan gratis.

**Ucapan:**

> Setelah skill tersimpan, beranda langsung berisi ringkasan. Ada jumlah skill yang dimiliki, pekerjaan yang paling cocok, jumlah skill wajib yang masih kurang, dan jumlah pelatihan gratis yang tersedia. Di bawahnya ada enam pekerjaan dengan skor tertinggi.

**Layar:** Buka menu **Pekerjaan**. Tunjukkan daftar yang sudah terurut, coba kolom pencarian sebentar.

> Di halaman Pekerjaan, semua pekerjaan diurutkan dari yang paling cocok. Cara hitung skornya sederhana. Setiap pekerjaan punya skill wajib yang bernilai 2 poin dan skill tambahan yang bernilai 1 poin. Skor adalah poin skill yang kamu punya dibagi total poin skill pekerjaan itu.

**Layar:** Klik pekerjaan contoh yang sudah disiapkan. Tunjukkan skor dan **Rincian poin skill** (kotak-kotak skill).

> Kita buka salah satu pekerjaan. Di sini terlihat rinciannya. Setiap kotak adalah satu skill. Ada yang sudah dimiliki, ada skill wajib yang belum dimiliki, dan ada skill tambahan yang belum dimiliki. Jadi pengguna tahu persis apa yang harus dipelajari dulu.

**Layar:** Scroll ke kartu **Penjelasan AI**. Tunggu teksnya muncul.

> Di bagian ini AI menuliskan penjelasan dengan bahasa yang mudah dipahami: kenapa skornya segini, skill mana yang paling penting dikejar, dan pelatihan mana yang bisa membantu.
>
> Yang penting, AI tidak dibiarkan mengarang. Sebelum ditampilkan, server mengecek setiap angka dan nama skill di dalam penjelasan itu. Kalau ada yang tidak cocok dengan data, atau AI sedang tidak tersedia, yang tampil adalah penjelasan cadangan yang disusun langsung dari data. Jadi informasinya tetap bisa dipercaya.

**Layar:** Tunjukkan sekilas bagian **Pekerjaan serupa yang mungkin cocok**.

> Sistem juga menyarankan pekerjaan lain yang mirip, yang mungkin lebih cocok dengan skill pengguna saat ini.

---

## Bagian 4. Pelatihan dan lowongan kerja (Pembicara 4)

**Layar:** Masih di halaman detail pekerjaan, scroll ke **Pelatihan yang disarankan**.

**Ucapan:**

> Setelah tahu skill yang kurang, langkah berikutnya adalah belajar. Di bawah penjelasan AI ada pelatihan yang disarankan. Pelatihan yang menutup paling banyak skill wajib yang kurang akan ditaruh paling atas.

**Layar:** Buka menu **Pelatihan**. Ketik kata kunci, misalnya "Excel". Coba filter Biaya (gratis) dan Online/Offline.

> Pengguna juga bisa menjelajah sendiri di halaman Pelatihan. Datanya berasal dari katalog asli, lebih dari 370 lembaga LPK dan BLK dengan sekitar dua ribu program pelatihan. Pencariannya memakai pencarian semantik, jadi sistem mencari berdasarkan makna, bukan hanya kata yang persis sama. Hasilnya bisa disaring berdasarkan biaya, online atau tatap muka, dan pekerjaan yang dituju.

**Layar:** Buka menu **Lowongan**. Tunjukkan filter kota (ketik "Jakarta"), lalu ubah urutan ke **Paling cocok**. Buka satu lowongan untuk melihat detail dan tombol **Lamar**.

> Terakhir, halaman Lowongan. Ini lowongan kerja asli di Indonesia, diambil otomatis dari halaman karier perusahaan setiap 6 jam dan dari Google Jobs setiap hari. Lowongan yang sudah ditutup atau terlalu lama tidak diperbarui tidak akan ditampilkan.
>
> Lowongan bisa diurutkan berdasarkan yang paling cocok dengan skill pengguna. Kalau tertarik, pengguna klik Lamar dan langsung diarahkan ke situs sumber aslinya. CareerBridge tidak menampung lamaran, jadi pengguna melamar langsung ke perusahaannya.

**Layar (opsional):** Buka **Profil**, tunjukkan bagian **Privasi CV** dan tombol ganti password.

> Di halaman Profil, pengguna bisa mengubah data diri, mengganti password, dan mengelola atau menghapus CV-nya.

---

## Bagian 5. Admin, teknologi, dan penutup (Pembicara 5)

**Layar:** Logout, lalu login dengan akun admin. Otomatis masuk ke **Kamus skill**.

**Ucapan:**

> Selain pengguna biasa, ada juga halaman admin untuk mengelola data.

**Layar:** Tunjukkan berurutan, masing-masing beberapa detik:

1. **Kamus skill**: daftar skill dan aliasnya.
2. **Pekerjaan**: skill wajib (2 poin) dan tambahan (1 poin) per pekerjaan, beserta kode KBJI.
3. **LPK & pelatihan**: lembaga dan program beserta skill yang diajarkan. Sebutkan fitur import CSV.
4. **Lowongan**: tombol untuk menyembunyikan atau mengizinkan lowongan tampil.

> Admin mengelola empat hal. Kamus skill beserta nama lainnya, supaya hasil bacaan CV bisa disamakan. Daftar pekerjaan beserta skill wajib dan tambahannya, yang menjadi dasar perhitungan skor. Data LPK dan pelatihan, yang bisa ditambahkan sekaligus lewat file CSV. Dan moderasi lowongan, untuk menyembunyikan lowongan yang tidak relevan.

**Layar:** Tampilkan satu slide sederhana berisi diagram teknologi (lihat bagian "Slide teknologi" di bawah).

> Dari sisi teknologi, frontend dibangun dengan React dan Vite. Backend ditulis dengan Rust memakai Actix Web, sehingga cepat dan hemat memori. Database memakai PostgreSQL di Supabase dengan pgvector untuk pencarian semantik. Model embedding E5 dijalankan langsung di server kami sendiri, sedangkan AI Gemini dipakai untuk membaca CV dan menulis penjelasan.
>
> Keamanan juga kami perhatikan. Sesi login memakai cookie HttpOnly, setiap pengguna hanya bisa mengakses datanya sendiri lewat Row Level Security, dan hasil bacaan CV disimpan dalam keadaan terenkripsi. Aplikasi dijalankan dengan Docker di VPS dengan HTTPS, dan setiap perubahan kode dicek otomatis oleh pemindai keamanan.
>
> Jadi dengan CareerBridge, pencari kerja tidak lagi menebak-nebak. Mereka tahu posisi skill-nya sekarang, tahu apa yang harus dipelajari, dan tahu ke mana harus melamar.
>
> Terima kasih sudah menonton. Kami tim CareerBridge.

**Layar:** Semua anggota tampil bersama (kalau memungkinkan), lalu logo CareerBridge.

---

## Slide teknologi (untuk Bagian 5)

Cukup satu slide dengan susunan seperti ini:

```
 Pengguna (browser)
        │
        ▼
 Frontend: React + Vite + Tailwind
        │  HTTPS (Caddy)
        ▼
 Backend: Rust + Actix Web
   ├── Embedding E5 lokal (pencarian semantik)
   ├── Gemini (baca CV, tulis penjelasan)
   └── Worker lowongan (karier perusahaan + Google Jobs)
        │
        ▼
 Supabase: PostgreSQL + pgvector, Storage CV privat
```

---

## Kalau ada kendala saat rekam

| Masalah | Yang dilakukan |
| --- | --- |
| Upload CV lama | Rekam tetap jalan, potong bagian menunggu saat editing. |
| Penjelasan AI tidak muncul atau tampil penjelasan cadangan | Tidak apa-apa. Jelaskan bahwa ini justru fitur: sistem tetap memberi jawaban dari data walau AI tidak tersedia. |
| Skor semua 0% | Skill belum tersimpan. Kembali ke halaman CV & Skill dan klik simpan. |
| Lowongan kosong | Hapus filter, atau pilih kategori pekerjaan lain. |
| Salah klik atau salah ucap | Berhenti sebentar, ulangi kalimatnya dari awal. Bagian yang salah dipotong saat editing. |

---

## Tips umum untuk yang tampil di kamera

- Bicara pelan dan jelas. Lebih baik sedikit lambat daripada terburu-buru.
- Gerakkan mouse dengan tenang. Arahkan kursor ke bagian yang sedang dibicarakan.
- Setiap pembicara rekam bagiannya masing-masing, lalu digabung saat editing. Lebih mudah daripada satu kali rekam panjang.
- Mulai dan akhiri setiap bagian dengan jeda 2 detik tanpa suara, supaya mudah dipotong.
- Sebelum rekam final, latihan sekali bersama dan hitung durasinya.

---

## Checklist akhir

- [ ] Server backend dan frontend jalan
- [ ] Akun pengguna dan akun admin siap
- [ ] CV contoh (fiktif, PDF, di bawah 2 MB) siap dan sudah dites
- [ ] Pekerjaan contoh sudah dipilih
- [ ] Browser bersih, notifikasi mati, zoom pas
- [ ] Tidak ada `.env`, password, atau API key yang terlihat di layar
- [ ] Slide teknologi siap
- [ ] Setiap pembicara sudah latihan bagiannya
