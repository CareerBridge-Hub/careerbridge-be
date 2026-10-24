ALTER ROLE careerbridge_api SET search_path TO public, extensions;
ALTER ROLE careerbridge_worker SET search_path TO public, extensions;
DO $$ BEGIN
 IF EXISTS(SELECT FROM pg_namespace WHERE nspname='extensions') THEN
  GRANT USAGE ON SCHEMA extensions TO careerbridge_api,careerbridge_worker;
 END IF;
END $$;
