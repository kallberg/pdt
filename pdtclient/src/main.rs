use std::net::TcpStream;
use std::process::Command;

use pdtcore::*;

struct State {
    active: bool,
}

fn handle_message(state: &mut State, message: Message) {
    match message {
        Message::DeviceInfo(_) => todo!(),
        Message::ScreenOff => {
            Command::new("xset")
                .args(["dpms", "force", "off"])
                .spawn()
                .unwrap()
                .wait()
                .unwrap();
        }
        Message::End => {
            state.active = false;
        }
    }
}

fn run_client() {
    let mut tcp_stream = TcpStream::connect("127.0.0.1:2039").unwrap();

    let mut state = State { active: true };

    let device_info = pdtcore::DeviceInfo {
        name: String::from("ASH"),
    };

    Message::DeviceInfo(device_info)
        .send(&mut tcp_stream)
        .unwrap();

    while state.active {
        match Message::receive(&mut tcp_stream) {
            Ok(message) => handle_message(&mut state, message),
            Err(error) => {
                println!("{error:?}");

                return;
            }
        }
    }

    Message::End.send(&mut tcp_stream).unwrap();

    tcp_stream.shutdown(std::net::Shutdown::Both).unwrap();
}

fn main() {
    run_client();
}
