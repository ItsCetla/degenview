use crate::model::Kline;
use eframe::egui;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

/// A Binance combined-stream kline feed for one workspace's Binance charts. Runs
/// a background thread that reconnects with backoff and pushes `(symbol, kline)`
/// updates onto the receiver.
pub struct BinanceLiveFeed {
    pub receiver: Receiver<(String, Kline)>,
    signature: (Vec<String>, String),
    shutdown: Arc<AtomicBool>,
}

impl BinanceLiveFeed {
    pub fn connect(symbols: Vec<String>, interval: String, ctx: egui::Context) -> Self {
        let (sender, receiver) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let signature = (symbols.clone(), interval.clone());
        std::thread::spawn(move || {
            run(symbols, interval, sender, thread_shutdown, ctx);
        });
        Self {
            receiver,
            signature,
            shutdown,
        }
    }

    /// The symbols and interval this feed was opened for, so the owner can tell
    /// when a reconnect is needed.
    pub fn signature(&self) -> &(Vec<String>, String) {
        &self.signature
    }
}

impl Drop for BinanceLiveFeed {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
    }
}

fn run(
    symbols: Vec<String>,
    interval: String,
    sender: mpsc::Sender<(String, Kline)>,
    shutdown: Arc<AtomicBool>,
    ctx: egui::Context,
) {
    let streams = symbols
        .iter()
        .map(|symbol| format!("{}@kline_{interval}", symbol.to_lowercase()))
        .collect::<Vec<_>>()
        .join("/");
    let url = format!("wss://stream.binance.com/stream?streams={streams}");
    let mut backoff = crate::service::ws::Backoff::new();

    loop {
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        if let Ok(mut socket) = crate::service::ws::connect(&url, "stream.binance.com", 443) {
            let mut health = crate::service::ws::Health::new(Duration::from_secs(60));
            while !shutdown.load(Ordering::Acquire) {
                match health.read(&mut socket) {
                    Ok(Some(tungstenite::Message::Text(text))) => {
                        backoff.reset();
                        if let Some((symbol, kline)) = parse_kline(&text) {
                            if sender.send((symbol, kline)).is_err() {
                                return;
                            }
                            ctx.request_repaint();
                        }
                    }
                    Ok(Some(tungstenite::Message::Close(_))) => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
        }
        if shutdown.load(Ordering::Acquire) {
            return;
        }
        backoff.wait(&shutdown);
    }
}

/// `{"stream":"btcusdt@kline_1h","data":{"k":{"t","o","h","l","c","v",…}}}`.
fn parse_kline(text: &str) -> Option<(String, Kline)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let stream = value.get("stream")?.as_str()?;
    let symbol = stream.split('@').next()?.to_uppercase();
    let k = value.pointer("/data/k")?;
    let t = k.get("t").and_then(Value::as_i64)?;
    // `T` is the bar's last millisecond, as in the REST rows. Without it the forming bar would read
    // as already closed (`close_time <= now`) and Pine would rebuild on every tick.
    let close_time = k.get("T").and_then(Value::as_i64).unwrap_or(t);
    let open = k.get("o").and_then(Value::as_str)?.parse().ok()?;
    let high = k.get("h").and_then(Value::as_str)?.parse().ok()?;
    let low = k.get("l").and_then(Value::as_str)?.parse().ok()?;
    let close = k.get("c").and_then(Value::as_str)?.parse().ok()?;
    let volume = k.get("v").and_then(Value::as_str)?.parse().ok()?;
    Some((
        symbol,
        Kline {
            open_time: t,
            close_time,
            open,
            high,
            low,
            close,
            volume,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_kline_stream_message() {
        let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"T":1700003599999,"o":"10","h":"12","l":"9","c":"11","v":"42.5","q":"0","x":false}}}"#;
        let (symbol, kline) = parse_kline(text).expect("parse");
        assert_eq!(symbol, "BTCUSDT");
        assert_eq!(kline.open_time, 1_700_000_000_000);
        assert_eq!(kline.close_time, 1_700_003_599_999);
        assert_eq!(kline.high, 12.0);
        assert_eq!(kline.low, 9.0);
        assert_eq!(kline.close, 11.0);
        assert_eq!(kline.volume, 42.5);
    }

    #[test]
    fn a_message_without_a_close_time_falls_back_to_the_open_time() {
        let text = r#"{"stream":"btcusdt@kline_1h","data":{"k":{"t":1700000000000,"o":"10","h":"12","l":"9","c":"11","v":"1"}}}"#;
        let (_, kline) = parse_kline(text).expect("parse");
        assert_eq!(kline.close_time, 1_700_000_000_000);
    }
}
