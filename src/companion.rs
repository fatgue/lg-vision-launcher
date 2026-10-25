//! Phone setup page for Home Assistant. Runs only while the setup panel is open; requests
//! must carry the one-time key from the QR code, so other devices on the LAN cannot submit.
use crate::log::log;
use crate::model::ServiceRequest;
use crate::net;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

pub const PORT: u16 = 39880;

pub struct Companion {
    stop: Arc<AtomicBool>,
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

pub fn start(key: String, requests: Sender<ServiceRequest>) -> std::io::Result<Companion> {
    let listener = TcpListener::bind(("0.0.0.0", PORT))?;
    listener.set_nonblocking(true)?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new().name("companion".into()).spawn(move || {
        while !flag.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((stream, _)) => {
                    if let Err(error) = handle(stream, &key, &requests) {
                        log!("companion: {error}");
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                Err(error) => {
                    log!("companion: {error}");
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    })?;
    Ok(Companion { stop })
}

fn handle(mut stream: TcpStream, key: &str, requests: &Sender<ServiceRequest>) -> std::io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut data = Vec::new();
    let mut buffer = [0u8; 4096];
    let header_end = loop {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            return Ok(());
        }
        data.extend_from_slice(&buffer[..n]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
        if data.len() > 16 * 1024 {
            return respond(&mut stream, "413 Payload Too Large", &page("Too large", "<p>Request too large.</p>"));
        }
    };
    let head = String::from_utf8_lossy(&data[..header_end]).into_owned();
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or("/"));
    let length: usize = lines
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0)
        .min(16 * 1024);
    while data.len() < header_end + length {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buffer[..n]);
    }
    let body = String::from_utf8_lossy(&data[header_end..]).into_owned();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    match (method, path) {
        ("GET", "/") if net::form_field(query, "k").as_deref() == Some(key) => respond(&mut stream, "200 OK", &form(key, None)),
        ("POST", "/save") if net::form_field(&body, "k").as_deref() == Some(key) => {
            let url = normalise_url(&net::form_field(&body, "url").unwrap_or_default());
            let token = net::form_field(&body, "token").unwrap_or_default().trim().to_string();
            match verify(&url, &token) {
                Ok(()) => {
                    let _ = requests.send(ServiceRequest::HaConfigured { url: url.clone(), token });
                    respond(
                        &mut stream,
                        "200 OK",
                        &page("Connected", &format!("<h1>Connected</h1><p>Vision is now connected to <b>{}</b>. You can close this page.</p>", escape(&url))),
                    )
                }
                Err(error) => respond(&mut stream, "200 OK", &form(key, Some(&error))),
            }
        }
        _ => respond(&mut stream, "403 Forbidden", &page("Vision", "<p>Scan the code shown on the TV to open this page.</p>")),
    }
}

fn verify(url: &str, token: &str) -> Result<(), String> {
    if url.is_empty() || token.is_empty() {
        return Err("Please enter both the address and the token.".into());
    }
    let auth = format!("Bearer {token}");
    match net::get_json(&net::agent(), &format!("{url}/api/"), &[("Authorization", &auth)]) {
        Ok(_) => Ok(()),
        Err(error) if error.contains("401") => Err("Home Assistant rejected the token.".into()),
        Err(error) => Err(format!("Could not reach Home Assistant at {url} ({error}).")),
    }
}

fn normalise_url(raw: &str) -> String {
    let url = raw.trim().trim_end_matches('/');
    if url.is_empty() || url.contains("://") {
        url.to_string()
    } else {
        format!("http://{url}")
    }
}

fn respond(stream: &mut TcpStream, status: &str, html: &str) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{html}",
        html.len()
    );
    stream.write_all(response.as_bytes())
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn form(key: &str, error: Option<&str>) -> String {
    let error = error.map(|e| format!("<p class=\"error\">{}</p>", escape(e))).unwrap_or_default();
    page(
        "Connect Home Assistant",
        &format!(
            r#"<h1>Connect Home Assistant</h1>
<p>In Home Assistant, open your profile → <b>Security</b> → <b>Long-lived access tokens</b> and create a token for Vision.</p>
{error}
<form method="post" action="/save">
<input type="hidden" name="k" value="{key}">
<label>Home Assistant address<input name="url" placeholder="http://homeassistant.local:8123" autocomplete="url" required></label>
<label>Long-lived access token<textarea name="token" rows="5" required></textarea></label>
<button type="submit">Connect</button>
</form>"#
        ),
    )
}

fn page(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>{title}</title>
<style>
body{{margin:0;padding:24px 20px;background:#07080B;color:#F5F1E8;font:16px/1.5 system-ui,sans-serif}}
h1{{font-size:24px;margin:0 0 12px}}p{{color:#C9C5BC}}b{{color:#F5F1E8}}
label{{display:block;margin:18px 0 6px;font-weight:600}}
input,textarea{{display:block;width:100%;box-sizing:border-box;margin-top:6px;padding:12px;border-radius:12px;border:1px solid #33363d;background:#14161C;color:#F5F1E8;font:inherit}}
button{{margin-top:22px;width:100%;padding:14px;border:0;border-radius:24px;background:#F5F1E8;color:#07080B;font:600 16px system-ui,sans-serif}}
.error{{color:#FF8A80}}
</style></head><body>{body}</body></html>"#
    )
}
