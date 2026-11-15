use std::sync::{Arc, Mutex};

use reqwest::Url;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

/// One scripted answer.
pub struct Reply {
    status: u16,
    set_cookie: Option<&'static str>,
    location: Option<String>,
    body: String,
}

impl Reply {
    pub fn new(status: u16, body: &str) -> Self {
        Self {
            status,
            set_cookie: None,
            location: None,
            body: body.to_owned(),
        }
    }

    /// A 302 pointing at `location`.
    pub fn redirect(location: &str) -> Self {
        Self {
            location: Some(location.to_owned()),
            ..Self::status(302)
        }
    }

    pub fn with_cookie(mut self, cookie: &'static str) -> Self {
        self.set_cookie = Some(cookie);
        self
    }

    pub fn status(status: u16) -> Self {
        Self::new(status, "")
    }

    fn to_bytes(&self) -> Vec<u8> {
        let cookie = self
            .set_cookie
            .map(|cookie| format!("set-cookie: {cookie}\r\n"))
            .unwrap_or_default();
        let location = self
            .location
            .as_ref()
            .map(|location| format!("location: {location}\r\n"))
            .unwrap_or_default();
        format!(
            "HTTP/1.1 {} Scripted\r\ncontent-type: application/json\r\n{cookie}{location}content-length: {}\r\nconnection: close\r\n\r\n{}",
            self.status,
            self.body.len(),
            self.body
        )
        .into_bytes()
    }
}

#[derive(Debug, Clone)]
pub struct Recorded {
    pub head: String,
    pub body: String,
}

impl Recorded {
    pub fn request_line(&self) -> &str {
        self.head.lines().next().unwrap_or_default()
    }

    pub fn has_header(&self, line: &str) -> bool {
        self.head.to_lowercase().contains(&line.to_lowercase())
    }
}

/// A local HTTP server that answers one connection per scripted reply, in
/// order, and records what it received. Extra requests get a 500.
pub struct TestServer {
    pub base: Url,
    recorded: Arc<Mutex<Vec<Recorded>>>,
    task: JoinHandle<()>,
}

impl TestServer {
    pub async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("local address");
        let recorded = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(serve(listener, replies, Arc::clone(&recorded)));
        Self {
            base: Url::parse(&format!("http://{address}")).expect("valid URL"),
            recorded,
            task,
        }
    }

    pub fn requests(&self) -> Vec<Recorded> {
        self.recorded.lock().expect("lock").clone()
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(listener: TcpListener, replies: Vec<Reply>, recorded: Arc<Mutex<Vec<Recorded>>>) {
    let mut replies = replies.into_iter();
    loop {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let request = read_request(&mut stream).await;
        recorded.lock().expect("lock").push(request);
        let reply = replies
            .next()
            .unwrap_or_else(|| Reply::new(500, "unscripted"));
        stream.write_all(&reply.to_bytes()).await.expect("write");
        stream.shutdown().await.expect("shutdown");
    }
}

async fn read_request(stream: &mut TcpStream) -> Recorded {
    let mut buffer = Vec::new();
    let head_end = loop {
        read_more(stream, &mut buffer).await;
        if let Some(position) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let head = String::from_utf8_lossy(&buffer[..head_end]).into_owned();
    let length = head
        .to_lowercase()
        .lines()
        .find_map(|line| line.strip_prefix("content-length:")?.trim().parse().ok())
        .unwrap_or(0);
    while buffer.len() < head_end + length {
        read_more(stream, &mut buffer).await;
    }
    let body = String::from_utf8_lossy(&buffer[head_end..]).into_owned();
    Recorded { head, body }
}

async fn read_more(stream: &mut TcpStream, buffer: &mut Vec<u8>) {
    let mut chunk = [0_u8; 1024];
    let read = stream.read(&mut chunk).await.expect("read");
    assert!(read > 0, "client closed the connection early");
    buffer.extend_from_slice(&chunk[..read]);
}
