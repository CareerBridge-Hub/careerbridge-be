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
    /// Null only for a user's AI-inferred occupation; catalog rows always carry KBJI.
    pub kode_kbji: Option<String>,
    pub bidang: String,
    pub deskripsi: String,
    pub skills: Vec<JobSkill>,
    /// True when the occupation was inferred from the caller's own CV (visible only to them).
    #[serde(default)]
    pub custom: bool,
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
/// Occupation the AI inferred from one CV, already validated against the catalogs it was shown.
#[derive(ToSchema, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum OccupationSuggestion {
    /// The CV fits an existing catalog occupation; nothing is created.
    #[serde(rename_all = "camelCase")]
    Catalog {
        occupation_id: Uuid,
        nama: String,
        evidence: Vec<String>,
    },
    /// No catalog occupation is the same role; saved as the user's own occupation on confirm.
    #[serde(rename_all = "camelCase")]
    Custom {
        nama: String,
        bidang: String,
        deskripsi: String,
        /// Closest catalog occupation, whose vacancies are recommended for this role.
        base_occupation_id: Option<Uuid>,
        wajib: Vec<Uuid>,
        opsional: Vec<Uuid>,
        evidence: Vec<String>,
    },
}
/// Raw provider answer. Indices refer to the catalog lists sent with the prompt.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RawOccupationSuggestion {
    pub evidence: Vec<String>,
    pub confidence: String,
    pub catalog_index: Option<usize>,
    pub related_index: Option<usize>,
    pub nama: String,
    pub bidang: String,
    pub deskripsi: String,
    pub wajib: Vec<usize>,
    pub opsional: Vec<usize>,
}
/// Catalog entry offered to the provider; its position in the slice is its prompt index.
pub struct CatalogOption<'a> {
    pub id: Uuid,
    pub nama: &'a str,
}
fn clean_text(text: &str, min: usize, max: usize) -> Option<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let len = text.chars().count();
    (min..=max).contains(&len).then_some(text)
}
/// Returns `None` when the answer is unsupported by the CV, low-confidence or malformed.
/// `cv_terms` are the exact experience titles and skill phrases the provider received.
pub fn validate_occupation_suggestion(
    raw: &RawOccupationSuggestion,
    cv_terms: &[&str],
    occupations: &[CatalogOption],
    skills: &[CatalogOption],
) -> Option<OccupationSuggestion> {
    if !matches!(raw.confidence.as_str(), "high" | "medium") {
        return None;
    }
    // Every quoted phrase must occur, as whole words, in the CV; one invented quote reveals a guess.
    let terms: Vec<String> = cv_terms
        .iter()
        .map(|t| format!(" {} ", normalize(t)))
        .collect();
    let mut seen = HashSet::new();
    let mut evidence = vec![];
    for quote in raw.evidence.iter().take(5) {
        let text = clean_text(quote, 2, 100)?;
        let key = normalize(&text);
        if key.is_empty() || !terms.iter().any(|t| t.contains(&format!(" {key} "))) {
            return None;
        }
        if seen.insert(key) {
            evidence.push(text);
        }
    }
    if evidence.is_empty() {
        return None;
    }
    if let Some(index) = raw.catalog_index {
        let o = occupations.get(index)?;
        return Some(OccupationSuggestion::Catalog {
            occupation_id: o.id,
            nama: o.nama.to_owned(),
            evidence,
        });
    }
    let nama = clean_text(&raw.nama, 2, 80)?;
    if nama.chars().any(char::is_control) || !nama.chars().any(char::is_alphabetic) {
        return None;
    }
    // Same name as a catalog occupation means the provider missed the match; reuse the catalog row.
    if let Some(o) = occupations
        .iter()
        .find(|o| normalize(o.nama) == normalize(&nama))
    {
        return Some(OccupationSuggestion::Catalog {
            occupation_id: o.id,
            nama: o.nama.to_owned(),
            evidence,
        });
    }
    let bidang = clean_text(&raw.bidang, 2, 80)?;
    let deskripsi = clean_text(&raw.deskripsi, 10, 300)?;
    let base_occupation_id = match raw.related_index {
        Some(index) => Some(occupations.get(index)?.id),
        None => None,
    };
    let mut picked = HashSet::new();
    let mut ids = |indices: &[usize], max: usize| -> Option<Vec<Uuid>> {
        if indices.len() > max {
            return None;
        }
        indices
            .iter()
            .map(|i| skills.get(*i).filter(|_| picked.insert(*i)).map(|s| s.id))
            .collect()
    };
    let wajib = ids(&raw.wajib, 8)?;
    let opsional = ids(&raw.opsional, 10)?;
    // Two required skills keep the score from being all-or-nothing.
    if wajib.len() < 2 {
        return None;
    }
    Some(OccupationSuggestion::Custom {
        nama,
        bidang,
        deskripsi,
        base_occupation_id,
        wajib,
        opsional,
        evidence,
    })
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
            kode_kbji: Some("1234".into()),
            custom: false,
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
    /// Proposal Tabel 4.3: (wajib, opsional, wajib owned, opsional owned) -> rounded score.
    #[test]
    fn weighted_score_matches_manual_cases() {
        let cases = [
            (5, 4, 3, 2, 57),
            (5, 3, 5, 3, 100),
            (5, 4, 0, 0, 0),
            (4, 4, 4, 0, 67),
            (4, 3, 2, 2, 55),
            (4, 4, 3, 3, 75),
            (5, 3, 2, 1, 38),
            (4, 4, 1, 3, 42),
            (4, 3, 3, 1, 64),
            (4, 3, 3, 2, 73),
        ];
        for (w, o, hw, ho, expected) in cases {
            let wajib: Vec<Uuid> = (0..w).map(|_| Uuid::new_v4()).collect();
            let opsional: Vec<Uuid> = (0..o).map(|_| Uuid::new_v4()).collect();
            let skills = wajib
                .iter()
                .map(|id| (id, "wajib"))
                .chain(opsional.iter().map(|id| (id, "opsional")))
                .map(|(id, tipe)| JobSkill {
                    skill_id: *id,
                    tipe: tipe.into(),
                })
                .collect();
            let job = Occupation {
                id: Uuid::new_v4(),
                nama: "Kasus".into(),
                kode_kbji: Some("0000".into()),
                custom: false,
                bidang: "Uji".into(),
                deskripsi: "".into(),
                skills,
            };
            let have: HashSet<Uuid> = wajib[..hw].iter().chain(&opsional[..ho]).copied().collect();
            let m = match_job(&job, &have);
            let earned = (hw * 2 + ho) as i64;
            let total = (w * 2 + o) as i64;
            assert_eq!((m.earned, m.total, m.score), (earned, total, expected));
            assert_eq!(m.missing_wajib.len(), w - hw);
            assert_eq!(m.missing_opsional.len(), o - ho);
        }
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
            kode_kbji: Some("2512".into()),
            custom: false,
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
            kode_kbji: Some("1234".into()),
            custom: false,
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

#[cfg(test)]
mod occupation_suggestion_tests {
    use super::*;
    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn occupations() -> Vec<CatalogOption<'static>> {
        vec![
            CatalogOption {
                id: id(1),
                nama: "Web Developer",
            },
            CatalogOption {
                id: id(2),
                nama: "DevOps Engineer",
            },
        ]
    }
    fn skills() -> Vec<CatalogOption<'static>> {
        ["AWS", "Linux", "Terraform", "Docker", "Python"]
            .iter()
            .enumerate()
            .map(|(i, nama)| CatalogOption {
                id: id(100 + i as u128),
                nama,
            })
            .collect()
    }
    const CV: &[&str] = &["Cloud Engineer", "System Administrator", "AWS", "Terraform"];
    fn cloud() -> RawOccupationSuggestion {
        RawOccupationSuggestion {
            evidence: vec!["Cloud Engineer".into(), "AWS".into()],
            confidence: "high".into(),
            catalog_index: None,
            related_index: Some(1),
            nama: "  Cloud   Engineer ".into(),
            bidang: "Teknologi Informasi".into(),
            deskripsi: "Merancang dan mengelola infrastruktur di layanan cloud.".into(),
            wajib: vec![0, 1, 2],
            opsional: vec![3],
        }
    }
    fn check(raw: &RawOccupationSuggestion) -> Option<OccupationSuggestion> {
        validate_occupation_suggestion(raw, CV, &occupations(), &skills())
    }

    #[test]
    fn new_role_becomes_custom_with_catalog_ids() {
        assert_eq!(
            check(&cloud()),
            Some(OccupationSuggestion::Custom {
                nama: "Cloud Engineer".into(),
                bidang: "Teknologi Informasi".into(),
                deskripsi: "Merancang dan mengelola infrastruktur di layanan cloud.".into(),
                base_occupation_id: Some(id(2)),
                wajib: vec![id(100), id(101), id(102)],
                opsional: vec![id(103)],
                evidence: vec!["Cloud Engineer".into(), "AWS".into()],
            })
        );
    }

    #[test]
    fn catalog_match_by_index_or_by_name_reuses_catalog_row() {
        let by_index = RawOccupationSuggestion {
            catalog_index: Some(0),
            ..cloud()
        };
        let by_name = RawOccupationSuggestion {
            nama: "devops engineer".into(),
            ..cloud()
        };
        assert!(
            matches!(check(&by_index), Some(OccupationSuggestion::Catalog { occupation_id, .. }) if occupation_id == id(1))
        );
        assert!(
            matches!(check(&by_name), Some(OccupationSuggestion::Catalog { occupation_id, nama, .. }) if occupation_id == id(2) && nama == "DevOps Engineer")
        );
    }

    #[test]
    fn rejects_low_confidence_and_unsupported_evidence() {
        for confidence in ["low", "", "HIGH"] {
            let raw = RawOccupationSuggestion {
                confidence: confidence.into(),
                ..cloud()
            };
            assert_eq!(check(&raw), None, "confidence {confidence:?}");
        }
        for evidence in [
            vec![],
            vec!["Kubernetes".into()],
            // One invented quote invalidates the answer even next to a real one.
            vec!["AWS".into(), "Google Cloud Architect".into()],
            // Whole words only: "Engine" is not quoted from "Cloud Engineer".
            vec!["Engine".into()],
        ] {
            let raw = RawOccupationSuggestion {
                evidence: evidence.clone(),
                ..cloud()
            };
            assert_eq!(check(&raw), None, "evidence {evidence:?}");
        }
    }

    #[test]
    fn rejects_out_of_range_duplicate_or_too_few_skills() {
        for (wajib, opsional) in [
            (vec![0, 9], vec![]),
            (vec![0, 1], vec![1]),
            (vec![0, 0], vec![]),
            (vec![0], vec![1, 2]),
            (vec![0, 1, 2, 3, 4, 0, 1, 2, 3], vec![]),
        ] {
            let raw = RawOccupationSuggestion {
                wajib: wajib.clone(),
                opsional: opsional.clone(),
                ..cloud()
            };
            assert_eq!(check(&raw), None, "wajib {wajib:?} opsional {opsional:?}");
        }
    }

    #[test]
    fn rejects_bad_indices_and_text() {
        let cases = [
            RawOccupationSuggestion {
                catalog_index: Some(5),
                ..cloud()
            },
            RawOccupationSuggestion {
                related_index: Some(5),
                ..cloud()
            },
            RawOccupationSuggestion {
                nama: "x".into(),
                ..cloud()
            },
            RawOccupationSuggestion {
                nama: "1234".into(),
                ..cloud()
            },
            RawOccupationSuggestion {
                nama: "a".repeat(81),
                ..cloud()
            },
            RawOccupationSuggestion {
                bidang: "".into(),
                ..cloud()
            },
            RawOccupationSuggestion {
                deskripsi: "Cloud".into(),
                ..cloud()
            },
        ];
        for raw in cases {
            assert_eq!(check(&raw), None, "{raw:?}");
        }
    }

    #[test]
    fn related_occupation_is_optional_and_suggestion_round_trips_as_json() {
        let raw = RawOccupationSuggestion {
            related_index: None,
            ..cloud()
        };
        let suggestion = check(&raw).unwrap();
        let json = serde_json::to_value(&suggestion).unwrap();
        assert_eq!(json["kind"], "custom");
        assert_eq!(json["baseOccupationId"], serde_json::Value::Null);
        assert_eq!(
            serde_json::from_value::<OccupationSuggestion>(json).unwrap(),
            suggestion
        );
        // Provider fields arrive camelCase; unknown or missing fields default safely.
        let parsed: RawOccupationSuggestion = serde_json::from_value(serde_json::json!({
            "evidence": ["AWS"], "confidence": "medium", "catalogIndex": 1, "extra": true
        }))
        .unwrap();
        assert_eq!(parsed.catalog_index, Some(1));
        assert!(
            matches!(check(&parsed), Some(OccupationSuggestion::Catalog { occupation_id, .. }) if occupation_id == id(2))
        );
    }
}
