-- Skills are no longer limited to an admin-curated (mostly IT/office) list: the CV worker
-- may add AI-named skills for any field of work. Admin rows stay 'admin'.
ALTER TABLE skills ADD COLUMN sumber text NOT NULL DEFAULT 'admin' CHECK(sumber IN ('admin','ai'));

-- The worker gets no direct INSERT on the catalog. This function validates input, never
-- touches admin rows, and only attaches aliases to AI skills. Returns the stored skill ID
-- (an existing skill with the same name wins over p_id).
CREATE FUNCTION public.add_ai_skill(p_id uuid, p_nama text, p_kategori text, p_aliases text[])
RETURNS uuid LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE v uuid;
BEGIN
 IF p_nama IS NULL OR length(btrim(p_nama)) NOT BETWEEN 2 AND 80
  OR p_kategori IS NULL OR length(btrim(p_kategori)) NOT BETWEEN 2 AND 80
  OR coalesce(cardinality(p_aliases),0) > 10 THEN
  RAISE EXCEPTION 'invalid AI skill';
 END IF;
 INSERT INTO public.skills(id,nama,kategori,sumber) VALUES(p_id,btrim(p_nama),btrim(p_kategori),'ai')
  ON CONFLICT DO NOTHING;
 SELECT id INTO v FROM public.skills WHERE lower(nama)=lower(btrim(p_nama)) OR id=p_id
  ORDER BY (lower(nama)=lower(btrim(p_nama))) DESC LIMIT 1;
 INSERT INTO public.skill_aliases(alias,skill_id)
  SELECT btrim(a),v FROM unnest(coalesce(p_aliases,'{}')) a
  WHERE length(btrim(a)) BETWEEN 1 AND 100
   AND EXISTS(SELECT 1 FROM public.skills s WHERE s.id=v AND s.sumber='ai')
  ON CONFLICT DO NOTHING;
 RETURN v;
END $$;
REVOKE ALL ON FUNCTION public.add_ai_skill(uuid,text,text,text[]) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.add_ai_skill(uuid,text,text,text[]) TO careerbridge_worker;

-- Catalog occupations across fields, so vacancies of every field are searched (each
-- occupation gets a search-queue keyword through the queue_search trigger) and scored.
-- Columns: nama, KBJI, bidang, deskripsi, search keyword, title pattern (Rust regex).
-- Patterns name whole phrases: the classifier picks the occupation with the longest match.
CREATE TEMP TABLE seed_occupation(nama text, kbji text, bidang text, deskripsi text, keyword text, pattern text) ON COMMIT DROP;
INSERT INTO seed_occupation VALUES
 ('Perawat','3221','Kesehatan','Memberikan asuhan keperawatan dan memantau kondisi pasien di fasilitas kesehatan.','perawat','(?i)\b(perawat|nurse|nursing|keperawatan)\b'),
 ('Bidan','3222','Kesehatan','Memberikan pelayanan kehamilan, persalinan, nifas, dan keluarga berencana.','bidan','(?i)\b(bidan|midwife|kebidanan)\b'),
 ('Dokter Umum','2211','Kesehatan','Memeriksa, mendiagnosis, dan menangani pasien dengan keluhan kesehatan umum.','dokter umum','(?i)\b(dokter umum|dokter jaga|general practitioner|doctor|dokter)\b'),
 ('Apoteker','2262','Kesehatan','Mengelola obat dan memberikan pelayanan kefarmasian kepada pasien.','apoteker','(?i)\b(apoteker|pharmacist|farmasi|pharmacy)\b'),
 ('Asisten Apoteker','3213','Kesehatan','Membantu peracikan, penyiapan, dan penyerahan obat di apotek atau rumah sakit.','asisten apoteker','(?i)\b(asisten apoteker|tenaga teknis kefarmasian|ttk|pharmacy assistant|pharmacist assistant)\b'),
 ('Analis Laboratorium Medis','3212','Kesehatan','Mengambil sampel dan melakukan pemeriksaan laboratorium klinik.','analis laboratorium','(?i)\b(analis laborat\w*|ahli teknologi laborat\w*|medical laborat\w*|laboratory analyst|lab analyst|atlm)\b'),
 ('Fisioterapis','2264','Kesehatan','Memulihkan fungsi gerak pasien melalui latihan dan modalitas terapi.','fisioterapis','(?i)\b(fisioterapis|fisioterapi|physiotherapist|physical therapist)\b'),
 ('Ahli Gizi','2265','Kesehatan','Menilai status gizi dan menyusun menu serta edukasi gizi.','ahli gizi','(?i)\b(ahli gizi|nutrisionis|nutritionist|dietisien|dietitian)\b'),
 ('Psikolog','2634','Kesehatan','Melakukan asesmen psikologis dan konseling bagi individu atau organisasi.','psikolog','(?i)\b(psikolog\w*|psychologist|konselor|counselor)\b'),
 ('Guru','2330','Pendidikan','Merencanakan, menyampaikan, dan mengevaluasi pembelajaran bagi peserta didik.','guru','(?i)\b(guru|teacher|pengajar|tutor|instruktur)\b'),
 ('Guru PAUD','2342','Pendidikan','Mendidik dan menstimulasi tumbuh kembang anak usia dini.','guru paud','(?i)\b(guru paud|guru tk|early childhood\w*|preschool teacher|kindergarten teacher)\b'),
 ('Kasir','5230','Penjualan dan Ritel','Melayani pembayaran pelanggan dan mengelola transaksi kas di toko.','kasir','(?i)\b(kasir|cashier)\b'),
 ('Pramuniaga','5223','Penjualan dan Ritel','Melayani pelanggan, menata produk, dan membantu penjualan di toko.','pramuniaga','(?i)\b(pramuniaga|shop assistant|sales assistant|spg|spb|sales promotion\w*|store crew|crew store|beauty advisor)\b'),
 ('Sales Executive','3322','Penjualan dan Ritel','Mencari pelanggan baru dan menjual produk atau jasa sesuai target.','sales executive','(?i)\b(sales executive|sales representative|sales officer|account executive|tenaga penjual\w*|salesman|canvasser|sales)\b'),
 ('Telemarketing','5244','Penjualan dan Ritel','Menawarkan produk dan melayani pelanggan melalui telepon.','telemarketing','(?i)\b(telemarketing|telemarketer|telesales)\b'),
 ('Kepala Toko','1420','Penjualan dan Ritel','Mengelola operasional, tim, stok, dan penjualan sebuah toko atau gerai.','kepala toko','(?i)\b(kepala toko|store manager|store leader|kepala cabang|branch manager|area manager)\b'),
 ('Juru Masak','5120','Perhotelan dan Kuliner','Menyiapkan dan memasak hidangan sesuai standar resep dan kebersihan.','juru masak','(?i)\b(juru masak|cook|koki|cook helper|kitchen crew|kitchen staff|commis|line cook)\b'),
 ('Chef','3434','Perhotelan dan Kuliner','Memimpin dapur, mengembangkan menu, dan menjaga mutu hidangan.','chef','(?i)\b(chef|head chef|sous chef|chef de partie)\b'),
 ('Barista','5246','Perhotelan dan Kuliner','Menyiapkan minuman kopi dan melayani pelanggan di kedai.','barista','(?i)\b(barista|coffee maker)\b'),
 ('Pramusaji','5131','Perhotelan dan Kuliner','Melayani pesanan makanan dan minuman tamu restoran.','pramusaji','(?i)\b(pramusaji|waiter|waitress|crew restoran|crew resto|crew outlet|food and beverage service|f&b service)\b'),
 ('Resepsionis','4226','Perhotelan dan Kuliner','Menerima tamu, menangani reservasi, dan memberikan informasi di meja depan.','resepsionis','(?i)\b(resepsionis|receptionist|front office|front desk|guest service\w*)\b'),
 ('Housekeeping','9112','Perhotelan dan Kuliner','Membersihkan dan merapikan kamar serta area hotel sesuai standar.','housekeeping','(?i)\b(housekeeping|room attendant|room boy|public area attendant)\b'),
 ('Staf Tiket dan Travel','4221','Perhotelan dan Kuliner','Melayani pemesanan tiket, paket perjalanan, dan informasi wisata.','travel consultant','(?i)\b(travel consultant|travel agent|ticketing|staf tiket|tour guide|pemandu wisata|reservation staff)\b'),
 ('Petugas Kebersihan','9112','Layanan Umum','Menjaga kebersihan gedung, kantor, atau fasilitas umum.','cleaning service','(?i)\b(cleaning service|cleaner|petugas kebersihan|office boy|office girl)\b'),
 ('Satpam','5414','Layanan Umum','Menjaga keamanan aset, orang, dan lingkungan kerja.','satpam','(?i)\b(satpam|security guard|security officer|petugas keamanan|security staff|pengamanan)\b'),
 ('Terapis Kecantikan','5142','Kecantikan','Memberikan perawatan wajah, tubuh, rambut, atau tata rias kepada pelanggan.','beauty therapist','(?i)\b(beauty therapist|beautician|terapis kecantikan|terapis spa|spa therapist|hair stylist|hairdresser|penata rambut|kapster|barber|makeup artist|mua|nail artist)\b'),
 ('Pengemudi','8322','Logistik dan Transportasi','Mengemudikan kendaraan untuk mengantar orang atau barang dengan aman.','driver','(?i)\b(driver|pengemudi|sopir|supir|chauffeur)\b'),
 ('Kurir','9621','Logistik dan Transportasi','Mengantar paket atau dokumen ke penerima tepat waktu.','kurir','(?i)\b(kurir|courier|delivery man|delivery staff|pengantar barang|rider)\b'),
 ('Staf Gudang','4321','Logistik dan Transportasi','Menerima, menyimpan, mencatat, dan mengeluarkan barang di gudang.','staff gudang','(?i)\b(gudang|warehouse|stock keeper|storekeeper|checker|picker|packer|packing|admin gudang|inventory staff)\b'),
 ('Staf Logistik','4323','Logistik dan Transportasi','Mengatur pengiriman, dokumen, dan alur barang dalam rantai pasok.','staff logistik','(?i)\b(logisti\w*|logistic\w*|supply chain|ekspedisi|shipping|freight|export import|ekspor impor)\b'),
 ('Staf Purchasing','3323','Logistik dan Transportasi','Mengadakan barang dan jasa dari pemasok dengan harga dan mutu yang sesuai.','purchasing','(?i)\b(purchasing|procurement|pengadaan|pembelian|buyer)\b'),
 ('Operator Forklift','8344','Logistik dan Transportasi','Mengoperasikan forklift untuk memindahkan dan menyusun barang.','operator forklift','(?i)\b(operator forklift|forklift operator|forklift|reach truck)\b'),
 ('Operator Alat Berat','8342','Teknik dan Konstruksi','Mengoperasikan alat berat seperti excavator di proyek atau tambang.','operator alat berat','(?i)\b(operator alat berat|heavy equipment|excavator|bulldozer|dump truck|operator crane|crane operator)\b'),
 ('Teknisi Listrik','7411','Teknik dan Konstruksi','Memasang, memeriksa, dan memperbaiki instalasi listrik.','teknisi listrik','(?i)\b(teknisi listrik|electrician|electrical technician|teknisi elektrikal|instalasi listrik|listrik)\b'),
 ('Mekanik Otomotif','7231','Otomotif','Merawat dan memperbaiki kendaraan bermotor di bengkel.','mekanik otomotif','(?i)\b(mekanik otomotif|mekanik|mechanic|teknisi otomotif|automotive technician)\b'),
 ('Teknisi Mesin','7233','Manufaktur','Merawat dan memperbaiki mesin produksi agar tetap beroperasi.','teknisi maintenance','(?i)\b(teknisi mesin|teknisi maintenance|maintenance technician|maintenance|mekanik mesin|machine technician|mechanical technician)\b'),
 ('Teknisi AC','7127','Teknik dan Konstruksi','Memasang, merawat, dan memperbaiki pendingin ruangan.','teknisi ac','(?i)\b(teknisi ac|ac technician|refrigeration|hvac|tata udara)\b'),
 ('Juru Las','7212','Manufaktur','Menyambung logam dengan proses pengelasan sesuai gambar kerja.','welder','(?i)\b(welder|juru las|tukang las|welding|pengelasan)\b'),
 ('Operator Jahit','7533','Manufaktur','Menjahit bagian pakaian atau produk tekstil dengan mesin jahit.','operator jahit','(?i)\b(operator jahit|penjahit|tailor|sewing operator|sewing|garment)\b'),
 ('Quality Control','7543','Manufaktur','Memeriksa mutu bahan dan produk sesuai standar.','quality control','(?i)\b(quality control|qc|quality assurance|qa|quality inspector|inspector)\b'),
 ('Staf PPIC','4322','Manufaktur','Merencanakan jadwal produksi dan mengendalikan persediaan bahan.','ppic','(?i)\b(ppic|production planning|perencanaan produksi|inventory control)\b'),
 ('HSE Officer','2263','Manufaktur','Menerapkan program keselamatan dan kesehatan kerja serta lingkungan.','hse officer','(?i)\b(hse|qhse|hsse|k3|safety officer|health safety|ahli k3|safety)\b'),
 ('Insinyur Sipil','2142','Teknik dan Konstruksi','Merencanakan dan mengawasi pembangunan struktur dan infrastruktur.','civil engineer','(?i)\b(civil engineer\w*|teknik sipil|insinyur sipil|structural engineer|site engineer|quantity surveyor|estimator)\b'),
 ('Drafter','3118','Teknik dan Konstruksi','Membuat gambar teknik untuk proyek konstruksi atau manufaktur.','drafter','(?i)\b(drafter|draftsman|juru gambar|cad operator)\b'),
 ('Pengawas Lapangan','3123','Teknik dan Konstruksi','Mengawasi pekerjaan konstruksi agar sesuai gambar, jadwal, dan K3.','pengawas lapangan','(?i)\b(pengawas lapangan|site supervisor|supervisor proyek|pelaksana lapangan|pelaksana proyek|mandor|foreman|site manager)\b'),
 ('Staf HRD','2423','Sumber Daya Manusia','Mengelola rekrutmen, administrasi kepegawaian, dan pengembangan karyawan.','staff hrd','(?i)\b(hrd|hr|human resource\w*|sumber daya manusia|personalia|recruit\w*|talent acquisition)\b'),
 ('Sekretaris','3343','Administrasi Perkantoran','Mengelola agenda, korespondensi, dan dokumen pimpinan.','sekretaris','(?i)\b(sekretaris|secretary|personal assistant|executive assistant)\b'),
 ('Staf Legal','3411','Hukum','Menyusun kontrak, memastikan kepatuhan hukum, dan mengurus perizinan.','staff legal','(?i)\b(legal|hukum|lawyer|advokat|pengacara|paralegal|notaris)\b'),
 ('Auditor','2411','Keuangan dan Perbankan','Memeriksa laporan keuangan dan pengendalian internal organisasi.','auditor','(?i)\b(auditor|audit\w*|internal control)\b'),
 ('Teller Bank','4211','Keuangan dan Perbankan','Melayani transaksi tunai dan nontunai nasabah di kantor bank.','teller bank','(?i)\b(teller|kasir bank)\b'),
 ('Analis Kredit','3312','Keuangan dan Perbankan','Menilai kelayakan pengajuan kredit dan risiko calon debitur.','analis kredit','(?i)\b(analis kredit|credit analyst|credit officer|account officer|loan officer|marketing kredit)\b'),
 ('Staf Penagihan','4214','Keuangan dan Perbankan','Menagih piutang pelanggan dan menegosiasikan pembayaran.','collection','(?i)\b(collection|collector|penagihan|desk collection|field collection|debt collector)\b'),
 ('Agen Asuransi','3321','Keuangan dan Perbankan','Menawarkan produk asuransi dan membantu nasabah memilih perlindungan.','agen asuransi','(?i)\b(agen asuransi|insurance agent|asuransi|insurance|financial advisor|financial planner|unit link)\b'),
 ('Desainer Grafis','2166','Kreatif dan Media','Merancang materi visual untuk promosi, publikasi, dan identitas merek.','graphic designer','(?i)\b(graphic designer|desainer grafis|desain grafis|visual designer|illustrator|layouter)\b'),
 ('Video Editor','3521','Kreatif dan Media','Menyunting video dan membuat konten audiovisual.','video editor','(?i)\b(video editor|editor video|videografer|videographer|content creator|kreator konten|motion graphic\w*)\b'),
 ('Penulis Konten','2641','Kreatif dan Media','Menulis artikel, naskah, dan materi komunikasi untuk berbagai media.','content writer','(?i)\b(content writer|penulis konten|penulis|copywriter|writer|jurnalis|journalist|reporter|wartawan|editor naskah)\b'),
 ('Public Relations','2432','Pemasaran','Membangun citra organisasi melalui hubungan media, acara, dan komunikasi publik.','public relations','(?i)\b(public relation\w*|humas|marketing communication\w*|marcom|event organizer|event officer)\b'),
 ('Staf Agronomi','2132','Pertanian dan Peternakan','Mengelola budidaya tanaman, pemupukan, dan pengendalian hama di kebun.','agronomi','(?i)\b(agronom\w*|pertanian|agriculture|perkebunan|plantation|asisten kebun|mandor kebun|petani)\b'),
 ('Peternak','6121','Pertanian dan Peternakan','Memelihara ternak, memberi pakan, dan menjaga kesehatan kandang.','anak kandang','(?i)\b(peternak\w*|anak kandang|kandang|livestock|poultry|unggas)\b');

-- Columns: occupation, requirement type, skill, skill category (used only for new skills).
CREATE TEMP TABLE seed_requirement(occupation text, tipe text, skill text, kategori text) ON COMMIT DROP;
INSERT INTO seed_requirement VALUES
 ('Perawat','wajib','Asuhan Keperawatan','Kesehatan'),('Perawat','wajib','Pemeriksaan Tanda Vital','Kesehatan'),('Perawat','wajib','Pertolongan Pertama','Kesehatan'),('Perawat','opsional','Bantuan Hidup Dasar','Kesehatan'),('Perawat','opsional','Rekam Medis','Kesehatan'),('Perawat','opsional','Komunikasi','Umum'),
 ('Bidan','wajib','Asuhan Kebidanan','Kesehatan'),('Bidan','wajib','Pemeriksaan Kehamilan','Kesehatan'),('Bidan','wajib','Pertolongan Persalinan','Kesehatan'),('Bidan','opsional','Konseling Keluarga Berencana','Kesehatan'),('Bidan','opsional','Rekam Medis','Kesehatan'),
 ('Dokter Umum','wajib','Pemeriksaan Fisik','Kesehatan'),('Dokter Umum','wajib','Diagnosis Medis','Kesehatan'),('Dokter Umum','wajib','Penanganan Kegawatdaruratan','Kesehatan'),('Dokter Umum','opsional','Rekam Medis','Kesehatan'),('Dokter Umum','opsional','Bantuan Hidup Dasar','Kesehatan'),
 ('Apoteker','wajib','Pelayanan Kefarmasian','Kesehatan'),('Apoteker','wajib','Pengelolaan Obat','Kesehatan'),('Apoteker','wajib','Konseling Obat','Kesehatan'),('Apoteker','opsional','Farmakologi','Kesehatan'),('Apoteker','opsional','Komunikasi','Umum'),
 ('Asisten Apoteker','wajib','Peracikan Obat','Kesehatan'),('Asisten Apoteker','wajib','Pengelolaan Obat','Kesehatan'),('Asisten Apoteker','wajib','Pelayanan Kefarmasian','Kesehatan'),('Asisten Apoteker','opsional','Pengoperasian Mesin Kasir','Penjualan dan Ritel'),('Asisten Apoteker','opsional','Layanan Pelanggan','Layanan Pelanggan'),
 ('Analis Laboratorium Medis','wajib','Flebotomi','Kesehatan'),('Analis Laboratorium Medis','wajib','Pemeriksaan Laboratorium Klinik','Kesehatan'),('Analis Laboratorium Medis','wajib','Pengoperasian Alat Laboratorium','Kesehatan'),('Analis Laboratorium Medis','opsional','Pemantapan Mutu Laboratorium','Kesehatan'),('Analis Laboratorium Medis','opsional','K3 Dasar','Produksi'),
 ('Fisioterapis','wajib','Asesmen Fisioterapi','Kesehatan'),('Fisioterapis','wajib','Terapi Latihan','Kesehatan'),('Fisioterapis','wajib','Elektroterapi','Kesehatan'),('Fisioterapis','opsional','Rekam Medis','Kesehatan'),('Fisioterapis','opsional','Komunikasi','Umum'),
 ('Ahli Gizi','wajib','Asesmen Gizi','Kesehatan'),('Ahli Gizi','wajib','Penyusunan Menu Diet','Kesehatan'),('Ahli Gizi','wajib','Edukasi Gizi','Kesehatan'),('Ahli Gizi','opsional','Higiene Sanitasi Pangan','Perhotelan dan Kuliner'),('Ahli Gizi','opsional','Rekam Medis','Kesehatan'),
 ('Psikolog','wajib','Asesmen Psikologi','Kesehatan'),('Psikolog','wajib','Konseling','Kesehatan'),('Psikolog','wajib','Psikotes','Kesehatan'),('Psikolog','opsional','Rekrutmen dan Seleksi','Sumber Daya Manusia'),('Psikolog','opsional','Komunikasi','Umum'),
 ('Guru','wajib','Perencanaan Pembelajaran','Pendidikan'),('Guru','wajib','Pengelolaan Kelas','Pendidikan'),('Guru','wajib','Evaluasi Pembelajaran','Pendidikan'),('Guru','opsional','Media Pembelajaran Digital','Pendidikan'),('Guru','opsional','Microsoft PowerPoint','Perkantoran'),('Guru','opsional','Bahasa Inggris','Umum'),
 ('Guru PAUD','wajib','Pendidikan Anak Usia Dini','Pendidikan'),('Guru PAUD','wajib','Stimulasi Tumbuh Kembang Anak','Pendidikan'),('Guru PAUD','wajib','Pengelolaan Kelas','Pendidikan'),('Guru PAUD','opsional','Perencanaan Pembelajaran','Pendidikan'),('Guru PAUD','opsional','Komunikasi','Umum'),
 ('Kasir','wajib','Pengoperasian Mesin Kasir','Penjualan dan Ritel'),('Kasir','wajib','Penanganan Uang Tunai','Penjualan dan Ritel'),('Kasir','wajib','Layanan Pelanggan','Layanan Pelanggan'),('Kasir','opsional','Stock Opname','Logistik dan Transportasi'),('Kasir','opsional','Komunikasi','Umum'),
 ('Pramuniaga','wajib','Teknik Penjualan','Penjualan dan Ritel'),('Pramuniaga','wajib','Layanan Pelanggan','Layanan Pelanggan'),('Pramuniaga','wajib','Display Produk','Penjualan dan Ritel'),('Pramuniaga','opsional','Stock Opname','Logistik dan Transportasi'),('Pramuniaga','opsional','Komunikasi','Umum'),
 ('Sales Executive','wajib','Teknik Penjualan','Penjualan dan Ritel'),('Sales Executive','wajib','Negosiasi','Umum'),('Sales Executive','wajib','Prospecting Pelanggan','Penjualan dan Ritel'),('Sales Executive','opsional','CRM','Layanan Pelanggan'),('Sales Executive','opsional','Presentasi Produk','Penjualan dan Ritel'),('Sales Executive','opsional','Microsoft Excel','Perkantoran'),
 ('Telemarketing','wajib','Penjualan via Telepon','Penjualan dan Ritel'),('Telemarketing','wajib','Komunikasi','Umum'),('Telemarketing','wajib','Penanganan Keberatan Pelanggan','Penjualan dan Ritel'),('Telemarketing','opsional','CRM','Layanan Pelanggan'),('Telemarketing','opsional','Penanganan Keluhan','Layanan Pelanggan'),
 ('Kepala Toko','wajib','Manajemen Toko','Penjualan dan Ritel'),('Kepala Toko','wajib','Kepemimpinan Tim','Umum'),('Kepala Toko','wajib','Pengelolaan Stok','Logistik dan Transportasi'),('Kepala Toko','opsional','Laporan Penjualan','Penjualan dan Ritel'),('Kepala Toko','opsional','Microsoft Excel','Perkantoran'),
 ('Juru Masak','wajib','Teknik Memasak','Perhotelan dan Kuliner'),('Juru Masak','wajib','Higiene Sanitasi Pangan','Perhotelan dan Kuliner'),('Juru Masak','wajib','Persiapan Bahan Makanan','Perhotelan dan Kuliner'),('Juru Masak','opsional','Manajemen Dapur','Perhotelan dan Kuliner'),('Juru Masak','opsional','Pengelolaan Stok','Logistik dan Transportasi'),
 ('Chef','wajib','Teknik Memasak','Perhotelan dan Kuliner'),('Chef','wajib','Pengembangan Menu','Perhotelan dan Kuliner'),('Chef','wajib','Manajemen Dapur','Perhotelan dan Kuliner'),('Chef','wajib','Higiene Sanitasi Pangan','Perhotelan dan Kuliner'),('Chef','opsional','Food Costing','Perhotelan dan Kuliner'),('Chef','opsional','Kepemimpinan Tim','Umum'),
 ('Barista','wajib','Penyeduhan Kopi','Perhotelan dan Kuliner'),('Barista','wajib','Layanan Pelanggan','Layanan Pelanggan'),('Barista','wajib','Higiene Sanitasi Pangan','Perhotelan dan Kuliner'),('Barista','opsional','Latte Art','Perhotelan dan Kuliner'),('Barista','opsional','Pengoperasian Mesin Kasir','Penjualan dan Ritel'),
 ('Pramusaji','wajib','Pelayanan Makanan dan Minuman','Perhotelan dan Kuliner'),('Pramusaji','wajib','Layanan Pelanggan','Layanan Pelanggan'),('Pramusaji','wajib','Higiene Sanitasi Pangan','Perhotelan dan Kuliner'),('Pramusaji','opsional','Bahasa Inggris','Umum'),('Pramusaji','opsional','Pengoperasian Mesin Kasir','Penjualan dan Ritel'),
 ('Resepsionis','wajib','Pelayanan Front Office','Perhotelan dan Kuliner'),('Resepsionis','wajib','Reservasi','Perhotelan dan Kuliner'),('Resepsionis','wajib','Komunikasi','Umum'),('Resepsionis','opsional','Bahasa Inggris','Umum'),('Resepsionis','opsional','Penanganan Keluhan','Layanan Pelanggan'),
 ('Housekeeping','wajib','Tata Graha','Perhotelan dan Kuliner'),('Housekeeping','wajib','Teknik Pembersihan','Layanan Umum'),('Housekeeping','wajib','K3 Dasar','Produksi'),('Housekeeping','opsional','Laundry','Perhotelan dan Kuliner'),('Housekeeping','opsional','Bahasa Inggris','Umum'),
 ('Staf Tiket dan Travel','wajib','Reservasi','Perhotelan dan Kuliner'),('Staf Tiket dan Travel','wajib','Sistem Reservasi Tiket','Perhotelan dan Kuliner'),('Staf Tiket dan Travel','wajib','Layanan Pelanggan','Layanan Pelanggan'),('Staf Tiket dan Travel','opsional','Bahasa Inggris','Umum'),('Staf Tiket dan Travel','opsional','Pengetahuan Destinasi Wisata','Perhotelan dan Kuliner'),
 ('Petugas Kebersihan','wajib','Teknik Pembersihan','Layanan Umum'),('Petugas Kebersihan','wajib','Penggunaan Bahan Kimia Pembersih','Layanan Umum'),('Petugas Kebersihan','wajib','K3 Dasar','Produksi'),('Petugas Kebersihan','opsional','Manajemen Waktu','Umum'),
 ('Satpam','wajib','Patroli Keamanan','Keamanan'),('Satpam','wajib','Pengamanan Aset','Keamanan'),('Satpam','wajib','Penanganan Keadaan Darurat','Keamanan'),('Satpam','opsional','Pelaporan Insiden','Keamanan'),('Satpam','opsional','Komunikasi','Umum'),
 ('Terapis Kecantikan','wajib','Perawatan Wajah','Kecantikan'),('Terapis Kecantikan','wajib','Tata Rias Wajah','Kecantikan'),('Terapis Kecantikan','wajib','Higiene Salon','Kecantikan'),('Terapis Kecantikan','opsional','Penataan Rambut','Kecantikan'),('Terapis Kecantikan','opsional','Layanan Pelanggan','Layanan Pelanggan'),
 ('Pengemudi','wajib','Mengemudi Mobil','Logistik dan Transportasi'),('Pengemudi','wajib','Keselamatan Berkendara','Logistik dan Transportasi'),('Pengemudi','wajib','Pengetahuan Rute','Logistik dan Transportasi'),('Pengemudi','opsional','Perawatan Kendaraan Dasar','Otomotif'),('Pengemudi','opsional','Komunikasi','Umum'),
 ('Kurir','wajib','Mengemudi Sepeda Motor','Logistik dan Transportasi'),('Kurir','wajib','Pengetahuan Rute','Logistik dan Transportasi'),('Kurir','wajib','Serah Terima Paket','Logistik dan Transportasi'),('Kurir','opsional','Aplikasi Navigasi','Logistik dan Transportasi'),('Kurir','opsional','Layanan Pelanggan','Layanan Pelanggan'),
 ('Staf Gudang','wajib','Pengelolaan Stok','Logistik dan Transportasi'),('Staf Gudang','wajib','Penerimaan dan Pengiriman Barang','Logistik dan Transportasi'),('Staf Gudang','wajib','Stock Opname','Logistik dan Transportasi'),('Staf Gudang','opsional','Operasi Forklift','Produksi'),('Staf Gudang','opsional','Microsoft Excel','Perkantoran'),
 ('Staf Logistik','wajib','Manajemen Rantai Pasok','Logistik dan Transportasi'),('Staf Logistik','wajib','Dokumen Pengiriman','Logistik dan Transportasi'),('Staf Logistik','wajib','Pengelolaan Stok','Logistik dan Transportasi'),('Staf Logistik','opsional','Kepabeanan Ekspor Impor','Logistik dan Transportasi'),('Staf Logistik','opsional','Microsoft Excel','Perkantoran'),
 ('Staf Purchasing','wajib','Pengadaan Barang dan Jasa','Logistik dan Transportasi'),('Staf Purchasing','wajib','Negosiasi','Umum'),('Staf Purchasing','wajib','Evaluasi Pemasok','Logistik dan Transportasi'),('Staf Purchasing','opsional','Manajemen Rantai Pasok','Logistik dan Transportasi'),('Staf Purchasing','opsional','Microsoft Excel','Perkantoran'),
 ('Operator Forklift','wajib','Operasi Forklift','Produksi'),('Operator Forklift','wajib','Penerimaan dan Pengiriman Barang','Logistik dan Transportasi'),('Operator Forklift','wajib','K3 Dasar','Produksi'),('Operator Forklift','opsional','Pengelolaan Stok','Logistik dan Transportasi'),
 ('Operator Alat Berat','wajib','Pengoperasian Alat Berat','Teknik dan Konstruksi'),('Operator Alat Berat','wajib','Perawatan Alat Berat','Teknik dan Konstruksi'),('Operator Alat Berat','wajib','K3 Dasar','Produksi'),('Operator Alat Berat','opsional','Keselamatan Berkendara','Logistik dan Transportasi'),
 ('Teknisi Listrik','wajib','Instalasi Listrik','Teknik dan Konstruksi'),('Teknisi Listrik','wajib','Membaca Diagram Kelistrikan','Teknik dan Konstruksi'),('Teknisi Listrik','wajib','K3 Listrik','Teknik dan Konstruksi'),('Teknisi Listrik','opsional','Perawatan Panel Listrik','Teknik dan Konstruksi'),('Teknisi Listrik','opsional','PLC','Manufaktur'),
 ('Mekanik Otomotif','wajib','Servis Berkala Kendaraan','Otomotif'),('Mekanik Otomotif','wajib','Diagnosa Kerusakan Kendaraan','Otomotif'),('Mekanik Otomotif','wajib','Penggunaan Alat Bengkel','Otomotif'),('Mekanik Otomotif','opsional','Sistem Injeksi Kendaraan','Otomotif'),('Mekanik Otomotif','opsional','K3 Dasar','Produksi'),
 ('Teknisi Mesin','wajib','Perawatan Mesin Dasar','Produksi'),('Teknisi Mesin','wajib','Preventive Maintenance','Manufaktur'),('Teknisi Mesin','wajib','Membaca Gambar Teknik','Produksi'),('Teknisi Mesin','opsional','Sistem Pneumatik dan Hidrolik','Manufaktur'),('Teknisi Mesin','opsional','Pengelasan','Manufaktur'),('Teknisi Mesin','opsional','K3 Dasar','Produksi'),
 ('Teknisi AC','wajib','Instalasi AC','Teknik dan Konstruksi'),('Teknisi AC','wajib','Servis AC','Teknik dan Konstruksi'),('Teknisi AC','wajib','Sistem Refrigerasi','Teknik dan Konstruksi'),('Teknisi AC','opsional','Instalasi Listrik','Teknik dan Konstruksi'),('Teknisi AC','opsional','K3 Dasar','Produksi'),
 ('Juru Las','wajib','Pengelasan','Manufaktur'),('Juru Las','wajib','Membaca Gambar Teknik','Produksi'),('Juru Las','wajib','K3 Dasar','Produksi'),('Juru Las','opsional','Pengelasan SMAW','Manufaktur'),('Juru Las','opsional','Pengelasan GTAW','Manufaktur'),
 ('Operator Jahit','wajib','Menjahit','Manufaktur'),('Operator Jahit','wajib','Pengoperasian Mesin Jahit','Manufaktur'),('Operator Jahit','wajib','Pengendalian Kualitas','Produksi'),('Operator Jahit','opsional','Membuat Pola Pakaian','Manufaktur'),
 ('Quality Control','wajib','Pengendalian Kualitas','Produksi'),('Quality Control','wajib','Inspeksi Produk','Manufaktur'),('Quality Control','wajib','Penggunaan Alat Ukur','Manufaktur'),('Quality Control','opsional','ISO 9001','Manufaktur'),('Quality Control','opsional','Membaca Gambar Teknik','Produksi'),
 ('Staf PPIC','wajib','Perencanaan Produksi','Manufaktur'),('Staf PPIC','wajib','Pengelolaan Stok','Logistik dan Transportasi'),('Staf PPIC','wajib','Microsoft Excel','Perkantoran'),('Staf PPIC','opsional','ERP','Manufaktur'),('Staf PPIC','opsional','Manajemen Rantai Pasok','Logistik dan Transportasi'),
 ('HSE Officer','wajib','Sistem Manajemen K3','Manufaktur'),('HSE Officer','wajib','Identifikasi Bahaya dan Risiko','Manufaktur'),('HSE Officer','wajib','Investigasi Kecelakaan Kerja','Manufaktur'),('HSE Officer','opsional','K3 Dasar','Produksi'),('HSE Officer','opsional','ISO 45001','Manufaktur'),
 ('Insinyur Sipil','wajib','AutoCAD','Teknik dan Konstruksi'),('Insinyur Sipil','wajib','Perencanaan Struktur','Teknik dan Konstruksi'),('Insinyur Sipil','wajib','Manajemen Proyek Konstruksi','Teknik dan Konstruksi'),('Insinyur Sipil','opsional','Rencana Anggaran Biaya','Teknik dan Konstruksi'),('Insinyur Sipil','opsional','Pengawasan Lapangan','Teknik dan Konstruksi'),
 ('Drafter','wajib','AutoCAD','Teknik dan Konstruksi'),('Drafter','wajib','Membaca Gambar Teknik','Produksi'),('Drafter','wajib','Penggambaran Teknik','Teknik dan Konstruksi'),('Drafter','opsional','SketchUp','Teknik dan Konstruksi'),('Drafter','opsional','Revit','Teknik dan Konstruksi'),
 ('Pengawas Lapangan','wajib','Pengawasan Lapangan','Teknik dan Konstruksi'),('Pengawas Lapangan','wajib','Manajemen Proyek Konstruksi','Teknik dan Konstruksi'),('Pengawas Lapangan','wajib','K3 Konstruksi','Teknik dan Konstruksi'),('Pengawas Lapangan','opsional','Rencana Anggaran Biaya','Teknik dan Konstruksi'),('Pengawas Lapangan','opsional','Membaca Gambar Teknik','Produksi'),
 ('Staf HRD','wajib','Rekrutmen dan Seleksi','Sumber Daya Manusia'),('Staf HRD','wajib','Administrasi Kepegawaian','Sumber Daya Manusia'),('Staf HRD','wajib','Hukum Ketenagakerjaan','Sumber Daya Manusia'),('Staf HRD','opsional','Penggajian','Sumber Daya Manusia'),('Staf HRD','opsional','Pelatihan dan Pengembangan','Sumber Daya Manusia'),('Staf HRD','opsional','Microsoft Excel','Perkantoran'),
 ('Sekretaris','wajib','Pengelolaan Agenda','Perkantoran'),('Sekretaris','wajib','Korespondensi Bisnis','Perkantoran'),('Sekretaris','wajib','Notulensi Rapat','Perkantoran'),('Sekretaris','opsional','Microsoft Word','Perkantoran'),('Sekretaris','opsional','Pengarsipan','Perkantoran'),('Sekretaris','opsional','Bahasa Inggris','Umum'),
 ('Staf Legal','wajib','Penyusunan Kontrak','Hukum'),('Staf Legal','wajib','Riset Hukum','Hukum'),('Staf Legal','wajib','Kepatuhan Regulasi','Hukum'),('Staf Legal','opsional','Perizinan Usaha','Hukum'),('Staf Legal','opsional','Litigasi','Hukum'),
 ('Auditor','wajib','Audit Keuangan','Keuangan'),('Auditor','wajib','Standar Akuntansi Keuangan','Keuangan'),('Auditor','wajib','Laporan Keuangan','Keuangan'),('Auditor','opsional','Manajemen Risiko','Keuangan'),('Auditor','opsional','Microsoft Excel','Perkantoran'),
 ('Teller Bank','wajib','Transaksi Perbankan','Keuangan'),('Teller Bank','wajib','Penanganan Uang Tunai','Penjualan dan Ritel'),('Teller Bank','wajib','Layanan Pelanggan','Layanan Pelanggan'),('Teller Bank','opsional','Produk Perbankan','Keuangan'),('Teller Bank','opsional','Anti Pencucian Uang','Keuangan'),
 ('Analis Kredit','wajib','Analisis Kredit','Keuangan'),('Analis Kredit','wajib','Analisis Laporan Keuangan','Keuangan'),('Analis Kredit','wajib','Manajemen Risiko','Keuangan'),('Analis Kredit','opsional','Produk Perbankan','Keuangan'),('Analis Kredit','opsional','Negosiasi','Umum'),
 ('Staf Penagihan','wajib','Penagihan Piutang','Keuangan'),('Staf Penagihan','wajib','Negosiasi','Umum'),('Staf Penagihan','wajib','Komunikasi','Umum'),('Staf Penagihan','opsional','Penagihan dan Faktur','Keuangan'),('Staf Penagihan','opsional','Microsoft Excel','Perkantoran'),
 ('Agen Asuransi','wajib','Produk Asuransi','Keuangan'),('Agen Asuransi','wajib','Teknik Penjualan','Penjualan dan Ritel'),('Agen Asuransi','wajib','Prospecting Pelanggan','Penjualan dan Ritel'),('Agen Asuransi','opsional','Perencanaan Keuangan','Keuangan'),('Agen Asuransi','opsional','Negosiasi','Umum'),
 ('Desainer Grafis','wajib','Adobe Photoshop','Desain'),('Desainer Grafis','wajib','Adobe Illustrator','Desain'),('Desainer Grafis','wajib','Tipografi','Desain'),('Desainer Grafis','opsional','CorelDRAW','Desain'),('Desainer Grafis','opsional','Canva','Desain'),('Desainer Grafis','opsional','Desain Konten','Pemasaran Digital'),
 ('Video Editor','wajib','Editing Video','Kreatif dan Media'),('Video Editor','wajib','Adobe Premiere Pro','Kreatif dan Media'),('Video Editor','wajib','Penulisan Naskah Video','Kreatif dan Media'),('Video Editor','opsional','Adobe After Effects','Kreatif dan Media'),('Video Editor','opsional','Fotografi','Kreatif dan Media'),
 ('Penulis Konten','wajib','Copywriting','Pemasaran Digital'),('Penulis Konten','wajib','Penulisan Artikel','Kreatif dan Media'),('Penulis Konten','wajib','Riset Konten','Kreatif dan Media'),('Penulis Konten','opsional','SEO','Pemasaran Digital'),('Penulis Konten','opsional','Penyuntingan Naskah','Kreatif dan Media'),
 ('Public Relations','wajib','Hubungan Media','Pemasaran'),('Public Relations','wajib','Penulisan Siaran Pers','Pemasaran'),('Public Relations','wajib','Manajemen Acara','Pemasaran'),('Public Relations','opsional','Pemasaran Media Sosial','Pemasaran Digital'),('Public Relations','opsional','Copywriting','Pemasaran Digital'),
 ('Staf Agronomi','wajib','Budidaya Tanaman','Pertanian dan Peternakan'),('Staf Agronomi','wajib','Pemupukan','Pertanian dan Peternakan'),('Staf Agronomi','wajib','Pengendalian Hama dan Penyakit Tanaman','Pertanian dan Peternakan'),('Staf Agronomi','opsional','Pengoperasian Alat Pertanian','Pertanian dan Peternakan'),('Staf Agronomi','opsional','Microsoft Excel','Perkantoran'),
 ('Peternak','wajib','Pemeliharaan Ternak','Pertanian dan Peternakan'),('Peternak','wajib','Pemberian Pakan Ternak','Pertanian dan Peternakan'),('Peternak','wajib','Biosekuriti Kandang','Pertanian dan Peternakan'),('Peternak','opsional','Kesehatan Hewan Dasar','Pertanian dan Peternakan'),('Peternak','opsional','Pencatatan Hasil Produksi','Produksi');

-- CV and vacancy wording that should resolve to the seeded skills without the AI.
CREATE TEMP TABLE seed_alias(skill text, alias text) ON COMMIT DROP;
INSERT INTO seed_alias VALUES
 ('Asuhan Keperawatan','nursing care'),('Asuhan Keperawatan','askep'),('Asuhan Keperawatan','patient care'),
 ('Pemeriksaan Tanda Vital','vital signs'),('Pemeriksaan Tanda Vital','tanda vital'),('Pemeriksaan Tanda Vital','tanda-tanda vital'),
 ('Pertolongan Pertama','first aid'),('Pertolongan Pertama','p3k'),
 ('Bantuan Hidup Dasar','basic life support'),('Bantuan Hidup Dasar','bls'),
 ('Rekam Medis','medical record'),('Rekam Medis','rekam medik'),
 ('Asuhan Kebidanan','midwifery care'),('Pemeriksaan Kehamilan','antenatal care'),('Pemeriksaan Kehamilan','anc'),
 ('Pertolongan Persalinan','apn'),('Pertolongan Persalinan','asuhan persalinan normal'),
 ('Penanganan Kegawatdaruratan','emergency care'),('Penanganan Kegawatdaruratan','gawat darurat'),
 ('Pelayanan Kefarmasian','pharmaceutical care'),('Pengelolaan Obat','drug management'),('Flebotomi','phlebotomy'),
 ('Konseling','counseling'),('Konseling','counselling'),('Psikotes','psychological test'),
 ('Perencanaan Pembelajaran','rpp'),('Perencanaan Pembelajaran','lesson plan'),('Perencanaan Pembelajaran','modul ajar'),
 ('Pengelolaan Kelas','classroom management'),
 ('Pengoperasian Mesin Kasir','point of sale'),('Pengoperasian Mesin Kasir','mesin kasir'),
 ('Penanganan Uang Tunai','cash handling'),('Teknik Penjualan','selling skills'),('Teknik Penjualan','sales'),('Teknik Penjualan','penjualan'),
 ('Display Produk','visual merchandising'),('Negosiasi','negotiation'),('Prospecting Pelanggan','prospecting'),
 ('Penjualan via Telepon','telesales'),('Kepemimpinan Tim','leadership'),('Kepemimpinan Tim','kepemimpinan'),
 ('Pengelolaan Stok','inventory management'),('Pengelolaan Stok','inventory control'),('Pengelolaan Stok','manajemen stok'),
 ('Teknik Memasak','cooking'),('Teknik Memasak','memasak'),
 ('Higiene Sanitasi Pangan','food safety'),('Higiene Sanitasi Pangan','haccp'),('Higiene Sanitasi Pangan','food hygiene'),
 ('Persiapan Bahan Makanan','food preparation'),('Manajemen Dapur','kitchen management'),
 ('Penyeduhan Kopi','espresso'),('Penyeduhan Kopi','brewing'),('Penyeduhan Kopi','manual brew'),
 ('Pelayanan Makanan dan Minuman','food and beverage service'),('Pelayanan Makanan dan Minuman','f&b service'),
 ('Pelayanan Front Office','front office'),('Reservasi','reservation'),('Tata Graha','housekeeping'),('Teknik Pembersihan','cleaning'),
 ('Patroli Keamanan','security patrol'),('Patroli Keamanan','patroli'),
 ('Perawatan Wajah','facial'),('Tata Rias Wajah','makeup'),('Tata Rias Wajah','make up'),('Penataan Rambut','hair styling'),
 ('Mengemudi Mobil','sim a'),('Mengemudi Mobil','driving'),('Mengemudi Sepeda Motor','sim c'),
 ('Keselamatan Berkendara','defensive driving'),('Aplikasi Navigasi','google maps'),
 ('Penerimaan dan Pengiriman Barang','receiving'),('Penerimaan dan Pengiriman Barang','inbound outbound'),
 ('Manajemen Rantai Pasok','supply chain management'),('Manajemen Rantai Pasok','scm'),
 ('Pengadaan Barang dan Jasa','procurement'),('Pengadaan Barang dan Jasa','purchasing'),
 ('Pengoperasian Alat Berat','heavy equipment operation'),('Instalasi Listrik','electrical installation'),
 ('Diagnosa Kerusakan Kendaraan','troubleshooting kendaraan'),('Preventive Maintenance','perawatan preventif'),
 ('Sistem Refrigerasi','refrigeration'),('Pengelasan','welding'),('Pengelasan','las listrik'),
 ('Pengelasan SMAW','smaw'),('Pengelasan GTAW','gtaw'),('Pengelasan GTAW','tig welding'),
 ('Menjahit','sewing'),('Inspeksi Produk','inspection'),('Penggunaan Alat Ukur','measuring tools'),('Penggunaan Alat Ukur','jangka sorong'),
 ('Perencanaan Produksi','production planning'),('ERP','sap'),
 ('Sistem Manajemen K3','smk3'),('Sistem Manajemen K3','ohsms'),('Identifikasi Bahaya dan Risiko','hiradc'),('Identifikasi Bahaya dan Risiko','hira'),('Identifikasi Bahaya dan Risiko','jsa'),
 ('Rencana Anggaran Biaya','rab'),('Rencana Anggaran Biaya','cost estimation'),('Perencanaan Struktur','structural design'),
 ('Rekrutmen dan Seleksi','recruitment'),('Rekrutmen dan Seleksi','rekrutmen'),('Rekrutmen dan Seleksi','talent acquisition'),
 ('Penggajian','payroll'),('Hukum Ketenagakerjaan','labor law'),('Administrasi Kepegawaian','personnel administration'),
 ('Notulensi Rapat','minutes of meeting'),('Notulensi Rapat','notulen'),('Penyusunan Kontrak','contract drafting'),('Penyusunan Kontrak','legal drafting'),
 ('Audit Keuangan','financial audit'),('Standar Akuntansi Keuangan','psak'),('Standar Akuntansi Keuangan','ifrs'),
 ('Manajemen Risiko','risk management'),('Analisis Kredit','credit analysis'),('Penagihan Piutang','debt collection'),
 ('Anti Pencucian Uang','apu ppt'),('Anti Pencucian Uang','aml'),
 ('Adobe Photoshop','photoshop'),('Adobe Premiere Pro','premiere pro'),('Adobe After Effects','after effects'),('Editing Video','video editing'),
 ('Penulisan Artikel','article writing'),('Hubungan Media','media relations'),('Penulisan Siaran Pers','press release'),('Manajemen Acara','event management'),
 ('Budidaya Tanaman','crop cultivation'),('Pengendalian Hama dan Penyakit Tanaman','pest control');

INSERT INTO skills(id,nama,kategori)
SELECT DISTINCT ON (skill) gen_random_uuid(),skill,kategori FROM seed_requirement ORDER BY skill
ON CONFLICT (nama) DO NOTHING;
INSERT INTO skill_aliases(alias,skill_id)
SELECT a.alias,s.id FROM seed_alias a JOIN skills s ON s.nama=a.skill
ON CONFLICT (alias) DO NOTHING;

INSERT INTO occupations(id,nama,kode_kbji,bidang,deskripsi)
SELECT gen_random_uuid(),nama,kbji,bidang,deskripsi FROM seed_occupation
ON CONFLICT (nama) WHERE owner_id IS NULL DO NOTHING;
-- Requirements only for occupations that have none yet: an admin's existing
-- occupation with the same name keeps its own requirements.
INSERT INTO occupation_skills(occupation_id,skill_id,tipe)
SELECT o.id,s.id,r.tipe FROM seed_requirement r
JOIN occupations o ON o.nama=r.occupation AND o.owner_id IS NULL
JOIN skills s ON s.nama=r.skill
WHERE NOT EXISTS(SELECT 1 FROM occupation_skills x WHERE x.occupation_id=o.id)
ON CONFLICT (occupation_id,skill_id) DO NOTHING;
-- The queue_search trigger added the default keyword; give it the curated keyword and pattern.
UPDATE job_search_queries q SET keyword=s.keyword||' jobs in Jakarta',title_pattern=s.pattern
FROM seed_occupation s JOIN occupations o ON o.nama=s.nama AND o.owner_id IS NULL
WHERE q.occupation_id=o.id AND q.title_pattern IS NULL
 AND q.keyword=left(lower(o.nama),180)||' jobs in Jakarta'
 AND NOT EXISTS(SELECT 1 FROM job_search_queries d WHERE d.occupation_id=o.id AND d.id<>q.id AND d.keyword=s.keyword||' jobs in Jakarta');

-- Employer boards now classify every title against every pattern. The original patterns
-- only filtered their own keyword's results, so bare words such as "engineer", "data" or
-- "customer" would claim unrelated roles. Patterns an admin has since edited stay as they are.
UPDATE job_search_queries q SET title_pattern=s.pattern
FROM (VALUES
 ('Software Engineer','(?i)\b(software|engineer\w*|developer|programmer)\b','(?i)\b(software\s+(?:engineer\w*|develop\w*)|(?:back[ -]?end|front[ -]?end|full[ -]?stack|mobile|android|ios|application|java|golang|php|python|node\.?js|react|qa|test|automation|security|cyber\s*security|machine\s+learning|ml|ai|embedded|firmware|game)\s+(?:engineer\w*|develop\w*)|developer|programmer)\b'),
 ('Web Developer','(?i)\b(web|front[ -]?end|full[ -]?stack|developer|programmer|engineer|react\w*)\b','(?i)\b(web\s+(?:develop\w*|engineer\w*|programmer|master)|front[ -]?end|full[ -]?stack|wordpress\s+develop\w*)\b'),
 ('Data Analyst','(?i)\b(data|analis|analyst|analytics|bi)\b','(?i)\b(data\s+(?:analyst|analis|analytics|scien\w*|engineer\w*)|analis\s+data|business\s+intelligence|bi\s+(?:analyst|developer|engineer)|analytics\s+(?:engineer|analyst|specialist))\b'),
 ('UI/UX Designer','(?i)\b(ui|ux|designer|desainer)\b','(?i)\b(ui|ux|ux\s+research\w*|product\s+designer|user\s+experience|user\s+interface|interaction\s+designer)\b'),
 ('IT Support','(?i)\b(support|helpdesk|help desk|service desk|technician|teknisi)\b','(?i)\b(it\s+support|it\s+helpdesk|technical\s+support|helpdesk|help\s+desk|service\s+desk|desktop\s+support|it\s+technician|teknisi\s+(?:it|komputer|jaringan)|network\s+(?:engineer|administrator|technician)|system\s+administrator|sysadmin)\b'),
 ('Staf Administrasi','(?i)\b(admin\w*|administrasi|administration)\b','(?i)\b(admin|administrasi|administration|administrative|administrator)\b'),
 ('Staf Akuntansi','(?i)\b(account\w*|akunt\w*|finance|keuangan|pajak|tax)\b','(?i)\b(accounting|accountant|akuntan\w*|akuntansi|finance|keuangan|pajak|perpajakan|tax)\b'),
 ('Customer Service','(?i)\b(customer|pelanggan|call cent\w*|cs)\b','(?i)\b(customer\s+(?:service\w*|care|support|success|relation\w*|experience)|layanan\s+pelanggan|pelayanan\s+pelanggan|call\s+cent\w*|contact\s+cent\w*|cs)\b'),
 ('Digital Marketing','(?i)\b(marketing|pemasaran|seo|sem|ads|social media|content|digital)\b','(?i)\b(digital\s+marketing|marketing\s+digital|marketing|pemasaran|seo|sem|social\s+media\w*|content\s+marketing|performance\s+marketing|ads\s+specialist)\b'),
 ('Operator Produksi','(?i)\b(operator|produksi|production)\b','(?i)\b(operator\s+produksi|production\s+operator|operator\s+mesin|machine\s+operator|operator|produksi|production\s+(?:staff|crew|worker|leader|supervisor))\b'),
 ('DevOps Engineer','(?i)\b(devops|devsecops|sre|site reliability|cloud|platform|infra\w*)\b','(?i)\b(devops|devsecops|infraops|site\s+reliability|sre|cloud\s+(?:engineer\w*|architect\w*|ops)|platform\s+engineer\w*|infra\w*\s+engineer\w*)\b')
) s(nama,old,pattern) JOIN occupations o ON o.nama=s.nama AND o.owner_id IS NULL
WHERE q.occupation_id=o.id AND q.title_pattern=s.old;

-- Sources still scheduled on the old 6-12 hour interval run on the next scheduler tick and
-- then every JOB_SCRAPE_INTERVAL_SECONDS. The metered daily source keeps its slot.
ALTER TABLE job_sources NO FORCE ROW LEVEL SECURITY;
UPDATE job_sources SET next_run_at=now() WHERE key<>'openwebninja' AND next_run_at>now();
ALTER TABLE job_sources FORCE ROW LEVEL SECURITY;
