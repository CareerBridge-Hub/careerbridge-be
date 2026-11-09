-- Catalog skills mentioned in a vacancy's text; written by the importer, read by the API.
CREATE TABLE vacancy_skills (
 vacancy_id uuid NOT NULL REFERENCES job_vacancies ON DELETE CASCADE,
 skill_id uuid NOT NULL REFERENCES skills ON DELETE CASCADE,
 PRIMARY KEY(vacancy_id,skill_id)
);
CREATE INDEX vacancy_skills_skill ON vacancy_skills(skill_id);
ALTER TABLE vacancy_skills ENABLE ROW LEVEL SECURITY;
ALTER TABLE vacancy_skills FORCE ROW LEVEL SECURITY;
REVOKE ALL ON vacancy_skills FROM PUBLIC,anon,authenticated;
GRANT SELECT,INSERT,DELETE ON vacancy_skills TO careerbridge_importer;
GRANT SELECT ON vacancy_skills TO careerbridge_api;
CREATE POLICY importer ON vacancy_skills TO careerbridge_importer USING(true) WITH CHECK(true);
-- Visibility follows job_vacancies RLS through the join in the vacancy page query.
CREATE POLICY api_read ON vacancy_skills FOR SELECT TO careerbridge_api USING(true);
-- The importer reads the alias index to tag vacancies.
GRANT SELECT ON skills,skill_aliases TO careerbridge_importer;
CREATE POLICY importer_read ON skills FOR SELECT TO careerbridge_importer USING(true);
CREATE POLICY importer_read ON skill_aliases FOR SELECT TO careerbridge_importer USING(true);
