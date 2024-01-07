use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering}
    }
};
use chrono::Local;
use log::{info, debug, warn};
use anyhow::{Context, Result, bail};
use tokio::{
    signal,
    time::{sleep, Duration},
    sync::Mutex,
    join
};
use sf_api::{
    sso::SFAccount,
    session::CharacterSession,
    gamestate::GameState,
    command::Command,
};

use sfscraper::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
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
    let game_state = GameState::new(login_response)?;

    info!(target: "main", "Logged in as {} ({})", session.username(), session.server_url());

    let session = Arc::new(Mutex::new(session));
    let game_state = Arc::new(Mutex::new(game_state));

    let signal_task = tokio::spawn(signal_task_entry(running.clone()));
    let player_update_task = tokio::spawn(player_update_task_entry(running.clone(), session.clone(), game_state.clone()));
    let search_and_attack_task = tokio::spawn(search_and_attack_entry(running.clone(), session.clone(), game_state.clone()));

    let results = join!(signal_task, player_update_task, search_and_attack_task);
    results.0??; results.1??; results.2??;

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

pub async fn signal_task_entry(
    running: Arc<AtomicBool>) -> Result<()>
{
    signal::ctrl_c().await?;
    running.store(false, Ordering::SeqCst);

    Ok(())
}

pub async fn player_update_task_entry(
    running: Arc<AtomicBool>,
    session: Arc<Mutex<CharacterSession>>,
    game_state: Arc<Mutex<GameState>>) -> Result<()>
{
    while running.load(Ordering::SeqCst) {
        {
            let (mut session, mut game_state) = join!(session.lock(), game_state.lock());

            debug!(target: "update", "Sending player update");
            command(&mut session, &mut game_state, &Command::UpdatePlayer).await?;
        }

        debug!(target: "update", "Waiting 5 minutes before next update");
        sleep(Duration::from_secs(300)).await;
    }

    Ok(())
}

pub async fn search_and_attack_entry(
    running: Arc<AtomicBool>,
    session: Arc<Mutex<CharacterSession>>,
    game_state: Arc<Mutex<GameState>>) -> Result<()>
{
    let mut page = 20_000 / 30;

    let mut scrapbook = {
        let (mut session, mut game_state) = join!(session.lock(), game_state.lock());

        info!(target: "search", "Updating scrapbook contents");
        command(&mut session, &mut game_state, &Command::ViewScrapbook).await?;

        game_state.unlocks.scrapbok
            .clone()
            .context("Your character doesn't have an active scrapbook!")?
    };

    while running.load(Ordering::SeqCst) {
        let hall_entries = {
            let (mut session, mut game_state) = join!(session.lock(), game_state.lock());

            info!(target: "search", "Getting players from hall of fame (index: {})", page * 30);
            command(&mut session, &mut game_state, &Command::HallOfFamePage { page }).await?;

            game_state.other_players.hall_of_fame.clone()
        };

        for hall_entry in hall_entries.iter() {
            let player = {
                let (mut session, mut game_state) = join!(session.lock(), game_state.lock());

                debug!(target: "search", "Viewing player {} details", hall_entry.name);
                command(&mut session, &mut game_state, &Command::ViewPlayer { ident: hall_entry.name.clone() }).await?;

                game_state.other_players.lookup_name(&hall_entry.name)
                    .context("Player lookup failed")?
                    .clone()
            };

            for equip in player.equipment.0.iter().flatten() {
                let equip_ident = equip.equipment_ident().unwrap();
                if !scrapbook.items.contains(&equip_ident) {
                    info!(target: "search", "Player {} has an item you haven't discovered yet", player.name);

                    loop {
                        let free_fight = {
                            let (mut session, mut game_state) = join!(session.lock(), game_state.lock());

                            info!(target: "search", "Checking if free fight is available");
                            command(&mut session, &mut game_state, &Command::CheckArena).await?;

                            game_state.arena.next_free_fight.context("Free fight unavailable")?
                        };

                        let wait_time = free_fight.time() - Local::now().time();

                        if wait_time.num_milliseconds() > 0 {
                            let wait_time = wait_time + chrono::Duration::seconds(5);
                            info!(target: "search", "Waiting {} seconds until free fight is available", wait_time.num_seconds());
                            sleep(Duration::from_millis(u64::try_from(wait_time.num_milliseconds()).unwrap())).await;
                        } else {
                            break;
                        }
                    }

                    {
                        let (mut session, mut game_state) = join!(session.lock(), game_state.lock());

                        info!(target: "search", "Fighting player {}", player.name);
                        command(&mut session, &mut game_state, &Command::Fight { name: player.name.clone(), use_mushroom: false }).await?;

                        info!(target: "search", "Updating scrapbook contents");
                        command(&mut session, &mut game_state, &Command::ViewScrapbook).await?;

                        scrapbook = game_state.unlocks.scrapbok
                            .clone()
                            .context("Your character doesn't have an active scrapbook!")?;
                    }

                    break;
                }
            }
        }

        page -= 1;
    }

    Ok(())
}