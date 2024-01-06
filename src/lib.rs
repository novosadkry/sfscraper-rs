use std::env;
use dotenv::dotenv;
use anyhow::{Context, Result, bail};
use sf_api::session::{CharacterSession, ServerConnection};

pub struct Config {
    login: String,
    password: String,
    server_url: String
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let _ = dotenv();

        Ok(Self {
            login: env::var("SFGAME_USERNAME").context("SFGAME_USERNAME")?,
            password: env::var("SFGAME_PASSWORD").context("SFGAME_PASSWORD")?,
            server_url: env::var("SFGAME_SERVER_URL").context("SFGAME_SERVER_URL")?
        })
    }
}

pub trait CharacterSessionExt {
    fn from_config(config: Config) -> Result<CharacterSession>;
}

impl CharacterSessionExt for CharacterSession {
    fn from_config(config: Config) -> Result<Self> {
        Ok(CharacterSession::new(
            &config.login,
            &config.password,
            match ServerConnection::new(&config.server_url) {
                Some(connection) => connection,
                None => bail!("Invalid server URL!"),
            },
        ))
    }
}