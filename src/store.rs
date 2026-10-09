use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};
use teloxide::{
    types::{ChatId, User, UserId},
    utils::html,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Participant {
    pub id: UserId,
    pub name: String,
    pub username: Option<String>,
}

impl Participant {
    pub fn from_user(user: &User) -> Self {
        Self {
            id: user.id,
            name: user.full_name(),
            username: user.username.clone(),
        }
    }

    /// HTML-escaped display name, e.g. `Jane Doe (@jane)`.
    pub fn display(&self) -> String {
        match &self.username {
            Some(username) => format!("{} (@{})", html::escape(&self.name), html::escape(username)),
            None => html::escape(&self.name),
        }
    }
}

/// A Secret Santa round in one group chat.
#[derive(Debug, Serialize, Deserialize)]
pub struct Game {
    pub title: String,
    /// The admin who ran /start; only they can draw names.
    pub organizer: Participant,
    pub participants: Vec<Participant>,
    /// `(giver, receiver)` pairs, set once names have been drawn.
    pub assignments: Option<Vec<(UserId, UserId)>>,
}

impl Game {
    pub fn is_drawn(&self) -> bool {
        self.assignments.is_some()
    }

    pub fn is_registered(&self, user: UserId) -> bool {
        self.participants.iter().any(|p| p.id == user)
    }

    pub fn participant(&self, user: UserId) -> Option<&Participant> {
        self.participants.iter().find(|p| p.id == user)
    }
}

/// All games, keyed by group chat id, persisted to a JSON file after every change so
/// registrations and draws survive a bot restart.
pub struct Store {
    path: PathBuf,
    games: Mutex<HashMap<i64, Game>>,
}

impl Store {
    pub fn load(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let games = match fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e),
        };
        Ok(Self { path, games: Mutex::new(games) })
    }

    /// Runs `f` on the game for `chat` without modifying anything.
    pub fn read<R>(&self, chat: ChatId, f: impl FnOnce(Option<&Game>) -> R) -> R {
        let games = self.games.lock().unwrap();
        f(games.get(&chat.0))
    }

    /// Runs `f` on the game for `chat` and saves the result, or returns `None` if no game
    /// has been started there.
    pub fn update<R>(&self, chat: ChatId, f: impl FnOnce(&mut Game) -> R) -> Option<R> {
        let mut games = self.games.lock().unwrap();
        let result = f(games.get_mut(&chat.0)?);
        self.save(&games);
        Some(result)
    }

    /// Starts `game` in `chat`, unless one is already running there, in which case that
    /// game's organiser is returned.
    pub fn start(&self, chat: ChatId, game: Game) -> Result<(), Participant> {
        let mut games = self.games.lock().unwrap();
        if let Some(existing) = games.get(&chat.0) {
            return Err(existing.organizer.clone());
        }
        games.insert(chat.0, game);
        self.save(&games);
        Ok(())
    }

    pub fn remove(&self, chat: ChatId) {
        let mut games = self.games.lock().unwrap();
        games.remove(&chat.0);
        self.save(&games);
    }

    /// Everyone `user` has to buy a gift for, across all groups, as `(group title, receiver)`.
    pub fn matches_for(&self, user: UserId) -> Vec<(String, Participant)> {
        let games = self.games.lock().unwrap();
        games
            .values()
            .filter_map(|game| {
                let (_, receiver) = game.assignments.as_ref()?.iter().find(|(giver, _)| *giver == user)?;
                Some((game.title.clone(), game.participant(*receiver)?.clone()))
            })
            .collect()
    }

    fn save(&self, games: &HashMap<i64, Game>) {
        if let Err(e) = write_atomically(&self.path, games) {
            log::error!("Failed to save state to {}: {e}", self.path.display());
        }
    }
}

fn write_atomically(path: &Path, games: &HashMap<i64, Game>) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(games)?)?;
    fs::rename(&tmp, path)
}
