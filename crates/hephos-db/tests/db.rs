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
    // Claiming does NOT increment attempts (attempts counts handler failures, not
    // deliveries), so a successful first delivery still shows 0.
    assert_eq!(my_job.attempts, 0);
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
    // Claim does not increment; this is the first delivery, 0 prior failures.
    assert_eq!(target.attempts, 0);

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
    // One prior failure (from attempt 1); claim added nothing.
    assert_eq!(target2.attempts, 1);

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
async fn jobs_crash_reclaim_does_not_burn_attempts() {
    // Audit fix: a worker that claims a job and then crashes (never completes/fails)
    // must not consume a retry when the job is reclaimed via the visibility timeout.
    let db = hephos_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");
    let jobs = db.jobs();
    jobs.migrate().await.expect("migrate jobs table");

    let job = jobs
        .enqueue(
            "crashy_task",
            &serde_json::json!({ "n": 1 }),
            hephos_db::JobOptions::new().max_attempts(3),
        )
        .await
        .expect("enqueue");

    let vis = std::time::Duration::from_secs(1);

    // Delivery 1: claim, then simulate a crash (never complete/fail).
    let claimed = jobs
        .claim("worker-crash-1", 10, vis)
        .await
        .expect("claim 1");
    let t1 = claimed
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("claimed 1");
    assert_eq!(t1.attempts, 0);

    // Wait past the visibility timeout so the orphan is reclaimable.
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;

    // Delivery 2: reclaim the orphaned job — attempts must still be 0.
    let reclaimed = jobs
        .claim("worker-crash-2", 10, vis)
        .await
        .expect("claim 2");
    let t2 = reclaimed
        .into_iter()
        .find(|j| j.id == job.id)
        .expect("reclaimed");
    assert_eq!(t2.attempts, 0, "a crash/reclaim must not burn a retry");
    assert_eq!(t2.locked_by.as_deref(), Some("worker-crash-2"));
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

#[tokio::test]
#[ignore = "requires Postgres (DATABASE_URL)"]
async fn workflows_postgres_checkpointing_resumption_and_saga() {
    let db = hephos_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");
    let workflows = db.workflows();
    workflows.migrate().await.expect("migrate workflows schema");

    let wf_id = uuid::Uuid::new_v4();
    let step1_counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Phase 1: Run step 1, then crash before step 2
    {
        let mut flow = workflows
            .start_with_id(
                wf_id,
                "onboarding_flow",
                &serde_json::json!({ "user": "bob" }),
            )
            .await
            .expect("start workflow");

        let c = step1_counter.clone();
        let val: i32 = flow
            .step("provision_resource", || async move {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(123)
            })
            .await
            .expect("step 1");
        assert_eq!(val, 123);
        assert_eq!(step1_counter.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Register compensation
        flow.register_compensation(
            "provision_resource",
            "deprovision",
            &serde_json::json!({ "resource_id": 123 }),
        )
        .await
        .expect("register compensation");
        // Simulated process termination
    }

    // Phase 2: Resume after crash — step 1 must be skipped using cached Postgres checkpoint
    {
        let mut flow = workflows.resume(wf_id).await.expect("resume workflow");

        let c = step1_counter.clone();
        let val: i32 = flow
            .step("provision_resource", || async move {
                c.fetch_add(100, std::sync::atomic::Ordering::SeqCst);
                Ok(999)
            })
            .await
            .expect("step 1 on resume");
        // Value came from Postgres checkpoint, closure was not invoked
        assert_eq!(val, 123);
        assert_eq!(step1_counter.load(std::sync::atomic::Ordering::SeqCst), 1);

        // Concurrently approve from background task
        let wf_clone = workflows.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            wf_clone
                .approve(
                    wf_id,
                    "manager_review",
                    Some(serde_json::json!({ "approved": true })),
                )
                .await
                .expect("approve");
        });

        let decision = flow
            .await_approval("manager_review", std::time::Duration::from_secs(2))
            .await
            .expect("approval decision");
        assert_eq!(
            decision,
            hephos_db::ApprovalDecision::Approved(Some(serde_json::json!({ "approved": true })))
        );

        // Step 2 completes
        let welcome: String = flow
            .step(
                "send_welcome",
                || async move { Ok("welcome_sent".to_string()) },
            )
            .await
            .expect("step 2");
        assert_eq!(welcome, "welcome_sent");

        flow.complete(&serde_json::json!({ "status": "ok" }))
            .await
            .expect("complete workflow");
    }

    let record = workflows.get(wf_id).await.unwrap().unwrap();
    assert_eq!(record.state, hephos_db::WorkflowState::Completed);

    let steps = workflows.steps(wf_id).await.unwrap();
    assert_eq!(steps.len(), 2);
}
