use anyhow::Result;
use redis::AsyncCommands;
use redis::aio::ConnectionManager;

/// Increments a key and ensures it has an expiry window. Returns current count after increment.
pub async fn incr_with_expiry(conn: &mut ConnectionManager, key: &str, window_secs: usize) -> Result<u64> {
    // Use INCR and set EXPIRE if newly created
    let cnt: u64 = conn.incr(key, 1u64).await?;
    if cnt == 1 {
        let _: () = conn.expire(key, window_secs as usize).await?;
    }
    Ok(cnt)
}
