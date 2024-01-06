use anyhow::{Context, Result};
use sf_api::{
    session::CharacterSession,
    gamestate::GameState,
    command::Command
};
use tokio::time::{sleep, Duration};

use sfscraper::{Config, CharacterSessionExt};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()
        .context("Invalid or missing configuration")?;

    let mut session = CharacterSession::from_config(config)?;

    let login_response = session.login().await?;
    let mut game_state = GameState::new(login_response)?;

    println!("Logged in as {}", game_state.character.name);

    println!("Sending player update");
    let response = session.send_command(&Command::UpdatePlayer).await?;
    game_state.update(response)?;

    println!("Waiting a few milliseconds");
    sleep(Duration::from_millis(250)).await;

    println!("Getting scrapbook contents");
    let response = session.send_command(&Command::ViewScrapbook).await?;
    game_state.update(response)?;

    let scrapbook = game_state.unlocks.scrapbok
        .context("Your character doesn't have an active scrapbook!")?;

    println!("{:?}", scrapbook.items);

    Ok(())
}
