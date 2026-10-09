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
    /// The admin who ran /start; only they can draw names. Rounds saved before organisers
    /// existed have none, and any admin can draw those.
    #[serde(default)]
    pub organizer: Option<Participant>,
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

    /// Who `giver` is buying a gift for, once names have been drawn.
    pub fn match_for(&self, giver: UserId) -> Option<&Participant> {
        let (_, receiver) = self.assignments.as_ref()?.iter().find(|(g, _)| *g == giver)?;
        self.participant(*receiver)
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
    /// game's organiser (if it has one) is returned.
    pub fn start(&self, chat: ChatId, game: Game) -> Result<(), Option<Participant>> {
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
            .filter_map(|game| Some((game.title.clone(), game.match_for(user)?.clone())))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_rounds_saved_before_organisers_existed() {
        let old_state = r#"{
            "-1001234567890": {
                "title": "Family",
                "participants": [{ "id": 42, "name": "Jane Doe", "username": "jane" }],
                "assignments": null
            }
        }"#;
        let games: HashMap<i64, Game> = serde_json::from_str(old_state).unwrap();

        let game = &games[&-1001234567890];
        assert!(game.organizer.is_none());
        assert!(game.is_registered(UserId(42)));
    }

    #[test]
    fn removing_a_round_forgets_its_matches() {
        let path = std::env::temp_dir().join(format!("santa_test_{}.json", std::process::id()));
        let store = Store::load(&path).unwrap();
        let chat = ChatId(-100);
        let person = |id| Participant { id: UserId(id), name: format!("P{id}"), username: None };
        let game = Game {
            title: "Family".into(),
            organizer: Some(person(1)),
            participants: vec![person(1), person(2), person(3)],
            assignments: Some(vec![(UserId(1), UserId(2)), (UserId(2), UserId(3)), (UserId(3), UserId(1))]),
        };
        store.start(chat, game).unwrap();
        assert_eq!(store.matches_for(UserId(1)).len(), 1);

        store.remove(chat);
        assert!(store.matches_for(UserId(1)).is_empty());
        assert!(Store::load(&path).unwrap().matches_for(UserId(1)).is_empty());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn matches_are_kept_per_group() {
        let path = std::env::temp_dir().join(format!("santa_test_groups_{}.json", std::process::id()));
        let store = Store::load(&path).unwrap();
        let person = |id| Participant { id: UserId(id), name: format!("P{id}"), username: None };
        let drawn = |title: &str, pairs: Vec<(u64, u64)>| Game {
            title: title.into(),
            organizer: Some(person(1)),
            participants: (1..=4).map(person).collect(),
            assignments: Some(pairs.into_iter().map(|(g, r)| (UserId(g), UserId(r))).collect()),
        };
        store.start(ChatId(-1), drawn("Family", vec![(1, 2), (2, 3), (3, 1)])).unwrap();
        store.start(ChatId(-2), drawn("Work", vec![(1, 4), (4, 2), (2, 1)])).unwrap();

        let in_family = store.read(ChatId(-1), |game| game.unwrap().match_for(UserId(1)).unwrap().id);
        let in_work = store.read(ChatId(-2), |game| game.unwrap().match_for(UserId(1)).unwrap().id);
        assert_eq!(in_family, UserId(2));
        assert_eq!(in_work, UserId(4));
        assert_eq!(store.matches_for(UserId(1)).len(), 2);

        fs::remove_file(path).unwrap();
    }
}
