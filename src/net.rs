//! Small HTTP helpers over ureq (rustls). Short timeouts: a slow server must never hold up
//! the next refresh, and nothing here runs on the UI thread.
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .user_agent(concat!("Vision/", env!("CARGO_PKG_VERSION")))
        .build()
}

fn describe(error: ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, response) => format!("HTTP {code} from {}", response.get_url()),
        ureq::Error::Transport(transport) => transport.to_string(),
    }
}

pub fn get_json(agent: &ureq::Agent, url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let mut request = agent.get(url).set("Accept", "application/json");
    for (key, value) in headers {
        request = request.set(key, value);
    }
    let response = request.call().map_err(describe)?;
    serde_json::from_reader(response.into_reader().take(16 << 20)).map_err(|e| format!("{url}: {e}"))
}

pub fn post_json(agent: &ureq::Agent, url: &str, headers: &[(&str, &str)], body: Option<&Value>) -> Result<Value, String> {
    let mut request = agent.post(url).set("Accept", "application/json");
    for (key, value) in headers {
        request = request.set(key, value);
    }
    let response = match body {
        Some(body) => request.set("Content-Type", "application/json").send_string(&body.to_string()),
        None => request.call(),
    }
    .map_err(describe)?;
    let text = response.into_string().map_err(|e| e.to_string())?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

pub fn get_bytes(agent: &ureq::Agent, url: &str, headers: &[(&str, &str)], max: u64) -> Result<Vec<u8>, String> {
    let mut request = agent.get(url);
    for (key, value) in headers {
        request = request.set(key, value);
    }
    let response = request.call().map_err(describe)?;
    let mut bytes = Vec::new();
    response.into_reader().take(max).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

/// This device's LAN address (a UDP "connect" picks the route; no packet is sent).
pub fn local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:80").ok()?;
    Some(socket.local_addr().ok()?.ip().to_string())
}

pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Decodes application/x-www-form-urlencoded text.
pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                if let Ok(b) = u8::from_str_radix(&text[i + 1..i + 3], 16) {
                    out.push(b);
                    i += 2;
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn form_field(body: &str, name: &str) -> Option<String> {
    body.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (decode(key) == name).then(|| decode(value))
    })
}
