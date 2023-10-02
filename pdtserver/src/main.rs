use std::{
    net::{SocketAddr, TcpListener},
    str::FromStr,
    sync::{Arc, Mutex, PoisonError},
};

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing, Router,
};

use pdtcore::*;
mod server;

use server::{SendError, Server};
use tracing::{metadata::LevelFilter, *};
use tracing_subscriber::{layer::SubscriberExt, EnvFilter, Registry};
use ulid::Ulid;

type ServerReference = Particularity<Server>;
type AppStateReference = Particularity<AppState>;

struct Config {
    server_address: SocketAddr,
    web_interface_address: SocketAddr,
}

impl Config {
    fn with_env(self) -> Self {
        use std::env;

        let configure = |config: Result<String, env::VarError>, default: SocketAddr| {
            config
                .map(|string| SocketAddr::from_str(&string).ok())
                .ok()
                .flatten()
                .unwrap_or(default)
        };

        let server_address = configure(env::var("SERVER_ADDRESS"), self.server_address);

        let web_interface_address = configure(
            env::var("WEB_INTERFACE_ADDRESS"),
            self.web_interface_address,
        );

        Self {
            server_address,
            web_interface_address,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_address: SocketAddr::from(([0, 0, 0, 0], 2039)),
            web_interface_address: SocketAddr::from(([0, 0, 0, 0], 2040)),
        }
    }
}

impl AppState {
    fn reference(server_reference: ServerReference) -> AppStateReference {
        Arc::new(Mutex::new(Self {
            server: server_reference,
        }))
    }
}

impl From<Server> for ServerReference {
    fn from(value: Server) -> Self {
        Arc::new(Mutex::new(value))
    }
}

struct AppState {
    server: ServerReference,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    style: String,
    script: String,
    clients: Vec<Client>,
}

enum AppError {
    Deadlock,
    ServerSend(SendError),
}

#[derive(Debug)]
enum StartupError {
    Tracing,
    TcpBindAddress(std::io::Error),
    Mutex,
    AxumServe,
}

impl<T> From<PoisonError<T>> for AppError {
    fn from(_value: PoisonError<T>) -> Self {
        AppError::Deadlock
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        match self {
            AppError::Deadlock => {
                error!("could not acquire lock");

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unexpected error".to_string(),
                )
            }
            AppError::ServerSend(error) => {
                error!(error =? error, "send");

                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unexpected error\n\nerror={:?}", error),
                )
            }
        }
        .into_response()
    }
}

async fn index(State(state): State<AppStateReference>) -> Result<IndexTemplate, AppError> {
    static STYLE: &str = include_str!("../static/style.min.css");
    static SCRIPT: &str = include_str!("../static/vendored/htmx.min.js");

    let app_state_guard = state.lock()?;
    let app_state = &*app_state_guard;

    let server_guard = app_state.server.lock()?;

    let server = &*server_guard;

    let clients = server.get_clients();

    let template = IndexTemplate {
        clients,
        style: STYLE.into(),
        script: SCRIPT.into(),
    };

    Ok(template)
}

async fn screen_off(
    Path(client_id): Path<Ulid>,
    State(state): State<AppStateReference>,
) -> Result<String, AppError> {
    let state_guard = state.lock()?;

    let state = &*state_guard;

    let mut server_guard = state.server.lock()?;

    let server = &mut *server_guard;

    match server.send(client_id, Message::Client(ClientMessage::ScreenOff)) {
        Ok(_) => Ok("OK".to_string()),
        Err(error) => Err(AppError::ServerSend(error)),
    }
}

async fn screen_on(
    Path(client_id): Path<Ulid>,
    State(state): State<AppStateReference>,
) -> Result<String, AppError> {
    let state_guard = state.lock()?;

    let state = &*state_guard;

    let mut server_guard = state.server.lock()?;

    let server = &mut *server_guard;

    match server.send(client_id, Message::Client(ClientMessage::ScreenOn)) {
        Ok(_) => Ok("OK".to_string()),
        Err(error) => Err(AppError::ServerSend(error)),
    }
}

fn setup_tracing() -> Result<(), StartupError> {
    let layer = tracing_logfmt::builder().with_target(false).layer();

    let filter = match EnvFilter::try_from_default_env() {
        Ok(filter) => filter,
        _ => EnvFilter::builder()
            .with_default_directive(LevelFilter::INFO.into())
            .parse("")
            .map_err(|_| StartupError::Tracing)?,
    };

    let subscriber = Registry::default().with(layer).with(filter);
    tracing::subscriber::set_global_default(subscriber).map_err(|_| StartupError::Tracing)?;

    Ok(())
}

async fn serve_web_interface(
    server_reference: ServerReference,
    web_interface_address: SocketAddr,
) -> Result<(), StartupError> {
    let state = AppState::reference(server_reference);

    let web = Router::new()
        .route("/", routing::get(index))
        .route("/screen-off/:client_id", routing::get(screen_off))
        .route("/screen-on/:client_id", routing::get(screen_on))
        .with_state(state.clone());

    info!(address =? web_interface_address, "starting web interface server");
    axum::Server::bind(&web_interface_address)
        .serve(web.into_make_service())
        .await
        .map_err(|_| StartupError::AxumServe)?;

    Ok(())
}

fn spawn_tcp_server(
    server_reference: ServerReference,
    server_address: SocketAddr,
) -> Result<(), StartupError> {
    let tcp_server_listener =
        TcpListener::bind(server_address).map_err(StartupError::TcpBindAddress)?;

    let mut guard = server_reference.lock().map_err(|_| StartupError::Mutex)?;
    let server = &mut *guard;

    info!(address = ?server_address, "starting pdt server");
    server.run(tcp_server_listener);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), StartupError> {
    setup_tracing()?;

    let config = Config::default().with_env();
    let server = Server::default();

    let server_reference = ServerReference::from(server);

    spawn_tcp_server(server_reference.clone(), config.server_address)?;
    serve_web_interface(server_reference, config.web_interface_address).await
}
