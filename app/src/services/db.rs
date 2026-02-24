// app/src/services/db.rs
//! PostgreSQL persistence layer using sqlx.
//! Indexes on-chain wager state for fast queryability.
//! All queries use runtime-checked sqlx (no compile-time DB required).

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
        sqlx::query(
            "INSERT INTO nonces (wallet, nonce, expires_at) VALUES ($1, $2, $3)"
        )
        .bind(wallet)
        .bind(nonce)
        .bind(expires_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn consume_nonce(&self, wallet: &str, nonce: &str) -> Result<Option<NonceRecord>> {
        let row = sqlx::query_as::<_, NonceRecord>(
            r#"SELECT id, wallet, nonce, used, expires_at, created_at FROM nonces
               WHERE wallet = $1 AND nonce = $2 AND used = FALSE AND expires_at > NOW() LIMIT 1"#,
        )
        .bind(wallet)
        .bind(nonce)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = &row {
            sqlx::query("UPDATE nonces SET used = TRUE WHERE id = $1")
                .bind(r.id)
                .execute(&self.pool)
                .await?;
        }

        Ok(row)
    }

    pub async fn get_latest_unused_nonce(&self, wallet: &str) -> Result<Option<NonceRecord>> {
        let row = sqlx::query_as::<_, NonceRecord>(
            r#"SELECT id, wallet, nonce, used, expires_at, created_at FROM nonces
               WHERE wallet = $1 AND used = FALSE AND expires_at > NOW() ORDER BY created_at DESC LIMIT 1"#,
        )
        .bind(wallet)
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
        sqlx::query(
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
        )
        .bind(&w.id)
        .bind(&w.on_chain_address)
        .bind(w.wager_id)
        .bind(&w.initiator)
        .bind(&w.challenger)
        .bind(w.stake_lamports)
        .bind(&w.description)
        .bind(&w.status)
        .bind(&w.resolution_source)
        .bind(&w.resolver)
        .bind(w.expiry_ts)
        .bind(w.created_at)
        .bind(w.resolved_at)
        .bind(&w.winner)
        .bind(w.protocol_fee_bps as i16)
        .bind(&w.oracle_feed)
        .bind(w.oracle_target)
        .bind(w.dispute_opened_at)
        .bind(&w.dispute_opener)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_wager_by_address(&self, address: &str) -> Result<Option<WagerRecord>> {
        let row = sqlx::query_as::<_, WagerRecord>(
            r#"SELECT
                id, on_chain_address, wager_id, initiator, challenger,
                stake_lamports, description, status, resolution_source,
                resolver, expiry_ts, created_at, resolved_at, winner,
                protocol_fee_bps, oracle_feed, oracle_target,
                dispute_opened_at, dispute_opener
              FROM wagers WHERE on_chain_address = $1"#,
        )
        .bind(address)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_wagers(&self, q: &WagerListQuery) -> Result<Vec<WagerRecord>> {
        let limit  = q.limit.unwrap_or(20).min(100);
        let offset = q.offset.unwrap_or(0);

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
        sqlx::query(
            "UPDATE wagers SET status = $1 WHERE on_chain_address = $2"
        )
        .bind(status)
        .bind(address)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── User Profile ──────────────────────────────────────────────────────────

    pub async fn get_user(&self, wallet: &str) -> Result<Option<UserRecord>> {
        let row = sqlx::query_as::<_, UserRecord>(
            r#"SELECT id, wallet_address, email, display_name, avatar_url,
                      wins, losses, created_at, updated_at
               FROM users WHERE wallet_address = $1"#,
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn upsert_user(
        &self,
        wallet: &str,
        req: &UpdateProfileRequest,
    ) -> Result<UserRecord> {
        let row = sqlx::query_as::<_, UserRecord>(
            r#"INSERT INTO users (wallet_address, email, display_name, avatar_url)
               VALUES ($1, $2, $3, $4)
               ON CONFLICT (wallet_address) DO UPDATE SET
                   email        = COALESCE(EXCLUDED.email,        users.email),
                   display_name = COALESCE(EXCLUDED.display_name, users.display_name),
                   avatar_url   = COALESCE(EXCLUDED.avatar_url,   users.avatar_url),
                   updated_at   = NOW()
               RETURNING id, wallet_address, email, display_name, avatar_url,
                         wins, losses, created_at, updated_at"#,
        )
        .bind(wallet)
        .bind(&req.email)
        .bind(&req.display_name)
        .bind(&req.avatar_url)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    // ── Delete User ──────────────────────────────────────────────────────────

    pub async fn delete_user(&self, wallet: &str) -> Result<()> {
        // Delete related data first
        sqlx::query("DELETE FROM notifications WHERE user_wallet = $1")
            .bind(wallet)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM notification_settings WHERE user_wallet = $1")
            .bind(wallet)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM nonces WHERE wallet = $1")
            .bind(wallet)
            .execute(&self.pool)
            .await?;
        sqlx::query("DELETE FROM users WHERE wallet_address = $1")
            .bind(wallet)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ── User Stats ──────────────────────────────────────────────────────────

    pub async fn get_user_stats(&self, wallet: &str) -> Result<crate::models::UserStats> {
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM wagers WHERE status = 'active' AND (initiator = $1 OR challenger = $1)"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let completed: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)::BIGINT FROM wagers WHERE status = 'resolved' AND (initiator = $1 OR challenger = $1)"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_stake: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(stake_lamports), 0)::BIGINT FROM wagers WHERE (initiator = $1 OR challenger = $1)"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        let total_won: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(stake_lamports * 2), 0)::BIGINT FROM wagers WHERE status = 'resolved' AND winner = $1"
        )
        .bind(wallet)
        .fetch_one(&self.pool)
        .await?;

        Ok(crate::models::UserStats {
            live_count: live,
            completed_count: completed,
            total_stake,
            total_won,
        })
    }

    // ── Notification Settings ────────────────────────────────────────────────

    pub async fn get_notification_settings(&self, wallet: &str) -> Result<crate::models::NotificationSettings> {
        let row = sqlx::query_as::<_, crate::models::NotificationSettings>(
            "SELECT user_wallet, challenges, funds, disputes, marketing FROM notification_settings WHERE user_wallet = $1"
        )
        .bind(wallet)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.unwrap_or(crate::models::NotificationSettings {
            user_wallet: wallet.to_string(),
            challenges: true,
            funds: true,
            disputes: true,
            marketing: false,
        }))
    }

    pub async fn upsert_notification_settings(
        &self,
        wallet: &str,
        settings: &crate::models::UpdateNotificationSettings,
    ) -> Result<crate::models::NotificationSettings> {
        let row = sqlx::query_as::<_, crate::models::NotificationSettings>(
            r#"INSERT INTO notification_settings (user_wallet, challenges, funds, disputes, marketing)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (user_wallet) DO UPDATE SET
                   challenges = COALESCE($2, notification_settings.challenges),
                   funds      = COALESCE($3, notification_settings.funds),
                   disputes   = COALESCE($4, notification_settings.disputes),
                   marketing  = COALESCE($5, notification_settings.marketing)
               RETURNING user_wallet, challenges, funds, disputes, marketing"#
        )
        .bind(wallet)
        .bind(settings.challenges)
        .bind(settings.funds)
        .bind(settings.disputes)
        .bind(settings.marketing)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }
}