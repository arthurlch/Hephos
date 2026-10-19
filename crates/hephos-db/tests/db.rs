//! Live-database test for `Db`/`Tx`. `#[ignore]`d by default so the offline suite
//! stays green; the CI Postgres job runs it with `-- --include-ignored` and a
//! `DATABASE_URL` set. Uses a TEMP table, so no migration/schema is required.

#[tokio::test]
#[ignore = "requires Postgres (DATABASE_URL)"]
async fn connect_begin_query_commit_roundtrip() {
    let db = hephos_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");

    // Everything runs on the single connection owned by `tx`.
    let mut tx = db.begin().await.unwrap();
    sqlx::query("create temporary table hephos_probe (id int not null)")
        .execute(tx.exec())
        .await
        .unwrap();
    sqlx::query("insert into hephos_probe (id) values (1), (2)")
        .execute(tx.exec())
        .await
        .unwrap();
    let (count,): (i64,) = sqlx::query_as("select count(*) from hephos_probe")
        .fetch_one(tx.exec())
        .await
        .unwrap();
    assert_eq!(count, 2);
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "requires Postgres (DATABASE_URL)"]
async fn ping_succeeds_against_live_database() {
    let db = hephos_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");
    db.ping()
        .await
        .expect("ping must succeed against live database");
}

#[tokio::test]
#[ignore = "requires Postgres (DATABASE_URL)"]
async fn jobs_enqueue_claim_complete_and_idempotency_roundtrip() {
    let db = hephos_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");
    let jobs = db.jobs();
    jobs.migrate().await.expect("migrate jobs table");

    let idemp_key = format!("test-key-{}", uuid::Uuid::new_v4());
    let payload = serde_json::json!({ "account_id": "acc-100", "amount": 50 });

    // 1. Enqueue job
    let job1 = jobs
        .enqueue(
            "process_payment",
            &payload,
            hephos_db::JobOptions::new().idempotency_key(&idemp_key),
        )
        .await
        .expect("enqueue job1");

    assert_eq!(job1.name, "process_payment");
    assert_eq!(job1.state, hephos_db::JobState::Ready);
    assert_eq!(job1.attempts, 0);

    // 2. Enqueue again with the same idempotency key -> returns identical job
    let job2 = jobs
        .enqueue(
            "process_payment",
            &payload,
            hephos_db::JobOptions::new().idempotency_key(&idemp_key),
        )
        .await
        .expect("enqueue job2");

    assert_eq!(job1.id, job2.id, "idempotency key must return existing job");

    // 3. Claim job
    let claimed = jobs
        .claim("worker-test-1", 5, std::time::Duration::from_secs(60))
        .await
        .expect("claim jobs");

    let my_job = claimed.iter().find(|j| j.id == job1.id);
    assert!(my_job.is_some(), "job should be claimed");
    let my_job = my_job.unwrap();
    assert_eq!(my_job.state, hephos_db::JobState::Running);
    assert_eq!(my_job.attempts, 1);
    assert_eq!(my_job.locked_by.as_deref(), Some("worker-test-1"));

    // 4. Complete job
    jobs.complete(job1.id).await.expect("complete job");

    let completed = jobs
        .find(job1.id)
        .await
        .expect("find job")
        .expect("job exists");
    assert_eq!(completed.state, hephos_db::JobState::Completed);
    assert!(completed.locked_at.is_none());
    assert!(completed.locked_by.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres (DATABASE_URL)"]
async fn jobs_retry_backoff_and_dead_letter() {
    let db = hephos_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");
    let jobs = db.jobs();
    jobs.migrate().await.expect("migrate jobs table");

    let payload = serde_json::json!({ "task": "flaky" });
    let job = jobs
        .enqueue(
            "flaky_task",
            &payload,
            hephos_db::JobOptions::new().max_attempts(2),
        )
        .await
        .expect("enqueue job");

    // Claim attempt 1
    let claimed = jobs
        .claim("worker-fail-test", 10, std::time::Duration::from_secs(60))
        .await
        .expect("claim");
    let target = claimed
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("claimed");
    assert_eq!(target.attempts, 1);

    // Fail attempt 1 (not dead letter yet)
    jobs.fail(
        target.id,
        "transient network glitch",
        std::time::Duration::from_millis(50),
        false,
    )
    .await
    .expect("fail attempt 1");

    let state1 = jobs.find(job.id).await.unwrap().unwrap();
    assert_eq!(state1.state, hephos_db::JobState::Ready);
    assert_eq!(
        state1.last_error.as_deref(),
        Some("transient network glitch")
    );

    // Wait until run_at is in past
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // Claim attempt 2
    let claimed2 = jobs
        .claim("worker-fail-test", 10, std::time::Duration::from_secs(60))
        .await
        .expect("claim attempt 2");
    let target2 = claimed2
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("claimed 2");
    assert_eq!(target2.attempts, 2);

    // Fail attempt 2 (exceeded max_attempts 2 -> dead letter)
    jobs.fail(target2.id, "fatal error", std::time::Duration::ZERO, true)
        .await
        .expect("fail attempt 2");

    let state2 = jobs.find(job.id).await.unwrap().unwrap();
    assert_eq!(state2.state, hephos_db::JobState::DeadLetter);
    assert_eq!(state2.last_error.as_deref(), Some("fatal error"));

    // Dead letter queue lookup
    let dlq = jobs.dead_letter_jobs(10).await.expect("query dlq");
    assert!(dlq.iter().any(|j| j.id == job.id));

    // Retry from dead letter
    jobs.retry_dead_letter(job.id)
        .await
        .expect("retry dead letter");
    let retried = jobs.find(job.id).await.unwrap().unwrap();
    assert_eq!(retried.state, hephos_db::JobState::Ready);
    assert_eq!(retried.attempts, 0);
    assert!(retried.last_error.is_none());
}

#[tokio::test]
#[ignore = "requires Postgres (DATABASE_URL)"]
async fn outbox_enqueue_tx_commit_and_drain_roundtrip() {
    let db = hephos_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");
    let outbox = db.outbox();
    outbox.migrate().await.expect("migrate outbox table");

    // 1. Rollback test: nothing is written
    let mut tx_rollback = db.begin().await.expect("begin rollback tx");
    outbox
        .enqueue_tx(
            &mut tx_rollback,
            "user_signup",
            &serde_json::json!({ "user_id": 999 }),
        )
        .await
        .expect("enqueue in rollback tx");
    tx_rollback.rollback().await.expect("rollback");

    // 2. Commit test: atomically committed
    let mut tx_commit = db.begin().await.expect("begin commit tx");
    let payload = serde_json::json!({ "user_id": 1001, "role": "admin" });
    let entry_id = outbox
        .enqueue_tx(&mut tx_commit, "user_signup", &payload)
        .await
        .expect("enqueue in commit tx");
    tx_commit.commit().await.expect("commit");

    // Claim pending
    let claimed = outbox.claim_pending(10).await.expect("claim pending");
    let entry = claimed
        .into_iter()
        .find(|e| e.id == entry_id)
        .expect("entry claimed");
    assert_eq!(entry.event_type, "user_signup");
    assert_eq!(entry.attempts, 1);
    assert_eq!(entry.state, hephos_db::OutboxState::Pending);

    // Mark dispatched
    outbox
        .mark_dispatched(entry_id)
        .await
        .expect("mark dispatched");
    let dispatched = outbox.find(entry_id).await.unwrap().unwrap();
    assert_eq!(dispatched.state, hephos_db::OutboxState::Dispatched);
    assert!(dispatched.dispatched_at.is_some());
}
