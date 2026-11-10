-- Add software job families to the occupation catalog so they appear in
-- Pekerjaan, can be scored against a user's skills, and can recommend training.
INSERT INTO occupations(id,nama,kode_kbji,bidang,deskripsi) VALUES
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','Software Engineer','2512','Teknologi Informasi','Merancang, membangun, menguji, dan memelihara perangkat lunak.'),
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5','DevOps Engineer','2523','Teknologi Informasi','Mengotomatisasi proses build, deployment, infrastruktur, dan pemantauan aplikasi.')
ON CONFLICT (id) DO NOTHING;

-- Reuse catalog skills already present; these are baseline indicators rather
-- than an exhaustive industry competency framework.
INSERT INTO occupation_skills(occupation_id,skill_id,tipe) VALUES
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','19273b8c-10a1-51f5-9984-2ed6b16bb9f8','wajib'), -- JavaScript
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','38509d8d-92c3-5c63-8627-4e4832834da8','wajib'), -- Git
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','957cb236-3563-5f88-b0d4-1fc57c051c99','wajib'), -- REST API
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','6a630561-2e6d-5665-a0b0-1b94f1576cf1','opsional'), -- Python
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','2a33bd32-6718-5991-a033-e7dd84e22587','opsional'), -- SQL
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','b04e1048-78e7-5fe6-94c3-02714d07c544','opsional'), -- React
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4','3c1c1e28-36ac-5858-8491-3afa85460f25','opsional'), -- Node.js
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5','38509d8d-92c3-5c63-8627-4e4832834da8','wajib'), -- Git
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5','5791db62-6c8f-5089-a459-de5b42271e82','wajib'), -- OS installation
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5','f3249580-5ed9-51f6-9071-a5777467d7ef','wajib'), -- Networking
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5','cea20c25-2de6-596f-bbc8-0a9403c5850c','opsional'), -- Network security
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5','6a630561-2e6d-5665-a0b0-1b94f1576cf1','opsional'), -- Python
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5','957cb236-3563-5f88-b0d4-1fc57c051c99','opsional') -- REST API
ON CONFLICT (occupation_id,skill_id) DO NOTHING;
