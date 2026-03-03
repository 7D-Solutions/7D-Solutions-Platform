use chrono::Utc;
use sqlx::PgPool;
use uuid::Uuid;

const DEFAULT_DB_URL: &str = "postgresql://doc_mgmt_user:doc_mgmt_pass@localhost:5455/doc_mgmt_db";

async fn get_pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| DEFAULT_DB_URL.to_string());
    let pool = PgPool::connect(&url).await.expect("connect doc_mgmt db");
    sqlx::migrate!("./db/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool
}

async fn seed_released_doc(pool: &PgPool, tenant_id: Uuid, doc_number: &str) -> (Uuid, Uuid) {
    let doc_id = Uuid::new_v4();
    let rev_id = Uuid::new_v4();
    let actor_id = Uuid::new_v4();
    let now = Utc::now();

    sqlx::query(
        "INSERT INTO documents (id, tenant_id, doc_number, title, doc_type, status, created_by, created_at, updated_at)
         VALUES ($1, $2, $3, 'GateA', 'spec', 'released', $4, $5, $5)",
    )
    .bind(doc_id)
    .bind(tenant_id)
    .bind(doc_number)
    .bind(actor_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert doc");

    sqlx::query(
        "INSERT INTO revisions (id, document_id, tenant_id, revision_number, body, change_summary, status, created_by, created_at)
         VALUES ($1, $2, $3, 1, '{}'::jsonb, 'Initial', 'released', $4, $5)",
    )
    .bind(rev_id)
    .bind(doc_id)
    .bind(tenant_id)
    .bind(actor_id)
    .bind(now)
    .execute(pool)
    .await
    .expect("insert revision");

    (doc_id, actor_id)
}

#[tokio::test]
async fn tenant_isolation_holds_under_concurrent_doc_ops() {
    let pool = get_pool().await;
    let tenant_a = Uuid::new_v4();
    let tenant_b = Uuid::new_v4();
    let (doc_a, actor_a) = seed_released_doc(&pool, tenant_a, &format!("A-{}", Uuid::new_v4())).await;
    let (doc_b, actor_b) = seed_released_doc(&pool, tenant_b, &format!("B-{}", Uuid::new_v4())).await;

    let p1 = pool.clone();
    let p2 = pool.clone();
    let set_retention_a = async move {
        sqlx::query(
            "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
             VALUES ($1, $2, 'spec', 365, $3, now(), now())
             ON CONFLICT (tenant_id, doc_type) DO UPDATE SET retention_days = EXCLUDED.retention_days, updated_at = now()",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_a)
        .bind(actor_a)
        .execute(&p1)
        .await
        .expect("set retention A");
    };
    let set_retention_b = async move {
        sqlx::query(
            "INSERT INTO retention_policies (id, tenant_id, doc_type, retention_days, created_by, created_at, updated_at)
             VALUES ($1, $2, 'spec', 730, $3, now(), now())
             ON CONFLICT (tenant_id, doc_type) DO UPDATE SET retention_days = EXCLUDED.retention_days, updated_at = now()",
        )
        .bind(Uuid::new_v4())
        .bind(tenant_b)
        .bind(actor_b)
        .execute(&p2)
        .await
        .expect("set retention B");
    };
    tokio::join!(set_retention_a, set_retention_b);

    let p3 = pool.clone();
    let p4 = pool.clone();
    let hold_a = async move {
        sqlx::query(
            "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
             VALUES ($1, $2, $3, 'audit', $4, now())",
        )
        .bind(Uuid::new_v4())
        .bind(doc_a)
        .bind(tenant_a)
        .bind(actor_a)
        .execute(&p3)
        .await
        .expect("insert hold A");
    };
    let hold_b = async move {
        sqlx::query(
            "INSERT INTO legal_holds (id, document_id, tenant_id, reason, held_by, held_at)
             VALUES ($1, $2, $3, 'audit', $4, now())",
        )
        .bind(Uuid::new_v4())
        .bind(doc_b)
        .bind(tenant_b)
        .bind(actor_b)
        .execute(&p4)
        .await
        .expect("insert hold B");
    };
    tokio::join!(hold_a, hold_b);

    let p5 = pool.clone();
    let p6 = pool.clone();
    let distribution_a = async move {
        let dist_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO document_distributions
             (id, tenant_id, document_id, recipient_ref, channel, template_key, payload_json, status, requested_by, requested_at, idempotency_key, created_at, updated_at)
             VALUES ($1, $2, $3, 'qa-a@fireproof.test', 'email', 'doc_distribution_notice', '{}'::jsonb, 'pending', $4, now(), $5, now(), now())",
        )
        .bind(dist_id)
        .bind(tenant_a)
        .bind(doc_a)
        .bind(actor_a)
        .bind(format!("iso-a-{}", Uuid::new_v4()))
        .execute(&p5)
        .await
        .expect("insert distribution A");

        sqlx::query(
            "INSERT INTO document_distribution_status_log
             (distribution_id, tenant_id, previous_status, new_status, idempotency_key, payload_json, changed_by, changed_at)
             VALUES ($1, $2, NULL, 'pending', $3, '{}'::jsonb, $4, now())",
        )
        .bind(dist_id)
        .bind(tenant_a)
        .bind(format!("iso-a-log-{}", Uuid::new_v4()))
        .bind(actor_a)
        .execute(&p5)
        .await
        .expect("insert distribution log A");
    };
    let distribution_b = async move {
        let dist_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO document_distributions
             (id, tenant_id, document_id, recipient_ref, channel, template_key, payload_json, status, requested_by, requested_at, idempotency_key, created_at, updated_at)
             VALUES ($1, $2, $3, 'qa-b@fireproof.test', 'email', 'doc_distribution_notice', '{}'::jsonb, 'pending', $4, now(), $5, now(), now())",
        )
        .bind(dist_id)
        .bind(tenant_b)
        .bind(doc_b)
        .bind(actor_b)
        .bind(format!("iso-b-{}", Uuid::new_v4()))
        .execute(&p6)
        .await
        .expect("insert distribution B");

        sqlx::query(
            "INSERT INTO document_distribution_status_log
             (distribution_id, tenant_id, previous_status, new_status, idempotency_key, payload_json, changed_by, changed_at)
             VALUES ($1, $2, NULL, 'pending', $3, '{}'::jsonb, $4, now())",
        )
        .bind(dist_id)
        .bind(tenant_b)
        .bind(format!("iso-b-log-{}", Uuid::new_v4()))
        .bind(actor_b)
        .execute(&p6)
        .await
        .expect("insert distribution log B");
    };
    tokio::join!(distribution_a, distribution_b);

    let docs_a: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE tenant_id = $1")
        .bind(tenant_a)
        .fetch_one(&pool)
        .await
        .expect("count docs A");
    let docs_b: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE tenant_id = $1")
        .bind(tenant_b)
        .fetch_one(&pool)
        .await
        .expect("count docs B");
    assert_eq!(docs_a, 1);
    assert_eq!(docs_b, 1);

    let revs_a: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE tenant_id = $1")
        .bind(tenant_a)
        .fetch_one(&pool)
        .await
        .expect("count revisions A");
    let revs_b: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM revisions WHERE tenant_id = $1")
        .bind(tenant_b)
        .fetch_one(&pool)
        .await
        .expect("count revisions B");
    assert_eq!(revs_a, 1);
    assert_eq!(revs_b, 1);

    let holds_a: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds WHERE tenant_id = $1 AND released_at IS NULL",
    )
    .bind(tenant_a)
    .fetch_one(&pool)
    .await
    .expect("count holds A");
    let holds_b: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM legal_holds WHERE tenant_id = $1 AND released_at IS NULL",
    )
    .bind(tenant_b)
    .fetch_one(&pool)
    .await
    .expect("count holds B");
    assert_eq!(holds_a, 1);
    assert_eq!(holds_b, 1);

    let dists_a: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM document_distributions WHERE tenant_id = $1")
            .bind(tenant_a)
            .fetch_one(&pool)
            .await
            .expect("count distributions A");
    let dists_b: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM document_distributions WHERE tenant_id = $1")
            .bind(tenant_b)
            .fetch_one(&pool)
            .await
            .expect("count distributions B");
    assert_eq!(dists_a, 1);
    assert_eq!(dists_b, 1);

    let policies_a: i32 = sqlx::query_scalar(
        "SELECT retention_days FROM retention_policies WHERE tenant_id = $1 AND doc_type = 'spec'",
    )
    .bind(tenant_a)
    .fetch_one(&pool)
    .await
    .expect("retention A");
    let policies_b: i32 = sqlx::query_scalar(
        "SELECT retention_days FROM retention_policies WHERE tenant_id = $1 AND doc_type = 'spec'",
    )
    .bind(tenant_b)
    .fetch_one(&pool)
    .await
    .expect("retention B");
    assert_eq!(policies_a, 365);
    assert_eq!(policies_b, 730);

    let cross_tenant_doc_lookup: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM documents WHERE tenant_id = $1 AND id = $2")
            .bind(tenant_b)
            .bind(doc_a)
            .fetch_one(&pool)
            .await
            .expect("cross-tenant lookup");
    assert_eq!(cross_tenant_doc_lookup, 0, "tenant B cannot see tenant A doc");
}
