//! 同一个岗位几位候选人的横向对比。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use axum::Router;
use axum::http::StatusCode;
use interview_llm::{ChatModel, ChatRequest, ChatStream, DeepSeekClient, LlmError};
use interview_platform::testing::{TestRequest, TestResponse};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{ANSWER_1, ANSWER_2, ReportModel, harness, stt};
use crate::{Harness, config, error_code};

const COMPARISON: &str = r#"{
  "summary": "两位都可以约，先约 A。",
  "order": ["A", "B"],
  "candidates": [
    {"label": "A", "standing": "排在前面，项目讲得具体。", "advantages": ["讲出了自己负责的部分"], "concerns": []},
    {"label": "B", "standing": "和 A 接近，细节少一些。", "advantages": [], "concerns": ["要再核实项目里的分工"]}
  ],
  "requirements": [],
  "differences": [{"topic": "项目经验", "labels": ["A", "B"], "detail": "A 讲得更具体。"}],
  "tie_breakers": [{"labels": ["A", "B"], "question": "请讲一次你自己排查线上故障的经过。", "purpose": "看谁真的做过"}],
  "caveats": []
}"#;

fn model() -> Arc<ReportModel> {
    Arc::new(ReportModel::complete().on("comparison", COMPARISON))
}

/// 一场有报告的面试，加上同一个岗位另外几场拿它的报告改了面试 ID 的面试。
/// 返回的第一个是真的走过一遍的那场。
async fn interviews_with_reports(h: &Harness, extra: usize) -> Vec<String> {
    let first = h.finished_interview("completed").await;
    let h = h.with_review(Some(model()), Some(stt(&[ANSWER_1, ANSWER_2])));
    assert_eq!(h.run_tasks().await, 2);
    let content: Value = sqlx::query_scalar(
        "SELECT content FROM interview_reports WHERE interview_id = $1::uuid AND status = 'done'",
    )
    .bind(&first)
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();

    let mut ids = vec![first];
    for _ in 0..extra {
        let id = h.create(config()).await;
        h.set_status(&id, "completed").await;
        let mut copy = content.clone();
        copy["interview_id"] = json!(id);
        sqlx::query(
            "INSERT INTO interview_reports (interview_id, version, status, content, generated_at) \
             VALUES ($1::uuid, 1, 'done', $2, now())",
        )
        .bind(&id)
        .bind(copy)
        .execute(&h.ctx.db)
        .await
        .unwrap();
        ids.push(id);
    }
    ids
}

impl Harness {
    fn comparison_uri(&self) -> String {
        format!("/api/v1/positions/{}/comparison", self.position_id)
    }

    async fn compare(&self, interview_ids: &[&str]) -> TestResponse {
        TestRequest::post(self.comparison_uri())
            .json(&json!({ "interview_ids": interview_ids }))
            .send(&self.app)
            .await
    }

    async fn comparison(&self) -> Value {
        let response = self.get(&self.comparison_uri()).await;
        assert_eq!(response.status, StatusCode::OK, "{}", response.text());
        response.json()
    }

    async fn comparison_rows(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM position_comparisons WHERE position_id = $1")
            .bind(self.position_id)
            .fetch_one(&self.ctx.db)
            .await
            .unwrap()
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn hr_compares_candidates_of_a_position(pool: PgPool) {
    let h = harness(pool).await;
    let ids = interviews_with_reports(&h, 2).await;
    let (a, b, c) = (ids[0].as_str(), ids[1].as_str(), ids[2].as_str());
    let model = model();
    let h = h.with_review(Some(model.clone()), None);

    // 还没比过
    let response = h.get(&h.comparison_uri()).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(error_code(&response), "comparison_not_found");

    let response = h.compare(&[b, a]).await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    let started = response.json();
    assert_eq!(started["status"], "pending");
    assert_eq!(started["interview_ids"], json!([b, a]));
    assert_eq!(started["result"], Value::Null);

    // 同样的几场再点一次不会多排一份，换一批人要等这一份做完
    let response = h.compare(&[a, b]).await;
    assert_eq!(response.status, StatusCode::ACCEPTED);
    assert_eq!(h.comparison_rows().await, 1);
    let response = h.compare(&[a, c]).await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(error_code(&response), "comparison_in_progress");

    assert_eq!(h.run_tasks().await, 1);
    assert_eq!(model.calls("comparison"), 1);

    let done = h.comparison().await;
    assert_eq!(done["status"], "done", "{done}");
    assert_eq!(done["error"], Value::Null);
    let result = &done["result"];
    assert!(result["generated_at"].is_string());
    // 姓名按比的时候的顺序给，和代号 A、B 对得上
    let people = result["people"].as_array().unwrap();
    assert_eq!(people.len(), 2);
    assert_eq!(people[0]["interview_id"], b);
    assert_eq!(people[1]["interview_id"], a);
    assert_eq!(people[0]["name"], "李明");
    assert_eq!(people[0]["candidate_id"], json!(h.candidate_id));
    let comparison = &result["comparison"];
    assert_eq!(comparison["summary"], "两位都可以约，先约 A。");
    let candidates = comparison["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0]["label"], "A");
    assert_eq!(candidates[0]["interview_id"], b);
    assert_eq!(candidates[1]["interview_id"], a);
    assert_eq!(comparison["tie_breakers"].as_array().unwrap().len(), 1);

    // 花的钱记了账
    let (calls, with_task): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(task_id) FROM llm_usage WHERE purpose = 'position.compare'",
    )
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    assert_eq!((calls, with_task), (1, 1));

    // 再比一次换一批人：新的好了以后只留新的
    let response = h.compare(&[a, c]).await;
    assert_eq!(response.status, StatusCode::ACCEPTED);
    let again = response.json();
    assert_eq!(again["status"], "pending");
    assert_eq!(
        again["result"]["people"][0]["interview_id"], b,
        "新的还没好，先看上一份"
    );
    assert_eq!(h.run_tasks().await, 1);
    let done = h.comparison().await;
    assert_eq!(done["status"], "done");
    assert_eq!(done["result"]["people"][1]["interview_id"], c);
    assert_eq!(h.comparison_rows().await, 1);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn rejects_selections_that_cannot_be_compared(pool: PgPool) {
    let h = harness(pool).await;
    let ids = interviews_with_reports(&h, 1).await;
    let (a, b) = (ids[0].as_str(), ids[1].as_str());
    let no_report = h.create(config()).await;
    let unknown = Uuid::now_v7().to_string();

    let cases: [(&[&str], &str); 4] = [
        (&[a], "至少选两场"),
        (&[a, a], "选了两次"),
        (&[a, &no_report], "还没有写好的报告"),
        (&[a, &unknown], "不是这个岗位的"),
    ];
    for (selection, expected) in cases {
        let response = h.compare(selection).await;
        assert_eq!(
            response.status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "{selection:?}"
        );
        let body = response.json();
        let message = body["error"]["details"]["interview_ids"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(message.contains(expected), "{body}");
    }
    let too_many: Vec<String> = (0..21).map(|_| Uuid::now_v7().to_string()).collect();
    let too_many: Vec<&str> = too_many.iter().map(String::as_str).collect();
    assert_eq!(
        h.compare(&too_many).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // 没有这个岗位
    let response = TestRequest::post(format!("/api/v1/positions/{}/comparison", Uuid::now_v7()))
        .json(&json!({ "interview_ids": [a, b] }))
        .send(&h.app)
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);

    // 没配大模型
    let offline = h.with_review(None, None);
    let response = offline.compare(&[a, b]).await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.text());
    assert_eq!(h.comparison_rows().await, 0);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_failed_comparison_keeps_the_last_good_one(pool: PgPool) {
    let h = harness(pool).await;
    let ids = interviews_with_reports(&h, 1).await;
    let (a, b) = (ids[0].as_str(), ids[1].as_str());
    let h = h.with_review(Some(model()), None);
    assert_eq!(h.compare(&[a, b]).await.status, StatusCode::ACCEPTED);
    assert_eq!(h.run_tasks().await, 1);
    assert_eq!(h.comparison().await["status"], "done");

    // 模型坏了：重试三次以后标成失败，上一份还能看
    let broken = h.with_review(
        Some(Arc::new(ReportModel::complete().failing("comparison"))),
        None,
    );
    assert_eq!(broken.compare(&[a, b]).await.status, StatusCode::ACCEPTED);
    assert_eq!(broken.run_tasks().await, 1);
    assert_eq!(broken.comparison().await["status"], "running");
    for _ in 0..2 {
        broken.skip_retry_delay().await;
        assert_eq!(broken.run_tasks().await, 1);
    }
    let failed = broken.comparison().await;
    assert_eq!(failed["status"], "failed", "{failed}");
    assert!(failed["error"].as_str().unwrap().contains("重试了 3 次"));
    assert_eq!(
        failed["result"]["comparison"]["candidates"][0]["label"],
        "A"
    );

    // 又失败一次：只留最近这次失败和上一份好的
    assert_eq!(broken.compare(&[a, b]).await.status, StatusCode::ACCEPTED);
    for _ in 0..3 {
        broken.skip_retry_delay().await;
        assert_eq!(broken.run_tasks().await, 1);
    }
    assert_eq!(broken.comparison().await["status"], "failed");
    assert_eq!(broken.comparison_rows().await, 2);

    // 选的面试里有一场的报告没了：直接失败，不重试
    sqlx::query("DELETE FROM interview_reports WHERE interview_id = $1::uuid")
        .bind(b)
        .execute(&h.ctx.db)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO position_comparisons (position_id, interview_ids) VALUES ($1, $2::uuid[])",
    )
    .bind(h.position_id)
    .bind(vec![a.to_owned(), b.to_owned()])
    .execute(&h.ctx.db)
    .await
    .unwrap();
    let orphan: Uuid = sqlx::query_scalar(
        "SELECT id FROM position_comparisons WHERE status = 'pending' AND position_id = $1",
    )
    .bind(h.position_id)
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    sqlx::query("INSERT INTO tasks (kind, payload) VALUES ('position.compare', $1)")
        .bind(json!({ "comparison_id": orphan }))
        .execute(&h.ctx.db)
        .await
        .unwrap();
    assert_eq!(h.run_tasks().await, 1);
    let failed = h.comparison().await;
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["error"].as_str().unwrap().contains("没有写好的报告"),
        "{failed}"
    );
}

/// 模型第一次被调用的时候，HR 正好在后台点了几下。用来测「对比生成到一半，别处动了数据」。
struct BusyHr {
    inner: Arc<ReportModel>,
    app: Router,
    requests: Vec<(String, Value)>,
    done: AtomicBool,
}

#[async_trait]
impl ChatModel for BusyHr {
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        if !self.done.swap(true, Ordering::SeqCst) {
            for (uri, body) in &self.requests {
                let response = TestRequest::post(uri.as_str())
                    .json(body)
                    .send(&self.app)
                    .await;
                assert!(response.status.is_success(), "{uri}：{}", response.text());
            }
        }
        self.inner.chat_stream(request).await
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn a_comparison_deleted_halfway_leaves_the_others_alone(pool: PgPool) {
    let h = harness(pool).await;
    let ids = interviews_with_reports(&h, 2).await;
    let (a, b, c) = (ids[0].as_str(), ids[1].as_str(), ids[2].as_str());
    let h = h.with_review(Some(model()), None);
    assert_eq!(h.compare(&[a, b]).await.status, StatusCode::ACCEPTED);
    assert_eq!(h.run_tasks().await, 1);
    let first = h.comparison().await;
    assert_eq!(first["status"], "done");

    // 比 b 和 c 的时候，模型还没答完，HR 把 c 那场面试彻底删了（这份对比跟着删掉），
    // 接着又点了一次 a 和 b 的对比
    let busy = h.with_review(
        Some(Arc::new(BusyHr {
            inner: model(),
            app: h.app.clone(),
            requests: vec![
                (format!("/api/v1/interviews/{c}/purge"), json!({})),
                (h.comparison_uri(), json!({ "interview_ids": [b, a] })),
            ],
            done: AtomicBool::new(false),
        })),
        None,
    );
    assert_eq!(busy.compare(&[b, c]).await.status, StatusCode::ACCEPTED);
    // 两个任务：被删掉的那一份，和后来点的那一份
    assert_eq!(busy.run_tasks().await, 2);

    // 被删掉的那一份做完以后什么都不该动：后来点的那一份照常生成
    let latest = busy.comparison().await;
    assert_eq!(latest["status"], "done", "{latest}");
    assert_eq!(latest["interview_ids"], json!([b, a]));
    assert_eq!(latest["result"]["people"].as_array().unwrap().len(), 2);
    assert_eq!(busy.comparison_rows().await, 1);
    // 被删掉的那一份不留结果
    let with_c: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM position_comparisons WHERE $1::uuid = ANY(interview_ids)",
    )
    .bind(c)
    .fetch_one(&busy.ctx.db)
    .await
    .unwrap();
    assert_eq!(with_c, 0);
}

/// 用真的大模型走一遍接口：报告线的三份样例报告（合成的，不是真人）挂到同一个岗位的三场面试上，
/// 点对比，等后台任务跑完，把结果打印出来看。
///
/// `cargo test -p interview-flow --test flow live_comparison -- --ignored --nocapture`
#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
#[ignore = "调用真实的 DeepSeek，约一分钟、不到一毛钱"]
async fn live_comparison_of_sample_reports(pool: PgPool) {
    let h = harness(pool).await;
    let samples = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../interview-report/samples/reports"
    );
    let mut ids = Vec::new();
    for name in [
        "backend-third-party",
        "backend-strong",
        "backend-fabricated",
    ] {
        let text = std::fs::read_to_string(format!("{samples}/{name}.json")).unwrap();
        let mut content: Value = serde_json::from_str(&text).unwrap();
        let id = h.create(config()).await;
        h.set_status(&id, "completed").await;
        content["interview_id"] = json!(id);
        sqlx::query(
            "INSERT INTO interview_reports (interview_id, version, status, content, generated_at) \
             VALUES ($1::uuid, 1, 'done', $2, now())",
        )
        .bind(&id)
        .bind(content)
        .execute(&h.ctx.db)
        .await
        .unwrap();
        println!("{name} 是 {id}");
        ids.push(id);
    }
    let model: Arc<dyn ChatModel> =
        Arc::new(DeepSeekClient::from_env().expect("需要设置 DEEPSEEK_API_KEY"));
    let h = h.with_review(Some(model), None);

    let selection: Vec<&str> = ids.iter().map(String::as_str).collect();
    let response = h.compare(&selection).await;
    assert_eq!(response.status, StatusCode::ACCEPTED, "{}", response.text());
    assert_eq!(h.run_tasks().await, 1);

    let done = h.comparison().await;
    assert_eq!(done["status"], "done", "{done}");
    let comparison = &done["result"]["comparison"];
    println!("\n{}\n", comparison["summary"].as_str().unwrap());
    let candidates = comparison["candidates"].as_array().unwrap();
    assert_eq!(candidates.len(), 3);
    for candidate in candidates {
        println!(
            "第 {} 位 {}（{}，{} 分）{}：{}",
            candidate["position"],
            candidate["label"].as_str().unwrap(),
            candidate["decision"].as_str().unwrap(),
            candidate["overall_score"],
            candidate["interview_id"].as_str().unwrap(),
            candidate["standing"].as_str().unwrap(),
        );
    }
    println!(
        "\n对照表 {} 行，差别 {} 处，区分的问题 {} 个，注意事项 {} 条",
        comparison["requirements"].as_array().unwrap().len(),
        comparison["differences"].as_array().unwrap().len(),
        comparison["tie_breakers"].as_array().unwrap().len(),
        comparison["caveats"].as_array().unwrap().len(),
    );
    let (cost, latency): (f64, i32) = sqlx::query_as(
        "SELECT cost_cny::float8, latency_ms FROM llm_usage WHERE purpose = 'position.compare'",
    )
    .fetch_one(&h.ctx.db)
    .await
    .unwrap();
    println!("花了 {cost:.3} 元，{:.0} 秒", f64::from(latency) / 1000.0);
}
