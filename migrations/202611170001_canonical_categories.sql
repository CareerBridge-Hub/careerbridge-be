-- Categories (occupations.bidang, skills.kategori) are free text written by admins and by the AI.
-- The filter lists DISTINCT values, so "teknologi  informasi" next to "Teknologi Informasi" showed
-- up as a second category. A category that already exists (ignoring case and extra spaces) is
-- now reused with its stored spelling; only a truly new name creates a new category.
CREATE FUNCTION public.category_text(text) RETURNS text LANGUAGE sql IMMUTABLE
AS $$ SELECT regexp_replace(btrim($1), '\s+', ' ', 'g') $$;

-- One-time cleanup: every variant takes the spelling most catalog rows already use.
WITH v AS (
  SELECT DISTINCT ON (lower(public.category_text(bidang))) lower(public.category_text(bidang)) k, public.category_text(bidang) b
  FROM occupations GROUP BY bidang
  ORDER BY lower(public.category_text(bidang)), count(*) FILTER (WHERE owner_id IS NULL) DESC, count(*) DESC, bidang
)
UPDATE occupations o SET bidang = v.b FROM v WHERE lower(public.category_text(o.bidang)) = v.k AND o.bidang <> v.b;

WITH v AS (
  SELECT DISTINCT ON (lower(public.category_text(kategori))) lower(public.category_text(kategori)) k, public.category_text(kategori) b
  FROM skills GROUP BY kategori
  ORDER BY lower(public.category_text(kategori)), count(*) DESC, kategori
)
UPDATE skills s SET kategori = v.b FROM v WHERE lower(public.category_text(s.kategori)) = v.k AND s.kategori <> v.b;

-- ponytail: lower() scan per write; fine for a catalog of hundreds/thousands rows, add an
-- expression index on lower(bidang)/lower(kategori) if these tables grow large.
CREATE FUNCTION public.canonical_bidang() RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
 NEW.bidang := public.category_text(NEW.bidang);
 NEW.bidang := coalesce((SELECT o.bidang FROM public.occupations o
   WHERE lower(o.bidang) = lower(NEW.bidang) AND o.id <> NEW.id
   ORDER BY (o.owner_id IS NULL) DESC LIMIT 1), NEW.bidang);
 RETURN NEW;
END $$;
CREATE TRIGGER canonical_bidang BEFORE INSERT OR UPDATE OF bidang ON occupations
 FOR EACH ROW EXECUTE FUNCTION public.canonical_bidang();

CREATE FUNCTION public.canonical_kategori() RETURNS trigger LANGUAGE plpgsql SET search_path = pg_catalog, public AS $$
BEGIN
 NEW.kategori := public.category_text(NEW.kategori);
 NEW.kategori := coalesce((SELECT s.kategori FROM public.skills s
   WHERE lower(s.kategori) = lower(NEW.kategori) AND s.id <> NEW.id LIMIT 1), NEW.kategori);
 RETURN NEW;
END $$;
CREATE TRIGGER canonical_kategori BEFORE INSERT OR UPDATE OF kategori ON skills
 FOR EACH ROW EXECUTE FUNCTION public.canonical_kategori();
