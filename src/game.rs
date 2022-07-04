//! Game
use crate::words::Words;
use anyhow::{anyhow, bail, Result};
use futures::TryStreamExt;
use rusqlite::{params, OptionalExtension};
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::Mutex,
    time::{Duration, Instant},
};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Model types
////////////////////////////////////////////////////////////////////////////////////////////////////

pub struct Player {
    pub id: i64,
    pub nick: String,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Schema
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Creates the database tables.
fn setup_schema(conn: &mut rusqlite::Connection) -> Result<()> {
    // players: ID -> nick, score (total score)
    // sessions (result of past sessions): ID -> start_date, end_date, planned_end_date, word, winner, is_current (whether the session is in progress)
    // current_session:
    // guesses (records all guesses made across all sessions): ID -> session ID, player ID, guess, cosine
    conn.execute_batch(
        // language=SQLITE-SQL
        r#"
CREATE TABLE IF NOT EXISTS players
         (id      INTEGER PRIMARY KEY,
          nick    TEXT UNIQUE NOT NULL,
          score   INTEGER);

CREATE TABLE IF NOT EXISTS sessions
         (id               INTEGER PRIMARY KEY,
		  start_date       INTEGER,
          end_date         INTEGER,
		  planned_end_date INTEGER,
		  word             TEXT NOT NULL,
		  winner_id        INTEGER REFERENCES players(id) ON DELETE NO ACTION,
		  playing          INTEGER);

CREATE TABLE IF NOT EXISTS current_session
         (id            INTEGER PRIMARY KEY DEFAULT 0,
          session_id    INTEGER REFERENCES sessions(id) ON DELETE NO ACTION);

INSERT OR IGNORE INTO current_session(id) VALUES (0);

CREATE TABLE IF NOT EXISTS guesses
         (id         INTEGER PRIMARY KEY,
          session_id INTEGER REFERENCES sessions(id) ON DELETE NO ACTION,
          player_id  INTEGER REFERENCES players(id) ON DELETE NO ACTION,
          guess      TEXT NOT NULL,
          cosine     NUMERIC);
          "#,
    )?;

    Ok(())
}

/// The outcome of a guess.
pub enum Outcome {
    /// The player found the word and won. The game is now ended.
    Win,
    /// The player did not find the word.
    Miss {
        /// The distance to the actual word.
        distance: f32,
    },
    /// The player did not enter a recognized word
    UnknownWord,
}

struct GameState {
    conn: rusqlite::Connection,
    /// Current session ID. `None` if there's no game in progress.
    session_id: Option<i64>,
    /// Current word to guess.
    word: String,
    /// Word database.
    words: Arc<Words>,
}

impl GameState {
    pub fn load(mut conn: rusqlite::Connection, words: Arc<Words>) -> Result<GameState> {
        setup_schema(&mut conn)?;
        // language=SQLITE-SQL
        let session_id: Option<i64> =
            conn.query_row(r#"SELECT session_id FROM current_session WHERE id=0"#, [], |row| {
                row.get(0)
            })?;

        if let Some(session_id) = session_id {
            let (planned_end_date, word): (u64, String) = conn.query_row(
                "SELECT planned_end_date, word FROM sessions WHERE id=?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            let end_date = UNIX_EPOCH.checked_add(Duration::from_secs(planned_end_date)).unwrap();

            info!(
                "loaded game session id = {}, the word to find is \"{}\"",
                session_id, word
            );
            Ok(GameState {
                conn,
                session_id: Some(session_id),
                word,
                words,
            })
        } else {
            Ok(GameState {
                conn,
                session_id: None,
                word: "".to_string(),
                words,
            })
        }
    }

    /// Records a guess into the DB.
    pub fn insert_guess(&mut self, session_id: i64, guess: String, player_id: i64, cosine: f32) -> Result<()> {
        self.conn.execute(
            r#"INSERT INTO guesses(session_id, player_id, guess, cosine) VALUES (?1,?2,?3,?4)"#,
            params![session_id, player_id, guess, cosine],
        )?;
        Ok(())
    }

    /// Processes a guess from a player
    pub fn process_guess(&mut self, player_nick: String, guess: String) -> Result<Outcome> {
        // return early if there's no game in progress
        let session_id = if let Some(id) = self.session_id {
            id
        } else {
            bail!("there's no game in progress");
        };

        // query or insert player ID
        // language=SQLITE-SQL
        let player_id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM players WHERE nick==?1", [&player_nick], |row| {
                row.get(0)
            })
            .optional()?;
        let player_id = if let Some(id) = player_id {
            id
        } else {
            // unknown player, insert into players
            // language=SQLITE-SQL
            self.conn.execute(
                "INSERT INTO players(nick, score) VALUES (?1,?2);",
                params![&player_nick, 0i64],
            )?;
            self.conn.last_insert_rowid()
        };

        // cleanup guess
        let guess = guess.trim().to_lowercase();

        // fetch guess vector
        let v_guess: &[f32] = if let Some(vec) = self.words.vector(&guess) {
            vec
        } else {
            // unknown word
            return Ok(Outcome::UnknownWord);
        };

        // calculate cosine distance
        let distance = if guess == self.word {
            1.0f32
        } else {
            let v_target = self.words.vector(&self.word).ok_or(anyhow::Error::msg(
                "could not find target word in vocabulary: this is a bug",
            ))?;
            v_guess.iter().zip(v_target.iter()).map(|(&a, &b)| a * b).sum()
        };

        // record the guess
        self.insert_guess(session_id, guess.clone(), player_id, distance)?;

        if distance == 1.0 {
            // player won, end the game
            self.end_game(Some(player_id));
            Ok(Outcome::Win)
        } else {
            // not a win
            Ok(Outcome::Miss { distance })
        }
    }

    fn end_game(&mut self, winner_id: Option<i64>) -> Result<()> {
        if let Some(session_id) = self.session_id {
            let actual_end_time = SystemTime::now();
            let actual_end_time_unix = actual_end_time.duration_since(UNIX_EPOCH).unwrap().as_secs();
            if let Some(winner_id) = winner_id {
                // language=SQLITE-SQL
                self.conn.execute(
                    r#"
                UPDATE sessions SET end_date=?1, winner=?2 WHERE id=?3;
                UPDATE current_session SET session_id=NULL WHERE id=0;
                "#,
                    params![actual_end_time_unix, winner_id, session_id],
                )?;
            } else {
                // language=SQLITE-SQL
                self.conn.execute(
                    r#"
                UPDATE sessions SET end_date=?1 WHERE id=?2;
                UPDATE current_session SET session_id=NULL WHERE id=0;
                "#,
                    params![actual_end_time_unix, session_id],
                )?;
            }
            self.session_id = None;
            Ok(())
        } else {
            bail!("there's no game in progress");
        }
    }

    /// If a game is in progress, ends it with no winner. Then, picks a random word from the dictionary and starts a new game.
    pub fn start_game(&mut self, game_duration: Duration) -> Result<()> {
        if self.session_id.is_some() {
            self.end_game(None)?;
        }

        // pick a word from the dictionary
        let word = self.words.pick_word();
        let start_time = SystemTime::now();
        let end_time = start_time.checked_add(game_duration).unwrap();
        let start_time_unix = start_time.duration_since(UNIX_EPOCH).unwrap().as_secs();
        let end_time_unix = end_time.duration_since(UNIX_EPOCH).unwrap().as_secs();

        // start session
        // language=SQLITE-SQL
        self.conn.execute(
            "INSERT INTO sessions(start_date, planned_end_date, word) VALUES (?1,?2,?3);",
            params![start_time_unix, end_time_unix, word.clone()],
        )?;
        let session_id = self.conn.last_insert_rowid();
        // language=SQLITE-SQL
        self.conn.execute(
            "UPDATE current_session SET session_id=?1 WHERE id=0;",
            params![session_id],
        )?;

        self.session_id = Some(session_id);

        info!(
            "new game started at {:?}, will end at {:?} (session_id={})",
            start_time, end_time, session_id
        );

        Ok(())
    }
}

#[derive(Clone)]
pub struct Game(Arc<Mutex<GameState>>);

impl Game {
    pub fn load(conn: rusqlite::Connection, words: Arc<Words>) -> Result<Game> {
        Ok(Game(Arc::new(Mutex::new(GameState::load(conn, words)?))))
    }

    pub async fn process_guess(&self, player_nick: String, guess: String) -> Result<Outcome> {
        let state = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = state.blocking_lock();
            state.process_guess(player_nick, guess)
        })
        .await?
    }

    pub async fn start_game(&self, game_duration: Duration) -> Result<()> {
        let state = self.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut state = state.blocking_lock();
            state.start_game(game_duration)
        })
        .await?
    }
}
