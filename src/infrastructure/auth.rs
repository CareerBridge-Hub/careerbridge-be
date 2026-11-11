use super::error::AppError;
use crate::presentation::http::State;
use actix_web::{FromRequest, HttpRequest, dev::Payload, web};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::Deserialize;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Mutex,
    time::{Duration, Instant},
};
use tokio::sync::RwLock;
use uuid::Uuid;
#[derive(Clone, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub email: Option<String>,
    pub exp: u64,
}
pub struct Auth {
    client: reqwest::Client,
    url: String,
    issuer: String,
    cache: RwLock<Option<(Instant, JwkSet)>>,
    refresh: tokio::sync::Mutex<()>,
    limits: Mutex<HashMap<Uuid, (Instant, u32)>>,
}
impl Auth {
    pub fn new(url: &str, client: reqwest::Client) -> Self {
        Self {
            client,
            url: format!("{url}/auth/v1/.well-known/jwks.json"),
            issuer: format!("{url}/auth/v1"),
            cache: RwLock::new(None),
            refresh: tokio::sync::Mutex::new(()),
            limits: Mutex::new(HashMap::new()),
        }
    }
    pub async fn verify(&self, token: &str) -> Result<Claims, AppError> {
        let h = decode_header(token).map_err(|_| AppError::Unauthorized)?;
        if !matches!(h.alg, Algorithm::ES256 | Algorithm::RS256) {
            return Err(AppError::Unauthorized);
        }
        let kid = h.kid.ok_or(AppError::Unauthorized)?;
        let stale = self.cache.read().await.as_ref().is_none_or(|(t, keys)| {
            t.elapsed() > Duration::from_secs(300)
                || (keys.find(&kid).is_none() && t.elapsed() > Duration::from_secs(30))
        });
        if stale {
            let _guard = self.refresh.lock().await;
            let stale = self.cache.read().await.as_ref().is_none_or(|(t, keys)| {
                t.elapsed() > Duration::from_secs(300)
                    || (keys.find(&kid).is_none() && t.elapsed() > Duration::from_secs(30))
            });
            if stale {
                let keys = self
                    .client
                    .get(&self.url)
                    .send()
                    .await
                    .map_err(|_| AppError::Provider)?
                    .error_for_status()
                    .map_err(|_| AppError::Provider)?
                    .json::<JwkSet>()
                    .await
                    .map_err(|_| AppError::Provider)?;
                *self.cache.write().await = Some((Instant::now(), keys));
            }
        }
        let cache = self.cache.read().await;
        let key = cache
            .as_ref()
            .and_then(|(_, keys)| keys.find(&kid))
            .ok_or(AppError::Unauthorized)?;
        let key = DecodingKey::from_jwk(key).map_err(|_| AppError::Unauthorized)?;
        let mut validation = Validation::new(h.alg);
        validation.set_audience(&["authenticated"]);
        validation.set_issuer(&[&self.issuer]);
        validation.set_required_spec_claims(&["exp", "sub", "iss", "aud"]);
        validation.validate_nbf = true;
        validation.leeway = 10;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| AppError::Unauthorized)?
            .claims;
        let mut limits = self.limits.lock().map_err(|_| AppError::Internal)?;
        limits.retain(|_, (t, _)| t.elapsed() < Duration::from_secs(60));
        let slot = limits.entry(claims.sub).or_insert((Instant::now(), 0));
        slot.1 += 1;
        if slot.1 > 120 {
            return Err(AppError::Busy);
        }
        Ok(claims)
    }
}
impl FromRequest for Claims {
    type Error = AppError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;
    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let state = req.app_data::<web::Data<State>>().cloned();
        let cookie_auth = req
            .app_data::<web::Data<crate::presentation::session::CookieAuth>>()
            .cloned();
        let request = req.clone();
        let token = req
            .headers()
            .get("Authorization")
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.strip_prefix("Bearer "))
            .filter(|t| t.len() <= 8192)
            .map(str::to_owned);
        Box::pin(async move {
            let state = state.ok_or(AppError::Internal)?;
            if let Some(auth) = cookie_auth {
                if !matches!(request.method().as_str(), "GET" | "HEAD" | "OPTIONS") {
                    auth.csrf(&request)?;
                }
                // Cookie authentication is mandatory in the production application.
                return crate::presentation::session::resolve(&state, &request, &auth)
                    .await
                    .map(|(claims, _)| claims);
            }
            state
                .auth
                .verify(&token.ok_or(AppError::Unauthorized)?)
                .await
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_unsigned_and_symmetric_tokens_without_network() {
        let a = Auth::new("https://example.invalid", reqwest::Client::new());
        for token in [
            "",
            "eyJhbGciOiJub25lIn0.e30.",
            "eyJhbGciOiJIUzI1NiJ9.e30.eA",
        ] {
            assert!(matches!(a.verify(token).await, Err(AppError::Unauthorized)));
        }
    }

    #[tokio::test]
    async fn signed_tokens_require_identity_claims_and_valid_times() {
        use jsonwebtoken::{EncodingKey, Header, encode, get_current_timestamp};
        use serde_json::json;
        let auth = Auth::new("https://example.invalid", reqwest::Client::new());
        let keys = serde_json::from_str(include_str!("../../tests/fixtures/jwks.json")).unwrap();
        *auth.cache.write().await = Some((Instant::now(), keys));
        let key = EncodingKey::from_ec_der(include_bytes!("../../tests/fixtures/test-jwt.der"));
        let jwks: JwkSet =
            serde_json::from_str(include_str!("../../tests/fixtures/jwks.json")).unwrap();
        let mut header = Header::new(Algorithm::ES256);
        header.kid = jwks.keys[0].common.key_id.clone();
        let now = get_current_timestamp();
        let valid = json!({"sub":Uuid::new_v4(),"iss":"https://example.invalid/auth/v1","aud":"authenticated","exp":now+300});
        let sign = |claims: &serde_json::Value| encode(&header, claims, &key).unwrap();
        assert!(auth.verify(&sign(&valid)).await.is_ok());
        for missing in ["exp", "sub", "iss", "aud"] {
            let mut claims = valid.clone();
            claims.as_object_mut().unwrap().remove(missing);
            assert!(
                matches!(
                    auth.verify(&sign(&claims)).await,
                    Err(AppError::Unauthorized)
                ),
                "missing {missing}"
            );
        }
        for (field, value) in [
            ("iss", json!("https://evil.invalid/auth/v1")),
            ("aud", json!("anon")),
            ("exp", json!(now - 60)),
            ("nbf", json!(now + 300)),
        ] {
            let mut claims = valid.clone();
            claims[field] = value;
            assert!(
                matches!(
                    auth.verify(&sign(&claims)).await,
                    Err(AppError::Unauthorized)
                ),
                "invalid {field}"
            );
        }
        let mut forged = sign(&valid);
        let index = forged.rfind('.').unwrap() + 1;
        forged.replace_range(
            index..index + 1,
            if &forged[index..index + 1] == "A" {
                "B"
            } else {
                "A"
            },
        );
        assert!(matches!(
            auth.verify(&forged).await,
            Err(AppError::Unauthorized)
        ));
    }
}
