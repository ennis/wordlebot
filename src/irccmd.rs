//! IRC bot interface
use crate::{game::Outcome, Game, Words};
use anyhow::Error;
use futures::StreamExt;
use irc::client::prelude::*;
use rand::Rng;
use std::{cmp::Ordering, collections::HashMap, fs::File, future::Future, io::BufReader, sync::Arc, time::Duration};
use tokio::time::Instant;
use word2vec::{vectorreader::WordVectorReader, wordvectors::WordVector};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Commands
////////////////////////////////////////////////////////////////////////////////////////////////////

////////////////////////////////////////////////////////////////////////////////////////////////////
// Parsing
////////////////////////////////////////////////////////////////////////////////////////////////////

/// Error while parsing a command string.
#[derive(Debug, thiserror::Error)]
pub enum GameCommandParseError {
    #[error("unrecognized command")]
    Unrecognized,
    #[error("invalid syntax; expected `{expected}`")]
    SyntaxError { expected: &'static str },
}

pub enum GameCommand {
    Start,
    Thesaurus { word: String, count: Option<usize> },
    Guess { word: String },
    Halp,
}

impl GameCommand {
    fn parse(msg: &str) -> Result<GameCommand, GameCommandParseError> {
        if msg.starts_with("!thesaurus ") {
            const SYNTAX_ERROR: GameCommandParseError = GameCommandParseError::SyntaxError {
                expected: "!thesaurus <word> <count>",
            };

            let split: Vec<_> = msg.split(' ').collect();
            if split.len() < 2 || split.len() > 3 {
                return Err(SYNTAX_ERROR);
            }

            let count = if split.len() == 3 {
                Some(split[2].parse::<usize>().map_err(|_| SYNTAX_ERROR)?)
            } else {
                None
            };

            Ok(GameCommand::Thesaurus {
                word: split[1].to_string(),
                count,
            })
        } else if msg.starts_with("!guess ") {
            const SYNTAX_ERROR: GameCommandParseError = GameCommandParseError::SyntaxError {
                expected: "!thesaurus <word>",
            };

            let split: Vec<_> = msg.split(' ').collect();
            if split.len() != 2 {
                return Err(SYNTAX_ERROR);
            }

            Ok(GameCommand::Guess {
                word: split[1].to_string(),
            })
        } else if msg == "!halp" {
            Ok(GameCommand::Halp)
        } else if msg == "!start" {
            Ok(GameCommand::Start)
        } else {
            Err(GameCommandParseError::Unrecognized)
        }
    }
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Handler
////////////////////////////////////////////////////////////////////////////////////////////////////

trait IrcSenderExt {
    fn say(&self, target: impl Into<String>, msg: impl Into<String>);
}

impl IrcSenderExt for Sender {
    fn say(&self, target: impl Into<String>, msg: impl Into<String>) {
        self.send(Command::PRIVMSG(target.into(), msg.into())).unwrap()
    }
}

const AWAKE_SECS: u64 = 15;

pub async fn irc_handler(words: Arc<Words>, game: Game) -> Result<(), Error> {
    // load IRC config
    let config = Config::load("ircconf.toml").expect("failed to load `ircconf.toml`");

    // Create IRC client
    let self_name = config.nickname.clone().unwrap_or("cabotin".to_string());
    let mut client = Client::from_config(config).await?;
    client.identify()?;

    let mut stream = client.stream()?;
    let sender = client.sender();

    let mut last_wakeup = Instant::now();

    // process messages
    while let Some(message) = stream.next().await.transpose()? {
        //trace!("{}", message);

        match message.command {
            Command::PRIVMSG(ref target, ref msg) => {
                let mut guess = None;

                let msg = msg.trim();

                if msg == self_name {
                    trace!("bot wakeup");
                    last_wakeup = Instant::now();
                    sender.say(target, "oui?");
                } else if msg.split_whitespace().count() == 1 {
                    let now = Instant::now();
                    if now.duration_since(last_wakeup).as_secs() < AWAKE_SECS {
                        // single word & still awake, consider that a guess
                        last_wakeup = now;
                        guess = Some(msg.to_string());
                    }
                } else {
                    match GameCommand::parse(msg) {
                        Ok(GameCommand::Thesaurus { word, count }) => {
                            // this query may take some time and block the bot, but it's more like a feature really
                            let result = words.thesaurus(&word, count.unwrap_or(1));
                            sender.say(target, result);
                        }
                        Ok(GameCommand::Guess { word }) => {
                            guess = Some(word);
                        }
                        Ok(GameCommand::Start) => {
                            let reply = match game.start_game(Duration::from_secs(3600 * 24)).await {
                                Ok(_) => "game started".to_string(),
                                Err(err) => {
                                    format!("something went wrong (`{}`)", err)
                                }
                            };
                            sender.say(target, reply);
                        }
                        Ok(GameCommand::Halp) => {
                            sender.say(target, "coming soon");
                        }
                        Err(err) => {
                            match err {
                                GameCommandParseError::Unrecognized => {
                                    // the message was not meant for us
                                }
                                GameCommandParseError::SyntaxError { expected } => {
                                    sender.say(target, format!("syntax error: {}", expected));
                                }
                            }
                        }
                    }
                }

                // handle guess
                if let Some(guess) = guess {
                    let nick = message.source_nickname();
                    if let Some(nick) = nick {
                        let outcome = game.process_guess(nick.to_string(), guess).await;
                        let reply = match outcome {
                            Ok(Outcome::Win) => "you guessed the word".to_string(),
                            Ok(Outcome::Miss { distance }) => format!("miss ({})", distance),
                            Ok(Outcome::UnknownWord) => format!("unknown word"),
                            Err(err) => format!("something went wrong (`{}`)", err),
                        };
                        sender.say(target.clone(), reply);
                    }
                }
            }
            _ => (),
        }
    }

    Ok(())
}
