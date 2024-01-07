use std::{
    env,
    sync::Arc,
    collections::HashSet
};
use log::{info, debug};
use anyhow::{Context, Result};
use tokio::{
    sync::Mutex,
    time::{sleep, Duration},
    join
};
use sf_api::{
    gamestate::{
        GameState,
        unlockables::ScrapBook
    },
    command::Command,
    session::CharacterSession
};

#[derive(Debug)]
pub struct Config {
    pub login: String,
    pub password: String
}

impl Config {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            login: env::var("SFGAME_USERNAME").context("SFGAME_USERNAME")?,
            password: env::var("SFGAME_PASSWORD").context("SFGAME_PASSWORD")?
        })
    }
}

pub async fn thread_player_update() {

}

pub async fn thread_player_attack() {

}

pub async fn thread_scrape_halloffame(session: Arc<Mutex<CharacterSession>>, game_state: Arc<Mutex<GameState>>) -> Result<()> {
    loop {
        let (mut session, mut game_state) = join!(session.lock(), game_state.lock());

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

        let players_to_attack = get_players_to_attack(&mut session, &mut game_state, &scrapbook).await?;
        debug!("{:?}", players_to_attack);
    }
}

pub async fn get_players_to_attack(session: &mut CharacterSession, game_state: &mut GameState, scrapbook: &ScrapBook) -> Result<HashSet<String>> {
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

    Ok(players_to_attack)
}