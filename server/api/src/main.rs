use crate::config::CONFIG;
use salvo::cors::Cors;
use salvo::handler::empty_handler;
use salvo::http::header::HeaderName;
use salvo::listener::TcpListener;
use salvo::{Router, Server};

mod cache;
mod config;
mod entity;
mod handler;
mod rpc;
mod sql;

#[tokio::main]
async fn main() -> common::Result<()> {
    tracing_subscriber::fmt()
        .event_format(
            tracing_subscriber::fmt::format()
                .with_line_number(true)
                .with_level(true)
                .with_target(true),
        )
        .with_max_level(tracing::Level::INFO)
        .try_init()
        .unwrap();
    let cors = Cors::builder()
        .allow_any_origin()
        .allow_methods(vec!["GET", "POST", "PUT", "DELETE", "OPTIONS", "HEAD"])
        .allow_headers(vec![
            HeaderName::from_static("content-type"),
            HeaderName::from_static("authorization"),
        ])
        .build();
    let router = Router::with_hoop(cors)
        .options(empty_handler)
        .push(
            Router::with_path("/user")
                .path("/user")
                .put(handler::user::login)
                .post(handler::user::signup)
                .delete(handler::user::logout),
        )
        .push(
            Router::with_path("/user/account")
                .delete(handler::user::sign_out)
                .post(handler::user::new_account_id),
        )
        .push(Router::with_path("/which_node/<user_id>").get(handler::user::which_node));
    Server::new(TcpListener::bind(CONFIG.server.address))
        .serve(router)
        .await;
    Ok(())
}
