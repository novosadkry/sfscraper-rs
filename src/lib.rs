use std::env;
use anyhow::{Context, Result};

#[derive(Debug)]
pub struct Config {
    pub login: String,
    pub password: String
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            login: env::var("SFGAME_USERNAME").context("SFGAME_USERNAME")?,
            password: env::var("SFGAME_PASSWORD").context("SFGAME_PASSWORD")?
        })
    }
}

pub fn init() -> Result<()> {
    dotenv::dotenv()?;

    let env = env_logger::Env::default().default_filter_or("info");
    env_logger::Builder::from_env(env).try_init()
        .context("Failed to initialize logger!")
}
