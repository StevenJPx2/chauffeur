//! A one-shot fake Jev endpoint over plain HTTP on loopback.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use chauffeur_judge_jev::JevConfig;
use serde_json::Value;

/// Serve one System One request: `answer` inspects the request body and
/// returns the `answers` object. Returns the config pointing at the server.
pub fn serve_jev(
    answer: impl FnOnce(&Value) -> Value + Send + 'static,
) -> (JevConfig, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    let server = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("accept");
        let mut reader = BufReader::new(stream);
        let mut length = 0;
        let mut line = String::new();

        loop {
            line.clear();
            reader.read_line(&mut line).expect("header");

            if line == "\r\n" {
                break;
            }
            if let Some((name, value)) = line.split_once(':')
                && name.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse().expect("content length");
            }
        }

        let mut body = vec![0; length];
        reader.read_exact(&mut body).expect("body");

        let request: Value = serde_json::from_slice(&body).expect("json");
        let reply = serde_json::json!({ "answers": answer(&request) }).to_string();

        write!(
            reader.get_mut(),
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
            reply.len()
        )
        .expect("reply");
    });
    let config = JevConfig {
        base_url: format!("http://127.0.0.1:{port}"),
        api_key: "test".into(),
        model: "jev-test".into(),
        timeout: Duration::from_secs(5),
    };

    (config, server)
}
