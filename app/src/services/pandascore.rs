use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::Value;

use crate::models::{
    CreateMatchRequest, CreateOpponentRequest, PandascoreProbeResponse, PandascoreSyncRequest,
};

#[derive(Debug, Clone)]
pub struct PandascoreConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: String,
    pub default_statuses: Vec<String>,
    pub default_videogame_slugs: Vec<String>,
    pub default_per_page: u32,
    pub default_max_pages: u32,
}

impl PandascoreConfig {
    pub fn from_env() -> Self {
        let api_key = std::env::var("PANDASCORE_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty());
        let enabled = env_bool("PANDASCORE_ENABLED", api_key.is_some());
        let base_url = std::env::var("PANDASCORE_BASE_URL")
            .unwrap_or_else(|_| "https://api.pandascore.co".to_string())
            .trim_end_matches('/')
            .to_string();
        let default_statuses = env_csv("PANDASCORE_DEFAULT_STATUSES")
            .unwrap_or_else(|| vec!["upcoming".to_string(), "running".to_string()]);
        let default_videogame_slugs =
            env_csv("PANDASCORE_VIDEOGAME_SLUGS").unwrap_or_default();
        let default_per_page = std::env::var("PANDASCORE_PER_PAGE")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(50)
            .clamp(1, 100);
        let default_max_pages = std::env::var("PANDASCORE_MAX_PAGES")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .unwrap_or(5)
            .clamp(1, 20);

        Self {
            enabled,
            api_key,
            base_url,
            default_statuses,
            default_videogame_slugs,
            default_per_page,
            default_max_pages,
        }
    }

    pub fn configured(&self) -> bool {
        self.enabled && self.api_key.is_some() && !self.base_url.is_empty()
    }
}

pub struct PandascoreService {
    config: PandascoreConfig,
    client: Client,
}

impl PandascoreService {
    pub fn new(config: PandascoreConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    pub fn config(&self) -> &PandascoreConfig {
        &self.config
    }

    pub async fn fetch_matches(
        &self,
        req: &PandascoreSyncRequest,
    ) -> Result<Vec<CreateMatchRequest>> {
        let api_key = self.api_key()?;
        let statuses = req
            .statuses
            .clone()
            .unwrap_or_else(|| self.config.default_statuses.clone());
        let videogame_slugs = req
            .videogame_slugs
            .clone()
            .unwrap_or_else(|| self.config.default_videogame_slugs.clone());
        let per_page = req
            .per_page
            .unwrap_or(self.config.default_per_page)
            .clamp(1, 100);
        let max_pages = req
            .max_pages
            .unwrap_or(self.config.default_max_pages)
            .clamp(1, 20);

        let mut all_matches = Vec::new();

        // PandaScore has per-status endpoints; iterate statuses then games
        for status in &statuses {
            let path = status_to_path(status);
            if videogame_slugs.is_empty() {
                self.fetch_pages(api_key, &path, None, req, per_page, max_pages, &mut all_matches)
                    .await?;
            } else {
                for slug in &videogame_slugs {
                    self.fetch_pages(
                        api_key,
                        &path,
                        Some(slug.as_str()),
                        req,
                        per_page,
                        max_pages,
                        &mut all_matches,
                    )
                    .await?;
                }
            }
        }

        Ok(all_matches)
    }

    pub async fn probe_matches(
        &self,
        req: &PandascoreSyncRequest,
    ) -> Result<PandascoreProbeResponse> {
        let api_key = self.api_key()?;
        let status = req
            .statuses
            .as_ref()
            .and_then(|s| s.first())
            .cloned()
            .or_else(|| self.config.default_statuses.first().cloned())
            .unwrap_or_else(|| "upcoming".to_string());
        let videogame_slug = req
            .videogame_slugs
            .as_ref()
            .and_then(|s| s.first())
            .cloned()
            .or_else(|| self.config.default_videogame_slugs.first().cloned());
        let per_page = req
            .per_page
            .unwrap_or(self.config.default_per_page)
            .clamp(1, 100);

        let path = status_to_path(&status);
        let url = format!("{}{}", self.config.base_url, path);

        let mut request = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .query(&[("page", "1"), ("per_page", &per_page.to_string())]);

        if let Some(slug) = videogame_slug.as_deref() {
            request = request.query(&[("videogame[]", slug)]);
        }
        if let Some(tournament_id) = req.tournament_id.as_deref() {
            request = request.query(&[("tournament_id", tournament_id)]);
        }
        if let Some(sort) = req.sort.as_deref() {
            request = request.query(&[("sort", sort)]);
        }

        let response = request.send().await?;
        let http_status = response.status();
        let body = response.text().await.unwrap_or_default();
        let mut item_count = 0usize;
        let mut parsed_count = 0usize;

        if http_status.is_success() {
            if let Ok(raw) = serde_json::from_str::<Value>(&body) {
                if let Some(items) = raw.as_array() {
                    item_count = items.len();
                    parsed_count = items
                        .iter()
                        .filter_map(|v| pandascore_match_from_value(v.clone()))
                        .count();
                }
            }
        }

        Ok(PandascoreProbeResponse {
            provider: "pandascore".to_string(),
            url,
            http_status: http_status.as_u16(),
            success: http_status.is_success(),
            item_count,
            parsed_count,
            body_preview: truncate_body(&body, 1200),
        })
    }

    fn api_key(&self) -> Result<&str> {
        if !self.config.enabled {
            return Err(anyhow!("PandaScore sync is disabled"));
        }
        self.config
            .api_key
            .as_ref()
            .map(String::as_str)
            .ok_or_else(|| anyhow!("PANDASCORE_API_KEY is not configured"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_pages(
        &self,
        api_key: &str,
        path: &str,
        videogame_slug: Option<&str>,
        req: &PandascoreSyncRequest,
        per_page: u32,
        max_pages: u32,
        out: &mut Vec<CreateMatchRequest>,
    ) -> Result<()> {
        let url = format!("{}{}", self.config.base_url, path);

        for page in 1..=max_pages {
            let mut request = self
                .client
                .get(&url)
                .header("Authorization", format!("Bearer {}", api_key))
                .query(&[
                    ("page", page.to_string()),
                    ("per_page", per_page.to_string()),
                    ("sort", req.sort.clone().unwrap_or_else(|| "scheduled_at".to_string())),
                ]);

            if let Some(slug) = videogame_slug {
                request = request.query(&[("videogame[]", slug)]);
            }
            if let Some(tournament_id) = req.tournament_id.as_deref() {
                request = request.query(&[("tournament_id", tournament_id)]);
            }
            if let Some(league_id) = req.league_id.as_deref() {
                request = request.query(&[("league_id", league_id)]);
            }
            if let Some(serie_id) = req.serie_id.as_deref() {
                request = request.query(&[("serie_id", serie_id)]);
            }

            let response = request.send().await?;
            if !response.status().is_success() {
                let status_code = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow!("PandaScore returned {}: {}", status_code, body));
            }

            let raw: Value = response.json().await?;
            let items = raw.as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                break;
            }

            out.extend(items.into_iter().filter_map(pandascore_match_from_value));
        }

        Ok(())
    }
}

/// Maps PandaScore status labels to endpoint paths.
fn status_to_path(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "upcoming" | "not_started" | "scheduled" => "/matches/upcoming".to_string(),
        "running" | "live" | "in_progress" => "/matches/running".to_string(),
        "past" | "finished" | "complete" | "ended" => "/matches/past".to_string(),
        _ => "/matches/upcoming".to_string(),
    }
}

fn pandascore_match_from_value(raw: Value) -> Option<CreateMatchRequest> {
    let id = raw.get("id")?.as_i64()?;
    let name = raw
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = if name.is_empty() {
        format!("PandaScore match {}", id)
    } else {
        name
    };

    let videogame = raw.get("videogame");
    let league = raw.get("league");
    let serie = raw.get("serie");
    let tournament = raw.get("tournament");

    let opponents = raw
        .get("opponents")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(pandascore_opponent_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(CreateMatchRequest {
        pandascore_id: id,
        slug: raw
            .get("slug")
            .and_then(Value::as_str)
            .map(str::to_string),
        name,
        videogame_id: videogame
            .and_then(|v| v.get("id"))
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        videogame_name: videogame
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        videogame_slug: videogame
            .and_then(|v| v.get("slug"))
            .and_then(Value::as_str)
            .map(str::to_string),
        league_id: league
            .and_then(|v| v.get("id"))
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        league_name: league
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        league_slug: league
            .and_then(|v| v.get("slug"))
            .and_then(Value::as_str)
            .map(str::to_string),
        league_image_url: league
            .and_then(|v| v.get("image_url"))
            .and_then(Value::as_str)
            .map(str::to_string),
        series_id: serie
            .and_then(|v| v.get("id"))
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        series_name: serie
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        series_full_name: serie
            .and_then(|v| v.get("full_name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        tournament_id: tournament
            .and_then(|v| v.get("id"))
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        tournament_name: tournament
            .and_then(|v| v.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        tournament_slug: tournament
            .and_then(|v| v.get("slug"))
            .and_then(Value::as_str)
            .map(str::to_string),
        scheduled_at: raw
            .get("scheduled_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        begin_at: raw
            .get("begin_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        end_at: raw
            .get("end_at")
            .and_then(Value::as_str)
            .map(str::to_string),
        match_type: raw
            .get("match_type")
            .and_then(Value::as_str)
            .map(str::to_string),
        number_of_games: raw
            .get("number_of_games")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        pandascore_status: raw
            .get("status")
            .and_then(Value::as_str)
            .map(|s| normalize_status(s)),
        opponents,
        streams_list: raw.get("streams_list").cloned(),
        raw_data: Some(raw),
        sui_network: None,
        sui_pool_object_id: None,
        source: Some("pandascore".to_string()),
    })
}

fn pandascore_opponent_from_value(raw: &Value) -> Option<CreateOpponentRequest> {
    let opponent = raw.get("opponent")?;
    let id = opponent.get("id")?.as_i64()?;
    let name = opponent.get("name")?.as_str()?.to_string();

    Some(CreateOpponentRequest {
        pandascore_id: i32::try_from(id).unwrap_or(id as i32),
        opponent_type: raw
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("Team")
            .to_string(),
        name,
        acronym: opponent
            .get("acronym")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        image_url: opponent
            .get("image_url")
            .and_then(Value::as_str)
            .map(str::to_string),
        location: opponent
            .get("location")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

fn normalize_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "not_started" | "upcoming" | "scheduled" => "not_started".to_string(),
        "running" | "live" => "running".to_string(),
        "finished" | "past" | "complete" => "finished".to_string(),
        "canceled" | "cancelled" => "canceled".to_string(),
        "postponed" => "postponed".to_string(),
        other => other.to_string(),
    }
}

fn truncate_body(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|v| match v.to_ascii_lowercase().as_str() {
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
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    Some(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_pandascore_match_payload() {
        let raw = serde_json::json!({
            "id": 136560,
            "name": "Phantom vs Walczaki",
            "slug": "phantom-2026-06-20",
            "status": "not_started",
            "scheduled_at": "2026-06-20T17:30:00Z",
            "match_type": "best_of",
            "number_of_games": 3,
            "videogame": { "id": 3, "name": "Counter-Strike", "slug": "cs-go" },
            "league": { "id": 5505, "name": "Stake Ranked", "slug": "cs-go-stake-ranked", "image_url": null },
            "serie": { "id": 10724, "name": "Episode 3", "full_name": "Episode 3: Closed Qualifier 2026" },
            "tournament": { "id": 21255, "name": "Playoffs", "slug": "cs-go-stake-ranked-playoffs" },
            "opponents": [
                { "type": "Team", "opponent": { "id": 136560, "name": "Phantom", "acronym": "PHA", "location": "PL", "image_url": "https://cdn.example.com/pha.png" } },
                { "type": "Team", "opponent": { "id": 138610, "name": "Walczaki", "acronym": "WAL", "location": "PL", "image_url": null } }
            ],
            "streams_list": [{ "main": true, "language": "en", "raw_url": "https://kick.com/starladder" }]
        });

        let parsed = pandascore_match_from_value(raw).expect("should parse");
        assert_eq!(parsed.source.as_deref(), Some("pandascore"));
        assert_eq!(parsed.pandascore_id, 136560);
        assert_eq!(parsed.name, "Phantom vs Walczaki");
        assert_eq!(parsed.pandascore_status.as_deref(), Some("not_started"));
        assert_eq!(parsed.videogame_slug.as_deref(), Some("cs-go"));
        assert_eq!(parsed.league_name.as_deref(), Some("Stake Ranked"));
        assert_eq!(parsed.tournament_name.as_deref(), Some("Playoffs"));
        assert_eq!(parsed.opponents.len(), 2);
        assert_eq!(parsed.opponents[0].name, "Phantom");
        assert_eq!(parsed.opponents[0].acronym.as_deref(), Some("PHA"));
        assert_eq!(parsed.opponents[1].name, "Walczaki");
    }

    #[test]
    fn status_path_mapping() {
        assert_eq!(status_to_path("upcoming"), "/matches/upcoming");
        assert_eq!(status_to_path("running"), "/matches/running");
        assert_eq!(status_to_path("past"), "/matches/past");
        assert_eq!(status_to_path("finished"), "/matches/past");
    }
}
