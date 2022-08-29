use crate::config::ServerConfigParams;
use askama::Template;
use axum::extract::{Extension, Form};
use axum::response::Redirect;
use axum::{routing::get, Router};
use fedimint_api::PeerId;
use http::StatusCode;
use serde::Deserialize;
use std::sync::Arc;

fn generate_config(num_peers: u64) -> ServerConfig {
    let peers = (0..num_peers).map(PeerId::from).collect::<Vec<_>>();
    let max_evil = hbbft::util::max_faulty(peers.len());
    println!(
        "Generating keys such that up to {} peers may fail/be evil",
        max_evil
    );
    let params = ServerConfigParams {
        hbbft_base_port,
        api_base_port,
        amount_tiers,
    };

    let (server_cfg, client_cfg) =
        ServerConfig::trusted_dealer_gen(&peers, max_evil, &params, &mut rng);
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate;

async fn home() -> HomeTemplate {
    HomeTemplate
}

#[derive(Template)]
#[template(path = "generate.html")]
struct GenerateTemplate;

#[derive(Deserialize, Debug, Clone)]
#[allow(dead_code)]
struct GenerateForm {
    num_peers: u64,
}

async fn generate() -> GenerateTemplate {
    GenerateTemplate
}

async fn generate_post(Form(input): Form<GenerateForm>) -> Result<Redirect, (StatusCode, String)> {
    dbg!(&input);
    Ok(Redirect::to("/".parse().unwrap()))
}

#[derive(Template)]
#[template(path = "upload.html")]
struct UploadTemplate;

async fn upload() -> UploadTemplate {
    UploadTemplate
}

// TODO: this needs an "out dir"
pub async fn run_setup() {
    // build our application with a single route
    // let app = Router::new().route("/", get(|| async { "Hello, World!" }));
    let app = Router::new()
        .route("/", get(home))
        .route("/generate", get(generate).post(generate_post))
        .route("/upload", get(upload));

    // run it with hyper on localhost:3000
    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();
}
