-- One login URL may select the existing restricted roles on session connections.
-- API/worker roles gain no owner membership and keep their existing RLS/grants.
DO $$
BEGIN
 IF current_user IN ('careerbridge_api', 'careerbridge_worker') THEN
  RAISE EXCEPTION 'Migrations require schema-owner login credentials';
 END IF;
 EXECUTE format('GRANT careerbridge_api, careerbridge_worker TO %I', current_user);
END $$;
