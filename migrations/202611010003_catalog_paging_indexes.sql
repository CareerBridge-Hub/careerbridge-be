-- Support deterministic page ordering and bounded skill-link lookups.
CREATE INDEX IF NOT EXISTS skills_name_page_idx ON skills(nama,id);
CREATE INDEX IF NOT EXISTS occupations_name_page_idx ON occupations(nama,id);
CREATE INDEX IF NOT EXISTS occupations_field_page_idx ON occupations(bidang,nama,id);
CREATE INDEX IF NOT EXISTS institutions_name_page_idx ON institutions(nama,id);
CREATE INDEX IF NOT EXISTS institutions_province_page_idx ON institutions(province_id,nama,id);
CREATE INDEX IF NOT EXISTS trainings_active_name_page_idx ON trainings(nama,id) WHERE aktif;
CREATE INDEX IF NOT EXISTS trainings_active_mode_page_idx ON trainings(moda,nama,id) WHERE aktif;
CREATE INDEX IF NOT EXISTS trainings_active_free_page_idx ON trainings(nama,id) WHERE aktif AND biaya=0;
CREATE INDEX IF NOT EXISTS training_skills_skill_training_idx ON training_skills(skill_id,training_id);
