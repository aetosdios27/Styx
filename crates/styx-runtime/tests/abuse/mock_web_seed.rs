#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub enum ScriptedHttpResponse {
    Correct(Vec<u8>),
    WrongBytes {
        range: Range<u64>,
        data: Vec<u8>,
    },
    Truncated {
        range: Range<u64>,
        data: Vec<u8>,
        first_n: usize,
    },
    NoContentRange {
        data: Vec<u8>,
    },
    HttpStatus(u16),
    ZeroLength,
}

pub struct MockWebSeed {
    scripts: HashMap<String, Vec<ScriptedHttpResponse>>,
}

impl MockWebSeed {
    pub fn new() -> Self {
        Self {
            scripts: HashMap::new(),
        }
    }

    pub fn add_script(mut self, path: &str, responses: Vec<ScriptedHttpResponse>) -> Self {
        self.scripts.insert(path.to_owned(), responses);
        self
    }

    pub async fn serve(self) -> (SocketAddr, JoinHandle<()>) {
        let scripts = Arc::new(Mutex::new(self.scripts));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let handle = tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => break,
                };
                let scripts = scripts.clone();
                tokio::spawn(async move {
                    handle_connection(stream, scripts).await;
                });
            }
        });

        (addr, handle)
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    scripts: Arc<Mutex<HashMap<String, Vec<ScriptedHttpResponse>>>>,
) {
    let mut reader = BufReader::new(&mut stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).await.is_err() {
        return;
    }

    let path: String = request_line
        .split_whitespace()
        .nth(1)
        .map(|s| s.to_owned())
        .unwrap_or_else(|| "/".to_owned());

    loop {
        let mut header = String::new();
        match reader.read_line(&mut header).await {
            Ok(0) => break,
            Ok(_) if header == "\r\n" || header == "\n" => break,
            _ => continue,
        }
    }

    let response = {
        let mut locked = scripts.lock().unwrap();
        locked
            .get_mut(&path)
            .and_then(|responses| {
                if !responses.is_empty() {
                    Some(responses.remove(0))
                } else {
                    None
                }
            })
            .unwrap_or(ScriptedHttpResponse::HttpStatus(404))
    };

    match response {
        ScriptedHttpResponse::Correct(data) => {
            let len = data.len();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {len}\r\n\
                 Content-Range: bytes 0-{end}/{len}\r\n\r\n",
                end = len.saturating_sub(1)
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&data).await.unwrap();
        }
        ScriptedHttpResponse::WrongBytes { range, data } => {
            let len = data.len();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {len}\r\n\
                 Content-Range: bytes {start}-{end}/{total}\r\n\r\n",
                start = range.start,
                end = range.end.saturating_sub(1),
                total = range.end,
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&data).await.unwrap();
        }
        ScriptedHttpResponse::Truncated {
            range,
            data,
            first_n,
        } => {
            let truncated_len = data.len().min(first_n);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {len}\r\n\
                 Content-Range: bytes {start}-{end}/{total}\r\n\r\n",
                len = truncated_len,
                start = range.start,
                end = range.end.saturating_sub(1),
                total = range.end,
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&data[..truncated_len]).await.unwrap();
        }
        ScriptedHttpResponse::NoContentRange { data } => {
            let len = data.len();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {len}\r\n\r\n"
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(&data).await.unwrap();
        }
        ScriptedHttpResponse::HttpStatus(status) => {
            let header = format!(
                "HTTP/1.1 {status} {}\r\nContent-Length: 0\r\n\r\n",
                status_text(status)
            );
            stream.write_all(header.as_bytes()).await.unwrap();
        }
        ScriptedHttpResponse::ZeroLength => {
            let header = "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                  Content-Length: 0\r\n\
                  Content-Range: bytes */0\r\n\r\n";
            stream.write_all(header.as_bytes()).await.unwrap();
        }
    }

    let _ = stream.flush().await;
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        410 => "Gone",
        416 => "Range Not Satisfiable",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        _ => "Unknown",
    }
}
