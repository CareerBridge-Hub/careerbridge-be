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
/// Catalog skills whose name or alias appears in `text` as a whole token sequence.
pub fn skills_in_text(text: &str, index: &HashMap<String, Uuid>) -> HashSet<Uuid> {
    let normalized = normalize(text);
    let tokens: Vec<&str> = normalized.split(' ').filter(|t| !t.is_empty()).collect();
    let max_n = index
        .keys()
        .map(|k| k.split(' ').count())
        .max()
        .unwrap_or(0);
    let mut found = HashSet::new();
    for i in 0..tokens.len() {
        for n in 1..=max_n.min(tokens.len() - i) {
            if let Some(id) = index.get(&tokens[i..i + n].join(" ")) {
                found.insert(*id);
            }
        }
    }
    found
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
/// LLM picks priority skills by catalog index only; its prose is checked by `validate_prose`.
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
/// Score after every missing required skill is learned.
pub fn projected_score(m: &JobMatch) -> i64 {
    if m.total == 0 {
        return 0;
    }
    let gain = m.missing_wajib.len() as i64 * 2;
    (100.0 * (m.earned + gain) as f64 / m.total as f64).round() as i64
}
/// Every figure the LLM may quote. Anything else in its prose is treated as invented.
pub fn explanation_numbers(m: &JobMatch, trainings: usize) -> HashSet<i64> {
    let count =
        |list: &[JobSkill], tipe: &str| list.iter().filter(|s| s.tipe == tipe).count() as i64;
    let w = count(&m.job.skills, "wajib");
    let o = count(&m.job.skills, "opsional");
    let ow = count(&m.owned, "wajib");
    let oo = count(&m.owned, "opsional");
    let (mw, mo) = (
        m.missing_wajib.len() as i64,
        m.missing_opsional.len() as i64,
    );
    HashSet::from([
        0,
        1,
        2,
        100,
        m.score,
        m.earned,
        m.total,
        w,
        o,
        w + o,
        ow,
        oo,
        ow + oo,
        mw,
        mo,
        mw + mo,
        mw * 2,
        mw * 2 + mo,
        100 - m.score,
        m.total - m.earned,
        projected_score(m),
        trainings as i64,
    ])
}
/// LLM prose is accepted only if it is a sane length, plain text, and quotes no number outside
/// `numbers`. Digits inside known names (e.g. "HTML5") are removed before the check.
pub fn validate_prose(text: &str, names: &[&str], numbers: &HashSet<i64>) -> bool {
    let len = text.chars().count();
    if !(150..=2500).contains(&len) || text.contains(['#', '*', '`', '<', '>']) {
        return false;
    }
    let mut scrubbed = text.to_string();
    let mut sorted = names.to_vec();
    sorted.sort_by_key(|n| std::cmp::Reverse(n.len()));
    for n in sorted.into_iter().filter(|n| !n.is_empty()) {
        scrubbed = scrubbed.replace(n, " ");
    }
    scrubbed
        .split(|c: char| !c.is_ascii_digit())
        .filter(|t| !t.is_empty())
        .all(|t| t.parse::<i64>().is_ok_and(|n| numbers.contains(&n)))
}
/// "a", "a dan b", "a, b dan c".
fn join_id(names: &[&str]) -> String {
    match names {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} dan {last}", rest.join(", ")),
    }
}
/// Paragraphs separated by a blank line. Every name and number comes from the match, the catalog,
/// or the ranked trainings, so the text cannot mention a skill or figure the data does not hold.
pub fn explanation(
    m: &JobMatch,
    skills: &[Skill],
    priority: &[Uuid],
    trainings: &[RankedTraining],
) -> String {
    let name = |id: &Uuid| {
        skills
            .iter()
            .find(|s| s.id == *id)
            .map_or("skill tanpa nama", |s| s.nama.as_str())
    };
    let names = |list: &[JobSkill]| list.iter().map(|s| name(&s.skill_id)).collect::<Vec<_>>();
    let owned_wajib = m.owned.iter().filter(|s| s.tipe == "wajib").count();
    let verdict = match m.score {
        80.. => "Profilmu sudah sangat dekat dengan kebutuhan pekerjaan ini.",
        50..=79 => "Fondasimu sudah cukup kuat, tinggal melengkapi beberapa skill.",
        _ => "Jaraknya masih cukup lebar, jadi fokuskan tenaga ke skill wajib lebih dulu.",
    };
    let mut out = vec![format!(
        "Skormu untuk {} {}% ({} dari {} poin). Kamu sudah punya {} dari {} skill yang dibutuhkan: {} wajib dan {} tambahan. {verdict}",
        m.job.nama,
        m.score,
        m.earned,
        m.total,
        m.owned.len(),
        m.job.skills.len(),
        owned_wajib,
        m.owned.len() - owned_wajib,
    )];

    let wajib = names(&m.missing_wajib);
    let opsional = names(&m.missing_opsional);
    let priority: Vec<_> = priority.iter().map(name).collect();
    if wajib.is_empty() && opsional.is_empty() {
        out.push("Semua skill wajib dan tambahan sudah kamu punya. Kamu siap melamar.".into());
    } else if wajib.is_empty() {
        out.push(format!(
            "Semua skill wajib sudah lengkap. Sisa poin ada di skill tambahan: {}. Masing-masing bernilai 1 poin dan membawa skormu ke 100% bila semuanya dikuasai.",
            join_id(&opsional)
        ));
    } else {
        let projected = projected_score(m);
        let mut p = format!(
            "Masih ada {} skill wajib yang belum kamu punya: {}. Skill wajib bernilai 2 poin, dua kali skill tambahan, jadi melengkapinya menaikkan skor paling cepat. Kalau semuanya terpenuhi, skormu naik menjadi {projected}%.",
            wajib.len(),
            join_id(&wajib)
        );
        if !opsional.is_empty() {
            p += &format!(" Setelah itu, {} bisa menambah poin.", join_id(&opsional));
        }
        out.push(p);
    }

    if !priority.is_empty() {
        let mut p = format!("Urutan belajar yang disarankan: {}.", join_id(&priority));
        if let Some(t) = trainings.first() {
            let covered: Vec<_> = t
                .covers_wajib
                .iter()
                .chain(&t.covers_opsional)
                .map(name)
                .collect();
            p += &format!(
                " Pelatihan {} mengajarkan {}, jadi bisa jadi langkah pertama.",
                t.training.nama,
                join_id(&covered)
            );
        }
        out.push(p);
    }
    out.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_catalog_skills_as_whole_token_sequences() {
        let (sql, excel, team, cpp, react) = (
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
        );
        let skill = |id, nama: &str, alias: &[&str]| Skill {
            id,
            nama: nama.into(),
            kategori: "T".into(),
            alias: alias.iter().map(|a| a.to_string()).collect(),
        };
        let index = aliases(&[
            skill(sql, "SQL", &[]),
            skill(excel, "Microsoft Excel", &["excel"]),
            skill(team, "Kerja Tim", &["kerja sama tim"]),
            skill(cpp, "C++", &[]),
            skill(react, "React", &["react.js"]),
        ]);
        let found = skills_in_text(
            "Menguasai SQL, EXCEL.\nMampu Kerja-Sama Tim; pengalaman C++/Java.",
            &index,
        );
        assert_eq!(found, HashSet::from([sql, excel, team, cpp]));
        assert!(skills_in_text("pakai mysqldump, sikap reaktif", &index).is_empty());
        assert_eq!(
            skills_in_text("React.js developer", &index),
            HashSet::from([react])
        );
        assert!(skills_in_text("", &index).is_empty());
        assert!(skills_in_text("SQL", &HashMap::new()).is_empty());
    }
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
    fn explanation_names_only_catalog_skills_and_real_numbers() {
        let skill = |nama: &str| Skill {
            id: Uuid::new_v4(),
            nama: nama.into(),
            kategori: "X".into(),
            alias: vec![],
        };
        let (git, sql, py) = (skill("Git"), skill("SQL"), skill("Python"));
        let req = |s: &Skill, tipe: &str| JobSkill {
            skill_id: s.id,
            tipe: tipe.into(),
        };
        let job = Occupation {
            id: Uuid::new_v4(),
            nama: "Software Engineer".into(),
            kode_kbji: "2512".into(),
            bidang: "TI".into(),
            deskripsi: "".into(),
            skills: vec![req(&git, "wajib"), req(&sql, "wajib"), req(&py, "opsional")],
        };
        let m = match_job(&job, &HashSet::from([sql.id]));
        let ranked = rank_trainings(
            &[Training {
                id: Uuid::new_v4(),
                institution_id: Uuid::new_v4(),
                nama: "Kelas Git".into(),
                biaya: Some(0),
                moda: None,
                durasi: "1 hari".into(),
                url_daftar: "https://example.test".into(),
                skill_ids: vec![git.id],
                sample: true,
            }],
            &m,
        );
        let text = explanation(&m, &[git.clone(), sql, py], &[git.id], &ranked);
        assert!(text.starts_with("Skormu untuk Software Engineer 40% (2 dari 5 poin)."));
        assert!(text.contains("1 skill wajib yang belum kamu punya: Git."));
        assert!(text.contains("skormu naik menjadi 80%."));
        assert!(text.contains("Pelatihan Kelas Git mengajarkan Git"));
        assert_eq!(text.split("\n\n").count(), 3);
        assert_eq!(join_id(&["a", "b", "c"]), "a, b dan c");

        let numbers = explanation_numbers(&m, ranked.len());
        let names = ["Software Engineer", "Git", "SQL", "Python", "Kelas Git"];
        let ok = "Skormu 40% karena kamu baru mengumpulkan 2 dari 5 poin. SQL sudah jadi modal yang bagus.\n\nGit adalah skill wajib senilai 2 poin. Kalau Git dikuasai, skormu naik ke 80%.\n\nMulai dari Kelas Git yang memang mengajarkan Git.";
        assert!(validate_prose(ok, &names, &numbers));
        assert!(
            !validate_prose(&ok.replace("80%", "95%"), &names, &numbers),
            "invented number"
        );
        assert!(
            !validate_prose(&ok.replace("Skormu", "**Skormu**"), &names, &numbers),
            "markdown"
        );
        assert!(
            !validate_prose("Skormu 40%.", &names, &numbers),
            "too short"
        );
        assert!(validate_prose(
            &ok.replace("Kelas Git", "Kelas Git 2"),
            &["Kelas Git 2", "Git"],
            &numbers
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
