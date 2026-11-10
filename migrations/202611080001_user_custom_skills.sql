CREATE TABLE user_custom_skills (
 user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 nama text NOT NULL CHECK(length(nama) BETWEEN 1 AND 80),
 created_at timestamptz NOT NULL DEFAULT now(),
 PRIMARY KEY(user_id,nama)
);
ALTER TABLE user_custom_skills ENABLE ROW LEVEL SECURITY;
REVOKE ALL ON user_custom_skills FROM anon,authenticated;
GRANT SELECT,INSERT,DELETE ON user_custom_skills TO careerbridge_api;
CREATE POLICY owner ON user_custom_skills TO careerbridge_api
 USING(user_id=nullif(current_setting('app.user_id',true),'')::uuid)
 WITH CHECK(user_id=nullif(current_setting('app.user_id',true),'')::uuid);
