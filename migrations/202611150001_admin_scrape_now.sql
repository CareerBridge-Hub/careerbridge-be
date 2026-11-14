-- Admin "scrape now" button and demand-ranked search queue.
-- demand = active users who want this catalog occupation: their target occupation, or the
-- closest catalog occupation of their own AI-inferred occupation. Keyword sources search
-- these first, so a user recommended as Data Analyst gets "data analyst" searched first.
ALTER TABLE job_search_queries ADD COLUMN demand int NOT NULL DEFAULT 0;

-- Aggregates only (no user ids leave the function). SECURITY DEFINER: the importer cannot
-- read users. Returns the prioritized keywords for the admin UI.
CREATE FUNCTION public.refresh_search_demand() RETURNS jsonb
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog, public AS $$
DECLARE top jsonb;
BEGIN
 WITH wanted AS (
  SELECT u.id user_id, COALESCE(o.base_occupation_id,o.id) occupation_id
  FROM public.users u JOIN public.occupations o ON o.id=u.target_occupation_id WHERE u.aktif
  UNION
  SELECT o.owner_id, o.base_occupation_id
  FROM public.occupations o JOIN public.users u ON u.id=o.owner_id
  WHERE u.aktif AND o.base_occupation_id IS NOT NULL
 ), counts AS (SELECT occupation_id, count(*)::int n FROM wanted GROUP BY occupation_id)
 UPDATE public.job_search_queries q SET demand=COALESCE(c.n,0)
 FROM public.job_search_queries q2 LEFT JOIN counts c ON c.occupation_id=q2.occupation_id
 WHERE q.id=q2.id AND q.demand<>COALESCE(c.n,0);
 SELECT COALESCE(jsonb_agg(jsonb_build_object('occupation',t.nama,'keyword',t.keyword,'demand',t.demand) ORDER BY t.demand DESC,t.keyword),'[]')
 INTO top FROM (
  SELECT o.nama,q.keyword,q.demand FROM public.job_search_queries q JOIN public.occupations o ON o.id=q.occupation_id
  WHERE q.enabled AND q.demand>0 ORDER BY q.demand DESC,q.keyword LIMIT 20
 ) t;
 RETURN top;
END $$;
REVOKE ALL ON FUNCTION public.refresh_search_demand() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.refresh_search_demand() TO careerbridge_importer,careerbridge_api;

-- Admins may only make a source due now; the importer still owns leases and results.
-- requested_at is a per-source cooldown so repeated clicks add no load.
ALTER TABLE job_sources ADD COLUMN requested_at timestamptz;
GRANT UPDATE(next_run_at,requested_at) ON job_sources TO careerbridge_api;
CREATE POLICY source_admin_request ON job_sources FOR UPDATE TO careerbridge_api
 USING(public.is_catalog_admin()) WITH CHECK(public.is_catalog_admin());
