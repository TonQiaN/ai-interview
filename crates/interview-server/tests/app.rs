//! 整个应用装起来之后的行为：健康检查、接口文档、默认要登录、前端页面托管。

use std::sync::Arc;

use axum::http::StatusCode;
use interview_platform::AppContext;
use interview_platform::blob::MemoryBlobStore;
use interview_platform::config::PlatformConfig;
use interview_platform::db::MIGRATOR;
use interview_platform::testing::{self, TestRequest};
use interview_server::Shared;
use sqlx::PgPool;

fn context(pool: PgPool, web_dir: &std::path::Path) -> AppContext {
    let config = PlatformConfig {
        web_dir: web_dir.to_owned(),
        ..PlatformConfig::for_tests()
    };
    AppContext::new(pool, Arc::new(MemoryBlobStore::new()), config)
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn health_docs_and_default_deny(pool: PgPool) {
    let web = tempfile::tempdir().unwrap();
    let ctx = context(pool, web.path());
    let app = interview_server::build(&ctx, &Shared::none());
    let router = app.router;

    let response = TestRequest::get("/healthz").send(&router).await;
    assert_eq!(
        (response.status, response.text().as_str()),
        (StatusCode::OK, "ok")
    );
    let response = TestRequest::get("/readyz").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(response.header("referrer-policy"), Some("no-referrer"));

    // 接口文档不用登录
    let response = TestRequest::get("/api/openapi.json").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    let spec = response.json();
    assert_eq!(spec["info"]["title"], "AI 面试");
    for path in [
        "/api/v1/auth/login",
        "/api/v1/auth/me",
        "/api/v1/settings/{key}",
        "/api/v1/system/status",
    ] {
        assert!(spec["paths"].get(path).is_some(), "接口文档里缺 {path}");
    }
    assert!(spec["components"]["schemas"].get("ErrorBody").is_some());
    assert!(
        spec["components"]["securitySchemes"]
            .get("admin_session")
            .is_some()
    );
    let response = TestRequest::get("/api/docs").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);

    // /api/v1 下的路径默认要登录，哪怕这个接口不存在
    let response = TestRequest::get("/api/v1/whatever").send(&router).await;
    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_eq!(response.json()["error"]["code"], "unauthenticated");

    // 登录后，不存在的接口返回统一结构的 404
    let admin = testing::create_admin(&ctx).await;
    let authed = testing::as_admin(router.clone(), admin);
    let response = TestRequest::get("/api/v1/whatever").send(&authed).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(response.json()["error"]["code"], "not_found");

    // 系统状态里有数据库、大模型、语音合成、语音识别和 ffmpeg
    let response = TestRequest::get("/api/v1/system/status")
        .send(&authed)
        .await;
    let names: Vec<String> = response.json()["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            format!(
                "{}={}",
                c["name"].as_str().unwrap(),
                c["state"].as_str().unwrap()
            )
        })
        .collect();
    // ffmpeg 装没装看机器，其余的在测试里是固定的
    let (ffmpeg, others): (Vec<String>, Vec<String>) = names
        .into_iter()
        .partition(|name| name.starts_with("ffmpeg="));
    assert_eq!(
        others,
        [
            "database=ok",
            "llm=not_configured",
            "tts=not_configured",
            "stt=not_configured"
        ]
    );
    assert!(
        ffmpeg == ["ffmpeg=ok"] || ffmpeg == ["ffmpeg=down"],
        "{ffmpeg:?}"
    );

    // 首页跳到 HR 后台
    let response = TestRequest::get("/").send(&router).await;
    assert_eq!(response.status, StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.header("location"), Some("/admin/"));

    let response = TestRequest::get("/nothing-here").send(&router).await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn serves_the_built_frontends(pool: PgPool) {
    let web = tempfile::tempdir().unwrap();
    for (app, marker) in [("admin", "HR 后台页面"), ("candidate", "候选人页面")] {
        let dir = web.path().join(app);
        std::fs::create_dir_all(dir.join("assets")).unwrap();
        std::fs::write(
            dir.join("index.html"),
            format!("<!doctype html><title>{marker}</title>"),
        )
        .unwrap();
        std::fs::write(dir.join("assets/app-abc123.js"), "console.log(1)").unwrap();
    }
    let ctx = context(pool, web.path());
    let router = interview_server::build(&ctx, &Shared::none()).router;

    let response = TestRequest::get("/admin/").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.text().contains("HR 后台页面"));
    assert_eq!(response.header("cache-control"), Some("no-cache"));

    // 前端自己的路由：没有对应文件，给 index.html
    let response = TestRequest::get("/admin/interviews/123")
        .send(&router)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.text().contains("HR 后台页面"));

    let response = TestRequest::get("/admin/assets/app-abc123.js")
        .send(&router)
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.text(), "console.log(1)");
    assert_eq!(
        response.header("cache-control"),
        Some("public, max-age=31536000, immutable")
    );

    // 候选人页面不用登录
    let response = TestRequest::get("/i/").send(&router).await;
    assert_eq!(response.status, StatusCode::OK);
    assert!(response.text().contains("候选人页面"));

    // 不能借静态文件服务读到目录外面的东西
    let response = TestRequest::get("/admin/../candidate/index.html")
        .send(&router)
        .await;
    assert!(!response.text().contains("候选人页面") || response.status != StatusCode::OK);
}

#[sqlx::test(migrator = "MIGRATOR")]
async fn explains_when_the_frontend_is_not_built(pool: PgPool) {
    let web = tempfile::tempdir().unwrap();
    let ctx = context(pool, web.path());
    let router = interview_server::build(&ctx, &Shared::none()).router;
    let response = TestRequest::get("/admin/").send(&router).await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(response.text().contains("pnpm build"));
}

#[tokio::test]
async fn openapi_can_be_generated_without_a_database() {
    let json = interview_server::serve::openapi_json().unwrap();
    let spec: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert!(spec["paths"].get("/api/v1/auth/login").is_some());
}

/// 前端的类型是照接口文档生成的。文档里枚举的取值必须是 JSON 里实际的写法（小写加下划线），
/// 不能是 Rust 的变体名。
#[tokio::test]
async fn openapi_enum_values_are_the_json_spelling() {
    let json = interview_server::serve::openapi_json().unwrap();
    let spec: serde_json::Value = serde_json::from_str(&json).unwrap();
    let schemas = spec["components"]["schemas"].as_object().unwrap();
    let mut checked = 0;
    for (name, schema) in schemas {
        let Some(values) = schema["enum"].as_array() else {
            continue;
        };
        for value in values.iter().filter_map(|v| v.as_str()) {
            assert!(
                value
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name} 在接口文档里的取值「{value}」不是 JSON 里的写法"
            );
            checked += 1;
        }
    }
    assert!(
        checked > 20,
        "接口文档里应该有不少枚举，只看到 {checked} 个取值"
    );
}

/// 前端生成类型时用 operationId 当函数名，重名会让 `pnpm gen:api` 失败。operationId 默认是
/// handler 的函数名，两个模块各有一个 `list` 这样的函数就会撞上。在这里拦住，不用等到前端构建。
#[tokio::test]
async fn openapi_operation_ids_are_unique() {
    let json = interview_server::serve::openapi_json().unwrap();
    let spec: serde_json::Value = serde_json::from_str(&json).unwrap();
    let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (path, item) in spec["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            let Some(id) = operation.get("operationId").and_then(|v| v.as_str()) else {
                continue;
            };
            let here = format!("{} {path}", method.to_uppercase());
            if let Some(other) = seen.insert(id.to_owned(), here.clone()) {
                panic!(
                    "operationId「{id}」重复了：{other} 和 {here}。给其中一个 handler 换个函数名"
                );
            }
        }
    }
    assert!(
        seen.len() > 30,
        "接口文档里应该有不少接口，只看到 {} 个",
        seen.len()
    );
}

/// 接口文档里一个类型的某个字段指向哪个类型，返回那个类型的定义。
fn field_type<'a>(spec: &'a serde_json::Value, owner: &str, field: &str) -> &'a serde_json::Value {
    let schemas = &spec["components"]["schemas"];
    let property = &schemas[owner]["properties"][field];
    let reference = property["$ref"]
        .as_str()
        .or_else(|| {
            property["oneOf"]
                .as_array()?
                .iter()
                .find_map(|option| option["$ref"].as_str())
        })
        .unwrap_or_else(|| panic!("{owner}.{field} 在接口文档里没有指向一个类型：{property}"));
    let name = reference.rsplit('/').next().unwrap();
    &schemas[name]
}

/// 几个模块里有同名的类型（面试语言和合成语言都叫 `Language`）。接口文档里类型按名字放，
/// 重名的只留下一个，另一个的字段就指到了别人的定义上，前端生成的类型跟着错。
#[tokio::test]
async fn openapi_keeps_same_named_types_apart() {
    let json = interview_server::serve::openapi_json().unwrap();
    let spec: serde_json::Value = serde_json::from_str(&json).unwrap();
    let values = |schema: &serde_json::Value| -> Vec<String> {
        schema["enum"]
            .as_array()
            .unwrap_or_else(|| panic!("不是枚举：{schema}"))
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };

    // 面试语言只有两种，合成语言有五种
    assert_eq!(
        values(field_type(&spec, "InterviewConfig", "language")),
        ["zh", "en"]
    );
    assert_eq!(
        values(field_type(&spec, "VoiceSettings", "language")),
        ["zh", "en", "ja", "es", "ar"]
    );
    // 对话记录里只有面试官和候选人；报告的证据里还可能是录到的第三个人
    assert_eq!(
        values(field_type(&spec, "TurnView", "speaker")),
        ["interviewer", "candidate"]
    );
    assert_eq!(
        values(field_type(&spec, "Evidence", "speaker")),
        ["interviewer", "candidate", "other"]
    );
    // 报告的用量带调用次数，单次调用的用量没有
    assert!(
        field_type(&spec, "ReportMeta", "usage")["properties"]
            .get("calls")
            .is_some()
    );
    assert!(
        field_type(&spec, "GenerationInfo", "usage")["properties"]
            .get("calls")
            .is_none()
    );
}

/// 上面那个测试只管已经知道的几对。这个测试翻源码，把所有会进接口文档的类型（派生了 `ToSchema`
/// 的，和 `text_enum!` 定义的）按文档里的名字归拢，不同模块里重名的就报出来。
///
/// 重名了怎么办：给其中一个加 `#[schema(as = 新名字)]`，它在文档里就叫新名字。
#[test]
fn schema_names_are_unique_across_crates() {
    // 这几个名字有两处定义，但不碍事：要么只有一处进了接口文档（另一处是模块内部用的，比如报告的输入、
    // 引擎给面试间的类型），要么两处的定义一模一样。后面的数字是现在有几处；多出一处来就要重新看。
    // 哪天另一处也要进文档而且长得不一样，把它从这里去掉，再给它加 `#[schema(as = ...)]`。
    const KNOWN_HARMLESS: &[(&str, usize)] = &[
        ("Candidate", 2),
        ("InterviewConfig", 2),
        ("Position", 2),
        ("Progress", 2),
        ("Speaker", 2),
        ("Utterance", 2),
        ("UtteranceKind", 2),
    ];

    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned();
    let mut files = Vec::new();
    let mut pending = vec![crates.clone()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                // 只看各个 crate 的 src
                if name != "target" && name != "tests" && name != "examples" {
                    pending.push(path);
                }
            } else if name.ends_with(".rs") && path.components().any(|c| c.as_os_str() == "src") {
                files.push(path);
            }
        }
    }

    let mut seen: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for file in &files {
        let source = std::fs::read_to_string(file).unwrap();
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let declaration = line.trim_start();
            let Some(rest) = declaration
                .strip_prefix("pub struct ")
                .or_else(|| declaration.strip_prefix("pub enum "))
            else {
                continue;
            };
            let type_name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if type_name.is_empty() {
                // 宏定义里的 `pub enum $name`
                continue;
            }
            // 这个类型前面紧挨着的属性和注释（往上到空行、上一个定义结束的地方为止）
            let mut attributes = String::new();
            for above in lines[..index].iter().rev() {
                let above = above.trim();
                if above.is_empty() || above.ends_with('}') || above.ends_with(';') {
                    break;
                }
                attributes.push_str(above);
                attributes.push('\n');
            }
            if !attributes.contains("ToSchema") && !attributes.contains("text_enum!") {
                continue;
            }
            let published = attributes
                .split("schema(as = ")
                .nth(1)
                .map(|renamed| {
                    renamed
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect()
                })
                .unwrap_or(type_name);
            let place = file.strip_prefix(&crates).unwrap().display().to_string();
            seen.entry(published).or_default().push(place);
        }
    }
    assert!(
        seen.len() > 100,
        "应该能找到一百多个进接口文档的类型，只找到 {} 个，是不是找的办法失效了",
        seen.len()
    );
    // 找的办法要认得 `text_enum!` 里定义的类型
    assert!(seen.contains_key("InterviewStatus"));

    let clashes: Vec<String> = seen
        .iter()
        .filter(|(name, places)| {
            places.len() > 1 && !KNOWN_HARMLESS.contains(&(name.as_str(), places.len()))
        })
        .map(|(name, places)| format!("{name}：{}", places.join("、")))
        .collect();
    assert!(
        clashes.is_empty(),
        "这些类型在接口文档里重名了，只会留下一个。给其中一个加 #[schema(as = 新名字)]：\n{}",
        clashes.join("\n")
    );
}
