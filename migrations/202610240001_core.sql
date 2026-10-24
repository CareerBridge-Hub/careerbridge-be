-- Supabase may already host pgvector in its extensions schema.
SET LOCAL search_path TO public, extensions;
CREATE EXTENSION IF NOT EXISTS vector WITH SCHEMA public;
CREATE TABLE roles (id smallint PRIMARY KEY, kode text UNIQUE NOT NULL, nama text NOT NULL);
INSERT INTO roles VALUES (1,'user','Pencari Kerja'),(2,'admin','Admin');
CREATE TABLE provinces (id smallint PRIMARY KEY, nama text UNIQUE NOT NULL);
CREATE TABLE skills (
 id uuid PRIMARY KEY, nama text UNIQUE NOT NULL CHECK(length(nama) BETWEEN 1 AND 100), kategori text NOT NULL,
 embedding vector(384), embedding_model text, embedding_hash text, embedded_at timestamptz,
 updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE skill_aliases (alias text PRIMARY KEY, skill_id uuid NOT NULL REFERENCES skills ON DELETE CASCADE);
CREATE TABLE occupations (
 id uuid PRIMARY KEY, nama text UNIQUE NOT NULL, kode_kbji char(4) NOT NULL, bidang text NOT NULL, deskripsi text NOT NULL,
 embedding vector(384), embedding_model text, embedding_hash text, embedded_at timestamptz,
 updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE occupation_skills (
 occupation_id uuid REFERENCES occupations ON DELETE CASCADE, skill_id uuid REFERENCES skills,
 tipe text NOT NULL CHECK(tipe IN ('wajib','opsional')), PRIMARY KEY(occupation_id,skill_id)
);
CREATE TABLE institutions (id uuid PRIMARY KEY,nama text NOT NULL,jenis text NOT NULL CHECK(jenis IN ('LPK','BLK','Platform')),province_id smallint REFERENCES provinces, kota text NOT NULL,website text NOT NULL);
CREATE TABLE trainings (
 id uuid PRIMARY KEY,institution_id uuid NOT NULL REFERENCES institutions,nama text NOT NULL,biaya bigint NOT NULL CHECK(biaya>=0),moda text NOT NULL CHECK(moda IN ('daring','luring')),durasi text NOT NULL,url_daftar text NOT NULL,aktif boolean NOT NULL DEFAULT true,sample boolean NOT NULL DEFAULT true,
 embedding vector(384),embedding_model text,embedding_hash text,embedded_at timestamptz,updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE training_skills (training_id uuid REFERENCES trainings ON DELETE CASCADE,skill_id uuid REFERENCES skills,PRIMARY KEY(training_id,skill_id));
CREATE TABLE users (
 id uuid PRIMARY KEY REFERENCES auth.users(id) ON DELETE CASCADE,role_id smallint NOT NULL DEFAULT 1 REFERENCES roles,
 nama text NOT NULL CHECK(length(nama) BETWEEN 1 AND 100),pendidikan text NOT NULL CHECK(pendidikan IN ('SMA','SMK','D3','D4','S1','S2')),jurusan text NOT NULL DEFAULT '' CHECK(length(jurusan)<=100),province_id smallint NOT NULL REFERENCES provinces,kota text NOT NULL CHECK(length(kota) BETWEEN 1 AND 80),
 target_occupation_id uuid REFERENCES occupations ON DELETE SET NULL,persetujuan_cv_at timestamptz NOT NULL,aktif boolean NOT NULL DEFAULT true,
 revision bigint NOT NULL DEFAULT 1,created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now()
);
CREATE TABLE cv_uploads (
 id uuid PRIMARY KEY,user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,file_name text NOT NULL CHECK(length(file_name)<=255),ukuran_byte int NOT NULL CHECK(ukuran_byte BETWEEN 1 AND 2097152),file_hash char(64) NOT NULL,
 file_path text,parsed_json text,normalized_json jsonb,status text NOT NULL DEFAULT 'proses' CHECK(status IN ('proses','selesai','gagal')),step smallint NOT NULL DEFAULT 0,
 pesan_error text,model_ai text,attempts smallint NOT NULL DEFAULT 0,lease_until timestamptz,deleted_at timestamptz,
 created_at timestamptz NOT NULL DEFAULT now(),updated_at timestamptz NOT NULL DEFAULT now()
);
ALTER TABLE users ADD COLUMN current_cv_id uuid REFERENCES cv_uploads(id) ON DELETE SET NULL;
CREATE INDEX cv_queue ON cv_uploads(created_at) WHERE status='proses';
CREATE INDEX cv_owner ON cv_uploads(user_id,created_at DESC);
CREATE UNIQUE INDEX cv_dedup ON cv_uploads(user_id,file_hash) WHERE deleted_at IS NULL AND status IN ('proses','selesai');
CREATE TABLE user_skills (user_id uuid REFERENCES users ON DELETE CASCADE,skill_id uuid REFERENCES skills,sumber text NOT NULL CHECK(sumber IN ('manual','cv_alias','cv_semantik')),PRIMARY KEY(user_id,skill_id));
CREATE TABLE unmatched_skills (teks text PRIMARY KEY CHECK(length(teks)<=100),jumlah int NOT NULL DEFAULT 1,status text NOT NULL DEFAULT 'baru',skill_id uuid REFERENCES skills,updated_at timestamptz NOT NULL DEFAULT now());
CREATE TABLE analyses (user_id uuid REFERENCES users ON DELETE CASCADE,occupation_id uuid REFERENCES occupations ON DELETE CASCADE,revision_key text NOT NULL,result jsonb NOT NULL,created_at timestamptz NOT NULL DEFAULT now(),PRIMARY KEY(user_id,occupation_id));
CREATE TABLE catalog_revision (id boolean PRIMARY KEY DEFAULT true CHECK(id), revision bigint NOT NULL DEFAULT 1);
INSERT INTO catalog_revision DEFAULT VALUES;
CREATE FUNCTION bump_catalog() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN UPDATE catalog_revision SET revision=revision+1; RETURN NULL; END $$;
DO $$ DECLARE t text; BEGIN
 FOREACH t IN ARRAY ARRAY['skills','skill_aliases','occupations','occupation_skills','institutions','trainings','training_skills'] LOOP
 EXECUTE format('CREATE TRIGGER catalog_changed AFTER INSERT OR UPDATE OR DELETE ON %I FOR EACH STATEMENT EXECUTE FUNCTION bump_catalog()',t);
 END LOOP;
END $$;
CREATE FUNCTION guard_occupation_skills() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF EXISTS(SELECT 1 FROM occupations o WHERE NOT EXISTS(SELECT 1 FROM occupation_skills s WHERE s.occupation_id=o.id AND s.tipe='wajib')) THEN RAISE EXCEPTION 'An occupation requires at least one required skill'; END IF;
 RETURN NULL;
END $$;
CREATE CONSTRAINT TRIGGER required_skill AFTER INSERT OR UPDATE ON occupations DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION guard_occupation_skills();
CREATE CONSTRAINT TRIGGER required_skill AFTER DELETE OR UPDATE ON occupation_skills DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION guard_occupation_skills();

-- Roles receive passwords separately, never through committed migrations.
DO $$ BEGIN
 IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='careerbridge_api') THEN CREATE ROLE careerbridge_api LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS; END IF;
 IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='careerbridge_worker') THEN CREATE ROLE careerbridge_worker LOGIN NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS; END IF;
END $$;
GRANT USAGE ON SCHEMA public TO careerbridge_api,careerbridge_worker;
GRANT SELECT ON roles,provinces,skills,skill_aliases,occupations,occupation_skills,institutions,trainings,training_skills,catalog_revision TO careerbridge_api,careerbridge_worker;
GRANT SELECT,INSERT,UPDATE,DELETE ON users,user_skills,cv_uploads,analyses TO careerbridge_api;
GRANT SELECT,UPDATE ON users TO careerbridge_worker;
GRANT SELECT,UPDATE,DELETE ON cv_uploads TO careerbridge_worker;
GRANT SELECT,INSERT,UPDATE ON unmatched_skills TO careerbridge_worker;
GRANT UPDATE(embedding,embedding_model,embedding_hash,embedded_at) ON skills,occupations,trainings TO careerbridge_worker;
-- No public Data API access to application tables, including catalog metadata.
DO $$ DECLARE t text; BEGIN
 FOREACH t IN ARRAY ARRAY['roles','provinces','skills','skill_aliases','occupations','occupation_skills','institutions','trainings','training_skills','catalog_revision','unmatched_skills','users','user_skills','cv_uploads','analyses'] LOOP
 EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY',t);
 EXECUTE format('REVOKE ALL ON %I FROM anon,authenticated',t);
 IF t IN ('users','user_skills','cv_uploads','analyses') THEN
 EXECUTE format('CREATE POLICY owner ON %I TO careerbridge_api USING (%I = nullif(current_setting(''app.user_id'',true),'''')::uuid) WITH CHECK (%I = nullif(current_setting(''app.user_id'',true),'''')::uuid)',t,CASE WHEN t='users' THEN 'id' ELSE 'user_id' END,CASE WHEN t='users' THEN 'id' ELSE 'user_id' END);
 ELSE
 EXECUTE format('CREATE POLICY backend ON %I TO careerbridge_api,careerbridge_worker USING (true) WITH CHECK (true)',t);
 END IF;
 END LOOP;
END $$;
-- user-owned tables use user_id rather than their own primary ID.
DROP POLICY owner ON user_skills;
DROP POLICY owner ON cv_uploads;
DROP POLICY owner ON analyses;
CREATE POLICY owner ON user_skills TO careerbridge_api USING(user_id=nullif(current_setting('app.user_id',true),'')::uuid) WITH CHECK(user_id=nullif(current_setting('app.user_id',true),'')::uuid);
CREATE POLICY owner ON cv_uploads TO careerbridge_api USING(user_id=nullif(current_setting('app.user_id',true),'')::uuid) WITH CHECK(user_id=nullif(current_setting('app.user_id',true),'')::uuid);
CREATE POLICY owner ON analyses TO careerbridge_api USING(user_id=nullif(current_setting('app.user_id',true),'')::uuid) WITH CHECK(user_id=nullif(current_setting('app.user_id',true),'')::uuid);
CREATE POLICY worker ON users TO careerbridge_worker USING(true);
CREATE POLICY worker ON cv_uploads TO careerbridge_worker USING(true) WITH CHECK(true);

-- Content changes invalidate only affected vectors, including relationship changes.
CREATE FUNCTION invalidate_embedding() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF (to_jsonb(NEW)-ARRAY['embedding','embedding_model','embedding_hash','embedded_at','updated_at']) IS DISTINCT FROM (to_jsonb(OLD)-ARRAY['embedding','embedding_model','embedding_hash','embedded_at','updated_at']) THEN
 NEW.embedding=NULL; NEW.embedding_hash=NULL; NEW.updated_at=now();
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER invalidate_vector BEFORE UPDATE ON skills FOR EACH ROW EXECUTE FUNCTION invalidate_embedding();
CREATE TRIGGER invalidate_vector BEFORE UPDATE ON occupations FOR EACH ROW EXECUTE FUNCTION invalidate_embedding();
CREATE TRIGGER invalidate_vector BEFORE UPDATE ON trainings FOR EACH ROW EXECUTE FUNCTION invalidate_embedding();
CREATE FUNCTION invalidate_relations() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF TG_TABLE_NAME='skill_aliases' THEN
 UPDATE skills SET embedding=NULL,embedding_hash=NULL,updated_at=now() WHERE id IN (OLD.skill_id,NEW.skill_id);
 ELSIF TG_TABLE_NAME='occupation_skills' THEN
 UPDATE occupations SET embedding=NULL,embedding_hash=NULL,updated_at=now() WHERE id IN (OLD.occupation_id,NEW.occupation_id);
 ELSE
 UPDATE trainings SET embedding=NULL,embedding_hash=NULL,updated_at=now() WHERE id IN (OLD.training_id,NEW.training_id);
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER invalidate_parent AFTER INSERT OR UPDATE OR DELETE ON skill_aliases FOR EACH ROW EXECUTE FUNCTION invalidate_relations();
CREATE TRIGGER invalidate_parent AFTER INSERT OR UPDATE OR DELETE ON occupation_skills FOR EACH ROW EXECUTE FUNCTION invalidate_relations();
CREATE TRIGGER invalidate_parent AFTER INSERT OR UPDATE OR DELETE ON training_skills FOR EACH ROW EXECUTE FUNCTION invalidate_relations();
CREATE FUNCTION invalidate_skill_dependents() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.nama IS DISTINCT FROM OLD.nama THEN
 UPDATE occupations SET embedding=NULL,embedding_hash=NULL,updated_at=now() WHERE id IN (SELECT occupation_id FROM occupation_skills WHERE skill_id=NEW.id);
 UPDATE trainings SET embedding=NULL,embedding_hash=NULL,updated_at=now() WHERE id IN (SELECT training_id FROM training_skills WHERE skill_id=NEW.id);
 END IF;
 RETURN NULL;
END $$;
CREATE TRIGGER invalidate_dependents AFTER UPDATE ON skills FOR EACH ROW EXECUTE FUNCTION invalidate_skill_dependents();
-- Runtime code cannot assign admin roles or deactivate/reactivate profiles.
REVOKE UPDATE ON users FROM careerbridge_api,careerbridge_worker;
GRANT UPDATE(nama,pendidikan,jurusan,province_id,kota,target_occupation_id,current_cv_id,revision,updated_at) ON users TO careerbridge_api;
