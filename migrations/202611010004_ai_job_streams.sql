ALTER TABLE cv_uploads ADD COLUMN reviewed_at timestamptz;
ALTER TABLE cv_uploads ADD COLUMN processing_ms bigint;
UPDATE cv_uploads c SET reviewed_at=now() FROM users u WHERE u.current_cv_id=c.id;
CREATE TABLE analysis_jobs (
 id uuid PRIMARY KEY,user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
 occupation_id uuid NOT NULL REFERENCES occupations ON DELETE CASCADE,revision_key text NOT NULL,
 status text NOT NULL DEFAULT 'proses' CHECK(status IN ('proses','selesai','gagal')),
 step smallint NOT NULL DEFAULT 0,result jsonb,error text,
 attempts smallint NOT NULL DEFAULT 0,lease_until timestamptz,
 created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX analysis_job_reuse ON analysis_jobs(user_id,occupation_id,revision_key) WHERE status IN ('proses','selesai');
CREATE INDEX analysis_job_queue ON analysis_jobs(created_at) WHERE status='proses';
CREATE INDEX analysis_job_owner ON analysis_jobs(user_id,created_at DESC);
ALTER TABLE analysis_jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE analysis_jobs FORCE ROW LEVEL SECURITY;
REVOKE ALL ON analysis_jobs FROM PUBLIC,anon,authenticated;
GRANT SELECT,INSERT ON analysis_jobs TO careerbridge_api;
GRANT SELECT,UPDATE,DELETE ON analysis_jobs TO careerbridge_worker;
CREATE POLICY owner ON analysis_jobs TO careerbridge_api
 USING(user_id=nullif(current_setting('app.user_id',true),'')::uuid)
 WITH CHECK(user_id=nullif(current_setting('app.user_id',true),'')::uuid);
CREATE POLICY worker ON analysis_jobs TO careerbridge_worker USING(true) WITH CHECK(true);
-- Notifications contain only a random job ID and kind. All data reads still use RLS.
CREATE FUNCTION notify_ai_job() RETURNS trigger LANGUAGE plpgsql SET search_path=pg_catalog AS $$
BEGIN
 PERFORM pg_catalog.pg_notify('careerbridge_jobs',pg_catalog.json_build_object('kind',TG_ARGV[0],'id',NEW.id)::text);
 RETURN NEW;
END $$;
CREATE TRIGGER cv_progress_notify AFTER INSERT OR UPDATE OF status,step,deleted_at ON cv_uploads
 FOR EACH ROW EXECUTE FUNCTION notify_ai_job('cv');
CREATE TRIGGER analysis_progress_notify AFTER INSERT OR UPDATE OF status,step ON analysis_jobs
 FOR EACH ROW EXECUTE FUNCTION notify_ai_job('analysis');
