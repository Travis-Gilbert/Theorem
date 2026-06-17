//! Postgres hot execution queue for Dispatch v2.
//!
//! The canonical coordination thread stays in the THG Dispatch v2 board. This
//! crate owns only hot execution state: claim leases, retries, completion, and
//! dead-letter visibility.

pub mod model;

pub use model::{
    priority_from_harness, priority_to_harness, ClaimedJob, FailureClass, Head, Job, JobState,
    ReapReport, StateCount,
};

use serde_json::Value;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::Row;
use std::time::Duration;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const MIGRATION_0001: &str = include_str!("../migrations/0001_dispatch_jobs.sql");

#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    #[error("postgres error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("invalid dispatch value: {0}")]
    Invalid(String),
    #[error("dispatch job not found or not mutable: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, DispatchError>;

#[derive(Clone)]
pub struct DispatchQueue {
    pool: PgPool,
}

impl DispatchQueue {
    pub async fn connect(database_url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await?;
        Self::from_pool(pool).await
    }

    pub async fn from_pool(pool: PgPool) -> Result<Self> {
        let queue = Self { pool };
        queue.migrate().await?;
        Ok(queue)
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn migrate(&self) -> Result<()> {
        sqlx::raw_sql(MIGRATION_0001).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn submit(&self, job: Job, priority: i16) -> Result<String> {
        validate_job(&job)?;
        let not_before = parse_not_before(job.not_before.as_deref())?;
        let max_attempts = job.max_attempts.unwrap_or(3).max(1);
        let target_head = job.target_head.as_str();
        let inserted = sqlx::query_scalar::<_, String>(
            r#"
            insert into dispatch_jobs (
                job_id, title, repo, spec_ref, spec_inline, target_head, priority,
                not_before, max_attempts, source_task_id
            )
            values (
                $1, $2, $3, $4, $5, $6::dispatch_head, $7,
                coalesce($8, now()), $9, $10
            )
            on conflict (job_id) do update
            set title = excluded.title,
                repo = excluded.repo,
                spec_ref = excluded.spec_ref,
                spec_inline = excluded.spec_inline,
                target_head = excluded.target_head,
                priority = excluded.priority,
                not_before = excluded.not_before,
                max_attempts = excluded.max_attempts,
                source_task_id = excluded.source_task_id,
                updated_at = now()
            where dispatch_jobs.state = 'pending'::dispatch_state
            returning job_id
            "#,
        )
        .bind(&job.job_id)
        .bind(&job.title)
        .bind(&job.repo)
        .bind(&job.spec_ref)
        .bind(&job.spec_inline)
        .bind(target_head)
        .bind(priority)
        .bind(not_before)
        .bind(max_attempts)
        .bind(&job.source_task_id)
        .fetch_optional(&self.pool)
        .await?;

        match inserted {
            Some(job_id) => Ok(job_id),
            None => self
                .load_job_id(&job.job_id)
                .await?
                .ok_or_else(|| DispatchError::NotFound(job.job_id)),
        }
    }

    pub async fn claim_next(
        &self,
        worker_id: &str,
        head: Head,
        lease: Duration,
    ) -> Result<Option<ClaimedJob>> {
        self.claim_next_for_heads(worker_id, &[head], lease).await
    }

    pub async fn claim_next_for_heads(
        &self,
        worker_id: &str,
        heads: &[Head],
        lease: Duration,
    ) -> Result<Option<ClaimedJob>> {
        let lease_seconds = model::duration_seconds(lease).map_err(DispatchError::Invalid)?;
        let head_values = heads
            .iter()
            .copied()
            .filter(|head| *head != Head::Either)
            .map(|head| head.as_str().to_string())
            .collect::<Vec<_>>();
        if head_values.is_empty() {
            return Ok(None);
        }
        let row = sqlx::query(
            r#"
            update dispatch_jobs
            set state = 'claimed'::dispatch_state,
                claimed_by = $1,
                claimed_at = now(),
                lease_expires_at = now() + ($2::double precision * interval '1 second'),
                attempts = attempts + 1,
                updated_at = now()
            where job_id = (
                select job_id
                from dispatch_jobs
                where state = 'pending'::dispatch_state
                  and not_before <= now()
                  and (target_head::text = any($3) or target_head = 'either'::dispatch_head)
                order by priority, not_before, created_at, job_id
                for update skip locked
                limit 1
            )
            returning
                job_id, title, repo, spec_ref, spec_inline, target_head::text as target_head,
                priority, state::text as state, not_before, claimed_by, claimed_at,
                lease_expires_at, attempts, max_attempts, result, source_task_id,
                created_at, updated_at
            "#,
        )
        .bind(worker_id)
        .bind(lease_seconds)
        .bind(head_values)
        .fetch_optional(&self.pool)
        .await?;
        row.map(row_to_claimed).transpose()
    }

    pub async fn renew_lease(&self, job_id: &str, lease: Duration) -> Result<()> {
        let lease_seconds = model::duration_seconds(lease).map_err(DispatchError::Invalid)?;
        let result = sqlx::query(
            r#"
            update dispatch_jobs
            set state = 'running'::dispatch_state,
                lease_expires_at = now() + ($2::double precision * interval '1 second'),
                updated_at = now()
            where job_id = $1
              and state in ('claimed'::dispatch_state, 'running'::dispatch_state)
            "#,
        )
        .bind(job_id)
        .bind(lease_seconds)
        .execute(&self.pool)
        .await?;
        ensure_mutated(job_id, result.rows_affected())
    }

    pub async fn complete(&self, job_id: &str, result: Value) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            update dispatch_jobs
            set state = 'done'::dispatch_state,
                result = $2,
                lease_expires_at = null,
                updated_at = now()
            where job_id = $1
              and state in ('claimed'::dispatch_state, 'running'::dispatch_state)
            "#,
        )
        .bind(job_id)
        .bind(result)
        .execute(&mut *tx)
        .await?;
        ensure_mutated(job_id, result.rows_affected())?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn fail(&self, job_id: &str, class: FailureClass, error: Value) -> Result<JobState> {
        match class {
            FailureClass::Fatal => self.dead_letter(job_id, error).await,
            FailureClass::Retryable => self.retry_or_dead(job_id, error).await,
        }
    }

    pub async fn reap(&self) -> Result<ReapReport> {
        let mut tx = self.pool.begin().await?;
        let requeued = sqlx::query(
            r#"
            update dispatch_jobs
            set state = 'pending'::dispatch_state,
                claimed_by = null,
                claimed_at = null,
                lease_expires_at = null,
                updated_at = now()
            where state in ('claimed'::dispatch_state, 'running'::dispatch_state)
              and lease_expires_at < now()
              and attempts < max_attempts
            "#,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        let dead = sqlx::query(
            r#"
            update dispatch_jobs
            set state = 'dead'::dispatch_state,
                lease_expires_at = null,
                updated_at = now()
            where state in ('claimed'::dispatch_state, 'running'::dispatch_state)
              and lease_expires_at < now()
              and attempts >= max_attempts
            "#,
        )
        .execute(&mut *tx)
        .await?
        .rows_affected();
        tx.commit().await?;
        Ok(ReapReport { requeued, dead })
    }

    pub async fn state_counts(&self) -> Result<Vec<StateCount>> {
        let rows = sqlx::query(
            r#"
            select state::text as state, count(*)::bigint as count
            from dispatch_jobs
            group by state
            order by state
            "#,
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                let state: String = row.try_get("state")?;
                let count: i64 = row.try_get("count")?;
                Ok(StateCount {
                    state: JobState::try_from(state.as_str()).map_err(DispatchError::Invalid)?,
                    count,
                })
            })
            .collect()
    }

    async fn load_job_id(&self, job_id: &str) -> Result<Option<String>> {
        Ok(
            sqlx::query_scalar("select job_id from dispatch_jobs where job_id = $1")
                .bind(job_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    async fn dead_letter(&self, job_id: &str, error: Value) -> Result<JobState> {
        let state = sqlx::query_scalar::<_, String>(
            r#"
            update dispatch_jobs
            set state = 'dead'::dispatch_state,
                result = $2,
                lease_expires_at = null,
                updated_at = now()
            where job_id = $1
            returning state::text
            "#,
        )
        .bind(job_id)
        .bind(error)
        .fetch_optional(&self.pool)
        .await?;
        state_to_result(job_id, state)
    }

    async fn retry_or_dead(&self, job_id: &str, error: Value) -> Result<JobState> {
        let state = sqlx::query_scalar::<_, String>(
            r#"
            update dispatch_jobs
            set state = case
                    when attempts >= max_attempts then 'dead'::dispatch_state
                    else 'pending'::dispatch_state
                end,
                not_before = case
                    when attempts >= max_attempts then not_before
                    else now() + (
                        least(900, greatest(30, attempts::integer * 30))::double precision
                        * interval '1 second'
                    )
                end,
                claimed_by = case when attempts >= max_attempts then claimed_by else null end,
                claimed_at = case when attempts >= max_attempts then claimed_at else null end,
                lease_expires_at = null,
                result = $2,
                updated_at = now()
            where job_id = $1
            returning state::text
            "#,
        )
        .bind(job_id)
        .bind(error)
        .fetch_optional(&self.pool)
        .await?;
        state_to_result(job_id, state)
    }
}

fn validate_job(job: &Job) -> Result<()> {
    if job.job_id.trim().is_empty() {
        return Err(DispatchError::Invalid("job_id is required".to_string()));
    }
    if job.title.trim().is_empty() {
        return Err(DispatchError::Invalid("title is required".to_string()));
    }
    if job.spec_ref.as_deref().unwrap_or("").trim().is_empty()
        && job.spec_inline.as_deref().unwrap_or("").trim().is_empty()
    {
        return Err(DispatchError::Invalid(
            "spec_ref or spec_inline is required".to_string(),
        ));
    }
    Ok(())
}

fn parse_not_before(value: Option<&str>) -> Result<Option<OffsetDateTime>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if let Some(epoch) = parse_epoch_timestamp(value)? {
        return Ok(Some(epoch));
    }
    OffsetDateTime::parse(value, &Rfc3339)
        .map(Some)
        .map_err(|error| DispatchError::Invalid(format!("invalid not_before '{value}': {error}")))
}

fn parse_epoch_timestamp(value: &str) -> Result<Option<OffsetDateTime>> {
    let Some(value) = value.strip_suffix('Z') else {
        return Ok(None);
    };
    let Some((secs, nanos)) = value.split_once('.') else {
        return Ok(None);
    };
    if !secs.chars().all(|ch| ch.is_ascii_digit()) || !nanos.chars().all(|ch| ch.is_ascii_digit()) {
        return Ok(None);
    }
    let secs = secs
        .parse::<i64>()
        .map_err(|error| DispatchError::Invalid(format!("invalid epoch seconds: {error}")))?;
    let nanos = nanos
        .chars()
        .take(9)
        .collect::<String>()
        .parse::<i64>()
        .map_err(|error| DispatchError::Invalid(format!("invalid epoch nanos: {error}")))?;
    let timestamp = OffsetDateTime::from_unix_timestamp(secs)
        .map_err(|error| DispatchError::Invalid(format!("invalid epoch timestamp: {error}")))?
        + time::Duration::nanoseconds(nanos);
    Ok(Some(timestamp))
}

fn row_to_claimed(row: PgRow) -> Result<ClaimedJob> {
    let head: String = row.try_get("target_head")?;
    let state: String = row.try_get("state")?;
    Ok(ClaimedJob {
        job_id: row.try_get("job_id")?,
        title: row.try_get("title")?,
        repo: row.try_get("repo")?,
        spec_ref: row.try_get("spec_ref")?,
        spec_inline: row.try_get("spec_inline")?,
        target_head: Head::try_from(head.as_str()).map_err(DispatchError::Invalid)?,
        priority: row.try_get("priority")?,
        state: JobState::try_from(state.as_str()).map_err(DispatchError::Invalid)?,
        not_before: row.try_get("not_before")?,
        claimed_by: row.try_get("claimed_by")?,
        claimed_at: row.try_get("claimed_at")?,
        lease_expires_at: row.try_get("lease_expires_at")?,
        attempts: row.try_get("attempts")?,
        max_attempts: row.try_get("max_attempts")?,
        result: row.try_get("result")?,
        source_task_id: row.try_get("source_task_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn ensure_mutated(job_id: &str, rows_affected: u64) -> Result<()> {
    if rows_affected == 0 {
        Err(DispatchError::NotFound(job_id.to_string()))
    } else {
        Ok(())
    }
}

fn state_to_result(job_id: &str, state: Option<String>) -> Result<JobState> {
    let state = state.ok_or_else(|| DispatchError::NotFound(job_id.to_string()))?;
    JobState::try_from(state.as_str()).map_err(DispatchError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn database_url() -> Option<String> {
        std::env::var("THEOREM_DISPATCH_TEST_DATABASE_URL")
            .ok()
            .filter(|value| !value.trim().is_empty())
    }

    fn job(id: &str) -> Job {
        Job {
            job_id: id.to_string(),
            title: format!("test {id}"),
            repo: Some("Travis-Gilbert/Theorem".to_string()),
            spec_ref: Some("docs/plans/x/HANDOFF.md".to_string()),
            spec_inline: None,
            target_head: Head::Either,
            not_before: None,
            source_task_id: None,
            max_attempts: None,
        }
    }

    async fn clear_prefix(queue: &DispatchQueue, prefix: &str) {
        sqlx::query("delete from dispatch_jobs where job_id like $1")
            .bind(format!("{prefix}%"))
            .execute(queue.pool())
            .await
            .unwrap();
    }

    #[test]
    fn parses_harness_epoch_timestamp() {
        let parsed = parse_not_before(Some("1781554580.123456789Z"))
            .unwrap()
            .unwrap();
        assert_eq!(parsed.unix_timestamp(), 1_781_554_580);
        assert_eq!(parsed.nanosecond(), 123_456_789);
    }

    #[tokio::test]
    async fn live_postgres_acceptance_claim_reap_retry_counts_and_heartbeat() {
        let Some(url) = database_url() else {
            eprintln!("skipping live Postgres test: THEOREM_DISPATCH_TEST_DATABASE_URL not set");
            return;
        };
        let queue = DispatchQueue::connect(&url).await.unwrap();
        let prefix = format!("test-{}-", std::process::id());
        clear_prefix(&queue, &prefix).await;

        let first = format!("{prefix}claim-a");
        let second = format!("{prefix}claim-b");
        queue.submit(job(&first), 100).await.unwrap();
        queue.submit(job(&second), 0).await.unwrap();

        let q1 = queue.clone();
        let q2 = queue.clone();
        let (a, b) = tokio::join!(
            q1.claim_next("worker-a", Head::Codex, Duration::from_secs(30)),
            q2.claim_next("worker-b", Head::Codex, Duration::from_secs(30))
        );
        let a = a.unwrap().unwrap();
        let b = b.unwrap().unwrap();
        assert_ne!(
            a.job_id, b.job_id,
            "SKIP LOCKED must prevent duplicate claims"
        );
        assert_eq!(a.job_id, second, "lower priority value claims first");
        queue
            .complete(&a.job_id, json!({"result": "ok"}))
            .await
            .unwrap();

        let expired = format!("{prefix}expired");
        queue.submit(job(&expired), 10).await.unwrap();
        let expired_claim = queue
            .claim_next("worker-c", Head::Codex, Duration::from_millis(250))
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(350)).await;
        let reaped = queue.reap().await.unwrap();
        assert_eq!(reaped.requeued, 1);
        assert_eq!(reaped.dead, 0);
        let reclaimed = queue
            .claim_next("worker-d", Head::Codex, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reclaimed.job_id, expired_claim.job_id);

        let dead = format!("{prefix}dead");
        let mut dead_job = job(&dead);
        dead_job.max_attempts = Some(1);
        queue.submit(dead_job, 20).await.unwrap();
        let dead_claim = queue
            .claim_next("worker-e", Head::Codex, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        let state = queue
            .fail(
                &dead_claim.job_id,
                FailureClass::Retryable,
                json!({"error": "boom"}),
            )
            .await
            .unwrap();
        assert_eq!(state, JobState::Dead);

        let heartbeat = format!("{prefix}heartbeat");
        queue.submit(job(&heartbeat), 30).await.unwrap();
        let heartbeat_claim = queue
            .claim_next("worker-f", Head::Codex, Duration::from_millis(250))
            .await
            .unwrap()
            .unwrap();
        queue
            .renew_lease(&heartbeat_claim.job_id, Duration::from_secs(2))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(350)).await;
        let heartbeat_reap = queue.reap().await.unwrap();
        assert_eq!(heartbeat_reap.requeued, 0);
        assert_eq!(heartbeat_reap.dead, 0);

        let counts = queue.state_counts().await.unwrap();
        assert!(counts.iter().any(|count| count.state == JobState::Done));
        assert!(counts.iter().any(|count| count.state == JobState::Dead));

        clear_prefix(&queue, &prefix).await;
    }
}
