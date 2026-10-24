use crate::{application::CvTextExtractor, infrastructure::error::AppError};
use async_trait::async_trait;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use zeroize::Zeroizing;
pub struct SubprocessPdf {
    pub executable: std::path::PathBuf,
}
#[async_trait]
impl CvTextExtractor for SubprocessPdf {
    async fn extract(&self, bytes: &[u8]) -> Result<Zeroizing<String>, AppError> {
        let mut child = tokio::process::Command::new(&self.executable)
            .arg("--extract-cv")
            .env_clear()
            .env(
                "LD_LIBRARY_PATH",
                std::env::var_os("LD_LIBRARY_PATH").unwrap_or_default(),
            )
            .current_dir(std::env::temp_dir())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| AppError::Internal)?;
        let mut stdin = child.stdin.take().ok_or(AppError::Internal)?;
        stdin
            .write_all(bytes)
            .await
            .map_err(|_| AppError::Invalid("PDF tidak dapat dibaca."))?;
        drop(stdin);
        let output = tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
            .await
            .map_err(|_| AppError::Invalid("PDF terlalu kompleks untuk diproses."))?
            .map_err(|_| AppError::Invalid("PDF tidak dapat dibaca."))?;
        if !output.status.success() {
            return Err(AppError::Invalid(
                "PDF tidak dapat dibaca atau terlalu kompleks.",
            ));
        }
        let text = Zeroizing::new(
            String::from_utf8(output.stdout)
                .map_err(|_| AppError::Invalid("Teks PDF tidak valid."))?,
        );
        Ok(text)
    }
}
