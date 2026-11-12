-- AI-inferred occupations owned by one user. owner_id NULL = admin catalog.
-- Owned rows reuse the occupation pipeline (scoring, training, analysis) but stay
-- invisible to other users, admins' catalog editing, the importer and the job-search queue.
ALTER TABLE occupations
 ADD COLUMN owner_id uuid REFERENCES users ON DELETE CASCADE,
 -- Closest catalog occupation, used to recommend vacancies for an owned occupation.
 ADD COLUMN base_occupation_id uuid REFERENCES occupations ON DELETE SET NULL,
 ALTER COLUMN kode_kbji DROP NOT NULL,
 ADD CONSTRAINT occupations_kbji_catalog CHECK(owner_id IS NOT NULL OR kode_kbji IS NOT NULL),
 ADD CONSTRAINT occupations_base_owned CHECK(owner_id IS NOT NULL OR base_occupation_id IS NULL);
ALTER TABLE occupations DROP CONSTRAINT occupations_nama_key;
CREATE UNIQUE INDEX occupations_catalog_name ON occupations(nama) WHERE owner_id IS NULL;
-- One AI occupation per user; a newer confirmed CV replaces it in place.
CREATE UNIQUE INDEX occupations_one_per_owner ON occupations(owner_id) WHERE owner_id IS NOT NULL;

DROP POLICY catalog_read ON occupations;
DROP POLICY catalog_admin ON occupations;
DROP POLICY worker_vector ON occupations;
DROP POLICY importer_read ON occupations;
CREATE POLICY catalog_read ON occupations FOR SELECT TO careerbridge_api
 USING(owner_id IS NULL OR owner_id=nullif(current_setting('app.user_id',true),'')::uuid);
CREATE POLICY worker_read ON occupations FOR SELECT TO careerbridge_worker USING(owner_id IS NULL);
CREATE POLICY worker_vector ON occupations FOR UPDATE TO careerbridge_worker USING(owner_id IS NULL) WITH CHECK(owner_id IS NULL);
CREATE POLICY importer_read ON occupations FOR SELECT TO careerbridge_importer USING(owner_id IS NULL);
CREATE POLICY catalog_admin ON occupations TO careerbridge_api
 USING(public.is_catalog_admin() AND owner_id IS NULL) WITH CHECK(public.is_catalog_admin() AND owner_id IS NULL);
CREATE POLICY owner ON occupations TO careerbridge_api
 USING(owner_id=nullif(current_setting('app.user_id',true),'')::uuid)
 WITH CHECK(owner_id=nullif(current_setting('app.user_id',true),'')::uuid);

-- Requirement rows follow their occupation's visibility (the subquery is itself under RLS).
DROP POLICY catalog_read ON occupation_skills;
DROP POLICY catalog_admin ON occupation_skills;
CREATE POLICY catalog_read ON occupation_skills FOR SELECT TO careerbridge_api,careerbridge_worker
 USING(EXISTS(SELECT 1 FROM occupations o WHERE o.id=occupation_id));
CREATE POLICY catalog_admin ON occupation_skills TO careerbridge_api
 USING(public.is_catalog_admin() AND EXISTS(SELECT 1 FROM occupations o WHERE o.id=occupation_id AND o.owner_id IS NULL))
 WITH CHECK(public.is_catalog_admin() AND EXISTS(SELECT 1 FROM occupations o WHERE o.id=occupation_id AND o.owner_id IS NULL));
CREATE POLICY owner ON occupation_skills TO careerbridge_api
 USING(EXISTS(SELECT 1 FROM occupations o WHERE o.id=occupation_id AND o.owner_id=nullif(current_setting('app.user_id',true),'')::uuid))
 WITH CHECK(EXISTS(SELECT 1 FROM occupations o WHERE o.id=occupation_id AND o.owner_id=nullif(current_setting('app.user_id',true),'')::uuid));

-- Owned occupations never spend the metered job-search quota.
CREATE OR REPLACE FUNCTION public.queue_occupation_search() RETURNS trigger
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
BEGIN
 IF NEW.owner_id IS NULL THEN
  INSERT INTO public.job_search_queries(occupation_id,keyword)
  VALUES (NEW.id,left(lower(NEW.nama),180)||' jobs in Jakarta') ON CONFLICT DO NOTHING;
 END IF;
 RETURN NULL;
END $$;
