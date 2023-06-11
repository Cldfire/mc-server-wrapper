use std::net::SocketAddr;

use askama_escape::MarkupDisplay;
use axum::{extract::State, response::IntoResponse, routing::get, Router};
use axum_live_view::{
    event_data::EventData, html, js_command, live_view::Updated, Html, LiveView, LiveViewUpgrade,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc};

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum LiveViewFromServer {
    LogMessage(String),
}

pub enum LiveViewFromClient {
    ConsoleInput(String),
}

pub async fn run_web_server(
    from_server: broadcast::Sender<LiveViewFromServer>,
    from_client: mpsc::Sender<LiveViewFromClient>,
) {
    let app_state = AppState {
        from_server,
        from_client,
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
    from_client: mpsc::Sender<LiveViewFromClient>,
}

async fn root(State(state): State<AppState>, live: LiveViewUpgrade) -> impl IntoResponse {
    let view = MainView {
        messages: vec![],
        input_value: String::new(),
        from_server: state.from_server.clone(),
        from_client: state.from_client.clone(),
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
    from_client: mpsc::Sender<LiveViewFromClient>,
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
                    log::error!("Failed to send message to liveview component");
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
                let from_client_clone = self.from_client.clone();
                let input_value_clone = self.input_value.clone();
                tokio::spawn(async move {
                    let _ = from_client_clone
                        .send(LiveViewFromClient::ConsoleInput(input_value_clone))
                        .await;
                });

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
