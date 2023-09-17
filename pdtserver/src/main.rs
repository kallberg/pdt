use std::{
    net::{SocketAddr, TcpListener, TcpStream},
    ops::Deref,
    sync::{Arc, Mutex},
    thread,
};

mod tcpserver;
use askama::Template;
use axum::{extract::State, response::Html, routing, Router};
use pdtcore::*;
use tcpserver::PDTServer;

struct Client {
    info: Option<DeviceInfo>,
    connection: TcpStream,
}

struct AppState {
    tcp_server: PDTServer,
}

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate {
    device_names: Vec<String>,
}

type SharedAppState = Arc<Mutex<AppState>>;

async fn index(State(state): State<SharedAppState>) -> Html<String> {
    let state = state.lock().unwrap();

    let template = IndexTemplate {
        device_names: vec!["placeholder".to_owned()],
    };

    let template_str = template.render().unwrap().to_string();

    Html(template_str)
}

async fn screen_off(State(state): State<SharedAppState>) -> String {
    let state = &mut *state.lock().unwrap();

    for ulid in state.tcp_server.sessions.clone().keys() {
        let _ = state.tcp_server.send(ulid, Message::ScreenOff);
    }

    "OK".to_string()
}

#[tokio::main]
async fn main() {
    let state = Arc::new(Mutex::new(AppState {
        tcp_server: PDTServer::new(TcpListener::bind("127.0.0.1:2039").unwrap()),
    }));

    let web = Router::new()
        .route("/", routing::get(index))
        .route("/screen-off", routing::get(screen_off))
        .with_state(state.clone());

    let web_address = SocketAddr::from(([127, 0, 0, 1], 2040));

    //let tcp_server_thread = thread::spawn(move || state.lock().unwrap().tcp_server.serve());

    axum::Server::bind(&web_address)
        .serve(web.into_make_service())
        .await
        .unwrap();

    //let _ = tcp_server_thread.join().unwrap();
}
