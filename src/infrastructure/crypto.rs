use super::error::AppError;
use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, AeadCore, OsRng},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use regex::{Regex, RegexBuilder};
use zeroize::Zeroizing;
pub struct Cipher(Aes256Gcm);
impl Cipher {
    pub fn new(key: &str) -> Result<Self, AppError> {
        let key = Zeroizing::new(STANDARD.decode(key).map_err(|_| AppError::Internal)?);
        Ok(Self(
            Aes256Gcm::new_from_slice(&key).map_err(|_| AppError::Internal)?,
        ))
    }
    pub fn encrypt(&self, text: &str) -> Result<String, AppError> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let encrypted = self
            .0
            .encrypt(&nonce, text.as_bytes())
            .map_err(|_| AppError::Internal)?;
        let mut bytes = nonce.to_vec();
        bytes.extend(encrypted);
        Ok(STANDARD.encode(bytes))
    }
    pub fn decrypt(&self, text: &str) -> Result<Zeroizing<String>, AppError> {
        let bytes = STANDARD.decode(text).map_err(|_| AppError::Internal)?;
        if bytes.len() < 28 {
            return Err(AppError::Internal);
        }
        let plain = Zeroizing::new(
            self.0
                .decrypt((&bytes[..12]).into(), &bytes[12..])
                .map_err(|_| AppError::Internal)?,
        );
        Ok(Zeroizing::new(
            String::from_utf8(plain.to_vec()).map_err(|_| AppError::Internal)?,
        ))
    }
}
pub fn redact(text: &str, name: &str) -> Zeroizing<String> {
    let mut clean = Zeroizing::new(text.to_owned());
    for pattern in [
        r"(?i)[\w.+-]+@[\w-]+\.[\w.]+",
        r"(?:\+62|62|0)8\d{1,2}[\s-]?\d{3,4}[\s-]?\d{3,5}",
        r"\b\d{16}\b",
        r"(?i)(?:https?://)?(?:www\.)?linkedin\.com/\S+",
        r"(?im)^\s*(?:alamat\b|jl\.|jalan\b)[^\r\n]*",
    ] {
        let regex = Regex::new(pattern).expect("constant regex");
        *clean = regex.replace_all(&clean, "[REDACTED]").into_owned();
    }
    for part in std::iter::once(name)
        .chain(name.split_whitespace())
        .filter(|s| !s.is_empty())
    {
        if let Ok(regex) = RegexBuilder::new(&format!(r"\b{}\b", regex::escape(part)))
            .case_insensitive(true)
            .build()
        {
            *clean = regex.replace_all(&clean, "[REDACTED]").into_owned();
        }
    }
    clean
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pii_is_removed() {
        let s = redact(
            "Rina Putri\nrina@example.id 081234567890 1234567890123456\nAlamat: Jalan Contoh 9\nlinkedin.com/in/rina\nSQL",
            "Rina Putri",
        );
        for pii in [
            "Rina",
            "Putri",
            "@",
            "0812",
            "1234567890123456",
            "Jalan Contoh",
            "linkedin.com",
        ] {
            assert!(!s.contains(pii), "{pii}");
        }
        assert!(s.contains("SQL"));
    }
    #[test]
    fn encryption_is_random_and_authenticated() {
        let c = Cipher::new(&STANDARD.encode([7u8; 32])).unwrap();
        let a = c.encrypt("private").unwrap();
        let b = c.encrypt("private").unwrap();
        assert_ne!(a, b);
        assert_eq!(&*c.decrypt(&a).unwrap(), "private");
        assert!(c.decrypt("AAAA").is_err());
    }
}
