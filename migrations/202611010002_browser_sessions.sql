-- Only the holder of the random session secret may access a session row.
-- No Auth/Storage managed tables are modified.
CREATE TABLE browser_sessions (
 secret_hash char(64) PRIMARY KEY,
 user_id uuid NOT NULL REFERENCES auth.users(id) ON DELETE CASCADE,
 encrypted_tokens text NOT NULL,
 expires_at timestamptz NOT NULL DEFAULT now() + interval '7 days'
);
ALTER TABLE browser_sessions ENABLE ROW LEVEL SECURITY;
ALTER TABLE browser_sessions FORCE ROW LEVEL SECURITY;
REVOKE ALL ON browser_sessions FROM PUBLIC, anon, authenticated;
GRANT SELECT, INSERT, UPDATE, DELETE ON browser_sessions TO careerbridge_api;
CREATE POLICY session_holder ON browser_sessions TO careerbridge_api
 USING (secret_hash = nullif(current_setting('app.session_hash', true), ''))
 WITH CHECK (secret_hash = nullif(current_setting('app.session_hash', true), ''));
-- Worker can only remove expired sessions; it cannot read tokens.
GRANT DELETE, SELECT(expires_at) ON browser_sessions TO careerbridge_worker;
CREATE POLICY expired_session_read ON browser_sessions FOR SELECT TO careerbridge_worker
 USING (expires_at <= now());
CREATE POLICY expired_session_cleanup ON browser_sessions FOR DELETE TO careerbridge_worker
 USING (expires_at <= now());

-- A verified user may revoke all of their sessions after changing a password.
CREATE POLICY own_session_read ON browser_sessions FOR SELECT TO careerbridge_api
 USING (user_id = nullif(current_setting('app.user_id', true), '')::uuid);
CREATE POLICY own_session_revoke ON browser_sessions FOR DELETE TO careerbridge_api
 USING (user_id = nullif(current_setting('app.user_id', true), '')::uuid);
