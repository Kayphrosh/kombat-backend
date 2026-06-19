use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Value};

use crate::models::{CreateMatchRequest, CreateOpponentRequest, GridSyncRequest};

#[derive(Debug, Clone)]
pub struct GridConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: String,
    pub matches_path: String,
    pub auth_header: String,
    pub default_statuses: Vec<String>,
    pub default_videogame_slugs: Vec<String>,
    pub default_per_page: u32,
    pub default_max_pages: u32,
}

impl GridConfig {
    pub fn from_env() -> Self {
        let api_key = std::env::var("GRID_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty());
        let enabled = env_bool("GRID_ENABLED", api_key.is_some());
        let base_url = std::env::var("GRID_BASE_URL")
            .unwrap_or_else(|_| "https://api.grid.gg".to_string())
            .trim_end_matches('/')
            .to_string();
        let matches_path = std::env::var("GRID_MATCHES_PATH")
            .unwrap_or_else(|_| "/matches".to_string())
            .trim()
            .trim_start_matches('/')
            .to_string();
        let auth_header =
            std::env::var("GRID_AUTH_HEADER").unwrap_or_else(|_| "x-api-key".to_string());
        let default_statuses = env_csv("GRID_DEFAULT_STATUSES")
            .unwrap_or_else(|| vec!["upcoming".to_string(), "running".to_string()]);
        let default_videogame_slugs = env_csv("GRID_VIDEOGAME_SLUGS").unwrap_or_default();
        let default_per_page = std::env::var("GRID_PER_PAGE")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(100)
            .clamp(1, 100);
        let default_max_pages = std::env::var("GRID_MAX_PAGES")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(3)
            .clamp(1, 20);

        Self {
            enabled,
            api_key,
            base_url,
            matches_path,
            auth_header,
            default_statuses,
            default_videogame_slugs,
            default_per_page,
            default_max_pages,
        }
    }

    pub fn configured(&self) -> bool {
        self.enabled && self.api_key.is_some()
    }
}

pub struct GridService {
    config: GridConfig,
    client: Client,
}

impl GridService {
    pub fn new(config: GridConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    pub fn config(&self) -> &GridConfig {
        &self.config
    }

    pub async fn fetch_matches(&self, req: &GridSyncRequest) -> Result<Vec<CreateMatchRequest>> {
        if !self.config.enabled {
            return Err(anyhow!("GRID sync is disabled"));
        }
        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow!("GRID_API_KEY is not configured"))?;

        let statuses = req
            .statuses
            .clone()
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| self.config.default_statuses.clone());
        let videogame_slugs = req
            .videogame_slugs
            .clone()
            .unwrap_or_else(|| self.config.default_videogame_slugs.clone());
        let max_pages = req
            .max_pages
            .unwrap_or(self.config.default_max_pages)
            .clamp(1, 20);
        let per_page = req
            .per_page
            .unwrap_or(self.config.default_per_page)
            .clamp(1, 100);

        let mut matches = Vec::new();
        for status in statuses {
            if videogame_slugs.is_empty() {
                self.fetch_status(
                    api_key,
                    &status,
                    None,
                    req.tournament_id.as_deref(),
                    req.tournament_slug.as_deref(),
                    max_pages,
                    per_page,
                    &mut matches,
                )
                .await?;
            } else {
                for slug in &videogame_slugs {
                    self.fetch_status(
                        api_key,
                        &status,
                        Some(slug.as_str()),
                        req.tournament_id.as_deref(),
                        req.tournament_slug.as_deref(),
                        max_pages,
                        per_page,
                        &mut matches,
                    )
                    .await?;
                }
            }
        }

        Ok(matches)
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_status(
        &self,
        api_key: &str,
        status: &str,
        videogame_slug: Option<&str>,
        tournament_id: Option<&str>,
        tournament_slug: Option<&str>,
        max_pages: u32,
        per_page: u32,
        out: &mut Vec<CreateMatchRequest>,
    ) -> Result<()> {
        for page in 1..=max_pages {
            let path = format!("{}/{}", self.config.base_url, self.config.matches_path);
            let mut request = self
                .client
                .get(&path)
                .header(&self.config.auth_header, api_key)
                .query(&[
                    ("page", page.to_string()),
                    ("per_page", per_page.to_string()),
                    ("limit", per_page.to_string()),
                    ("status", status.to_string()),
                ]);
            if let Some(slug) = videogame_slug {
                request = request.query(&[
                    ("videogame", slug.to_string()),
                    ("videogame_slug", slug.to_string()),
                ]);
            }
            if let Some(id) = tournament_id {
                request = request.query(&[("tournament_id", id.to_string())]);
            }
            if let Some(slug) = tournament_slug {
                request = request.query(&[("tournament_slug", slug.to_string())]);
            }

            let response = request.send().await?;
            if !response.status().is_success() {
                let status_code = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow!("GRID returned {}: {}", status_code, body));
            }

            let raw: Value = response.json().await?;
            let raw_matches = extract_items(&raw);
            if raw_matches.is_empty() {
                break;
            }

            out.extend(raw_matches.into_iter().filter_map(grid_match_from_value));
        }

        Ok(())
    }
}

fn grid_match_from_value(raw: Value) -> Option<CreateMatchRequest> {
    let external_id = string_or_number_at_any(
        &raw,
        &[
            &["id"],
            &["grid_id"],
            &["match_id"],
            &["match", "id"],
            &["data", "id"],
        ],
    )?;
    let pandascore_id = external_id_to_i64(&external_id);
    let opponents = opponents_from_value(&raw);
    let raw_data = with_provider_metadata(raw, &external_id);

    Some(CreateMatchRequest {
        pandascore_id,
        slug: string_at_any(
            &raw_data,
            &[&["slug"], &["name"], &["match", "slug"], &["data", "slug"]],
        ),
        name: string_at_any(
            &raw_data,
            &[
                &["name"],
                &["title"],
                &["match", "name"],
                &["data", "name"],
                &["fixture", "name"],
            ],
        )
        .unwrap_or_else(|| format!("GRID match {}", external_id)),
        videogame_id: i32_at_any(&raw_data, &[&["videogame", "id"], &["game", "id"]]),
        videogame_name: string_at_any(
            &raw_data,
            &[
                &["videogame", "name"],
                &["game", "name"],
                &["title", "name"],
            ],
        ),
        videogame_slug: string_at_any(
            &raw_data,
            &[
                &["videogame", "slug"],
                &["game", "slug"],
                &["title", "slug"],
                &["game", "key"],
            ],
        ),
        league_id: i32_at_any(
            &raw_data,
            &[&["league", "id"], &["competition", "id"], &["series", "id"]],
        ),
        league_name: string_at_any(
            &raw_data,
            &[
                &["league", "name"],
                &["competition", "name"],
                &["series", "name"],
            ],
        ),
        league_slug: string_at_any(
            &raw_data,
            &[
                &["league", "slug"],
                &["competition", "slug"],
                &["series", "slug"],
            ],
        ),
        league_image_url: string_at_any(
            &raw_data,
            &[&["league", "image_url"], &["competition", "image_url"]],
        ),
        series_id: i32_at_any(&raw_data, &[&["series", "id"], &["serie", "id"]]),
        series_name: string_at_any(&raw_data, &[&["series", "name"], &["serie", "name"]]),
        series_full_name: string_at_any(
            &raw_data,
            &[&["series", "full_name"], &["serie", "full_name"]],
        ),
        tournament_id: i32_at_any(&raw_data, &[&["tournament", "id"], &["stage", "id"]]),
        tournament_name: string_at_any(&raw_data, &[&["tournament", "name"], &["stage", "name"]]),
        tournament_slug: string_at_any(&raw_data, &[&["tournament", "slug"], &["stage", "slug"]]),
        scheduled_at: string_at_any(
            &raw_data,
            &[
                &["scheduled_at"],
                &["scheduledAt"],
                &["start_time"],
                &["startTime"],
                &["startDate"],
            ],
        ),
        begin_at: string_at_any(
            &raw_data,
            &[&["begin_at"], &["beginAt"], &["started_at"], &["startedAt"]],
        ),
        end_at: string_at_any(
            &raw_data,
            &[&["end_at"], &["endAt"], &["ended_at"], &["endedAt"]],
        ),
        match_type: string_at_any(
            &raw_data,
            &[&["match_type"], &["matchType"], &["format"], &["type"]],
        ),
        number_of_games: i32_at_any(
            &raw_data,
            &[
                &["number_of_games"],
                &["numberOfGames"],
                &["best_of"],
                &["bestOf"],
            ],
        ),
        pandascore_status: string_at_any(&raw_data, &[&["status"], &["state"]])
            .map(|status| normalize_status(&status)),
        opponents,
        streams_list: raw_data.get("streams_list").cloned(),
        raw_data: Some(raw_data),
        sui_network: None,
        sui_pool_object_id: None,
        source: Some("grid".to_string()),
    })
}

fn opponents_from_value(raw: &Value) -> Vec<CreateOpponentRequest> {
    let arrays = [
        &["opponents"][..],
        &["competitors"][..],
        &["participants"][..],
        &["teams"][..],
        &["sides"][..],
        &["match", "opponents"][..],
    ];

    for path in arrays {
        if let Some(items) = value_at(raw, path).and_then(Value::as_array) {
            let opponents = items
                .iter()
                .filter_map(opponent_from_value)
                .collect::<Vec<_>>();
            if !opponents.is_empty() {
                return opponents;
            }
        }
    }

    Vec::new()
}

fn opponent_from_value(raw: &Value) -> Option<CreateOpponentRequest> {
    let id = string_or_number_at_any(
        raw,
        &[
            &["opponent", "id"],
            &["team", "id"],
            &["participant", "id"],
            &["competitor", "id"],
            &["id"],
        ],
    )?;
    let name = string_at_any(
        raw,
        &[
            &["opponent", "name"],
            &["team", "name"],
            &["participant", "name"],
            &["competitor", "name"],
            &["name"],
        ],
    )?;

    Some(CreateOpponentRequest {
        pandascore_id: external_id_to_i32(&id),
        opponent_type: string_at_any(raw, &[&["type"], &["opponent_type"], &["kind"]])
            .unwrap_or_else(|| "Team".to_string()),
        name,
        acronym: string_at_any(raw, &[&["acronym"], &["shortName"], &["abbreviation"]]),
        image_url: string_at_any(raw, &[&["image_url"], &["imageUrl"], &["logoUrl"]]),
        location: string_at_any(raw, &[&["location"], &["country"], &["region"]]),
    })
}

fn extract_items(value: &Value) -> Vec<Value> {
    if let Some(items) = value.as_array() {
        return items.clone();
    }

    for path in [
        &["data"][..],
        &["matches"][..],
        &["items"][..],
        &["results"][..],
        &["nodes"][..],
        &["data", "matches"][..],
        &["data", "items"][..],
        &["data", "results"][..],
        &["data", "nodes"][..],
    ] {
        if let Some(items) = value_at(value, path).and_then(Value::as_array) {
            return items.clone();
        }
    }

    Vec::new()
}

fn with_provider_metadata(mut raw: Value, external_id: &str) -> Value {
    if let Some(obj) = raw.as_object_mut() {
        obj.insert("provider".to_string(), json!("grid"));
        obj.insert("grid_external_id".to_string(), json!(external_id));
    }
    raw
}

fn normalize_status(status: &str) -> String {
    match status.to_ascii_lowercase().as_str() {
        "scheduled" | "created" | "not_started" | "not-started" | "upcoming" => {
            "not_started".to_string()
        }
        "live" | "running" | "in_progress" | "in-progress" | "started" => "running".to_string(),
        "complete" | "completed" | "finished" | "ended" => "finished".to_string(),
        "cancelled" | "canceled" | "deleted" => "canceled".to_string(),
        "postponed" | "delayed" => "postponed".to_string(),
        other => other.to_string(),
    }
}

fn string_at_any(value: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| value_at(value, path).and_then(value_to_string))
        .filter(|value| !value.is_empty())
}

fn string_or_number_at_any(value: &Value, paths: &[&[&str]]) -> Option<String> {
    string_at_any(value, paths)
}

fn i32_at_any(value: &Value, paths: &[&[&str]]) -> Option<i32> {
    paths
        .iter()
        .find_map(|path| value_at(value, path).and_then(Value::as_i64))
        .and_then(|value| i32::try_from(value).ok())
}

fn value_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn external_id_to_i64(value: &str) -> i64 {
    value
        .parse::<i64>()
        .unwrap_or_else(|_| stable_hash(value, i64::MAX as u64) as i64)
}

fn external_id_to_i32(value: &str) -> i32 {
    value
        .parse::<i32>()
        .unwrap_or_else(|_| stable_hash(value, i32::MAX as u64) as i32)
}

fn stable_hash(value: &str, max: u64) -> u64 {
    let mut hash = 14_695_981_039_346_656_037u64;
    for byte in value.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(1_099_511_628_211);
    }
    (hash % max).max(1)
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn env_csv(name: &str) -> Option<Vec<String>> {
    let values = std::env::var(name).ok()?;
    let values = values
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    Some(values)
}
