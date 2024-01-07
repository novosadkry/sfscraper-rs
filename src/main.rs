use std::{io, sync::Arc};
use log::{info, debug};
use anyhow::{Context, Result, bail};
use tokio::{signal, sync::Mutex};
use sf_api::{
    sso::SFAccount,
    session::CharacterSession,
    gamestate::GameState,
    command::Command
};

use sfscraper::Config;

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv()?;

    let env = env_logger::Env::default().default_filter_or("info");
    env_logger::Builder::from_env(env).try_init()
        .context("Failed to initialize logger!")?;

    let config = Config::from_env()
        .context("Invalid or missing configuration")?;

    let account = SFAccount::login(config.login, config.password).await?;
    let mut sessions: Vec<CharacterSession> = account.characters().await?
        .into_iter().flatten()
        .collect();

    println!("Please enter desired character number:");
    for (i, session) in sessions.iter().enumerate()
    {
        println!("[{}] {} ({})", i, session.username(), session.server_url())
    }

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    let index = input.trim().parse::<usize>()?;
    let mut session = if index < sessions.len() {
        sessions.swap_remove(index)
    } else {
        bail!("Invalid selection");
    };

    let login_response = session.login().await?;
    let mut game_state = GameState::new(login_response)?;

    info!("Logged in as {} ({})", session.username(), session.server_url());

    info!("Sending player update");
    let response = session.send_command(&Command::UpdatePlayer).await?;
    game_state.update(response)?;

    let session = Arc::new(Mutex::new(session));
    let game_state = Arc::new(Mutex::new(game_state));

    let scrape_handle = tokio::spawn(sfscraper::thread_scrape_halloffame(session.clone(), game_state.clone()));

    signal::ctrl_c().await?;

    Ok(())
}
