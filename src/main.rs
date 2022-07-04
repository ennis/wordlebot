#[macro_use]
extern crate tracing;

mod game;
mod irccmd;
mod words;
//mod server;

use anyhow::Error;
use futures::prelude::*;
use serde::Deserialize;
use std::{fs::File, io::Read, sync::Arc, time::Duration};
use tokio::{join, try_join};

use crate::{game::Game, irccmd::irc_handler, words::Words};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Setup
////////////////////////////////////////////////////////////////////////////////////////////////////

/*fn load_word2vec_db() {
    let model =
        word2vec::wordvectors::WordVector::load_from_binary("vectors.bin").expect("Unable to load word vector model");
    println!("{:?}", model.cosine("snow", 10));
    let positive = vec!["woman", "king"];
    let negative = vec!["man"];
    println!("{:?}", model.analogy(positive, negative, 10));

    let clusters =
        word2vec::wordclusters::WordClusters::load_from_file("classes.txt").expect("Unable to load word clusters");
    println!("{:?}", clusters.get_cluster("belarus"));
    println!("{:?}", clusters.get_words_on_cluster(6));
}

struct Game {
    word_vector: Arc<WordVector>,
    bot_name: String,
}*/

////////////////////////////////////////////////////////////////////////////////////////////////////
// Config
////////////////////////////////////////////////////////////////////////////////////////////////////

fn default_model_file() -> String {
    "word2vec.bin".to_string()
}

fn default_db_path() -> String {
    "game.db".to_string()
}

fn default_game_duration() -> Duration {
    Duration::from_secs(3600 * 24)
}

#[derive(Debug, Deserialize)]
struct AppConfig {
    /// Word2Vec model binary
    #[serde(default = "default_model_file")]
    word2vec_model_file: String,
    /// Sqlite game DB path
    #[serde(default = "default_db_path")]
    db_path: String,
    /// Game duration in seconds.
    #[serde(default = "default_game_duration")]
    game_duration: Duration,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Daily game
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Main
////////////////////////////////////////////////////////////////////////////////////////////////////
#[tokio::main]
async fn main() -> Result<(), Error> {
    // install global collector configured based on RUST_LOG env var.
    tracing_subscriber::fmt::init();

    // load main config file (IRC config loaded separately)
    let config: AppConfig = {
        let mut config_str = String::new();
        File::open("cfg.toml")
            .expect("failed to open main configuration file `cfg.toml`")
            .read_to_string(&mut config_str)
            .expect("failed to read configuration file");
        toml::from_str(&config_str).expect("invalid config file")
    };

    trace!("word2vec model file : `{}`", config.word2vec_model_file);
    trace!("database file       : `{}`", config.db_path);

    info!("Loading word model file, this may take some time.");
    let words = Arc::new(Words::load(&config.word2vec_model_file).expect("could not load word database"));
    info!("Done loading word model.");

    let connection = rusqlite::Connection::open(&config.db_path).expect("can't connect to database file");
    let game = Game::load(connection, words.clone()).expect("could not start game");

    // spawn the tasks: IRC bot & web server
    let irc_task = tokio::spawn(irc_handler(words.clone(), game.clone()));
    //let server_task = tokio::spawn(launch_server(pool.clone()));

    let res = try_join!(irc_task);
    res.unwrap();
    Ok(())
}

// Game rules:
// Every day, players have to guess a random word.
//
// Once a player successfully guessed the word, the game ends, until the next one starts the next day.
//
// Implementation:
// - the IRC task is in charge of player input on the IRC channel.
//    -
