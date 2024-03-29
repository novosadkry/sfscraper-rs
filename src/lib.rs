use std::{
    env,
    str::FromStr,
    collections::{HashSet, HashMap}
};
use chrono::Local;
use log::{info, debug, warn};
use anyhow::{
    Context, Result,
    bail, anyhow
};
use tokio::time::{sleep, Duration};
use priority_queue::PriorityQueue;
use sf_api::{
    session::CharacterSession,
    gamestate::{
        GameState,
        unlockables::{
            ScrapBook,
            EquipmentIdent
        },
        arena::Fight,
        social::HallOfFameEntry
    },
    command::Command
};

const SFGAME_SCRAPBOOK_TOTAL: u32 = 2283;

#[derive(Debug)]
pub struct Config {
    pub login: String,
    pub password: String,
    pub steam_login: bool,
    pub level_threshold: u16,
    pub discover_threshold: usize,
    pub search_strategy: SearchStrategy,
    pub search_direction: SearchDirection
}

#[derive(Debug, Clone, Copy)]
pub enum SearchStrategy {
    Simple,
    Prefetch
}

#[derive(Debug, Clone, Copy)]
pub enum SearchDirection {
    Ascending,
    Descending
}

#[derive(Debug)]
pub struct SearchSettings {
    pub level_threshold: u16,
    pub discover_threshold: usize,
    pub search_direction: SearchDirection
}

#[derive(Debug)]
pub struct ScrapBookInfo {
    pub scrapbook: ScrapBook,
    pub progress: f32
}

#[derive(Debug)]
pub enum FightPriorityQueueItem {
    Ok(String),
    Skip(String)
}

#[derive(Debug)]
pub struct FightPriorityQueue {
    equipment_set: HashSet<EquipmentIdent>,
    equipment_map: HashMap<String, HashSet<EquipmentIdent>>,
    player_queue: PriorityQueue<String, usize>
}

impl FromStr for SearchStrategy {
    type Err = ();

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "simple" => Ok(SearchStrategy::Simple),
            "prefetch" => Ok(SearchStrategy::Prefetch),
            _ => Err(())
        }
    }
}

impl FromStr for SearchDirection {
    type Err = ();

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        match input {
            "ascending" => Ok(SearchDirection::Ascending),
            "descending" => Ok(SearchDirection::Descending),
            _ => Err(())
        }
    }
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

    pub fn pop(&mut self) -> Option<FightPriorityQueueItem> {
        let (player_name, _) = self.player_queue.pop()?;
        let equipment = self.equipment_map.remove(&player_name)?;

        let before_retain = self.equipment_set.len();
        self.equipment_set.retain(|&k| !equipment.contains(&k));

        if self.equipment_set.len() < before_retain {
            Some(FightPriorityQueueItem::Ok(player_name))
        } else {
            Some(FightPriorityQueueItem::Skip(player_name))
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
            steam_login: env::var("SFGAME_STEAM_LOGIN")
                .context("SFGAME_STEAM_LOGIN")
                .unwrap_or(String::from("false"))
                .parse::<bool>()
                .context("SFGAME_STEAM_LOGIN")?,
            level_threshold: env::var("SFGAME_LEVEL_THRESHOLD")
                .context("SFGAME_LEVEL_THRESHOLD")
                .unwrap_or(String::from("500"))
                .parse::<u16>()
                .context("SFGAME_LEVEL_THRESHOLD")?,
            discover_threshold: env::var("SFGAME_DISCOVER_THRESHOLD")
                .context("SFGAME_DISCOVER_THRESHOLD")
                .unwrap_or(String::from("1"))
                .parse::<usize>()
                .context("SFGAME_DISCOVER_THRESHOLD")?,
            search_strategy: env::var("SFGAME_SEARCH_STRATEGY")
                .context("SFGAME_SEARCH_STRATEGY")
                .unwrap_or(String::from("SIMPLE"))
                .parse::<SearchStrategy>()
                .map_err(|_| anyhow!("SFGAME_SEARCH_STRATEGY"))?,
            search_direction: env::var("SFGAME_SEARCH_DIRECTION")
                .context("SFGAME_SEARCH_DIRECTION")
                .unwrap_or(String::from("ASCENDING"))
                .parse::<SearchDirection>()
                .map_err(|_| anyhow!("SFGAME_SEARCH_DIRECTION"))?
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
        sleep(Duration::from_millis(250)).await;

        if let Err(error) = session
            .send_command(command).await
            .and_then(|response| game_state.update(response))
        {
            warn!("{:?}", error);
            warn!("Server error, attempting reconnect");

            let response = session.login().await?;
            game_state.update(response)?;

            sleep(Duration::from_secs(1)).await;
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
    search_strategy: SearchStrategy,
    search_settings: SearchSettings,
    mut page: usize) -> Result<()>
{
    let mut running = true;
    let mut last_fight: Option<Fight> = None;
    let mut fight_queue = FightPriorityQueue::new();
    let guild_members = get_guild_members(game_state);

    info!("Sending player update");
    command(session, game_state, &Command::UpdatePlayer).await?;

    let mut scrapbook_info = get_scrapbook_info(session, game_state).await?;
    info!("Scrapbook progress: {:.2}%", scrapbook_info.progress);

    wait_free_fight(session, game_state).await?;

    while running {
        match search_strategy {
            SearchStrategy::Prefetch => {
                get_players_to_fight(
                    session, game_state,
                    &mut fight_queue,
                    &scrapbook_info,
                    &search_settings,
                    &guild_members,
                    page).await?;

                while fight_queue.len() > 0 {
                    match fight_queue.pop() {
                        Some(FightPriorityQueueItem::Ok(player_name)) => {
                            info!("Fighting player {}", player_name);
                            command(session, game_state, &Command::Fight { name: player_name, use_mushroom: false }).await?;

                            last_fight = game_state.last_fight.take();

                            info!("Sending player update");
                            command(session, game_state, &Command::UpdatePlayer).await?;

                            scrapbook_info = get_scrapbook_info(session, game_state).await?;
                            info!("Scrapbook progress: {:.2}%", scrapbook_info.progress);

                            wait_free_fight(session, game_state).await?;
                        },
                        Some(FightPriorityQueueItem::Skip(player_name)) => {
                            info!("Player {} had all items discovered, skipping", player_name);
                        },
                        None => bail!("Item popped while fight_queue length is zero")
                    }
                }
            },
            SearchStrategy::Simple => {
                info!("Getting players from hall of fame (index: {})", page * 30);
                command(session, game_state, &Command::HallOfFamePage { page }).await?;

                let hall_entries = game_state.other_players.hall_of_fame.clone();

                for hall_entry in hall_entries.iter() {
                    let Some(player_name) = get_player_to_fight(
                        session, game_state,
                        hall_entry,
                        &scrapbook_info,
                        &search_settings,
                        &guild_members).await?
                    else { continue; };

                    info!("Fighting player {}", player_name);
                    command(session, game_state, &Command::Fight { name: player_name, use_mushroom: false }).await?;

                    last_fight = game_state.last_fight.take();

                    info!("Sending player update");
                    command(session, game_state, &Command::UpdatePlayer).await?;

                    scrapbook_info = get_scrapbook_info(session, game_state).await?;
                    info!("Scrapbook progress: {:.2}%", scrapbook_info.progress);

                    wait_free_fight(session, game_state).await?;
                }
            }
        }

        if let Some(fight) = &last_fight {
            if !fight.has_player_won {
                info!("Last fight lost, exiting");
                running = false;
            }
        }

        match search_settings.search_direction {
            SearchDirection::Ascending => {
                if page > 0 {
                    page -= 1;
                } else if running {
                    info!("Last page reached, exiting");
                    running = false;
                }
            }
            SearchDirection::Descending => {
                // TODO: Handle out-of-bounds pages
                page += 1;
            }
        }
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

pub async fn get_player_to_fight(
    session: &mut CharacterSession,
    game_state: &mut GameState,
    hall_entry: &HallOfFameEntry,
    scrapbook_info: &ScrapBookInfo,
    search_settings: &SearchSettings,
    guild_members: &Option<Vec<String>>) -> Result<Option<String>>
{
    if let Some(guild_members) = guild_members {
        if guild_members.contains(&hall_entry.name) {
            info!("Player {} is a guild member, skipping", hall_entry.name);
            return Ok(None)
        }
    }

    debug!("Viewing player {} details", hall_entry.name);
    command(session, game_state, &Command::ViewPlayer { ident: hall_entry.name.clone() }).await?;

    let player = game_state.other_players.lookup_name(&hall_entry.name)
        .context("Player lookup failed")?
        .clone();

    if player.level > search_settings.level_threshold {
        warn!("Player {} surpasses max level threshold, skipping", player.name);
        return Ok(None)
    }

    let mut missing_items = HashSet::new();

    for equip in player.equipment.0.iter()
        .flatten()
        .filter(|e| !e.is_legendary())
    {
        let equip_ident = equip.equipment_ident().unwrap();

        if !scrapbook_info.scrapbook.items.contains(&equip_ident) {
            missing_items.insert(equip_ident);
        }
    }

    if missing_items.len() >= search_settings.discover_threshold {
        info!("Player {} has an item you haven't discovered yet ({})", player.name, missing_items.len());
        return Ok(Some(player.name.clone()));
    }

    Ok(None)
}

pub async fn get_players_to_fight(
    session: &mut CharacterSession,
    game_state: &mut GameState,
    fight_queue: &mut FightPriorityQueue,
    scrapbook_info: &ScrapBookInfo,
    search_settings: &SearchSettings,
    guild_members: &Option<Vec<String>>,
    page: usize) -> Result<()>
{
    info!("Getting players from hall of fame (index: {})", page * 30);
    command(session, game_state, &Command::HallOfFamePage { page }).await?;

    let hall_entries = game_state.other_players.hall_of_fame.clone();

    for hall_entry in hall_entries.iter() {
        if let Some(guild_members) = guild_members {
            if guild_members.contains(&hall_entry.name) {
                info!("Player {} is a guild member, skipping", hall_entry.name);
                continue;
            }
        }

        debug!("Viewing player {} details", hall_entry.name);
        command(session, game_state, &Command::ViewPlayer { ident: hall_entry.name.clone() }).await?;

        let player = game_state.other_players.lookup_name(&hall_entry.name)
            .context("Player lookup failed")?
            .clone();

        if player.level > search_settings.level_threshold {
            warn!("Player {} surpasses max level threshold, skipping", player.name);
            continue;
        }

        let mut missing_items = HashSet::new();

        for equip in player.equipment.0.iter()
            .flatten()
            .filter(|e| !e.is_legendary())
        {
            let equip_ident = equip.equipment_ident().unwrap();

            if !scrapbook_info.scrapbook.items.contains(&equip_ident) {
                missing_items.insert(equip_ident);
            }
        }

        if missing_items.len() >= search_settings.discover_threshold {
            info!("Player {} has an item you haven't discovered yet ({})", player.name, missing_items.len());
            fight_queue.push((player.name.clone(), missing_items));
        }
    }

    Ok(())
}

pub async fn wait_free_fight(
    session: &mut CharacterSession,
    game_state: &mut GameState) -> Result<()>
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

    Ok(())
}

fn get_guild_members(
    game_state: &mut GameState) -> Option<Vec<String>>
{
    if let Some(guild_members) = game_state.unlocks.guild
        .as_ref()
        .map(|g| &g.members)
    {
        Some(
            guild_members
                .iter()
                .map(|p| p.name.clone())
                .collect::<Vec<String>>()
        )
    }
    else { None }
}