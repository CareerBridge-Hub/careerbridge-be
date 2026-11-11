use serde_json::Value;
use std::sync::LazyLock;

static CITIES: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("data/cities.json")).expect("bundled city dictionary")
});
static PROVINCES: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("data/provinces.json")).expect("bundled provinces")
});
pub fn province_id(name: &str) -> Option<i16> {
    let key = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase();
    let alias = match key.as_str() {
        "DAERAH ISTIMEWA YOGYAKARTA"
        | "YOGYAKARTA"
        | "D.I. YOGYAKARTA"
        | "SPECIAL REGION OF YOGYAKARTA" => Some(15),
        "DAERAH KHUSUS IBUKOTA JAKARTA" | "JAKARTA" | "DKI" => Some(11),
        "WEST JAVA" => Some(12),
        "CENTRAL JAVA" => Some(14),
        "EAST JAVA" => Some(16),
        "NORTH SUMATRA" => Some(2),
        "WEST SUMATRA" => Some(3),
        "SOUTH SUMATRA" => Some(7),
        "RIAU ISLANDS" => Some(5),
        "EAST KALIMANTAN" => Some(23),
        "SOUTH SULAWESI" => Some(29),
        "NORTH SULAWESI" => Some(25),
        "WEST NUSA TENGGARA" => Some(18),
        "EAST NUSA TENGGARA" => Some(19),
        "BANGKA BELITUNG" => Some(8),
        _ => None,
    };
    alias.or_else(|| {
        PROVINCES
            .as_object()?
            .iter()
            .find(|(_, v)| v.as_str().is_some_and(|s| s.to_uppercase() == key))
            .and_then(|(k, _)| k.parse().ok())
    })
}
pub fn city_region(raw: &str) -> (String, String) {
    let first = raw.split([';', '|']).next().unwrap_or(raw);
    let mut parts = Vec::new();
    static PREFIX: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r"(?i)^(?:kota\s+adm\.?|kota\s+administrasi|kota|kabupaten|kab\.?|adm\.?)\s+",
        )
        .unwrap()
    });
    static SUFFIX: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"(?i)\s+(?:city|regency)$").unwrap());
    for s in first.split(',').map(str::trim) {
        let cleaned = PREFIX.replace(s, "");
        let cleaned = SUFFIX.replace(&cleaned, "");
        let s = cleaned.as_ref();
        let low = s.to_lowercase();
        let alias = match low.as_str() {
            "greater jakarta" | "jakarta metropolitan area" | "jakarta raya" | "dki jakarta" => {
                "Jakarta"
            }
            "south jakarta" => "Jakarta Selatan",
            "north jakarta" => "Jakarta Utara",
            "west jakarta" => "Jakarta Barat",
            "east jakarta" => "Jakarta Timur",
            "central jakarta" => "Jakarta Pusat",
            "south tangerang" | "bsd" | "bsd city" | "tangsel" => "Tangerang Selatan",
            "jogja" | "jogjakarta" => "Yogyakarta",
            "surakarta" => "Solo",
            _ => s,
        };
        if !matches!(low.as_str(), "indonesia" | "id" | "remote" | "") {
            parts.push(alias.to_owned());
        }
    }
    let known = parts.iter().find_map(|p| {
        CITIES
            .as_object()?
            .values()
            .find(|c| {
                c["name"]
                    .as_str()
                    .is_some_and(|s| s.eq_ignore_ascii_case(p))
            })
            .map(|c| {
                (
                    c["name"].as_str().unwrap_or(p).to_owned(),
                    c["province_id"].as_i64().unwrap_or(0) as i16,
                )
            })
    });
    let (city, pid) = known.map(|(c, p)| (c, Some(p))).unwrap_or_else(|| {
        (
            parts.first().cloned().unwrap_or_else(|| "Indonesia".into()),
            parts.iter().find_map(|p| province_id(p)),
        )
    });
    let pid = pid.or_else(|| {
        if city.starts_with("Jakarta") {
            Some(11)
        } else {
            None
        }
    });
    let region = pid
        .and_then(|p| PROVINCES[&p.to_string()].as_str())
        .unwrap_or("")
        .to_owned();
    (city.chars().take(200).collect(), region)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn location_aliases_match_the_python_importer() {
        assert_eq!(
            city_region("South Jakarta, Jakarta, Indonesia"),
            ("Jakarta Selatan".into(), "DKI Jakarta".into())
        );
        assert_eq!(
            city_region("Kota Bandung, West Java, Indonesia"),
            ("Bandung".into(), "Jawa Barat".into())
        );
        assert_eq!(province_id("DAERAH ISTIMEWA YOGYAKARTA"), Some(15));
    }
}
