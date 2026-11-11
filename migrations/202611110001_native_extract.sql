-- Preserve the Python importer keys and IDs. All inserts tolerate existing imports.
INSERT INTO job_sources(key) VALUES
 ('kalibrr'),('techinasia'),('jobstreet'),('glints'),('dealls'),('linkedin'),
 ('greenhouse-xendit'),('lever-ninjavan'),('lever-lalamove'),
 ('workable-amartha'),('workable-ajaib'),('workable-fundingsocieties'),('skillhub')
 ON CONFLICT(key) DO NOTHING;

ALTER TABLE job_vacancies ADD COLUMN detail_complete boolean NOT NULL DEFAULT true;
ALTER TABLE job_vacancies ADD COLUMN detail_attempts integer NOT NULL DEFAULT 0;
ALTER TABLE job_vacancies ADD COLUMN detail_next_attempt_at timestamptz NOT NULL DEFAULT now();
UPDATE job_vacancies SET detail_complete=false
 WHERE source='linkedin' AND position('Deskripsi lengkap:' in description)>0;
CREATE INDEX linkedin_detail_queue ON job_vacancies(detail_next_attempt_at,first_seen_at)
 WHERE source='linkedin' AND NOT detail_complete AND status='active';

-- Each site has its own search rotation; never consume OpenWebNinja's metered queue.
CREATE TABLE extract_query_runs (
 source text NOT NULL REFERENCES job_sources(key),
 query_id uuid NOT NULL REFERENCES job_search_queries(id) ON DELETE CASCADE,
 last_run_at timestamptz NOT NULL, last_count integer NOT NULL DEFAULT 0,
 PRIMARY KEY(source,query_id)
);
ALTER TABLE extract_query_runs ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON extract_query_runs FROM PUBLIC,anon,authenticated;
GRANT SELECT,INSERT,UPDATE ON extract_query_runs TO careerbridge_importer;
CREATE POLICY importer ON extract_query_runs TO careerbridge_importer USING(true) WITH CHECK(true);

DO $$ BEGIN
 IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='careerbridge_catalog_importer') THEN
  CREATE ROLE careerbridge_catalog_importer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
 END IF;
 EXECUTE format('GRANT careerbridge_catalog_importer TO %I',current_user);
 EXECUTE format('GRANT TEMP ON DATABASE %I TO careerbridge_catalog_importer',current_database());
END $$;
GRANT USAGE ON SCHEMA public TO careerbridge_catalog_importer;
GRANT SELECT ON provinces,skills,skill_aliases,catalog_revision TO careerbridge_catalog_importer;
GRANT UPDATE(revision) ON catalog_revision TO careerbridge_catalog_importer;
GRANT SELECT,INSERT,UPDATE ON institutions,trainings,skillhub_training_history TO careerbridge_catalog_importer;
GRANT SELECT,INSERT,DELETE ON training_skills TO careerbridge_catalog_importer;
GRANT SELECT,UPDATE ON job_sources TO careerbridge_catalog_importer;
CREATE POLICY catalog_importer ON skills FOR SELECT TO careerbridge_catalog_importer USING(true);
CREATE POLICY catalog_importer ON skill_aliases FOR SELECT TO careerbridge_catalog_importer USING(true);
CREATE POLICY catalog_importer ON provinces FOR SELECT TO careerbridge_catalog_importer USING(true);
CREATE POLICY catalog_importer ON catalog_revision TO careerbridge_catalog_importer USING(true) WITH CHECK(true);
CREATE POLICY catalog_importer ON institutions TO careerbridge_catalog_importer
 USING(source='skillhub') WITH CHECK(source='skillhub');
CREATE POLICY catalog_importer ON trainings TO careerbridge_catalog_importer
 USING(source='skillhub') WITH CHECK(source='skillhub');
CREATE POLICY catalog_importer ON training_skills TO careerbridge_catalog_importer
 USING(EXISTS(SELECT 1 FROM trainings t WHERE t.id=training_id AND t.source='skillhub'))
 WITH CHECK(EXISTS(SELECT 1 FROM trainings t WHERE t.id=training_id AND t.source='skillhub'));
CREATE POLICY catalog_importer ON skillhub_training_history TO careerbridge_catalog_importer
 USING(EXISTS(SELECT 1 FROM institutions i WHERE i.id=institution_id AND i.source='skillhub'))
 WITH CHECK(EXISTS(SELECT 1 FROM institutions i WHERE i.id=institution_id AND i.source='skillhub'));
CREATE POLICY catalog_importer ON job_sources TO careerbridge_catalog_importer
 USING(key='skillhub') WITH CHECK(key='skillhub');

-- Keep the complete LPK profile and all detail tabs, including infrastructure,
-- equipment and accreditation, without misrepresenting history as enrollment.
CREATE TABLE skillhub_institution_details (
 institution_id uuid PRIMARY KEY REFERENCES institutions(id),
 payload jsonb NOT NULL, imported_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE skillhub_institution_details ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON skillhub_institution_details FROM PUBLIC,anon,authenticated;
GRANT SELECT,INSERT,UPDATE ON skillhub_institution_details TO careerbridge_catalog_importer;
CREATE POLICY catalog_importer ON skillhub_institution_details TO careerbridge_catalog_importer
 USING(EXISTS(SELECT 1 FROM institutions i WHERE i.id=institution_id AND i.source='skillhub'))
 WITH CHECK(EXISTS(SELECT 1 FROM institutions i WHERE i.id=institution_id AND i.source='skillhub'));

-- Optional sources from the extractor remain opt-in via JOB_SCRAPE_SOURCES.
INSERT INTO job_sources(key) VALUES
 ('kalibrr'),
 ('techinasia'),
 ('jobstreet'),
 ('glints'),
 ('dealls'),
 ('linkedin'),
 ('greenhouse-xendit'),
 ('lever-ninjavan'),
 ('lever-lalamove'),
 ('workable-amartha'),
 ('workable-ajaib'),
 ('workable-fundingsocieties'),
 ('kitalulus'),
 ('karir'),
 ('indeed'),
 ('glassdoor'),
 ('remotive'),
 ('remoteok'),
 ('arbeitnow'),
 ('hackernews'),
 ('adzuna'),
 ('greenhouse-binance'),
 ('greenhouse-okx'),
 ('greenhouse-datadog'),
 ('greenhouse-airbnb'),
 ('greenhouse-stripe'),
 ('greenhouse-gitlab'),
 ('lever-nium'),
 ('lever-dlocal'),
 ('lever-palantir'),
 ('lever-spotify'),
 ('ashby-airwallex'),
 ('ashby-notion'),
 ('ashby-openai'),
 ('ashby-ramp'),
 ('smartrecruiters-alodokter'),
 ('smartrecruiters-stockbit'),
 ('smartrecruiters-bukuwarung'),
 ('smartrecruiters-sayurbox'),
 ('smartrecruiters-dekoruma'),
 ('smartrecruiters-julo'),
 ('smartrecruiters-pasarpolis')
 ON CONFLICT(key) DO NOTHING;

-- Updated legacy cron scripts refuse writes once Rust takes over a source.
ALTER TABLE job_sources ADD COLUMN managed_by text NOT NULL DEFAULT 'legacy'
 CHECK(managed_by IN ('legacy','rust'));
