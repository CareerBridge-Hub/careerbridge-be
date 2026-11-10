-- Preserve provenance and unknown facts when importing the public Skillhub catalog.
ALTER TABLE institutions ADD COLUMN source text;
ALTER TABLE institutions ADD COLUMN source_id text;
ALTER TABLE institutions ADD COLUMN source_url text;
ALTER TABLE institutions ADD COLUMN imported_at timestamptz;
CREATE UNIQUE INDEX institution_source ON institutions(source,source_id);
ALTER TABLE trainings ALTER COLUMN biaya DROP NOT NULL;
ALTER TABLE trainings ALTER COLUMN moda DROP NOT NULL;
ALTER TABLE trainings ADD COLUMN source text;
ALTER TABLE trainings ADD COLUMN source_id text;
ALTER TABLE trainings ADD COLUMN source_kind text;
ALTER TABLE trainings ADD COLUMN source_url text;
ALTER TABLE trainings ADD COLUMN imported_at timestamptz;
CREATE UNIQUE INDEX training_source ON trainings(source,source_kind,source_id,institution_id);
-- Historical realization records are not enrollment opportunities.
CREATE TABLE skillhub_training_history (
 source_id uuid PRIMARY KEY,
 institution_id uuid NOT NULL REFERENCES institutions(id),
 payload jsonb NOT NULL,
 imported_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE skillhub_training_history ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON skillhub_training_history FROM PUBLIC,anon,authenticated;
GRANT SELECT ON skillhub_training_history TO careerbridge_api,careerbridge_worker;
CREATE POLICY backend ON skillhub_training_history TO careerbridge_api,careerbridge_worker USING(true);
