use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::broadcast;

const CHANNEL_CAPACITY: usize = 256;

#[derive(Clone)]
pub struct WsBroadcaster {
    sender: broadcast::Sender<String>,
}

impl WsBroadcaster {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { sender }
    }

    pub fn broadcast(&self, msg: String) {
        let _ = self.sender.send(msg);
    }

    pub async fn handle_socket(&self, socket: WebSocket) {
        let mut rx = self.sender.subscribe();
        let (mut ws_sender, mut ws_receiver) = socket.split();

        let send_task = tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(msg) => {
                        if ws_sender
                            .send(Message::Text(msg.into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });

        while let Some(result) = ws_receiver.next().await {
            match result {
                Ok(Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }

        send_task.abort();
    }
}
