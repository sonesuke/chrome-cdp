//! CDP WebSocket connection handling

use crate::{Error, Result};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};

type Responder = oneshot::Sender<Result<Value>>;

/// CDP connection managing WebSocket communication
pub struct CdpConnection {
    command_tx: mpsc::UnboundedSender<(u32, String, Value, Responder)>,
    next_id: Arc<Mutex<u32>>,
}

impl CdpConnection {
    /// Connect to Chrome DevTools Protocol via WebSocket
    pub async fn connect(ws_url: &str) -> Result<Self> {
        let (ws_stream, _) = connect_async(ws_url)
            .await
            .map_err(|e| Error::WebSocket(format!("Failed to connect to {}: {}", ws_url, e)))?;

        let (mut write, mut read) = ws_stream.split();

        let (command_tx, mut command_rx) =
            mpsc::unbounded_channel::<(u32, String, Value, Responder)>();
        let pending: Arc<Mutex<HashMap<u32, Responder>>> = Arc::new(Mutex::new(HashMap::new()));

        // Task for sending commands
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            while let Some((id, method, params, responder)) = command_rx.recv().await {
                let msg = json!({
                    "id": id,
                    "method": method,
                    "params": params
                });

                pending_clone.lock().await.insert(id, responder);

                if let Err(e) = write.send(Message::Text(msg.to_string().into())).await {
                    eprintln!("Failed to send CDP command: {}", e);
                    break;
                }
            }
        });

        // Task for receiving responses
        tokio::spawn(async move {
            while let Some(msg) = read.next().await {
                match msg {
                    Ok(Message::Text(text)) => {
                        if let Ok(v) = serde_json::from_str::<Value>(&text) {
                            // Handle response
                            if let Some(id) = v["id"].as_u64() {
                                let id = id as u32;
                                let responder = pending.lock().await.remove(&id);
                                if let Some(responder) = responder {
                                    if let Some(error) = v.get("error") {
                                        let message =
                                            error["message"].as_str().unwrap_or("unknown");
                                        let _ = responder.send(Err(Error::Cdp(format!(
                                            "CDP error for command {}: {} - {}",
                                            id,
                                            error["code"].as_i64().unwrap_or(-1),
                                            message
                                        ))));
                                    } else if let Some(result) = v.get("result") {
                                        let _ = responder.send(Ok(result.clone()));
                                    }
                                }
                            }
                            // Ignore events for now
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(e) => {
                        eprintln!("WebSocket error: {}", e);
                        break;
                    }
                    _ => {}
                }
            }
        });

        Ok(Self {
            command_tx,
            next_id: Arc::new(Mutex::new(1)),
        })
    }

    /// Send a CDP command and wait for response
    pub async fn send_command(&self, method: &str, params: Value) -> Result<Value> {
        let id = {
            let mut next_id = self.next_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        };

        let (tx, rx) = oneshot::channel();
        self.command_tx
            .send((id, method.to_string(), params, tx))
            .map_err(|_| Error::Cdp("Failed to send command to channel".to_string()))?;

        rx.await
            .map_err(|_| Error::Cdp("Response channel closed".to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    // Test helper to verify command ID increment
    struct TestConnection {
        next_id: Arc<Mutex<u32>>,
    }

    impl TestConnection {
        fn new() -> Self {
            Self {
                next_id: Arc::new(Mutex::new(1)),
            }
        }

        async fn get_next_id(&self) -> u32 {
            let mut next_id = self.next_id.lock().await;
            let id = *next_id;
            *next_id += 1;
            id
        }
    }

    #[tokio::test]
    async fn test_command_id_generation_starts_at_one() {
        let conn = TestConnection::new();
        assert_eq!(conn.get_next_id().await, 1);
    }

    #[tokio::test]
    async fn test_command_id_increments() {
        let conn = TestConnection::new();
        assert_eq!(conn.get_next_id().await, 1);
        assert_eq!(conn.get_next_id().await, 2);
        assert_eq!(conn.get_next_id().await, 3);
    }

    #[tokio::test]
    async fn test_command_id_thread_safety() {
        let conn = Arc::new(TestConnection::new());
        let mut handles = vec![];

        for _ in 0..10 {
            let conn_clone = conn.clone();
            handles.push(tokio::spawn(async move {
                conn_clone.get_next_id().await
            }));
        }

        let mut ids = vec![];
        for handle in handles {
            ids.push(handle.await.unwrap());
        }

        // All IDs should be unique
        let unique_ids: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique_ids.len(), 10);
    }

    #[test]
    fn test_response_json_id_extraction() {
        let json = r#"{"id":42,"result":{"status":"ok"}}"#;
        let value: Value = serde_json::from_str(json).unwrap();
        assert_eq!(value["id"].as_u64(), Some(42));
    }

    #[test]
    fn test_response_json_error_extraction() {
        let json = r#"{"id":1,"error":{"code":-32601,"message":"Method not found"}}"#;
        let value: Value = serde_json::from_str(json).unwrap();
        assert!(value.get("error").is_some());
        assert_eq!(value["error"]["code"].as_i64(), Some(-32601));
        assert_eq!(value["error"]["message"].as_str(), Some("Method not found"));
    }

    #[test]
    fn test_command_json_serialization() {
        let cmd = json!({
            "id": 1,
            "method": "Page.navigate",
            "params": {"url": "https://example.com"}
        });
        assert_eq!(cmd["id"], 1);
        assert_eq!(cmd["method"], "Page.navigate");
        assert_eq!(cmd["params"]["url"], "https://example.com");
    }

    #[tokio::test]
    async fn test_channel_closure_on_send_error() {
        let (tx, mut rx) = mpsc::unbounded_channel::<(u32, String, Value, Responder)>();
        let (responder_tx, _) = oneshot::channel();

        // Send should succeed
        assert!(tx
            .send((1, "Test.method".to_string(), json!({}), responder_tx))
            .is_ok());

        // Receive the message
        assert!(rx.recv().await.is_some());

        // Drop rx to simulate closure
        drop(rx);

        // Create new responder
        let (responder_tx2, _) = oneshot::channel();

        // Send should now fail (channel closed)
        assert!(tx
            .send((2, "Test.method".to_string(), json!({}), responder_tx2))
            .is_err());
    }

    #[tokio::test]
    async fn test_oneshot_channel_closure() {
        let (tx, rx) = oneshot::channel::<Result<Value>>();

        // Drop the receiver
        drop(rx);

        // Sending should fail
        assert!(tx.send(Ok(json!({}))).is_err());
    }

    #[tokio::test]
    async fn test_oneshot_channel_value_retrieval() {
        let (tx, rx) = oneshot::channel::<Result<Value>>();

        let expected = json!({"result": "success"});
        tokio::spawn(async move {
            tx.send(Ok(expected)).unwrap();
        });

        let result = rx.await.unwrap();
        assert_eq!(result.unwrap()["result"], "success");
    }
}
