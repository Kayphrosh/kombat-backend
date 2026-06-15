use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::Value;

use crate::models::{CreateMatchRequest, CreateOpponentRequest, PandaScoreSyncRequest};

#[derive(Debug, Clone)]
pub struct PandaScoreConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: String,
    pub default_statuses: Vec<String>,
    pub default_videogame_slugs: Vec<String>,
    pub default_per_page: u32,
}

impl PandaScoreConfig {
    pub fn from_env() -> Self {
        let api_key = std::env::var("PANDASCORE_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty());
        let enabled = env_bool("PANDASCORE_ENABLED", api_key.is_some());
        let base_url = std::env::var("PANDASCORE_BASE_URL")
            .unwrap_or_else(|_| "https://api.pandascore.co".to_string())
            .trim_end_matches('/')
            .to_string();
        let default_statuses = env_csv("PANDASCORE_DEFAULT_STATUSES").unwrap_or_else(|| {
            vec![
                "upcoming".to_string(),
                "running".to_string(),
                "past".to_string(),
            ]
        });
        let default_videogame_slugs = env_csv("PANDASCORE_VIDEOGAME_SLUGS").unwrap_or_default();
        let default_per_page = std::env::var("PANDASCORE_PER_PAGE")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(50)
            .clamp(1, 100);

        Self {
            enabled,
            api_key,
            base_url,
            default_statuses,
            default_videogame_slugs,
            default_per_page,
        }
    }

    pub fn configured(&self) -> bool {
        self.enabled && self.api_key.is_some()
    }
}

pub struct PandaScoreService {
    config: PandaScoreConfig,
    client: Client,
}

impl PandaScoreService {
    pub fn new(config: PandaScoreConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }

    pub fn config(&self) -> &PandaScoreConfig {
        &self.config
    }

    pub async fn fetch_matches(
        &self,
        req: &PandaScoreSyncRequest,
    ) -> Result<Vec<CreateMatchRequest>> {
        if !self.config.enabled {
            return Err(anyhow!("PandaScore sync is disabled"));
        }
        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow!("PANDASCORE_API_KEY is not configured"))?;

        let statuses = req
            .statuses
            .clone()
            .filter(|items| !items.is_empty())
            .unwrap_or_else(|| self.config.default_statuses.clone());
        let videogame_slugs = req
            .videogame_slugs
            .clone()
            .unwrap_or_else(|| self.config.default_videogame_slugs.clone());
        let max_pages = req.max_pages.unwrap_or(1).clamp(1, 10);
        let per_page = req
            .per_page
            .unwrap_or(self.config.default_per_page)
            .clamp(1, 100);

        let mut matches = Vec::new();
        for status in statuses {
            if videogame_slugs.is_empty() {
                self.fetch_status(api_key, &status, None, max_pages, per_page, &mut matches)
                    .await?;
            } else {
                for slug in &videogame_slugs {
                    self.fetch_status(
                        api_key,
                        &status,
                        Some(slug.as_str()),
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

    /// Fetch a single match by its PandaScore id. Returns the raw match JSON so
    /// callers can cross-check status and winner against an agent proposal.
    pub async fn fetch_match_by_id(&self, pandascore_id: i64) -> Result<Value> {
        if !self.config.enabled {
            return Err(anyhow!("PandaScore sync is disabled"));
        }
        let api_key = self
            .config
            .api_key
            .as_ref()
            .ok_or_else(|| anyhow!("PANDASCORE_API_KEY is not configured"))?;

        let path = format!("{}/matches/{}", self.config.base_url, pandascore_id);
        let response = self.client.get(&path).bearer_auth(api_key).send().await?;
        if !response.status().is_success() {
            let status_code = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!("PandaScore returned {}: {}", status_code, body));
        }

        Ok(response.json().await?)
    }

    async fn fetch_status(
        &self,
        api_key: &str,
        status: &str,
        videogame_slug: Option<&str>,
        max_pages: u32,
        per_page: u32,
        out: &mut Vec<CreateMatchRequest>,
    ) -> Result<()> {
        for page in 1..=max_pages {
            let path = match videogame_slug {
                Some(slug) => format!("{}/{}/matches/{}", self.config.base_url, slug, status),
                None => format!("{}/matches/{}", self.config.base_url, status),
            };

            let mut request = self.client.get(&path).bearer_auth(api_key).query(&[
                ("page", page.to_string()),
                ("per_page", per_page.to_string()),
            ]);
            if status == "past" {
                request = request.query(&[("sort", "-end_at")]);
            }

            let response = request.send().await?;
            if !response.status().is_success() {
                let status_code = response.status();
                let body = response.text().await.unwrap_or_default();
                return Err(anyhow!("PandaScore returned {}: {}", status_code, body));
            }

            let raw_matches: Vec<Value> = response.json().await?;
            if raw_matches.is_empty() {
                break;
            }

            out.extend(raw_matches.into_iter().filter_map(match_from_value));
        }

        Ok(())
    }
}

/// The finished result of a PandaScore match, used to cross-check agent
/// proposals. `winner_pandascore_id` is the opponent's PandaScore id.
#[derive(Debug, Clone)]
pub struct PandaScoreResult {
    pub finished: bool,
    pub winner_pandascore_id: Option<i64>,
    pub winner_name: Option<String>,
}

/// Extract the finished status and winner from a raw PandaScore match payload.
pub fn result_from_match(raw: &Value) -> PandaScoreResult {
    let finished = raw
        .get("status")
        .and_then(Value::as_str)
        .map(|s| s == "finished")
        .unwrap_or(false);
    let winner_pandascore_id = raw.get("winner_id").and_then(Value::as_i64).or_else(|| {
        raw.get("winner")
            .and_then(|w| w.get("id"))
            .and_then(Value::as_i64)
    });
    let winner_name = raw
        .get("winner")
        .and_then(|w| w.get("name"))
        .and_then(Value::as_str)
        .map(ToString::to_string);

    PandaScoreResult {
        finished,
        winner_pandascore_id,
        winner_name,
    }
}

fn match_from_value(raw: Value) -> Option<CreateMatchRequest> {
    let pandascore_id = raw.get("id")?.as_i64()?;
    let opponents = raw
        .get("opponents")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(opponent_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(CreateMatchRequest {
        pandascore_id,
        slug: string_at(&raw, &["slug"]),
        name: string_at(&raw, &["name"]).unwrap_or_else(|| format!("Match {}", pandascore_id)),
        videogame_id: i32_at(&raw, &["videogame", "id"]),
        videogame_name: string_at(&raw, &["videogame", "name"]),
        videogame_slug: string_at(&raw, &["videogame", "slug"]),
        league_id: i32_at(&raw, &["league", "id"]),
        league_name: string_at(&raw, &["league", "name"]),
        league_slug: string_at(&raw, &["league", "slug"]),
        league_image_url: string_at(&raw, &["league", "image_url"]),
        series_id: i32_at(&raw, &["serie", "id"]),
        series_name: string_at(&raw, &["serie", "name"]),
        series_full_name: string_at(&raw, &["serie", "full_name"]),
        tournament_id: i32_at(&raw, &["tournament", "id"]),
        tournament_name: string_at(&raw, &["tournament", "name"]),
        tournament_slug: string_at(&raw, &["tournament", "slug"]),
        scheduled_at: string_at(&raw, &["scheduled_at"]),
        begin_at: string_at(&raw, &["begin_at"]),
        end_at: string_at(&raw, &["end_at"]),
        match_type: string_at(&raw, &["match_type"]),
        number_of_games: i32_at(&raw, &["number_of_games"]),
        pandascore_status: string_at(&raw, &["status"]),
        opponents,
        streams_list: raw.get("streams_list").cloned(),
        raw_data: Some(raw),
    })
}

fn opponent_from_value(raw: &Value) -> Option<CreateOpponentRequest> {
    let opponent = raw.get("opponent")?;
    let pandascore_id = i32_at(opponent, &["id"])?;
    let name = string_at(opponent, &["name"])?;

    Some(CreateOpponentRequest {
        pandascore_id,
        opponent_type: string_at(raw, &["type"]).unwrap_or_else(|| "Team".to_string()),
        name,
        acronym: string_at(opponent, &["acronym"]),
        image_url: string_at(opponent, &["image_url"]),
        location: string_at(opponent, &["location"]),
    })
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .filter(|value| !value.is_empty())
}

fn i32_at(value: &Value, path: &[&str]) -> Option<i32> {
    path.iter()
        .try_fold(value, |current, key| current.get(*key))
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
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
