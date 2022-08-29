use askama::Template;
use axum::extract::Extension;
use axum::response::Redirect;
use axum::{extract::Form, response::Html, routing::get, Router};
use http::StatusCode;
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct Peer {
    host: String,
    pubkey: String,
}

#[derive(Deserialize, Debug)]
struct State {
    peers: Vec<Peer>,
}

pub async fn run_setup() {
    let state = Arc::new(State { peers: vec![] });

    // Set the RUST_LOG, if it hasn't been explicitly defined
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "example_form=debug")
    }
    tracing_subscriber::fmt::init();

    // build our application with some routes
    let app = Router::new()
        .route("/", get(show_form).post(accept_form))
        .layer(Extension(state));

    // run it with hyper
    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::debug!("listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[derive(Template)]
#[template(path = "add_peer.html")]
struct AddPeerTemplate {
    peers: Vec<Peer>,
}

async fn show_form(Extension(state): Extension<Arc<State>>) -> AddPeerTemplate {
    dbg!(&state);
    AddPeerTemplate {
        peers: state.peers.clone(),
    }
}

async fn accept_form(
    Extension(state): Extension<Arc<State>>,
    Form(input): Form<Peer>,
) -> Result<Redirect, (StatusCode, String)> {
    dbg!(&state);
    dbg!(&input);

    Ok(Redirect::to("/".parse().unwrap()))
}
