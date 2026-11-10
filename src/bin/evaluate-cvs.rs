//! Actual local PDF and embedding evaluation; optional real-provider extraction evaluation.
use careerbridge_be::{
    application::{EmbeddingKind, EmbeddingProvider, LlmProvider},
    domain::{ParsedCv, Skill, aliases, normalize},
    infrastructure::{
        crypto::redact,
        embedding::{LocalEmbedding, cosine},
        llm::CompatibleLlm,
    },
};
use serde_json::{Value, json};
use uuid::Uuid;
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let labels: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(
        root.join("tests/fixtures/cvs/labels.json"),
    )?)?;
    let seed: Value = serde_json::from_str(include_str!("../../docs/catalog.seed.json"))?;
    let skills: Vec<Skill> = seed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| Skill {
            id: Uuid::parse_str(s["id"].as_str().unwrap()).unwrap(),
            nama: s["nama"].as_str().unwrap().into(),
            kategori: s["kategori"].as_str().unwrap().into(),
            alias: serde_json::from_value(s["alias"].clone()).unwrap(),
        })
        .collect();
    let model = LocalEmbedding::load(
        std::env::var("MODEL_CACHE_DIR").unwrap_or_else(|_| ".cache/fastembed".into()),
    )
    .await?;
    let passages = model
        .embed(
            skills
                .iter()
                .map(|s| format!("{} {} {}", s.nama, s.kategori, s.alias.join(" ")))
                .collect(),
            EmbeddingKind::Passage,
        )
        .await?;
    let index = aliases(&skills);
    let mut rows = vec![];
    let mut pdf_ok = 0;
    let mut correct = 0;
    let mut top5 = 0;
    let mut extraction_hits = 0;
    let mut extraction_total = 0;
    let llm = match (std::env::var("LLM_API_KEY"), std::env::var("LLM_MODEL")) {
        (Ok(key), Ok(model)) => Some(CompatibleLlm {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            base_url: std::env::var("LLM_BASE_URL").unwrap_or_else(|_| {
                "https://generativelanguage.googleapis.com/v1beta/openai".into()
            }),
            key,
            model,
        }),
        _ => None,
    };
    for row in labels {
        let term = row["rawSkill"].as_str().unwrap();
        let expected = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "careerbridge:{}",
                row["expectedSkillSlug"].as_str().unwrap()
            )
            .as_bytes(),
        );
        let text = zeroize::Zeroizing::new(pdf_extract::extract_text(
            root.join("tests/fixtures/cvs")
                .join(row["file"].as_str().unwrap()),
        )?);
        let found = text.contains(term);
        if found {
            pdf_ok += 1;
        }
        let v = model.embed(vec![term.into()], EmbeddingKind::Query).await?;
        let mut scores: Vec<_> = skills
            .iter()
            .zip(&passages)
            .map(|(s, p)| (s.id, cosine(&v[0], p)))
            .collect();
        scores.sort_by(|a, b| b.1.total_cmp(&a.1));
        let exact = index.get(&normalize(term)).copied();
        let accepted = exact.or_else(|| (scores[0].1 >= 0.88).then_some(scores[0].0));
        if accepted == Some(expected) {
            correct += 1;
        }
        if scores.iter().take(5).any(|s| s.0 == expected) {
            top5 += 1;
        }
        let extraction = if let Some(llm) = &llm {
            extraction_total += 1;
            let data=llm.json("Extract explicitly stated CV skills only. JSON: {\"pendidikan\":[],\"pengalaman\":[],\"skills\":[\"skill\"]}.",json!({"cv":&*redact(&text,row["name"].as_str().unwrap())})).await;
            let hit = data
                .ok()
                .and_then(|v| serde_json::from_value::<ParsedCv>(v).ok())
                .is_some_and(|cv| {
                    cv.skills.iter().any(|s| {
                        normalize(s) == normalize(term)
                            || index.get(&normalize(s)) == Some(&expected)
                    })
                });
            if hit {
                extraction_hits += 1;
            }
            json!(hit)
        } else {
            Value::Null
        };
        rows.push(json!({"file":row["file"],"pdfTextMatchesLabel":found,"aliasMatch":exact.is_some(),"acceptedCorrect":accepted==Some(expected),"topSimilarity":scores[0].1,"expectedInTop5":scores.iter().take(5).any(|s|s.0==expected),"llmExtractionHit":extraction}));
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"synthetic":true,"cases":20,"model":model.version(),"pdfTextMatches":pdf_ok,"normalizationCorrect":correct,"expectedInTop5":top5,"llmExtractionEvaluated":extraction_total,"llmExtractionHits":extraction_hits,"limitation":"Synthetic narrow fixtures; rawSkill ground truth is supplied to normalization evaluation. Real LLM extraction runs only when LLM_API_KEY and LLM_MODEL are set. Not a universal accuracy estimate.","results":rows})
        )?
    );
    Ok(())
}
