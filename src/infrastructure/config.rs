use std::{env, io};

pub struct Config {
    pub host: String,
    pub port: u16,
}

impl Config {
    pub fn from_env() -> io::Result<Self> {
        let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into());
        let port = env::var("PORT")
            .unwrap_or_else(|_| "8080".into())
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;

        Ok(Self { host, port })
    }
}
