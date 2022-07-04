//! Web server to display game state
use crate::game::{fetch_players, Player};
use askama::Template;
use axum::{
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Extension, Router,
};
use std::{error::Error, fmt::Display, path::Path, time::Duration};

////////////////////////////////////////////////////////////////////////////////////////////////////
// Utilities
////////////////////////////////////////////////////////////////////////////////////////////////////

// https://www.reddit.com/r/rust/comments/ozc0m8/an_actixanyhow_compatible_error_helper_i_found/
pub trait IntoHttpError<T> {
    fn http_error(self, status_code: StatusCode, message: &str) -> Result<T, (StatusCode, String)>;

    fn http_internal_error(self, message: &str) -> Result<T, (StatusCode, String)>
    where
        Self: std::marker::Sized,
    {
        self.http_error(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl<T, E: Display> IntoHttpError<T> for Result<T, E> {
    fn http_error(self, status_code: StatusCode, message: &str) -> Result<T, (StatusCode, String)> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                error!("http_error: {}", err);
                Err((status_code, format!("{}:{}", message, err.to_string())))
            }
        }
    }
}

/*
impl<T> IntoHttpError<T> for anyhow::Result<T> {
    fn http_error(self, status_code: StatusCode, message: &str) -> Result<T, (StatusCode, String)> {
        match self {
            Ok(val) => Ok(val),
            Err(err) => {
                error!("http_error: {:?}", err);
                Err((status_code, format!("{}:{}", message, err.to_string())))
            }
        }
    }
}*/

////////////////////////////////////////////////////////////////////////////////////////////////////
// Templates
////////////////////////////////////////////////////////////////////////////////////////////////////

#[derive(Template)]
#[template(path = "game.html")]
struct GameTemplate {
    players: Vec<Player>,
}

////////////////////////////////////////////////////////////////////////////////////////////////////
// Server
////////////////////////////////////////////////////////////////////////////////////////////////////

struct State {}

async fn root(Extension(pool): Extension<SqlitePool>) -> Result<Html<String>, (StatusCode, String)> {
    let players = fetch_players(&pool)
        .await
        .http_internal_error("could not fetch players")?;
    let template = GameTemplate { players };
    let html = template.render().http_internal_error("failed to render template")?;
    Ok(Html(html))
}

pub async fn launch_server(pool: SqlitePool) {
    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root))
        .layer(Extension(pool));

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
