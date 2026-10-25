//! Service credentials and choices, kept in the data directory (never in the repository).
use serde_json::{json, Value};
use std::path::Path;

#[derive(Clone, Debug, Default)]
pub struct Config {
    pub ha_url: Option<String>,
    pub ha_token: Option<String>,
    /// Home Assistant entities chosen for the Home row, in display order (empty: automatic).
    pub ha_entities: Vec<String>,
    pub plex_client_id: String,
    pub plex_token: Option<String>,
    pub plex_user: Option<String>,
    pub plex_server: Option<String>,
    pub plex_server_token: Option<String>,
    /// TiviOne integration: None until the user decided.
    pub tivione: Option<bool>,
    /// Key shared with TiviOne when paired (base64); seals its home-screen snapshot.
    pub tivione_key: Option<String>,
    /// Weather location override; otherwise taken from Home Assistant.
    pub location: Option<(f64, f64)>,
    pub location_name: Option<String>,
}

impl Config {
    pub fn load(dir: &Path) -> Self {
        let value: Value = std::fs::read_to_string(dir.join("config.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or(Value::Null);
        let text = |key: &str| value.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        let mut config = Self {
            ha_url: text("haUrl"),
            ha_token: text("haToken"),
            ha_entities: value
                .get("haEntities")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            plex_client_id: text("plexClientId").unwrap_or_default(),
            plex_token: text("plexToken"),
            plex_user: text("plexUser"),
            plex_server: text("plexServer"),
            plex_server_token: text("plexServerToken"),
            tivione: value.get("tivione").and_then(Value::as_bool),
            tivione_key: text("tivioneKey"),
            location: value
                .get("location")
                .and_then(|l| Some((l.get("lat")?.as_f64()?, l.get("lon")?.as_f64()?))),
            location_name: value.pointer("/location/name").and_then(Value::as_str).map(str::to_string),
        };
        if config.plex_client_id.is_empty() {
            config.plex_client_id = random_id();
            let _ = config.save(dir);
        }
        config
    }

    /// Atomic write; readable by the owner only on the TV.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut body = json!({
            "version": 1,
            "haUrl": self.ha_url,
            "haToken": self.ha_token,
            "haEntities": self.ha_entities,
            "plexClientId": self.plex_client_id,
            "plexToken": self.plex_token,
            "plexUser": self.plex_user,
            "plexServer": self.plex_server,
            "plexServerToken": self.plex_server_token,
            "tivione": self.tivione,
            "tivioneKey": self.tivione_key,
        });
        if let Some((lat, lon)) = self.location {
            body["location"] = json!({ "lat": lat, "lon": lon, "name": self.location_name });
        }
        let temp = dir.join("config.json.tmp");
        std::fs::write(&temp, serde_json::to_vec_pretty(&body).unwrap_or_default())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&temp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(temp, dir.join("config.json"))
    }

    pub fn ha(&self) -> Option<(&str, &str)> {
        Some((self.ha_url.as_deref()?, self.ha_token.as_deref()?))
    }
}

/// 16 hex digits from the clock and process id (identifies this TV to Plex; not a secret).
pub fn random_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    hasher.write_u32(std::process::id());
    format!("{:016x}", hasher.finish())
}
