//! Live-database test for `Db`/`Tx`. `#[ignore]`d by default so the offline suite
//! stays green; the CI Postgres job runs it with `-- --include-ignored` and a
//! `DATABASE_URL` set. Uses a TEMP table, so no migration/schema is required.

#[tokio::test]
#[ignore = "requires Postgres (DATABASE_URL)"]
async fn connect_begin_query_commit_roundtrip() {
    let db = rivet_db::Db::connect_from_env()
        .await
        .expect("DATABASE_URL must point at a reachable Postgres");

    // Everything runs on the single connection owned by `tx`.
    let mut tx = db.begin().await.unwrap();
    sqlx::query("create temporary table rivet_probe (id int not null)")
        .execute(tx.exec())
        .await
        .unwrap();
    sqlx::query("insert into rivet_probe (id) values (1), (2)")
        .execute(tx.exec())
        .await
        .unwrap();
    let (count,): (i64,) = sqlx::query_as("select count(*) from rivet_probe")
        .fetch_one(tx.exec())
        .await
        .unwrap();
    assert_eq!(count, 2);
    tx.commit().await.unwrap();
}
