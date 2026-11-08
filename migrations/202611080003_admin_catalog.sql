-- Database authorization uses the application profile, never a browser-supplied role.
CREATE FUNCTION public.is_catalog_admin() RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog, public AS $$
 SELECT EXISTS(SELECT 1 FROM public.users WHERE id=nullif(current_setting('app.user_id',true),'')::uuid AND aktif AND role_id=2)
$$;
REVOKE ALL ON FUNCTION public.is_catalog_admin() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.is_catalog_admin() TO careerbridge_api;

DO $$ DECLARE t text; BEGIN
 FOREACH t IN ARRAY ARRAY['skills','skill_aliases','occupations','occupation_skills','institutions','trainings','training_skills','catalog_revision'] LOOP
  EXECUTE format('DROP POLICY backend ON %I',t);
  EXECUTE format('CREATE POLICY catalog_read ON %I FOR SELECT TO careerbridge_api,careerbridge_worker USING(true)',t);
  EXECUTE format('CREATE POLICY catalog_admin ON %I TO careerbridge_api USING(public.is_catalog_admin()) WITH CHECK(public.is_catalog_admin())',t);
 END LOOP;
END $$;
CREATE POLICY worker_vector ON skills FOR UPDATE TO careerbridge_worker USING(true) WITH CHECK(true);
CREATE POLICY worker_vector ON occupations FOR UPDATE TO careerbridge_worker USING(true) WITH CHECK(true);
CREATE POLICY worker_vector ON trainings FOR UPDATE TO careerbridge_worker USING(true) WITH CHECK(true);
CREATE POLICY worker_revision ON catalog_revision FOR UPDATE TO careerbridge_worker USING(true) WITH CHECK(true);
GRANT INSERT,UPDATE,DELETE ON skills,skill_aliases,occupations,occupation_skills,institutions,trainings,training_skills TO careerbridge_api;
GRANT UPDATE(revision) ON catalog_revision TO careerbridge_api;

-- Explicit admin deletions remove references atomically; deferred required-skill
-- guards still reject deleting the last required skill of an occupation.
ALTER TABLE occupation_skills DROP CONSTRAINT occupation_skills_skill_id_fkey,
 ADD CONSTRAINT occupation_skills_skill_id_fkey FOREIGN KEY(skill_id) REFERENCES skills ON DELETE CASCADE;
ALTER TABLE training_skills DROP CONSTRAINT training_skills_skill_id_fkey,
 ADD CONSTRAINT training_skills_skill_id_fkey FOREIGN KEY(skill_id) REFERENCES skills ON DELETE CASCADE;
ALTER TABLE user_skills DROP CONSTRAINT user_skills_skill_id_fkey,
 ADD CONSTRAINT user_skills_skill_id_fkey FOREIGN KEY(skill_id) REFERENCES skills ON DELETE CASCADE;
ALTER TABLE unmatched_skills DROP CONSTRAINT unmatched_skills_skill_id_fkey,
 ADD CONSTRAINT unmatched_skills_skill_id_fkey FOREIGN KEY(skill_id) REFERENCES skills ON DELETE SET NULL;
ALTER TABLE trainings DROP CONSTRAINT trainings_institution_id_fkey,
 ADD CONSTRAINT trainings_institution_id_fkey FOREIGN KEY(institution_id) REFERENCES institutions ON DELETE CASCADE;
ALTER TABLE skillhub_training_history DROP CONSTRAINT skillhub_training_history_institution_id_fkey,
 ADD CONSTRAINT skillhub_training_history_institution_id_fkey FOREIGN KEY(institution_id) REFERENCES institutions ON DELETE CASCADE;

ALTER TABLE job_vacancies ADD COLUMN hidden boolean NOT NULL DEFAULT false;
DROP POLICY published ON job_vacancies;
CREATE POLICY published ON job_vacancies FOR SELECT TO careerbridge_api
 USING(NOT hidden AND status='active' AND last_seen_at>now()-interval '7 days' AND (valid_until IS NULL OR valid_until>now()));
CREATE POLICY vacancy_admin ON job_vacancies TO careerbridge_api
 USING(public.is_catalog_admin()) WITH CHECK(public.is_catalog_admin());
GRANT UPDATE(hidden) ON job_vacancies TO careerbridge_api;
GRANT SELECT ON job_sources TO careerbridge_api;
CREATE POLICY source_admin ON job_sources FOR SELECT TO careerbridge_api USING(public.is_catalog_admin());

CREATE TABLE admin_audit (
 id uuid PRIMARY KEY, actor_id uuid NOT NULL, action text NOT NULL,
 entity text NOT NULL, entity_ids uuid[] NOT NULL, created_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE admin_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE admin_audit FORCE ROW LEVEL SECURITY;
REVOKE ALL ON admin_audit FROM PUBLIC,anon,authenticated;
GRANT INSERT ON admin_audit TO careerbridge_api;
CREATE POLICY audit_admin ON admin_audit FOR INSERT TO careerbridge_api
 WITH CHECK(public.is_catalog_admin() AND actor_id=nullif(current_setting('app.user_id',true),'')::uuid);
