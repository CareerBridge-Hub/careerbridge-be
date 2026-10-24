use super::error::AppError;
use futures_util::StreamExt;
pub struct Storage {
    pub client: reqwest::Client,
    pub url: String,
    pub key: String,
    pub bucket: String,
}
impl Storage {
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
        self.client
            .post(self.endpoint(path)?)
            .header("apikey", &self.key)
            .bearer_auth(&self.key)
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
            .client
            .get(self.endpoint(path)?)
            .header("apikey", &self.key)
            .bearer_auth(&self.key)
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
    pub async fn delete(&self, path: &str) -> Result<(), AppError> {
        let r = self
            .client
            .delete(self.endpoint(path)?)
            .header("apikey", &self.key)
            .bearer_auth(&self.key)
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
