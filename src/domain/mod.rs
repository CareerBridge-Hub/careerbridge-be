//! Pure business rules. No HTTP, database, or provider dependencies.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use unicode_normalization::{UnicodeNormalization, char::is_combining_mark};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: Uuid,
    pub nama: String,
    pub kategori: String,
    pub alias: Vec<String>,
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobSkill {
    pub skill_id: Uuid,
    #[schema(value_type = SkillRequirement)]
    pub tipe: String,
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Occupation {
    pub id: Uuid,
    pub nama: String,
    pub kode_kbji: String,
    pub bidang: String,
    pub deskripsi: String,
    pub skills: Vec<JobSkill>,
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Institution {
    pub id: Uuid,
    pub nama: String,
    #[schema(value_type = InstitutionType)]
    pub jenis: String,
    pub provinsi: String,
    pub kota: String,
    pub website: String,
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Training {
    pub id: Uuid,
    pub institution_id: Uuid,
    pub nama: String,
    /// Unknown source fee is null, not free.
    #[schema(minimum = 0)]
    pub biaya: Option<i64>,
    #[schema(value_type = Option<DeliveryMode>)]
    pub moda: Option<String>,
    pub durasi: String,
    pub url_daftar: String,
    pub skill_ids: Vec<Uuid>,
    pub sample: bool,
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
pub struct Experience {
    pub posisi: String,
    pub tempat: String,
    pub periode: String,
}
#[derive(ToSchema, Clone, Debug, Default, Serialize, Deserialize)]
pub struct ParsedCv {
    pub pendidikan: Vec<String>,
    pub pengalaman: Vec<Experience>,
    pub skills: Vec<String>,
}
impl ParsedCv {
    pub fn validate(&self) -> bool {
        self.skills.len() <= 100
            && self.pendidikan.len() <= 20
            && self.pengalaman.len() <= 30
            && self
                .skills
                .iter()
                .all(|s| !s.trim().is_empty() && s.chars().count() <= 100)
            && self.pendidikan.iter().all(|s| s.len() <= 1000)
            && self
                .pengalaman
                .iter()
                .all(|s| s.posisi.len() <= 300 && s.tempat.len() <= 300 && s.periode.len() <= 100)
    }
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UserSkill {
    pub skill_id: Uuid,
    #[schema(value_type = SkillSource)]
    pub sumber: String,
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobMatch {
    pub job: Occupation,
    pub score: i64,
    pub earned: i64,
    pub total: i64,
    pub owned: Vec<JobSkill>,
    pub missing_wajib: Vec<JobSkill>,
    pub missing_opsional: Vec<JobSkill>,
}
pub fn match_job(job: &Occupation, have: &HashSet<Uuid>) -> JobMatch {
    let mut m = JobMatch {
        job: job.clone(),
        score: 0,
        earned: 0,
        total: 0,
        owned: vec![],
        missing_wajib: vec![],
        missing_opsional: vec![],
    };
    for s in &job.skills {
        let points = if s.tipe == "wajib" { 2 } else { 1 };
        m.total += points;
        if have.contains(&s.skill_id) {
            m.earned += points;
            m.owned.push(s.clone());
        } else if s.tipe == "wajib" {
            m.missing_wajib.push(s.clone());
        } else {
            m.missing_opsional.push(s.clone());
        }
    }
    if m.total > 0 {
        m.score = (100.0 * m.earned as f64 / m.total as f64).round() as i64;
    }
    m
}
pub fn normalize(raw: &str) -> String {
    raw.nfkd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .map(|c| {
            if c.is_alphanumeric() || c == '+' || c == '#' {
                c
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn aliases(skills: &[Skill]) -> HashMap<String, Uuid> {
    skills
        .iter()
        .flat_map(|s| {
            std::iter::once(&s.nama)
                .chain(s.alias.iter())
                .map(|n| (normalize(n), s.id))
        })
        .collect()
}
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedTraining {
    pub training: Training,
    pub covers_wajib: Vec<Uuid>,
    pub covers_opsional: Vec<Uuid>,
}
pub fn rank_trainings(trainings: &[Training], m: &JobMatch) -> Vec<RankedTraining> {
    let w: HashSet<_> = m.missing_wajib.iter().map(|s| s.skill_id).collect();
    let o: HashSet<_> = m.missing_opsional.iter().map(|s| s.skill_id).collect();
    let mut result: Vec<_> = trainings
        .iter()
        .map(|t| RankedTraining {
            training: t.clone(),
            covers_wajib: t
                .skill_ids
                .iter()
                .filter(|id| w.contains(id))
                .copied()
                .collect(),
            covers_opsional: t
                .skill_ids
                .iter()
                .filter(|id| o.contains(id))
                .copied()
                .collect(),
        })
        .filter(|r| !r.covers_wajib.is_empty() || !r.covers_opsional.is_empty())
        .collect();
    result.sort_by(|a, b| {
        b.covers_wajib
            .len()
            .cmp(&a.covers_wajib.len())
            .then_with(|| {
                (b.covers_wajib.len() + b.covers_opsional.len())
                    .cmp(&(a.covers_wajib.len() + a.covers_opsional.len()))
            })
            .then_with(|| (a.training.biaya != Some(0)).cmp(&(b.training.biaya != Some(0))))
            .then_with(|| a.training.nama.cmp(&b.training.nama))
    });
    result
}
/// LLM prose never supplies arbitrary skill names: it chooses catalog IDs only.
#[derive(ToSchema, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplanationPlan {
    pub priority_skill_ids: Vec<Uuid>,
}
pub fn validate_explanation(plan: &ExplanationPlan, m: &JobMatch) -> bool {
    let allowed: HashSet<_> = m
        .missing_wajib
        .iter()
        .chain(&m.missing_opsional)
        .map(|s| s.skill_id)
        .collect();
    plan.priority_skill_ids.len() <= 5
        && plan.priority_skill_ids.iter().all(|s| allowed.contains(s))
        && plan.priority_skill_ids.iter().collect::<HashSet<_>>().len()
            == plan.priority_skill_ids.len()
}
pub fn explanation(m: &JobMatch, skills: &[Skill], priority: &[Uuid]) -> String {
    let names: Vec<_> = priority
        .iter()
        .filter_map(|id| skills.iter().find(|s| s.id == *id).map(|s| s.nama.as_str()))
        .collect();
    let lead = format!(
        "Skormu untuk {} {}% ({} dari {} poin).",
        m.job.nama, m.score, m.earned, m.total
    );
    if names.is_empty() {
        format!("{lead} Semua skill wajib sudah lengkap; tinjau skill tambahan jika masih ada.")
    } else {
        format!(
            "{lead} Prioritaskan mempelajari {}. Pilih pelatihan yang mengajarkan skill yang kurang.",
            names.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn coverage_is_not_jaccard() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let job = Occupation {
            id: Uuid::new_v4(),
            nama: "A".into(),
            kode_kbji: "1234".into(),
            bidang: "B".into(),
            deskripsi: "".into(),
            skills: vec![
                JobSkill {
                    skill_id: a,
                    tipe: "wajib".into(),
                },
                JobSkill {
                    skill_id: b,
                    tipe: "opsional".into(),
                },
            ],
        };
        let m = match_job(&job, &HashSet::from([a, Uuid::new_v4()]));
        assert_eq!((m.score, m.earned, m.total), (67, 2, 3));
        assert_eq!(match_job(&job, &HashSet::new()).score, 0);
        assert!(!validate_explanation(
            &ExplanationPlan {
                priority_skill_ids: vec![a]
            },
            &m
        ));
        assert!(validate_explanation(
            &ExplanationPlan {
                priority_skill_ids: vec![b]
            },
            &m
        ));
    }
    #[test]
    fn normalization_keeps_programming_names_distinct() {
        assert_eq!(normalize("  MS. Éxcel "), "ms excel");
        assert_ne!(normalize("C++"), normalize("C#"));
    }
}

#[cfg(test)]
mod ranking_tests {
    use super::*;
    fn training(id: Uuid, name: &str, ids: Vec<Uuid>, cost: i64) -> Training {
        Training {
            id,
            institution_id: Uuid::new_v4(),
            nama: name.into(),
            biaya: Some(cost),
            moda: Some("daring".into()),
            durasi: "1 hari".into(),
            url_daftar: "https://example.test".into(),
            skill_ids: ids,
            sample: true,
        }
    }
    #[test]
    fn required_coverage_wins_over_free_price() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let job = Occupation {
            id: Uuid::new_v4(),
            nama: "A".into(),
            kode_kbji: "1234".into(),
            bidang: "B".into(),
            deskripsi: "".into(),
            skills: vec![
                JobSkill {
                    skill_id: a,
                    tipe: "wajib".into(),
                },
                JobSkill {
                    skill_id: b,
                    tipe: "wajib".into(),
                },
                JobSkill {
                    skill_id: c,
                    tipe: "opsional".into(),
                },
            ],
        };
        let m = match_job(&job, &HashSet::new());
        let ts = vec![
            training(Uuid::new_v4(), "optional", vec![c], 0),
            training(Uuid::new_v4(), "free", vec![a], 0),
            training(Uuid::new_v4(), "paid", vec![a, b], 100),
            training(Uuid::new_v4(), "irrelevant", vec![Uuid::new_v4()], 0),
        ];
        assert_eq!(
            rank_trainings(&ts, &m)
                .iter()
                .map(|r| r.training.nama.as_str())
                .collect::<Vec<_>>(),
            vec!["paid", "free", "optional"]
        );
    }
    #[test]
    fn rejects_oversized_extraction_without_panicking() {
        let cv = ParsedCv {
            skills: vec!["a".repeat(101)],
            ..Default::default()
        };
        assert!(!cv.validate());
    }
    #[test]
    fn alias_lookup_uses_canonical_and_alias_names() {
        let id = Uuid::new_v4();
        let index = aliases(&[Skill {
            id,
            nama: "Microsoft Excel".into(),
            kategori: "Office".into(),
            alias: vec!["ms excel".into()],
        }]);
        assert_eq!(index.get(&normalize("MS. Excel")), Some(&id));
        assert_eq!(index.get(&normalize("microsoft excel")), Some(&id));
        assert!(!index.contains_key("unknown"));
    }
}

#[derive(ToSchema)]
pub enum Education {
    #[serde(rename = "SMA")]
    Sma,
    #[serde(rename = "SMK")]
    Smk,
    D3,
    D4,
    S1,
    S2,
}
#[derive(ToSchema)]
pub enum InstitutionType {
    #[serde(rename = "LPK")]
    Lpk,
    #[serde(rename = "BLK")]
    Blk,
    Platform,
}
#[derive(ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    Daring,
    Luring,
}
#[derive(ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkillSource {
    Manual,
    CvAlias,
    CvSemantik,
}
#[derive(ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum SkillRequirement {
    Wajib,
    Opsional,
}
