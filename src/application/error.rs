#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("Sesi berakhir. Silakan masuk lagi.")]
    Unauthorized,
    #[error("Permintaan tidak diizinkan.")]
    Forbidden,
    #[error("Data tidak ditemukan.")]
    NotFound,
    #[error("Layanan sedang sibuk. Coba lagi.")]
    Busy,
    #[error("Layanan eksternal tidak tersedia. Coba lagi.")]
    Provider,
    #[error("Terjadi kesalahan pada server.")]
    Internal,
}
