use std::{env, collections::{HashSet, HashMap}};
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
    pub discover_threshold: usize
}

#[derive(Debug)]
pub struct ScrapBookInfo {
    pub scrapbook: ScrapBook,
    pub progress: f32
}

pub struct FightPriorityQueue {
    equipment_set: HashSet<EquipmentIdent>,
    equipment_map: HashMap<String, HashSet<EquipmentIdent>>,
    player_queue: PriorityQueue<String, usize>
}

impl FightPriorityQueue {
    pub fn new() -> Self {
        Self {
            equipment_set: HashSet::new(),
            equipment_map: HashMap::new(),
            player_queue: PriorityQueue::new()
        }
    }

    pub fn push(&mut self, pair: (String, HashSet<EquipmentIdent>)) {
        self.player_queue.push(pair.0.clone(), pair.1.len());
        self.equipment_set.extend(pair.1.iter());
        self.equipment_map.insert(pair.0, pair.1);
    }

    pub fn pop(&mut self) -> Option<String> {
        let (player_name, _) = self.player_queue.pop()?;
        let equipment = self.equipment_map.remove(&player_name)?;

        let before_retain = self.equipment_set.len();
        self.equipment_set.retain(|&k| !equipment.contains(&k));

        if self.equipment_set.len() < before_retain {
            Some(player_name)
        } else {
            debug!("Skipping player {}", player_name);
            None
        }
    }

    pub fn len(&self) -> usize {
        self.player_queue.len()
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            login: env::var("SFGAME_USERNAME")
                .context("SFGAME_USERNAME")?,
            password: env::var("SFGAME_PASSWORD")
                .context("SFGAME_PASSWORD")?,
            discover_threshold: env::var("SFGAME_DISCOVER_THRESHOLD")
                .context("SFGAME_DISCOVER_THRESHOLD")
                .unwrap_or(String::from("1"))
                .parse::<usize>()
                .context("SFGAME_DISCOVER_THRESHOLD")?
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
            warn!("Server error, attempting reconnect");

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
    discover_threshold: usize,
    mut page: usize) -> Result<()>
{
    let mut running = true;
    let mut fight_queue = FightPriorityQueue::new();

    info!("Sending player update");
    command(session, game_state, &Command::UpdatePlayer).await?;

    let mut scrapbook_info = get_scrapbook_info(session, game_state).await?;
    info!("Scrapbook progress: {:.2}%", scrapbook_info.progress);

    while running {
        get_players_to_fight(
            session, game_state,
            &mut fight_queue,
            &scrapbook_info,
            discover_threshold,
            page
        ).await?;

        while fight_queue.len() > 0 {
            if let Some(player_name) = fight_queue.pop() {
                fight_player(session, game_state, player_name).await?;

                info!("Sending player update");
                command(session, game_state, &Command::UpdatePlayer).await?;

                scrapbook_info = get_scrapbook_info(session, game_state).await?;
                info!("Scrapbook progress: {:.2}%", scrapbook_info.progress);

                if let Some(last_fight) = game_state.last_fight.as_ref() {
                    if !last_fight.has_player_won {
                        info!("Last fight lost, exiting");
                        running = false;

                        break;
                    }
                }
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
    info!("Updating scrapbook contents");
    command(session, game_state, &Command::ViewScrapbook).await?;

    let scrapbook = game_state.unlocks.scrapbok
        .clone()
        .context("Your character doesn't have an active scrapbook!")?;

    let count = game_state.unlocks.scrapbook_count
        .context("Your character doesn't have an active scrapbook!")?;

    let progress = (count as f32 / SFGAME_SCRAPBOOK_TOTAL as f32) * 100.0;

    Ok(ScrapBookInfo { scrapbook, progress })
}

pub async fn get_players_to_fight(
    session: &mut CharacterSession,
    game_state: &mut GameState,
    fight_queue: &mut FightPriorityQueue,
    scrapbook_info: &ScrapBookInfo,
    discover_threshold: usize,
    page: usize) -> Result<()>
{
    info!("Getting players from hall of fame (index: {})", page * 30);
    command(session, game_state, &Command::HallOfFamePage { page }).await?;

    let hall_entries = game_state.other_players.hall_of_fame.clone();

    for hall_entry in hall_entries.iter() {
        debug!("Viewing player {} details", hall_entry.name);
        command(session, game_state, &Command::ViewPlayer { ident: hall_entry.name.clone() }).await?;

        let player = game_state.other_players.lookup_name(&hall_entry.name)
            .context("Player lookup failed")?
            .clone();

        let mut missing_items = HashSet::new();

        for equip in player.equipment.0.iter().flatten() {
            let equip_ident = equip.equipment_ident().unwrap();

            if !scrapbook_info.scrapbook.items.contains(&equip_ident) {
                missing_items.insert(equip_ident);
            }
        }

        if missing_items.len() >= discover_threshold {
            info!("Player {} has an item you haven't discovered yet ({})", player.name, missing_items.len());
            fight_queue.push((player.name.clone(), missing_items));
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
        info!("Checking if free fight is available");
        command(session, game_state, &Command::CheckArena).await?;

        let free_fight = game_state.arena.next_free_fight.context("Free fight unavailable")?;
        let mut wait_time = free_fight - Local::now();

        if wait_time.num_milliseconds() > 0 {
            wait_time = wait_time + chrono::Duration::seconds(5);
            info!("Waiting {} seconds until free fight is available", wait_time.num_seconds());
            sleep(Duration::from_millis(u64::try_from(wait_time.num_milliseconds()).unwrap())).await;
        } else { break; }
    }

    info!("Fighting player {}", player_name);
    command(session, game_state, &Command::Fight { name: player_name, use_mushroom: false }).await?;

    Ok(())
}
