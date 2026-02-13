// app/src/services/db.rs
//! PostgreSQL persistence layer using sqlx.
//! Indexes on-chain wager state for fast queryability.

use anyhow::Result;
use sqlx::{PgPool, postgres::PgPoolOptions};
use crate::models::{WagerRecord, WagerListQuery, UserRecord, UpdateProfileRequest, NotificationRecord, NonceRecord};
use serde_json::Value as JsonValue;

pub struct DbService {
    pool: PgPool,
}

impl DbService {
    pub async fn new(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(database_url)
            .await?;
        Ok(Self { pool })
    }

    // ── Notifications ────────────────────────────────────────────────────

    pub async fn create_notification(&self, user_wallet: &str, kind: &str, payload: Option<JsonValue>) -> Result<()> {
        sqlx::query(
            "INSERT INTO notifications (user_wallet, kind, payload) VALUES ($1, $2, $3)"
        )
        .bind(user_wallet)
        .bind(kind)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Nonce (auth) ──────────────────────────────────────────────────────

    pub async fn insert_nonce(&self, wallet: &str, nonce: &str, expires_at: chrono::DateTime<chrono::Utc>) -> Result<()> {
        sqlx::query!(
            "INSERT INTO nonces (wallet, nonce, expires_at) VALUES ($1, $2, $3)",
            wallet,
            nonce,
            expires_at,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn consume_nonce(&self, wallet: &str, nonce: &str) -> Result<Option<NonceRecord>> {
        // Attempt to find an unused, unexpired nonce matching wallet and nonce.
        let row = sqlx::query_as!(
            NonceRecord,
            r#"SELECT id, wallet, nonce, used, expires_at, created_at FROM nonces
               WHERE wallet = $1 AND nonce = $2 AND used = FALSE AND expires_at > NOW() LIMIT 1"#,
            wallet,
            nonce
        )
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = &row {
            // mark used
            sqlx::query!("UPDATE nonces SET used = TRUE WHERE id = $1", r.id)
                .execute(&self.pool)
                .await?;
        }

        Ok(row)
    }

    pub async fn get_latest_unused_nonce(&self, wallet: &str) -> Result<Option<NonceRecord>> {
        let row = sqlx::query_as!(
            NonceRecord,
            r#"SELECT id, wallet, nonce, used, expires_at, created_at FROM nonces
               WHERE wallet = $1 AND used = FALSE AND expires_at > NOW() ORDER BY created_at DESC LIMIT 1"#,
            wallet
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_notifications_for_user(&self, wallet: &str, limit: i64, offset: i64) -> Result<Vec<NotificationRecord>> {
        let rows = sqlx::query_as::<_, NotificationRecord>(
            "SELECT id, user_wallet, kind, payload, is_read, created_at FROM notifications WHERE user_wallet = $1 ORDER BY created_at DESC LIMIT $2 OFFSET $3"
        )
        .bind(wallet)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn mark_notification_read(&self, id: &str) -> Result<()> {
        sqlx::query(
            "UPDATE notifications SET is_read = TRUE WHERE id = $1"
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Wager CRUD ────────────────────────────────────────────────────────────

    pub async fn upsert_wager(&self, w: &WagerRecord) -> Result<()> {
        sqlx::query!(
            r#"
            INSERT INTO wagers (
                id, on_chain_address, wager_id, initiator, challenger,
                stake_lamports, description, status, resolution_source,
                resolver, expiry_ts, created_at, resolved_at, winner,
                protocol_fee_bps, oracle_feed, oracle_target,
                dispute_opened_at, dispute_opener
            ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16, $17, $18, $19
            )
            ON CONFLICT (on_chain_address) DO UPDATE SET
                challenger       = EXCLUDED.challenger,
                status           = EXCLUDED.status,
                resolved_at      = EXCLUDED.resolved_at,
                winner           = EXCLUDED.winner,
                dispute_opened_at = EXCLUDED.dispute_opened_at,
                dispute_opener   = EXCLUDED.dispute_opener
            "#,
            w.id,
            w.on_chain_address,
            w.wager_id,
            w.initiator,
            w.challenger,
            w.stake_lamports,
            w.description,
            w.status,
            w.resolution_source,
            w.resolver,
            w.expiry_ts,
            w.created_at,
            w.resolved_at,
            w.winner,
            w.protocol_fee_bps as i16,
            w.oracle_feed,
            w.oracle_target,
            w.dispute_opened_at,
            w.dispute_opener,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_wager_by_address(&self, address: &str) -> Result<Option<WagerRecord>> {
        let row = sqlx::query_as!(
            WagerRecord,
            r#"SELECT
                id, on_chain_address, wager_id, initiator, challenger,
                stake_lamports, description, status, resolution_source,
                resolver, expiry_ts, created_at, resolved_at, winner,
                protocol_fee_bps, oracle_feed, oracle_target,
                dispute_opened_at, dispute_opener
              FROM wagers WHERE on_chain_address = $1"#,
            address
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_wagers(&self, q: &WagerListQuery) -> Result<Vec<WagerRecord>> {
        let limit  = q.limit.unwrap_or(20).min(100);
        let offset = q.offset.unwrap_or(0);

        // Dynamic filtering via sqlx macro requires static queries;
        // we use a query builder instead.
        let mut qb = sqlx::QueryBuilder::new(
            r#"SELECT id, on_chain_address, wager_id, initiator, challenger,
               stake_lamports, description, status, resolution_source,
               resolver, expiry_ts, created_at, resolved_at, winner,
               protocol_fee_bps, oracle_feed, oracle_target,
               dispute_opened_at, dispute_opener
               FROM wagers WHERE 1=1"#,
        );

        if let Some(ref ini) = q.initiator {
            qb.push(" AND initiator = ").push_bind(ini.clone());
        }
        if let Some(ref ch) = q.challenger {
            qb.push(" AND challenger = ").push_bind(ch.clone());
        }
        if let Some(ref st) = q.status {
            qb.push(" AND status = ").push_bind(st.clone());
        }

        qb.push(" ORDER BY created_at DESC LIMIT ")
          .push_bind(limit)
          .push(" OFFSET ")
          .push_bind(offset);

        let rows = qb
            .build_query_as::<WagerRecord>()
            .fetch_all(&self.pool)
            .await?;

        Ok(rows)
    }

    pub async fn update_wager_status(&self, address: &str, status: &str) -> Result<()> {
        sqlx::query!(
            "UPDATE wagers SET status = $1 WHERE on_chain_address = $2",
            status,
            address
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── User Profile ──────────────────────────────────────────────────────────

    pub async fn get_user(&self, wallet: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query_as!(
            UserRecord,
            r#"SELECT id, wallet_address, display_name, avatar_url,
                      wins, losses, created_at, updated_at
               FROM users WHERE wallet_address = $1"#,
            wallet
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn upsert_user(
        &self,
        wallet: &str,
        req: &UpdateProfileRequest,
    ) -> Result<UserRecord> {
        let row = sqlx::query_as!(
            UserRecord,
            r#"INSERT INTO users (wallet_address, display_name, avatar_url)
               VALUES ($1, $2, $3)
               ON CONFLICT (wallet_address) DO UPDATE SET
                   display_name = COALESCE(EXCLUDED.display_name, users.display_name),
                   avatar_url   = COALESCE(EXCLUDED.avatar_url,   users.avatar_url),
                   updated_at   = NOW()
               RETURNING id, wallet_address, display_name, avatar_url,
                         wins, losses, created_at, updated_at"#,
            wallet,
            req.display_name,
            req.avatar_url,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }
}