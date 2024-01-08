use std::io;
use chrono::Local;
use log::{info, debug, warn};
use anyhow::{Context, Result, bail};
use tokio::time::{sleep, Duration};
use sf_api::{
    sso::SFAccount,
    session::CharacterSession,
    gamestate::GameState,
    command::Command,
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

    info!(target: "main", "Logged in as {} ({})", session.username(), session.server_url());

    search_and_attack(&mut session, &mut game_state).await?;

    Ok(())
}

pub async fn command(
    session: &mut CharacterSession,
    game_state: &mut GameState,
    command: &Command) -> Result<()>
{
    let mut retries = 0;

    while retries < 5 {
        let response = session.send_command(command).await?;
        sleep(Duration::from_millis(250)).await;

        if let Err(_) = game_state.update(response) {
            warn!(target: "main", "Server error, attempting reconnect");

            let response = session.login().await?;
            sleep(Duration::from_secs(1)).await;

            game_state.update(response)?;
        } else {
            return Ok(());
        }

        retries += 1;
    }

    bail!("Maximum number of retries reached")
}

pub async fn search_and_attack(
    session: &mut CharacterSession,
    game_state: &mut GameState) -> Result<()>
{
    let mut page = 20_000 / 30;

    info!(target: "search", "Updating scrapbook contents");
    command(session, game_state, &Command::ViewScrapbook).await?;

    let mut scrapbook = game_state.unlocks.scrapbok
        .clone()
        .context("Your character doesn't have an active scrapbook!")?;

    loop {
        info!(target: "search", "Sending player update");
        command(session, game_state, &Command::UpdatePlayer).await?;

        info!(target: "search", "Getting players from hall of fame (index: {})", page * 30);
        command(session, game_state, &Command::HallOfFamePage { page }).await?;

        let hall_entries = game_state.other_players.hall_of_fame.clone();

        for hall_entry in hall_entries.iter() {
            debug!(target: "search", "Viewing player {} details", hall_entry.name);
            command(session, game_state, &Command::ViewPlayer { ident: hall_entry.name.clone() }).await?;

            let player = game_state.other_players.lookup_name(&hall_entry.name)
                .context("Player lookup failed")?
                .clone();

            for equip in player.equipment.0.iter().flatten() {
                let equip_ident = equip.equipment_ident().unwrap();
                if !scrapbook.items.contains(&equip_ident) {
                    info!(target: "search", "Player {} has an item you haven't discovered yet", player.name);

                    loop {
                        info!(target: "search", "Checking if free fight is available");
                        command(session, game_state, &Command::CheckArena).await?;

                        let free_fight = game_state.arena.next_free_fight.context("Free fight unavailable")?;
                        let wait_time = free_fight.time() - Local::now().time();

                        if wait_time.num_milliseconds() > 0 {
                            let wait_time = wait_time + chrono::Duration::seconds(5);
                            info!(target: "search", "Waiting {} seconds until free fight is available", wait_time.num_seconds());
                            sleep(Duration::from_millis(u64::try_from(wait_time.num_milliseconds()).unwrap())).await;
                        } else { break; }
                    }

                    info!(target: "search", "Fighting player {}", player.name);
                    command(session, game_state, &Command::Fight { name: player.name.clone(), use_mushroom: false }).await?;

                    info!(target: "search", "Updating scrapbook contents");
                    command(session, game_state, &Command::ViewScrapbook).await?;

                    scrapbook = game_state.unlocks.scrapbok
                        .clone()
                        .context("Your character doesn't have an active scrapbook!")?;

                    break;
                }
            }
        }

        if let Some(last_fight) = game_state.last_fight.as_ref() {
            if !last_fight.has_player_won {
                info!("Last fight lost, exiting");
                break;
            }
        }

        page -= 1;
    }

    Ok(())
}