use std::io;
use log::info;
use anyhow::{Context, Result, bail};
use sf_api::{
    sso::SFAccount,
    session::CharacterSession,
    gamestate::GameState
};

use sfscraper::{
    search_and_attack,
    Config, SearchSettings
};

#[tokio::main]
async fn main() -> Result<()> {
    dotenv::dotenv()?;

    let env = env_logger::Env::default().default_filter_or("info");
    env_logger::Builder::from_env(env).try_init()
        .context("Failed to initialize logger!")?;

    let config = Config::from_env()
        .context("Invalid or missing configuration")?;

    info!("Logging into SFGames account");
    let account = if config.steam_login {
        SFAccount::login_with_steam().await?
    } else {
        SFAccount::login(config.login, config.password).await?
    };

    let mut sessions: Vec<CharacterSession> = account
        .characters().await?
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

    input.clear();

    println!("Please enter starting position in Hall of Fame:");
    io::stdin().read_line(&mut input)?;
    let hall_start = input.trim().parse::<usize>()?;

    let login_response = session.login().await?;
    let mut game_state = GameState::new(login_response)?;

    info!("Logged in as {} ({})", session.username(), session.server_url());
    info!("Using {:?} strategy", config.search_strategy);

    search_and_attack(
        &mut session, &mut game_state,
        config.search_strategy,
        SearchSettings {
            discover_threshold: config.discover_threshold,
            level_threshold: config.level_threshold,
            search_direction: config.search_direction
        },
        hall_start / 30,
    ).await?;

    Ok(())
}
