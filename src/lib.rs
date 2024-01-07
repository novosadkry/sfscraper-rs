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
