use std::{io, collections::HashSet};
use log::{info, debug};
use anyhow::{Context, Result};
use tokio::time::{sleep, Duration};
use sf_api::{
    sso::SFAccount,
    session::CharacterSession,
    gamestate::GameState,
    command::Command
};

use sfscraper::Config;

#[tokio::main]
async fn main() -> Result<()> {
    sfscraper::init()?;

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
    let session = sessions
        .get_mut(index)
        .context("Invalid selection")?;

    let login_response = session.login().await?;
    let mut game_state = GameState::new(login_response)?;

    info!("Logged in as {} ({})", session.username(), session.server_url());

    info!("Sending player update");
    let response = session.send_command(&Command::UpdatePlayer).await?;
    game_state.update(response)?;

    info!("Waiting a few milliseconds");
    sleep(Duration::from_millis(250)).await;

    info!("Getting scrapbook contents");
    let response = session.send_command(&Command::ViewScrapbook).await?;
    game_state.update(response)?;

    let scrapbook = game_state.unlocks.scrapbok
        .clone()
        .context("Your character doesn't have an active scrapbook!")?;

    debug!("{:?}", scrapbook.items);

    info!("Waiting a few milliseconds");
    sleep(Duration::from_millis(250)).await;

    info!("Getting players from hall of fame");
    let response = session.send_command(&Command::HallOfFamePage { page: 650 }).await?;
    game_state.update(response)?;

    let mut players_to_attack = HashSet::new();
    let hall_entries = game_state.other_players.hall_of_fame.clone();

    for hall_entry in hall_entries.iter() {
        info!("Waiting a few milliseconds");
        sleep(Duration::from_millis(1000)).await;

        info!("Viewing player {} details", hall_entry.name);
        let response = session.send_command(&Command::ViewPlayer { ident: hall_entry.name.clone() }).await?;
        game_state.update(response)?;

        if let Some(player) = game_state.other_players.lookup_name(&hall_entry.name) {
            for equip in player.equipment.0.iter().flatten() {
                let equip_ident = equip.equipment_ident().unwrap();
                if !scrapbook.items.contains(&equip_ident) {
                    debug!("Player {} has an item you haven't discovered yet", player.name);
                    players_to_attack.insert(player.name.clone());
                    break;
                }
            }
        }
    }

    Ok(())
}
