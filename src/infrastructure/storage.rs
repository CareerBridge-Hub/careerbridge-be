use super::error::AppError;
use futures_util::StreamExt;
pub struct Storage {
    pub client: reqwest::Client,
    pub url: String,
    pub key: String,
    pub bucket: String,
}
impl Storage {
    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = request.header("apikey", &self.key);
        // Modern opaque secret keys are API keys, not JWT bearer tokens.
        if self.key.starts_with("sb_secret_") {
            request
        } else {
            request.bearer_auth(&self.key)
        }
    }

    fn endpoint(&self, path: &str) -> Result<String, AppError> {
        if path
            .split('/')
            .any(|s| s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        {
            return Err(AppError::Internal);
        }
        Ok(format!(
            "{}/storage/v1/object/{}/{}",
            self.url, self.bucket, path
        ))
    }
    pub async fn put(&self, path: &str, bytes: Vec<u8>) -> Result<(), AppError> {
        self.authorize(self.client.post(self.endpoint(path)?))
            .header("Content-Type", "application/pdf")
            .body(bytes)
            .send()
            .await
            .map_err(|_| AppError::Provider)?
            .error_for_status()
            .map_err(|_| AppError::Provider)?;
        Ok(())
    }
    pub async fn get(&self, path: &str) -> Result<Vec<u8>, AppError> {
        let r = self
            .authorize(self.client.get(self.endpoint(path)?))
            .send()
            .await
            .map_err(|_| AppError::Provider)?
            .error_for_status()
            .map_err(|_| AppError::Provider)?;
        let mut stream = r.bytes_stream();
        let mut bytes = vec![];
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AppError::Provider)?;
            if bytes.len() + chunk.len() > 2097152 {
                return Err(AppError::Invalid("Ukuran file maksimal 2 MB."));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }
    fn delete_request(&self, path: &str) -> Result<reqwest::RequestBuilder, AppError> {
        let object = self.endpoint(path)?;
        let suffix = format!("/{path}");
        let bucket = object.strip_suffix(&suffix).ok_or(AppError::Internal)?;
        Ok(self
            .authorize(self.client.delete(bucket))
            .json(&serde_json::json!({"prefixes": [path]})))
    }
    pub async fn delete(&self, path: &str) -> Result<(), AppError> {
        let r = self
            .delete_request(path)?
            .send()
            .await
            .map_err(|_| AppError::Provider)?;
        if r.status().is_success() || r.status().as_u16() == 404 {
            Ok(())
        } else {
            Err(AppError::Provider)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deletion_uses_bucket_endpoint_and_exact_prefixes_body() {
        let storage = Storage {
            client: reqwest::Client::new(),
            url: "http://127.0.0.1".into(),
            key: "sb_secret_test_fixture".into(),
            bucket: "cv-private".into(),
        };
        let request = storage
            .delete_request("user-id/upload-id")
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.url().path(), "/storage/v1/object/cv-private");
        assert_eq!(request.method(), reqwest::Method::DELETE);
        let body: serde_json::Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(body, serde_json::json!({"prefixes": ["user-id/upload-id"]}));
        assert!(storage.delete_request("../private").is_err());
    }
    #[test]
    fn opaque_secret_keys_are_not_sent_as_jwt_bearer_tokens() {
        for (key, bearer) in [
            ("sb_secret_test_fixture", false),
            ("legacy-service-role-jwt", true),
        ] {
            let storage = Storage {
                client: reqwest::Client::new(),
                url: "http://127.0.0.1".into(),
                key: key.into(),
                bucket: "cv-private".into(),
            };
            let request = storage
                .authorize(storage.client.get("http://127.0.0.1/storage"))
                .build()
                .unwrap();
            assert_eq!(request.headers()["apikey"], key);
            assert_eq!(request.headers().contains_key("authorization"), bearer);
        }
    }
}
