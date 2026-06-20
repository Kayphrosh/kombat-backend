use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Value};

use crate::models::{
    CreateMatchRequest, CreateOpponentRequest, GridProbeResponse, GridSyncRequest,
};

#[derive(Debug, Clone)]
pub struct GridConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
    pub base_url: String,
    pub matches_path: String,
    pub auth_header: String,
    pub api_style: String,
    pub graphql_query: Option<String>,
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
            .unwrap_or_else(|_| "https://api-op.grid.gg".to_string())
            .trim_end_matches('/')
            .to_string();
        let matches_path = std::env::var("GRID_MATCHES_PATH")
            .unwrap_or_else(|_| "central-data/graphql".to_string())
            .trim()
            .trim_start_matches('/')
            .to_string();
        let auth_header =
            std::env::var("GRID_AUTH_HEADER").unwrap_or_else(|_| "x-api-key".to_string());
        let api_style = std::env::var("GRID_API_STYLE")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| {
                if matches_path.contains("graphql") {
                    "graphql".to_string()
                } else {
                    "rest".to_string()
                }
            });
        let graphql_query = std::env::var("GRID_GRAPHQL_QUERY")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let default_statuses = env_csv("GRID_DEFAULT_STATUSES").unwrap_or_default();
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
            api_style,
            graphql_query,
            default_statuses,
            default_videogame_slugs,
            default_per_page,
            default_max_pages,
        }
    }

    pub fn configured(&self) -> bool {
        self.enabled
            && self.api_key.is_some()
            && !self.base_url.is_empty()
            && !self.matches_path.is_empty()
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
        let api_key = self.api_key()?;

        let statuses = req
            .statuses
            .clone()
            .unwrap_or_else(|| self.config.default_statuses.clone())
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();
        let status_filters = if statuses.is_empty() {
            vec![None]
        } else {
            statuses.into_iter().map(Some).collect::<Vec<_>>()
        };
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

        if self.is_graphql() {
            return self.fetch_graphql_matches(api_key, req, per_page).await;
        }

        let mut matches = Vec::new();
        for status in status_filters {
            if videogame_slugs.is_empty() {
                self.fetch_status(
                    api_key,
                    status.as_deref(),
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
                        status.as_deref(),
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

    pub async fn probe_matches(&self, req: &GridSyncRequest) -> Result<GridProbeResponse> {
        let api_key = self.api_key()?;
        if self.is_graphql() {
            return self.probe_graphql_matches(api_key, req).await;
        }

        let status = req
            .statuses
            .as_ref()
            .and_then(|items| items.iter().find(|item| !item.trim().is_empty()))
            .cloned()
            .or_else(|| self.config.default_statuses.first().cloned())
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty());
        let videogame_slug = req
            .videogame_slugs
            .as_ref()
            .and_then(|items| items.iter().find(|item| !item.trim().is_empty()))
            .cloned()
            .or_else(|| self.config.default_videogame_slugs.first().cloned());
        let per_page = req
            .per_page
            .unwrap_or(self.config.default_per_page)
            .clamp(1, 100);
        let page = 1u32;
        let path = format!("{}/{}", self.config.base_url, self.config.matches_path);
        let mut request = self
            .client
            .get(&path)
            .header(&self.config.auth_header, api_key)
            .query(&[
                ("page", page.to_string()),
                ("per_page", per_page.to_string()),
                ("limit", per_page.to_string()),
            ]);

        let mut url_params = vec![
            format!("page={}", page),
            format!("per_page={}", per_page),
            format!("limit={}", per_page),
        ];

        if let Some(status) = status.as_deref() {
            request = request.query(&[("status", status.to_string())]);
            url_params.push(format!("status={}", status));
        }
        if let Some(slug) = videogame_slug.as_deref() {
            request = request.query(&[
                ("videogame", slug.to_string()),
                ("videogame_slug", slug.to_string()),
            ]);
            url_params.push(format!("videogame={}", slug));
            url_params.push(format!("videogame_slug={}", slug));
        }
        if let Some(id) = req.tournament_id.as_deref() {
            request = request.query(&[("tournament_id", id.to_string())]);
            url_params.push(format!("tournament_id={}", id));
        }
        if let Some(slug) = req.tournament_slug.as_deref() {
            request = request.query(&[("tournament_slug", slug.to_string())]);
            url_params.push(format!("tournament_slug={}", slug));
        }

        let response = request.send().await?;
        let http_status = response.status();
        let body = response.text().await.unwrap_or_default();
        let mut item_count = 0usize;
        let mut parsed_count = 0usize;

        if http_status.is_success() {
            if let Ok(raw) = serde_json::from_str::<Value>(&body) {
                let raw_matches = extract_items(&raw);
                item_count = raw_matches.len();
                parsed_count = raw_matches
                    .into_iter()
                    .filter_map(grid_match_from_value)
                    .count();
            }
        }

        Ok(GridProbeResponse {
            provider: "grid".to_string(),
            url: format!("{}?{}", path, url_params.join("&")),
            http_status: http_status.as_u16(),
            success: http_status.is_success(),
            item_count,
            parsed_count,
            body_preview: truncate_body(&body, 1200),
        })
    }

    fn is_graphql(&self) -> bool {
        self.config.api_style.eq_ignore_ascii_case("graphql")
            || self.config.matches_path.contains("graphql")
    }

    async fn fetch_graphql_matches(
        &self,
        api_key: &str,
        req: &GridSyncRequest,
        per_page: u32,
    ) -> Result<Vec<CreateMatchRequest>> {
        let (_, body, _) = self.send_graphql_request(api_key, req, per_page).await?;
        let raw: Value = serde_json::from_str(&body)?;
        if let Some(errors) = raw.get("errors") {
            return Err(anyhow!("GRID GraphQL returned errors: {}", errors));
        }

        let status_filters = normalized_status_filters(req, &self.config.default_statuses);
        let videogame_filters =
            normalized_videogame_filters(req, &self.config.default_videogame_slugs);
        let matches = extract_items(&raw)
            .into_iter()
            .filter_map(grid_match_from_value)
            .filter(|item| match_status_allowed(item, &status_filters))
            .filter(|item| match_videogame_allowed(item, &videogame_filters))
            .collect();

        Ok(matches)
    }

    async fn probe_graphql_matches(
        &self,
        api_key: &str,
        req: &GridSyncRequest,
    ) -> Result<GridProbeResponse> {
        let per_page = req
            .per_page
            .unwrap_or(self.config.default_per_page)
            .clamp(1, 100);
        let (http_status, body, url) = self.send_graphql_request(api_key, req, per_page).await?;
        let mut item_count = 0usize;
        let mut parsed_count = 0usize;

        if http_status.is_success() {
            if let Ok(raw) = serde_json::from_str::<Value>(&body) {
                let raw_matches = extract_items(&raw);
                item_count = raw_matches.len();
                parsed_count = raw_matches
                    .into_iter()
                    .filter_map(grid_match_from_value)
                    .count();
            }
        }

        Ok(GridProbeResponse {
            provider: "grid".to_string(),
            url,
            http_status: http_status.as_u16(),
            success: http_status.is_success(),
            item_count,
            parsed_count,
            body_preview: truncate_body(&body, 1200),
        })
    }

    async fn send_graphql_request(
        &self,
        api_key: &str,
        req: &GridSyncRequest,
        per_page: u32,
    ) -> Result<(reqwest::StatusCode, String, String)> {
        let path = format!("{}/{}", self.config.base_url, self.config.matches_path);
        let query = req
            .graphql_query
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| self.config.graphql_query.clone())
            .unwrap_or_else(default_graphql_query);
        let variables = graphql_variables(req, &self.config, per_page);
        let response = self
            .client
            .post(&path)
            .header(&self.config.auth_header, api_key)
            .header("content-type", "application/json")
            .json(&json!({
                "query": query,
                "variables": variables,
            }))
            .send()
            .await?;
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Ok((status, body, path))
    }

    fn api_key(&self) -> Result<&str> {
        if !self.config.enabled {
            return Err(anyhow!("GRID sync is disabled"));
        }
        if self.config.base_url.is_empty() {
            return Err(anyhow!("GRID_BASE_URL is not configured"));
        }
        if self.config.matches_path.is_empty() {
            return Err(anyhow!("GRID_MATCHES_PATH is not configured"));
        }
        self.config
            .api_key
            .as_ref()
            .map(String::as_str)
            .ok_or_else(|| anyhow!("GRID_API_KEY is not configured"))
    }

    #[allow(clippy::too_many_arguments)]
    async fn fetch_status(
        &self,
        api_key: &str,
        status: Option<&str>,
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
                ]);
            if let Some(status) = status {
                request = request.query(&[("status", status.to_string())]);
            }
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

fn default_graphql_query() -> String {
    // GRID Open Access "Central Data" schema: matches are exposed as `allSeries`.
    r#"
query KombatGridSeries(
  $first: Int!
  $filter: SeriesFilter
  $orderBy: SeriesOrderBy
  $orderDirection: OrderDirection
) {
  allSeries(
    first: $first
    filter: $filter
    orderBy: $orderBy
    orderDirection: $orderDirection
  ) {
    totalCount
    edges {
      node {
        id
        type
        startTimeScheduled
        title {
          id
          name
          nameShortened
        }
        tournament {
          id
          name
        }
        teams {
          baseInfo {
            id
            name
            nameShortened
            logoUrl
          }
        }
      }
    }
  }
}
"#
    .to_string()
}

fn graphql_variables(req: &GridSyncRequest, config: &GridConfig, per_page: u32) -> Value {
    let videogame_slugs = normalized_videogame_filters(req, &config.default_videogame_slugs);
    let title_ids = videogame_slugs
        .iter()
        .filter_map(|slug| videogame_slug_to_title_id(slug))
        .map(|id| id.to_string())
        .collect::<Vec<_>>();

    let mut filter = serde_json::Map::new();
    if !title_ids.is_empty() {
        filter.insert("titleIds".to_string(), json!({ "in": title_ids }));
    }
    if let Some(id) = req.tournament_id.as_deref() {
        filter.insert("tournamentId".to_string(), json!(id));
    }
    // Open Access is timeframe-based; pull series from the recent past onward so we
    // catch both upcoming fixtures and matches that are already live.
    let window_start = (chrono::Utc::now() - chrono::Duration::hours(12)).to_rfc3339();
    filter.insert("startTimeScheduled".to_string(), json!({ "gte": window_start }));

    json!({
        "first": per_page,
        "filter": Value::Object(filter),
        "orderBy": "StartTimeScheduled",
        "orderDirection": "ASC",
    })
}

/// Maps the videogame slugs we configure to GRID Central Data `titleId`s.
/// Only the titles licensed under the current Open Access key are mapped.
fn videogame_slug_to_title_id(slug: &str) -> Option<u32> {
    match slug.trim().to_ascii_lowercase().as_str() {
        "csgo" | "cs:go" | "counter-strike" => Some(1),
        "dota" | "dota2" | "dota-2" => Some(2),
        "cs2" | "counter-strike-2" => Some(28),
        _ => None,
    }
}

fn normalized_status_filters(req: &GridSyncRequest, defaults: &[String]) -> Vec<String> {
    req.statuses
        .clone()
        .unwrap_or_else(|| defaults.to_vec())
        .into_iter()
        .map(|item| normalize_status(&item))
        .filter(|item| !item.is_empty())
        .collect()
}

fn normalized_videogame_filters(req: &GridSyncRequest, defaults: &[String]) -> Vec<String> {
    req.videogame_slugs
        .clone()
        .unwrap_or_else(|| defaults.to_vec())
        .into_iter()
        .map(|item| item.trim().to_ascii_lowercase())
        .filter(|item| !item.is_empty())
        .collect()
}

fn match_status_allowed(item: &CreateMatchRequest, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }
    item.pandascore_status
        .as_deref()
        .map(|status| {
            filters
                .iter()
                .any(|filter| filter == &normalize_status(status))
        })
        .unwrap_or(false)
}

fn match_videogame_allowed(item: &CreateMatchRequest, filters: &[String]) -> bool {
    if filters.is_empty() {
        return true;
    }
    item.videogame_slug
        .as_deref()
        .map(|slug| {
            let slug = slug.trim().to_ascii_lowercase();
            filters.iter().any(|filter| filter == &slug)
        })
        .unwrap_or(false)
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
    let provider_numeric_id = external_id_to_i64(&external_id);
    let opponents = opponents_from_value(&raw);
    let raw_data = with_provider_metadata(raw, &external_id);

    // Build a human-readable "A vs B" name when the provider doesn't supply one
    // (GRID Central Data series have no `name` field).
    let derived_name = match opponents.as_slice() {
        [a, b, ..] => Some(format!("{} vs {}", a.name, b.name)),
        _ => None,
    };

    let scheduled_at = string_at_any(
        &raw_data,
        &[
            &["scheduled_at"],
            &["scheduledAt"],
            &["startTimeScheduled"],
            &["start_time"],
            &["startTime"],
            &["startDate"],
        ],
    );

    Some(CreateMatchRequest {
        pandascore_id: provider_numeric_id,
        slug: string_at_any(
            &raw_data,
            &[&["slug"], &["name"], &["match", "slug"], &["data", "slug"]],
        ),
        name: string_at_any(
            &raw_data,
            &[
                &["name"],
                &["match", "name"],
                &["data", "name"],
                &["fixture", "name"],
            ],
        )
        .or(derived_name)
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
                &["title", "nameShortened"],
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
        scheduled_at: scheduled_at.clone(),
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
        // GRID Central Data exposes no live match state, so fall back to deriving
        // the status from the scheduled start time when no explicit field is present.
        pandascore_status: string_at_any(&raw_data, &[&["status"], &["state"]])
            .map(|status| normalize_status(&status))
            .or_else(|| derive_status_from_schedule(scheduled_at.as_deref())),
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
            &["baseInfo", "id"],
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
            &["baseInfo", "name"],
            &["opponent", "name"],
            &["team", "name"],
            &["participant", "name"],
            &["competitor", "name"],
            &["name"],
        ],
    )?;

    let provider_numeric_id = external_id_to_i32(&id);

    Some(CreateOpponentRequest {
        pandascore_id: provider_numeric_id,
        opponent_type: string_at_any(raw, &[&["type"], &["opponent_type"], &["kind"]])
            .unwrap_or_else(|| "Team".to_string()),
        name,
        acronym: string_at_any(
            raw,
            &[
                &["baseInfo", "nameShortened"],
                &["acronym"],
                &["shortName"],
                &["abbreviation"],
            ],
        ),
        image_url: string_at_any(
            raw,
            &[
                &["baseInfo", "logoUrl"],
                &["image_url"],
                &["imageUrl"],
                &["logoUrl"],
            ],
        ),
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
        &["allSeries"][..],
        &["series"][..],
        &["items"][..],
        &["results"][..],
        &["nodes"][..],
        &["data", "matches"][..],
        &["data", "allSeries"][..],
        &["data", "series"][..],
        &["data", "items"][..],
        &["data", "results"][..],
        &["data", "nodes"][..],
    ] {
        if let Some(items) = value_at(value, path).and_then(Value::as_array) {
            return items.clone();
        }
    }

    for path in [
        &["edges"][..],
        &["data", "edges"][..],
        &["data", "matches", "edges"][..],
        &["data", "allSeries", "edges"][..],
        &["data", "series", "edges"][..],
        &["allSeries", "edges"][..],
        &["data", "items", "edges"][..],
    ] {
        if let Some(items) = value_at(value, path).and_then(Value::as_array) {
            return items
                .iter()
                .filter_map(|item| item.get("node").cloned().or_else(|| Some(item.clone())))
                .collect();
        }
    }

    Vec::new()
}

fn truncate_body(value: &str, max_chars: usize) -> String {
    let mut out = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

fn with_provider_metadata(mut raw: Value, external_id: &str) -> Value {
    if let Some(obj) = raw.as_object_mut() {
        obj.insert("provider".to_string(), json!("grid"));
        obj.insert("grid_external_id".to_string(), json!(external_id));
    }
    raw
}

/// GRID Central Data does not return a live match state. Approximate one from the
/// scheduled start time: future fixtures are `not_started`, anything that has
/// already started is treated as `running`.
fn derive_status_from_schedule(scheduled_at: Option<&str>) -> Option<String> {
    let scheduled = scheduled_at?;
    let start = chrono::DateTime::parse_from_rfc3339(scheduled).ok()?;
    if start > chrono::Utc::now() {
        Some("not_started".to_string())
    } else {
        Some("running".to_string())
    }
}

fn normalize_status(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "scheduled" | "created" | "not_started" | "not-started" | "upcoming" => {
            "not_started".to_string()
        }
        "live" | "running" | "in_progress" | "in-progress" | "started" | "ongoing" => {
            "running".to_string()
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rest_like_match_payload() {
        let raw = serde_json::json!({
            "id": "grid-match-1",
            "name": "Alpha vs Beta",
            "status": "scheduled",
            "scheduledAt": "2026-06-20T18:00:00Z",
            "game": { "id": 23, "name": "Call of Duty", "slug": "codm" },
            "competition": { "id": 77, "name": "GRID Cup", "slug": "grid-cup" },
            "tournament": { "id": 88, "name": "Playoffs", "slug": "grid-cup-playoffs" },
            "competitors": [
                { "id": "team-a", "name": "Alpha", "shortName": "ALP" },
                { "id": "team-b", "name": "Beta", "shortName": "BET" }
            ]
        });

        let parsed = grid_match_from_value(raw).expect("match should parse");
        assert_eq!(parsed.source.as_deref(), Some("grid"));
        assert_eq!(parsed.name, "Alpha vs Beta");
        assert_eq!(parsed.pandascore_status.as_deref(), Some("not_started"));
        assert_eq!(parsed.opponents.len(), 2);
        assert_eq!(parsed.opponents[0].name, "Alpha");
    }

    #[test]
    fn extracts_graphql_edges_nodes() {
        let raw = serde_json::json!({
            "data": {
                "matches": {
                    "edges": [
                        {
                            "node": {
                                "id": "grid-match-2",
                                "name": "Gamma vs Delta",
                                "state": "live",
                                "startTime": "2026-06-20T19:00:00Z",
                                "teams": [
                                    { "id": "team-g", "name": "Gamma" },
                                    { "id": "team-d", "name": "Delta" }
                                ]
                            }
                        }
                    ]
                }
            }
        });

        let items = extract_items(&raw);
        assert_eq!(items.len(), 1);
        let parsed = grid_match_from_value(items[0].clone()).expect("match should parse");
        assert_eq!(parsed.name, "Gamma vs Delta");
        assert_eq!(parsed.pandascore_status.as_deref(), Some("running"));
        assert_eq!(parsed.opponents.len(), 2);
    }

    #[test]
    fn parses_grid_central_data_series_payload() {
        // Mirrors the real GRID Open Access `allSeries` response shape: no top-level
        // `name`/`status`, nested `title.nameShortened` and `teams[].baseInfo`.
        let raw = serde_json::json!({
            "data": {
                "allSeries": {
                    "edges": [
                        {
                            "node": {
                                "id": "2949156",
                                "startTimeScheduled": "2999-01-01T08:00:00Z",
                                "title": { "id": "28", "name": "Counter Strike 2", "nameShortened": "cs2" },
                                "tournament": { "id": "55", "name": "CCT 2026 Europe Series 4" },
                                "teams": [
                                    { "baseInfo": { "id": "910", "name": "Team Nemesis", "nameShortened": "NEM", "logoUrl": "https://cdn.grid.gg/nem.png" } },
                                    { "baseInfo": { "id": "911", "name": "Team TDK", "nameShortened": "TDK" } }
                                ]
                            }
                        }
                    ]
                }
            }
        });

        let items = extract_items(&raw);
        assert_eq!(items.len(), 1);
        let parsed = grid_match_from_value(items[0].clone()).expect("series should parse");
        assert_eq!(parsed.source.as_deref(), Some("grid"));
        assert_eq!(parsed.name, "Team Nemesis vs Team TDK");
        assert_eq!(parsed.videogame_slug.as_deref(), Some("cs2"));
        assert_eq!(parsed.tournament_name.as_deref(), Some("CCT 2026 Europe Series 4"));
        // Future start time -> derived not_started status.
        assert_eq!(parsed.pandascore_status.as_deref(), Some("not_started"));
        assert_eq!(parsed.opponents.len(), 2);
        assert_eq!(parsed.opponents[0].name, "Team Nemesis");
        assert_eq!(parsed.opponents[0].acronym.as_deref(), Some("NEM"));
        assert_eq!(
            parsed.opponents[0].image_url.as_deref(),
            Some("https://cdn.grid.gg/nem.png")
        );
    }

    #[test]
    fn maps_open_access_slugs_to_title_ids() {
        assert_eq!(videogame_slug_to_title_id("csgo"), Some(1));
        assert_eq!(videogame_slug_to_title_id("dota"), Some(2));
        assert_eq!(videogame_slug_to_title_id("dota2"), Some(2));
        assert_eq!(videogame_slug_to_title_id("cs2"), Some(28));
        assert_eq!(videogame_slug_to_title_id("valorant"), None);
    }
}
