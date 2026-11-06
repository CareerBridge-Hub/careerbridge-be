-- Employer advertisements are separate from curated career-role/skill definitions.
DO $$ BEGIN
 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='careerbridge_importer') THEN
  CREATE ROLE careerbridge_importer NOLOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOINHERIT NOBYPASSRLS;
 END IF;
 EXECUTE format('GRANT careerbridge_importer TO %I', current_user);
END $$;
GRANT USAGE ON SCHEMA public TO careerbridge_importer;
CREATE TABLE job_sources (
 key text PRIMARY KEY,
 next_run_at timestamptz NOT NULL DEFAULT now(), lease_until timestamptz, run_id uuid,
 last_success_at timestamptz, last_error text, last_count int NOT NULL DEFAULT 0
);
INSERT INTO job_sources(key) VALUES ('grab'),('cermati'),('dkatalis'),('softwareseni'),('kredivo');
CREATE TABLE job_vacancies (
 id uuid PRIMARY KEY, source text NOT NULL REFERENCES job_sources(key), source_id text NOT NULL,
 company text NOT NULL CHECK(length(company) BETWEEN 1 AND 200),
 title text NOT NULL CHECK(length(title) BETWEEN 1 AND 300),
 country text NOT NULL CHECK(country='ID'), city text NOT NULL CHECK(length(city) BETWEEN 1 AND 200),
 region text NOT NULL DEFAULT '' CHECK(length(region)<=200),
 workplace text NOT NULL DEFAULT '' CHECK(length(workplace)<=80),
 employment_type text NOT NULL DEFAULT '' CHECK(length(employment_type)<=100),
 description text NOT NULL CHECK(length(description) BETWEEN 1 AND 100000),
 requirements text NOT NULL DEFAULT '' CHECK(length(requirements)<=100000),
 apply_url text NOT NULL CHECK(apply_url LIKE 'https://%'),
 source_url text NOT NULL CHECK(source_url LIKE 'https://%'),
 posted_at timestamptz, valid_until timestamptz,
 status text NOT NULL DEFAULT 'active' CHECK(status IN ('active','closed')),
 first_seen_at timestamptz NOT NULL DEFAULT now(), last_seen_at timestamptz NOT NULL DEFAULT now(),
 last_seen_run uuid NOT NULL, updated_at timestamptz NOT NULL DEFAULT now(),
 UNIQUE(source,source_id)
);
CREATE INDEX vacancies_page ON job_vacancies(posted_at DESC NULLS LAST,id) WHERE status='active';
CREATE INDEX vacancies_location ON job_vacancies(country,city,status);
CREATE INDEX vacancies_source ON job_vacancies(source,last_seen_run);
ALTER TABLE job_sources ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_sources FORCE ROW LEVEL SECURITY;
ALTER TABLE job_vacancies ENABLE ROW LEVEL SECURITY;
ALTER TABLE job_vacancies FORCE ROW LEVEL SECURITY;
REVOKE ALL ON job_sources,job_vacancies FROM PUBLIC,anon,authenticated;
GRANT SELECT,UPDATE ON job_sources TO careerbridge_importer;
GRANT SELECT,INSERT,UPDATE ON job_vacancies TO careerbridge_importer;
GRANT SELECT ON job_vacancies TO careerbridge_api;
CREATE POLICY importer ON job_sources TO careerbridge_importer USING(true) WITH CHECK(true);
CREATE POLICY importer ON job_vacancies TO careerbridge_importer USING(true) WITH CHECK(true);
CREATE POLICY published ON job_vacancies FOR SELECT TO careerbridge_api
 USING(status='active' AND last_seen_at>now()-interval '7 days' AND (valid_until IS NULL OR valid_until>now()));
