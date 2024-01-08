use std::{env, collections::HashSet};
use chrono::Local;
use log::{info, debug, warn};
use anyhow::{Context, Result, bail};
use tokio::time::{sleep, Duration};
use priority_queue::PriorityQueue;
use sf_api::{
    session::CharacterSession,
    gamestate::{
        GameState,
        unlockables::{
            ScrapBook,
            EquipmentIdent
        }
    },
    command::Command,
};

const SFGAME_SCRAPBOOK_TOTAL: u32 = 2283;

#[derive(Debug)]
pub struct Config {
    pub login: String,
    pub password: String,
    pub start_index: usize
}

#[derive(Debug)]
pub struct ScrapBookInfo {
    pub scrapbook: ScrapBook,
    pub progress: f32
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            login: env::var("SFGAME_USERNAME")
                .context("SFGAME_USERNAME")?,
            password: env::var("SFGAME_PASSWORD")
                .context("SFGAME_PASSWORD")?,
            start_index: env::var("SFGAME_START_INDEX")
                .context("SFGAME_START_INDEX")?
                .parse::<usize>()
                .context("SFGAME_START_INDEX")?
        })
    }
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
    game_state: &mut GameState,
    mut page: usize) -> Result<()>
{
    let mut players_to_attack = PriorityQueue::new();
    let mut items_to_discover: HashSet<EquipmentIdent> = HashSet::new();

    loop {
        info!(target: "search", "Sending player update");
        command(session, game_state, &Command::UpdatePlayer).await?;

        if players_to_attack.is_empty() {
            items_to_discover.clear();

            get_players_to_attack(
                session, game_state,
                &mut players_to_attack, &mut items_to_discover, page
            ).await?;
        }

        if let Some((player_name, _)) = players_to_attack.pop() {
            fight_player(session, game_state, player_name).await?;
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

pub async fn get_scrapbook_info(
    session: &mut CharacterSession,
    game_state: &mut GameState) -> Result<ScrapBookInfo>
{
    info!(target: "search", "Updating scrapbook contents");
    command(session, game_state, &Command::ViewScrapbook).await?;

    let scrapbook = game_state.unlocks.scrapbok
        .clone()
        .context("Your character doesn't have an active scrapbook!")?;

    let count = game_state.unlocks.scrapbook_count
        .context("Your character doesn't have an active scrapbook!")?;

    let progress = (count as f32 / SFGAME_SCRAPBOOK_TOTAL as f32) * 100.0;

    Ok(ScrapBookInfo { scrapbook, progress })
}

pub async fn get_players_to_attack(
    session: &mut CharacterSession,
    game_state: &mut GameState,
    players_to_attack: &mut PriorityQueue<String, u32>,
    items_to_discover: &mut HashSet<EquipmentIdent>,
    page: usize) -> Result<()>
{
    let scrapbook_info = get_scrapbook_info(session, game_state).await?;
    info!(target: "search", "Scrapbook progress: {:.2}%", scrapbook_info.progress);

    info!(target: "search", "Getting players from hall of fame (index: {})", page * 30);
    command(session, game_state, &Command::HallOfFamePage { page }).await?;

    let hall_entries = game_state.other_players.hall_of_fame.clone();

    for hall_entry in hall_entries.iter() {
        debug!(target: "search", "Viewing player {} details", hall_entry.name);
        command(session, game_state, &Command::ViewPlayer { ident: hall_entry.name.clone() }).await?;

        let player = game_state.other_players.lookup_name(&hall_entry.name)
            .context("Player lookup failed")?
            .clone();

        let mut uncovered = 0;

        for equip in player.equipment.0.iter().flatten() {
            let equip_ident = equip.equipment_ident().unwrap();

            if !scrapbook_info.scrapbook.items.contains(&equip_ident) {
                if items_to_discover.insert(equip_ident) {
                    uncovered += 1;
                }
            }
        }

        if uncovered > 0 {
            info!(target: "search", "Player {} has an item you haven't discovered yet ({})", player.name, uncovered);
            players_to_attack.push(player.name.clone(), uncovered);
        }
    }

    Ok(())
}

pub async fn fight_player(
    session: &mut CharacterSession,
    game_state: &mut GameState,
    player_name: String) -> Result<()>
{
    loop {
        info!(target: "fight", "Checking if free fight is available");
        command(session, game_state, &Command::CheckArena).await?;

        let free_fight = game_state.arena.next_free_fight.context("Free fight unavailable")?;
        let mut wait_time = free_fight - Local::now();

        if wait_time.num_milliseconds() > 0 {
            wait_time = wait_time + chrono::Duration::seconds(5);
            info!(target: "fight", "Waiting {} seconds until free fight is available", wait_time.num_seconds());
            sleep(Duration::from_millis(u64::try_from(wait_time.num_milliseconds()).unwrap())).await;
        } else { break; }
    }

    info!(target: "fight", "Fighting player {}", player_name);
    command(session, game_state, &Command::Fight { name: player_name, use_mushroom: false }).await?;

    Ok(())
}
