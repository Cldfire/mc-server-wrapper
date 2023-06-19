use std::{net::SocketAddr, sync::Arc};

use askama_escape::MarkupDisplay;
use axum::{extract::State, response::IntoResponse, routing::get, Router};
use axum_live_view::{
    event_data::EventData, html, js_command, live_view::Updated, Html, LiveView, LiveViewUpgrade,
};
use mc_server_wrapper_lib::{communication::ServerCommand, McServerManager};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

use crate::EdgeToCoreCommand;

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum LiveViewFromServer {
    LogMessage(String),
}

pub async fn run_web_server(
    from_server: broadcast::Sender<LiveViewFromServer>,
    edge_to_core_cmd_tx: mpsc::Sender<EdgeToCoreCommand>,
    mc_server: Arc<McServerManager>,
) {
    let app_state = AppState {
        from_server,
        edge_to_core_cmd_tx,
        mc_server,
    };

    let app = Router::new()
        .route("/", get(root))
        .route("/bundle.js", axum_live_view::precompiled_js())
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .await
        .unwrap();
}

#[derive(Clone)]
struct AppState {
    from_server: broadcast::Sender<LiveViewFromServer>,
    edge_to_core_cmd_tx: mpsc::Sender<EdgeToCoreCommand>,
    mc_server: Arc<McServerManager>,
}

async fn root(State(state): State<AppState>, live: LiveViewUpgrade) -> impl IntoResponse {
    let view = MainView {
        messages: vec![],
        input_value: String::new(),
        from_server: state.from_server.clone(),
        edge_to_core_cmd_tx: state.edge_to_core_cmd_tx.clone(),
        mc_server: state.mc_server.clone(),
    };

    live.response(move |embed| {
        html! {
            <!DOCTYPE html>
            <html>
                <head>
                </head>
                <body>
                    { embed.embed(view) }
                    <script src="/bundle.js"></script>
                    <link rel="stylesheet" href="https://unpkg.com/mvp.css@1.12/mvp.css"></link>
                </body>
            </html>
        }
    })
}

#[derive(Clone)]
struct MainView {
    messages: Vec<String>,
    input_value: String,
    from_server: broadcast::Sender<LiveViewFromServer>,
    edge_to_core_cmd_tx: mpsc::Sender<EdgeToCoreCommand>,
    mc_server: Arc<McServerManager>,
}

#[derive(Eq, PartialEq, Serialize, Deserialize)]
enum MainViewMessage {
    FromServer(LiveViewFromServer),
    InputChange,
    InputSubmit,
}

impl LiveView for MainView {
    type Message = MainViewMessage;

    fn mount(
        &mut self,
        _uri: axum::http::Uri,
        _request_headers: &axum::http::HeaderMap,
        handle: axum_live_view::live_view::ViewHandle<Self::Message>,
    ) {
        let mut server_rx = self.from_server.subscribe();
        tokio::spawn(async move {
            while let Ok(msg) = server_rx.recv().await {
                if handle.send(MainViewMessage::FromServer(msg)).await.is_err() {
                    // This means the view was shut down, no need to send any
                    // more messages here.
                    return;
                }
            }
        });
    }

    fn update(mut self, msg: Self::Message, data: Option<EventData>) -> Updated<Self> {
        let mut js_commands = Vec::new();

        match msg {
            MainViewMessage::FromServer(live_view_message) => match live_view_message {
                LiveViewFromServer::LogMessage(msg) => self.messages.push(msg),
            },
            MainViewMessage::InputChange => {
                self.input_value = data
                    .unwrap()
                    .as_input()
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned();
            }
            MainViewMessage::InputSubmit => {
                let edge_to_core_cmd_tx_clone = self.edge_to_core_cmd_tx.clone();
                let input_value_clone = self.input_value.clone();
                let mc_server_clone = self.mc_server.clone();
                tokio::spawn(handle_input(
                    input_value_clone,
                    edge_to_core_cmd_tx_clone,
                    mc_server_clone,
                ));

                self.input_value.clear();
                js_commands.push(js_command::clear_value("#console-input"));
            }
        }

        Updated::new(self).with_all(js_commands)
    }

    fn render(&self) -> Html<Self::Message> {
        html! {
            <div style="\"height: 100vh; display: grid; grid-template-rows: auto 1fr auto; margin: 0;\"">
                <h1> "mc-server-wrapper console" </h1>
                <div style="\"overflow: auto; display: flex; flex-direction: column-reverse;\"">
                    <div style="\"transform: translateZ(0);\"">
                        for msg in self.messages.iter() {
                            <div> { format!("{}", MarkupDisplay::new_unsafe(msg, askama_escape::Html)) } </div>
                        }
                    </div>
                </div>
                <form axm-submit={ MainViewMessage::InputSubmit }>
                    <input type="text" id="console-input" axm-input={ MainViewMessage::InputChange } autocomplete="off" style="\"height: 20px;\""></input>
                    <input
                        type="submit"
                        value="Send"
                        disabled=if self.input_value.is_empty() { Some(()) } else { None }
                    />
                </form>
            </div>
        }
    }
}

async fn handle_input(
    input_value: String,
    edge_to_core_cmd_tx: mpsc::Sender<EdgeToCoreCommand>,
    mc_server: Arc<McServerManager>,
) {
    if mc_server.running().await {
        edge_to_core_cmd_tx
            .send(EdgeToCoreCommand::MinecraftCommand(
                ServerCommand::WriteCommandToStdin(input_value),
            ))
            .await
            .unwrap();
    } else {
        // TODO: create a command parser for user input?
        // https://docs.rs/clap/2.33.1/clap/struct.App.html#method.get_matches_from_safe
        match input_value.as_str() {
            "start" => {
                edge_to_core_cmd_tx
                    .send(EdgeToCoreCommand::MinecraftCommand(
                        ServerCommand::StartServer { config: None },
                    ))
                    .await
                    .unwrap();
            }
            "stop" => {
                edge_to_core_cmd_tx
                    .send(EdgeToCoreCommand::MinecraftCommand(
                        ServerCommand::StopServer { forever: true },
                    ))
                    .await
                    .unwrap();
            }
            _ => {}
        }
    }
}
