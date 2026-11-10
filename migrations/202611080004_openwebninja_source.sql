-- Google for Jobs aggregate via OpenWeb Ninja (API-key, quota billed, runs at most daily).
-- Rows accumulate across rotating queries; RLS hides rows not re-seen within 7 days.
INSERT INTO job_sources(key) VALUES ('openwebninja') ON CONFLICT (key) DO NOTHING;

-- Search queue: one keyword per occupation (job category). The importer takes the
-- rows that waited longest (last_run_at NULLS FIRST), OPENWEBNINJA_QUERIES_PER_RUN per day.
CREATE TABLE job_search_queries (
 id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
 occupation_id uuid NOT NULL REFERENCES occupations ON DELETE CASCADE,
 keyword text NOT NULL CHECK(length(keyword) BETWEEN 3 AND 200),
 -- Optional Rust-regex (case-insensitive via (?i)) a listing title must match.
 title_pattern text CHECK(title_pattern IS NULL OR length(title_pattern)<=500),
 enabled boolean NOT NULL DEFAULT true,
 last_run_at timestamptz, last_count int NOT NULL DEFAULT 0, zero_streak int NOT NULL DEFAULT 0,
 created_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(occupation_id,keyword)
);
CREATE INDEX job_search_queue ON job_search_queries(last_run_at NULLS FIRST,keyword) WHERE enabled;
-- Not FORCEd: the owner (migrations, new-occupation trigger) manages rows; others need grants.
ALTER TABLE job_search_queries ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON job_search_queries FROM PUBLIC,anon,authenticated;
GRANT SELECT, UPDATE(last_run_at,last_count,zero_streak) ON job_search_queries TO careerbridge_importer;
CREATE POLICY importer ON job_search_queries TO careerbridge_importer USING(true) WITH CHECK(true);

-- Keywords verified live on 2026-11-08 (country=id, language=id): each returned 10
-- Indonesian listings and every title matched its pattern. "<x> jobs in Indonesia" and
-- keywords without a city returned empty or randomly located results.
INSERT INTO job_search_queries(occupation_id,keyword,title_pattern)
SELECT o.id,s.keyword,s.pattern FROM (VALUES
 ('96f251a2-ee07-5722-9da1-5a2e83e3361d'::uuid,'data analyst jobs in Jakarta','(?i)\b(data|analis|analyst|analytics|bi)\b'),
 ('0d15f37c-81cc-5931-8a81-b11d9aed89d3'::uuid,'web developer jobs in Jakarta','(?i)\b(web|front[ -]?end|full[ -]?stack|developer|programmer|engineer|react\w*)\b'),
 ('9ad9988a-72e0-5a48-8127-fbea2c367f04'::uuid,'ui ux designer jobs in Jakarta','(?i)\b(ui|ux|designer|desainer)\b'),
 ('0fb07a11-91c3-5aa0-8634-47cc5c7a1c14'::uuid,'it support jobs in Jakarta','(?i)\b(support|helpdesk|help desk|service desk|technician|teknisi)\b'),
 ('e3e219c3-a08e-5c71-ad02-171a30f6ce33'::uuid,'staff administrasi jobs in Jakarta','(?i)\b(admin\w*|administrasi|administration)\b'),
 ('827bb1b7-cc92-584f-b31d-7dad1c782bd0'::uuid,'accounting staff jobs in Jakarta','(?i)\b(account\w*|akunt\w*|finance|keuangan|pajak|tax)\b'),
 ('fbba7075-e739-52f2-93f9-c0840ec26b1b'::uuid,'customer service jobs in Jakarta','(?i)\b(customer|pelanggan|call cent\w*|cs)\b'),
 ('c3510403-716d-5c72-bffb-c032c3275201'::uuid,'digital marketing jobs in Jakarta','(?i)\b(marketing|pemasaran|seo|sem|ads|social media|content|digital)\b'),
 ('e3987a45-424b-5b72-a762-6d8eb2b17fb8'::uuid,'operator produksi jobs in Bekasi','(?i)\b(operator|produksi|production)\b'),
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec4'::uuid,'software engineer jobs in Jakarta','(?i)\b(software|engineer\w*|developer|programmer)\b'),
 ('a9619ac0-8e03-4ddd-bc4f-b31bfb190ec5'::uuid,'devops engineer jobs in Jakarta','(?i)\b(devops|devsecops|sre|site reliability|cloud|platform|infra\w*)\b')
) s(occupation_id,keyword,pattern) JOIN occupations o ON o.id=s.occupation_id
ON CONFLICT (occupation_id,keyword) DO NOTHING;
-- Occupations added by admins before this migration get an unverified default keyword.
INSERT INTO job_search_queries(occupation_id,keyword)
SELECT id,left(lower(nama),180)||' jobs in Jakarta' FROM occupations o
WHERE NOT EXISTS(SELECT 1 FROM job_search_queries q WHERE q.occupation_id=o.id)
ON CONFLICT DO NOTHING;
-- ...and so does every occupation created later.
CREATE FUNCTION public.queue_occupation_search() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
 INSERT INTO public.job_search_queries(occupation_id,keyword)
 VALUES (NEW.id,left(lower(NEW.nama),180)||' jobs in Jakarta') ON CONFLICT DO NOTHING;
 RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION public.queue_occupation_search() FROM PUBLIC;
CREATE TRIGGER queue_search AFTER INSERT ON occupations FOR EACH ROW EXECUTE FUNCTION public.queue_occupation_search();

-- Vacancies carry their occupation; the importer resolves employer-feed titles by name.
ALTER TABLE job_vacancies ADD COLUMN occupation_id uuid REFERENCES occupations ON DELETE SET NULL;
CREATE INDEX vacancies_occupation ON job_vacancies(occupation_id) WHERE status='active';
GRANT SELECT(id,nama) ON occupations TO careerbridge_importer;
CREATE POLICY importer_read ON occupations FOR SELECT TO careerbridge_importer USING(true);
-- Backfill with the same title rules as feed_occupation() in jobs.rs. job_vacancies is
-- FORCE RLS; lift it only inside this migration transaction.
ALTER TABLE job_vacancies NO FORCE ROW LEVEL SECURITY;
UPDATE job_vacancies v SET occupation_id=o.id FROM occupations o
WHERE v.occupation_id IS NULL AND o.nama=CASE
 WHEN v.title ~* '(devops|devsecops|infraops|site reliability|\msre\M)' THEN 'DevOps Engineer'
 WHEN v.title ~* '(data[ -]?analyst|business intelligence analyst|\mbi analyst)' THEN 'Data Analyst'
 ELSE 'Software Engineer' END;
ALTER TABLE job_vacancies FORCE ROW LEVEL SECURITY;
