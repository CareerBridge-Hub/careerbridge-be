use crate::infrastructure::error::AppError;
use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;

/// Anonymous collector: fixed hosts, no redirects, no application credentials,
/// bounded bodies and timeouts. A blocked source never triggers evasion/retry bursts.
pub struct Http {
    client: reqwest::Client,
    pause: Duration,
}
pub fn allowed(raw: &str) -> bool {
    reqwest::Url::parse(raw).is_ok_and(|u| {
        u.scheme() == "https"
            && u.username().is_empty()
            && u.password().is_none()
            && u.port_or_known_default() == Some(443)
            && matches!(
                u.host_str(),
                Some(
                    "www.linkedin.com"
                        | "www.kalibrr.com"
                        | "www.techinasia.com"
                        | "id.jobstreet.com"
                        | "glints.com"
                        | "api.sejutacita.id"
                        | "boards-api.greenhouse.io"
                        | "api.lever.co"
                        | "apply.workable.com"
                        | "api.kemnaker.go.id"
                        | "api.ashbyhq.com"
                        | "api.smartrecruiters.com"
                        | "gql.kitalulus.com"
                        | "gateway2-beta.karir.com"
                        | "id.indeed.com"
                        | "www.glassdoor.com"
                        | "remotive.com"
                        | "remoteok.com"
                        | "www.arbeitnow.com"
                        | "hn.algolia.com"
                        | "api.adzuna.com"
                )
            )
    })
}
impl Http {
    pub fn new(pause: Duration) -> Result<Self, AppError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent("CareerBridge-DataCollector/1.0")
                .connect_timeout(Duration::from_secs(5))
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| AppError::Internal)?,
            pause,
        })
    }
    pub async fn request(
        &self,
        url: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<(u16, String), AppError> {
        if !allowed(url) {
            return Err(AppError::Invalid("Unexpected extraction host"));
        }
        tokio::time::sleep(self.pause).await;
        let request = match body {
            Some(v) => self.client.post(url).json(v),
            None => self.client.get(url),
        };
        let response = request
            .query(query)
            .header("Accept-Language", "en-US,en;q=0.9,id;q=0.8")
            .header("Accept", "application/json,text/html;q=0.9")
            .send()
            .await
            .map_err(|_| AppError::Provider)?;
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|n| n > 8 * 1024 * 1024)
        {
            return Err(AppError::Provider);
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AppError::Provider)?;
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(AppError::Provider);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((
            status,
            String::from_utf8(bytes).map_err(|_| AppError::Provider)?,
        ))
    }
    pub async fn json(
        &self,
        url: &str,
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value, AppError> {
        let (status, text) = self.request(url, query, body).await?;
        if status != 200 {
            return Err(AppError::Provider);
        }
        serde_json::from_str(&text).map_err(|_| AppError::Provider)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosts_and_urls_are_exact() {
        assert!(allowed(
            "https://www.linkedin.com/jobs-guest/jobs/api/jobPosting/123"
        ));
        for u in [
            "http://www.linkedin.com/x",
            "https://www.linkedin.com.evil.test/x",
            "https://u:p@www.linkedin.com/x",
            "https://127.0.0.1/x",
            "https://www.linkedin.com:444/x",
        ] {
            assert!(!allowed(u));
        }
    }
}
