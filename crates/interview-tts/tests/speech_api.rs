//! 语音接口的集成测试：真数据库，假的合成后端，文件存在内存里。
//!
//! 要先起开发数据库：`DATABASE_URL=$(scripts/dev-db.sh url) cargo test -p interview-tts`。
//! 参考录音用的是现场生成的测试音，不是真人的声音。

use std::process::Command;
use std::sync::Arc;

use axum::Router;
use axum::http::StatusCode;
use interview_audio::{Audio, Ffmpeg, decode_wav, encode_wav};
use interview_platform::AppContext;
use interview_platform::testing::{self, TestRequest, TestResponse};
use interview_tts::http::{BlobAudioCache, Deps};
use interview_tts::{EngineId, FakeBackend, Synthesizer};
use serde_json::{Value, json};
use sqlx::PgPool;

const OCTET_STREAM: &str = "application/octet-stream";

struct Harness {
    ctx: AppContext,
    app: Router,
    tts: Arc<FakeBackend>,
}

impl Harness {
    async fn new(pool: PgPool) -> Self {
        Self::with_backend(pool, FakeBackend::new()).await
    }

    async fn with_backend(pool: PgPool, backend: FakeBackend) -> Self {
        let ctx = testing::test_context(pool);
        let admin = testing::create_admin(&ctx).await;
        let tts = Arc::new(backend);
        // 和服务进程里一样：合成结果缓存进 BlobStore
        let synth = Arc::new(
            Synthesizer::new(tts.clone())
                .with_cache(Arc::new(BlobAudioCache::new(ctx.blobs.clone()))),
        );
        let deps = Deps {
            ctx: ctx.clone(),
            synth,
            ffmpeg: Ffmpeg::from_env(),
            default_engine: EngineId::DEFAULT,
        };
        let (router, _api) = interview_tts::http::router(deps).split_for_parts();
        Self {
            ctx,
            app: testing::as_admin(router, admin),
            tts,
        }
    }

    async fn get(&self, uri: &str) -> TestResponse {
        TestRequest::get(uri).send(&self.app).await
    }

    async fn post(&self, uri: &str, body: Value) -> TestResponse {
        TestRequest::post(uri).json(&body).send(&self.app).await
    }

    async fn patch(&self, uri: &str, body: Value) -> TestResponse {
        TestRequest::patch(uri).json(&body).send(&self.app).await
    }

    async fn delete(&self, uri: &str) -> TestResponse {
        TestRequest::delete(uri).send(&self.app).await
    }

    /// 建一个音色，返回它的 JSON。
    async fn create(&self, body: Value) -> Value {
        let response = self.post("/api/v1/speech/voices", body).await;
        assert_eq!(response.status, StatusCode::CREATED, "{}", response.text());
        response.json()
    }

    async fn designed(&self, name: &str) -> Value {
        self.create(json!({
            "name": name,
            "engine": "breeze2",
            "kind": "designed",
            "description": "三十岁左右的女声，沉稳、亲切",
        }))
        .await
    }

    async fn cloned(&self, name: &str, engine: &str) -> Value {
        self.create(json!({ "name": name, "engine": engine, "kind": "cloned" }))
            .await
    }

    async fn upload(&self, id: &str, file: Vec<u8>) -> TestResponse {
        TestRequest::put(format!("/api/v1/speech/voices/{id}/reference"))
            .bytes(OCTET_STREAM, file)
            .send(&self.app)
            .await
    }

    async fn preview(&self, body: Value) -> TestResponse {
        self.post("/api/v1/speech/preview", body).await
    }
}

fn id_of(voice: &Value) -> &str {
    voice["id"].as_str().expect("音色没有 id")
}

fn code_of(response: &TestResponse) -> String {
    response.json()["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

/// 一段测试音的 WAV。
fn tone_wav(duration_ms: u64) -> Vec<u8> {
    encode_wav(&Audio::tone(24_000, 220.0, duration_ms, 0.3).unwrap())
}

/// 装了 ffmpeg 才有。CI 里没装直接失败，不让测试悄悄跳过。
async fn ffmpeg() -> Option<Ffmpeg> {
    let ffmpeg = Ffmpeg::from_env();
    match ffmpeg.version().await {
        Ok(_) => Some(ffmpeg),
        Err(error) if std::env::var_os("CI").is_some() => panic!("CI 里应该装了 ffmpeg：{error}"),
        Err(error) => {
            eprintln!("跳过要 ffmpeg 的部分：{error}");
            None
        }
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn engines_list_says_what_each_engine_can_do(pool: PgPool) {
    let h =
        Harness::with_backend(pool, FakeBackend::new().with_unavailable(EngineId::Index25)).await;
    let response = h.get("/api/v1/speech/engines").await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let body = response.json();
    assert_eq!(body["backend"], "fake");
    assert_eq!(body["simulated"], true);

    let items = body["items"].as_array().unwrap();
    let ids: Vec<&str> = items.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["index2", "index25", "breeze2"]);

    let index2 = &items[0];
    assert_eq!(index2["name"], "IndexTTS 2");
    assert_eq!(index2["is_default"], true);
    assert_eq!(index2["requires_reference_audio"], true);
    assert_eq!(index2["supports_voice_design"], false);
    assert_eq!(index2["supports_emotion"], true);
    assert_eq!(index2["available"], true);
    assert_eq!(index2["unavailable_reason"], Value::Null);

    let index25 = &items[1];
    assert_eq!(index25["available"], false);
    assert!(index25["unavailable_reason"].is_string());
    assert_eq!(index25["is_default"], false);

    let breeze = &items[2];
    assert_eq!(breeze["supports_voice_design"], true);
    assert_eq!(breeze["requires_reference_transcript"], true);
    assert_eq!(breeze["requires_reference_audio"], false);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn designed_voice_only_on_the_engine_that_supports_it(pool: PgPool) {
    let h = Harness::new(pool).await;

    let voice = h.designed("  亲切女声  ").await;
    assert_eq!(voice["name"], "亲切女声");
    assert_eq!(voice["kind"], "designed");
    assert_eq!(voice["engine"], "breeze2");
    assert_eq!(voice["description"], "三十岁左右的女声，沉稳、亲切");
    assert_eq!(voice["ready"], true);
    assert_eq!(voice["not_ready_reason"], Value::Null);
    assert_eq!(voice["has_reference"], false);
    assert_eq!(voice["is_default"], false);
    // 没填设置时用默认的
    assert_eq!(voice["settings"]["language"], "zh");
    assert_eq!(voice["settings"]["options"]["seed"], 42);

    // IndexTTS 只能克隆
    let response = h
        .post(
            "/api/v1/speech/voices",
            json!({ "name": "x", "engine": "index2", "kind": "designed", "description": "男声" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    // 设计音色要有描述
    let response = h
        .post(
            "/api/v1/speech/voices",
            json!({ "name": "x", "engine": "breeze2", "kind": "designed", "description": "  " }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    // 名字不能是空的；情绪强度要在范围里
    let response = h
        .post(
            "/api/v1/speech/voices",
            json!({ "name": " ", "engine": "index2", "kind": "cloned" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let response = h
        .post(
            "/api/v1/speech/voices",
            json!({
                "name": "x", "engine": "index2", "kind": "cloned",
                "settings": { "style": { "emotion": "calm", "emotion_strength": 1.5 } },
            }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    // 换到不支持设计的引擎也不行
    let response = h
        .patch(
            &format!("/api/v1/speech/voices/{}", id_of(&voice)),
            json!({ "engine": "index25" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    // 设计音色不收参考录音
    let response = h.upload(id_of(&voice), tone_wav(3_000)).await;
    assert_eq!(response.status, StatusCode::CONFLICT, "{}", response.text());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn cloned_voice_is_ready_once_the_reference_is_uploaded(pool: PgPool) {
    let h = Harness::new(pool).await;
    let voice = h.cloned("面试官", "index2").await;
    let id = id_of(&voice).to_owned();
    assert_eq!(voice["kind"], "cloned");
    assert_eq!(voice["ready"], false);
    assert_eq!(voice["not_ready_reason"], "还没上传参考录音");
    assert_eq!(voice["description"], Value::Null);

    // 还没传录音：听不了，也不能设成默认
    let response = h
        .get(&format!("/api/v1/speech/voices/{id}/reference"))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    let response = h
        .patch(
            &format!("/api/v1/speech/voices/{id}"),
            json!({ "is_default": true }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "voice_not_ready");

    // 传一段 44.1 kHz 双声道的录音：存下来的是单声道
    let stereo = {
        let mono = Audio::tone(44_100, 220.0, 3_000, 0.3).unwrap();
        let spec = hound_spec();
        let mut bytes = std::io::Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut bytes, spec).unwrap();
        for sample in mono.samples() {
            let value = (sample * f32::from(i16::MAX)) as i16;
            writer.write_sample(value).unwrap();
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
        bytes.into_inner()
    };
    let response = h.upload(&id, stereo).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let voice = response.json();
    assert_eq!(voice["has_reference"], true);
    assert_eq!(voice["reference_duration_ms"], 3_000);
    assert_eq!(voice["ready"], true);

    let response = h
        .get(&format!("/api/v1/speech/voices/{id}/reference"))
        .await;
    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.content_type(), Some("audio/wav"));
    let stored = decode_wav(&response.body).unwrap();
    assert_eq!(stored.sample_rate(), 44_100);
    assert_eq!(stored.duration_ms(), 3_000);
    let key = format!("voices/{id}/reference.wav");
    assert!(h.ctx.blobs.head(&key).await.unwrap().is_some());

    // 换到 Breeze：还差逐字稿
    let response = h
        .patch(
            &format!("/api/v1/speech/voices/{id}"),
            json!({ "engine": "breeze2" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let voice = response.json();
    assert_eq!(voice["ready"], false);
    assert!(
        voice["not_ready_reason"]
            .as_str()
            .unwrap()
            .contains("逐字稿")
    );
    assert_eq!(voice["has_reference"], true);

    let response = h
        .patch(
            &format!("/api/v1/speech/voices/{id}"),
            json!({ "reference_transcript": "这是一段测试音，没有人说话。" }),
        )
        .await;
    let voice = response.json();
    assert_eq!(voice["ready"], true);
    assert_eq!(
        voice["reference_transcript"],
        "这是一段测试音，没有人说话。"
    );

    // 只改名字，别的不动
    let response = h
        .patch(
            &format!("/api/v1/speech/voices/{id}"),
            json!({ "name": "新名字" }),
        )
        .await;
    let voice = response.json();
    assert_eq!(voice["name"], "新名字");
    assert_eq!(voice["engine"], "breeze2");
    assert_eq!(
        voice["reference_transcript"],
        "这是一段测试音，没有人说话。"
    );
    assert_eq!(voice["reference_duration_ms"], 3_000);
}

fn hound_spec() -> hound::WavSpec {
    hound::WavSpec {
        channels: 2,
        sample_rate: 44_100,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    }
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn bad_reference_files_are_rejected_with_a_reason(pool: PgPool) {
    let h = Harness::new(pool).await;
    let voice = h.cloned("面试官", "index2").await;
    let id = id_of(&voice);

    let response = h.upload(id, Vec::new()).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "empty_file");

    // 太短、太长
    let response = h.upload(id, tone_wav(1_000)).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "invalid_reference_duration");
    let response = h.upload(id, tone_wav(31_000)).await;
    assert_eq!(code_of(&response), "invalid_reference_duration");

    // 没有声音
    let silence = encode_wav(&Audio::silence(24_000, 5_000).unwrap());
    let response = h.upload(id, silence).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "unreadable_file");

    // 根本不是录音。没装 ffmpeg 时认不出来，只能说现在只收 WAV
    let response = h.upload(id, b"this is not audio at all".to_vec()).await;
    if ffmpeg().await.is_some() {
        assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(code_of(&response), "unreadable_file");
    } else {
        assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(code_of(&response), "ffmpeg_missing");
    }

    // 超过 20 MB
    let response = h.upload(id, vec![0_u8; 20 * 1024 * 1024 + 1]).await;
    assert_eq!(response.status, StatusCode::PAYLOAD_TOO_LARGE);

    // 上面都没成功，音色还是没有录音
    let voice = h.get(&format!("/api/v1/speech/voices/{id}")).await.json();
    assert_eq!(voice["has_reference"], false);

    let response = h
        .upload("00000000-0000-0000-0000-000000000000", tone_wav(3_000))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn phone_recordings_are_converted_to_wav(pool: PgPool) {
    let Some(ffmpeg) = ffmpeg().await else {
        return;
    };
    let h = Harness::new(pool).await;
    let voice = h.cloned("面试官", "index2").await;
    let id = id_of(&voice);

    // 手机录音常见的 M4A（AAC）。索引在文件末尾，要整个文件都在才读得出来
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("reference.m4a");
    let status = Command::new(ffmpeg.program())
        .args(["-nostdin", "-v", "error", "-y", "-f", "lavfi", "-i"])
        .arg("sine=frequency=220:duration=6:sample_rate=44100")
        .args(["-c:a", "aac", "-b:a", "64k"])
        .arg(&path)
        .status()
        .expect("起不了 ffmpeg");
    assert!(status.success(), "生成测试录音失败");

    let response = h.upload(id, std::fs::read(&path).unwrap()).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    let voice = response.json();
    assert_eq!(voice["ready"], true);
    let duration = voice["reference_duration_ms"].as_i64().unwrap();
    assert!((5_900..=6_200).contains(&duration), "{duration}");

    let response = h
        .get(&format!("/api/v1/speech/voices/{id}/reference"))
        .await;
    let stored = decode_wav(&response.body).unwrap();
    assert_eq!(stored.sample_rate(), 24_000);
    assert!(stored.peak() > 0.05);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn only_one_default_voice_and_it_cannot_be_deleted(pool: PgPool) {
    let h = Harness::new(pool).await;
    let first = h
        .create(json!({
            "name": "甲", "engine": "breeze2", "kind": "designed",
            "description": "沉稳的男声", "is_default": true,
        }))
        .await;
    assert_eq!(first["is_default"], true);
    let second = h.designed("乙").await;

    // 克隆音色没有录音，建的时候不能直接设成默认
    let response = h
        .post(
            "/api/v1/speech/voices",
            json!({ "name": "丙", "engine": "index2", "kind": "cloned", "is_default": true }),
        )
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "voice_not_ready");

    // 把乙设成默认，甲自动取消
    let response = h
        .patch(
            &format!("/api/v1/speech/voices/{}", id_of(&second)),
            json!({ "is_default": true }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["is_default"], true);

    let list = h.get("/api/v1/speech/voices").await.json();
    assert_eq!(list["total"], 2);
    let names: Vec<(&str, bool)> = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            (
                v["name"].as_str().unwrap(),
                v["is_default"].as_bool().unwrap(),
            )
        })
        .collect();
    // 默认音色排最前面
    assert_eq!(names, [("乙", true), ("甲", false)]);

    let list = h.get("/api/v1/speech/voices?q=%E7%94%B2").await.json();
    assert_eq!(list["total"], 1);

    // 默认音色不能删，也不能改成用不了的样子
    let response = h
        .delete(&format!("/api/v1/speech/voices/{}", id_of(&second)))
        .await;
    assert_eq!(response.status, StatusCode::CONFLICT);
    assert_eq!(code_of(&response), "voice_is_default");

    let response = h
        .delete(&format!("/api/v1/speech/voices/{}", id_of(&first)))
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
    let response = h
        .get(&format!("/api/v1/speech/voices/{}", id_of(&first)))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
    assert_eq!(h.get("/api/v1/speech/voices").await.json()["total"], 1);

    // 取消默认以后就能删了
    let response = h
        .patch(
            &format!("/api/v1/speech/voices/{}", id_of(&second)),
            json!({ "is_default": false }),
        )
        .await;
    assert_eq!(response.json()["is_default"], false);
    let response = h
        .delete(&format!("/api/v1/speech/voices/{}", id_of(&second)))
        .await;
    assert_eq!(response.status, StatusCode::NO_CONTENT);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn default_voice_must_stay_usable_and_deleting_removes_the_recording(pool: PgPool) {
    let h = Harness::new(pool).await;
    let voice = h
        .create(json!({
            "name": "面试官", "engine": "breeze2", "kind": "cloned",
            "reference_transcript": "这是一段测试音。",
        }))
        .await;
    let id = id_of(&voice).to_owned();
    let uri = format!("/api/v1/speech/voices/{id}");
    assert_eq!(h.upload(&id, tone_wav(4_000)).await.status, StatusCode::OK);
    let response = h.patch(&uri, json!({ "is_default": true })).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());

    // Breeze 克隆离不开逐字稿：默认音色不让清掉
    let response = h.patch(&uri, json!({ "reference_transcript": "" })).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "voice_not_ready");
    // 换到不需要逐字稿的引擎再清就可以
    let response = h
        .patch(
            &uri,
            json!({ "engine": "index2", "reference_transcript": "" }),
        )
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.json()["reference_transcript"], Value::Null);
    assert_eq!(response.json()["ready"], true);

    // 删掉音色，录音文件也没了
    let key = format!("voices/{id}/reference.wav");
    assert!(h.ctx.blobs.head(&key).await.unwrap().is_some());
    h.patch(&uri, json!({ "is_default": false })).await;
    assert_eq!(h.delete(&uri).await.status, StatusCode::NO_CONTENT);
    assert!(h.ctx.blobs.head(&key).await.unwrap().is_none());
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn preview_returns_wav_and_reuses_the_cache(pool: PgPool) {
    let h = Harness::new(pool).await;
    let voice = h.designed("亲切女声").await;
    let body = json!({ "text": "你好，请先做一个自我介绍。", "voice_id": id_of(&voice) });

    let response = h.preview(body.clone()).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.content_type(), Some("audio/wav"));
    assert_eq!(response.header("x-tts-engine"), Some("breeze2"));
    assert_eq!(response.header("x-tts-backend"), Some("fake"));
    assert_eq!(response.header("x-tts-from-cache"), Some("false"));
    assert_eq!(response.header("cache-control"), Some("no-store"));
    let audio = decode_wav(&response.body).unwrap();
    assert_eq!(
        response.header("x-audio-duration-ms"),
        Some(audio.duration_ms().to_string().as_str())
    );
    assert!(audio.duration_ms() > 0);

    // 同一句话、同一个音色，第二次直接拿缓存
    let again = h.preview(body).await;
    assert_eq!(again.header("x-tts-from-cache"), Some("true"));
    assert_eq!(again.body, response.body);
    assert_eq!(h.tts.calls(), 1);
    assert_eq!(h.ctx.blobs.list("tts-cache").await.unwrap().len(), 1);

    // 保存之前先听听改了设置以后的效果：设置不一样，要重新合成
    let response = h
        .preview(json!({
            "text": "你好，请先做一个自我介绍。",
            "voice_id": id_of(&voice),
            "style": { "instruction": "语速稍慢，像在和同事聊天" },
        }))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.header("x-tts-from-cache"), Some("false"));
    assert_eq!(h.tts.calls(), 2);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn preview_picks_the_voice_three_ways(pool: PgPool) {
    let h = Harness::new(pool).await;

    // 什么都不填，又没有默认音色
    let response = h.preview(json!({ "text": "你好" })).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "no_default_voice");

    // 临时用一句描述，不用先建音色
    let response = h
        .preview(json!({ "text": "你好", "description": "沉稳的男声" }))
        .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.header("x-tts-engine"), Some("breeze2"));
    // IndexTTS 不能用描述
    let response = h
        .preview(json!({ "text": "你好", "description": "沉稳的男声", "engine": "index2" }))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "invalid_synthesis_request");

    // 克隆音色：传了录音才能听；同一段录音可以换引擎听
    let cloned = h.cloned("面试官", "index2").await;
    let id = id_of(&cloned);
    let response = h.preview(json!({ "text": "你好", "voice_id": id })).await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "voice_not_ready");
    assert_eq!(h.upload(id, tone_wav(5_000)).await.status, StatusCode::OK);
    let response = h.preview(json!({ "text": "你好", "voice_id": id })).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.header("x-tts-engine"), Some("index2"));
    let response = h
        .preview(json!({ "text": "你好", "voice_id": id, "engine": "index25" }))
        .await;
    assert_eq!(response.header("x-tts-engine"), Some("index25"));
    // Breeze 克隆要逐字稿，这个音色没填
    let response = h
        .preview(json!({ "text": "你好", "voice_id": id, "engine": "breeze2" }))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(code_of(&response), "invalid_synthesis_request");

    // 有了默认音色，什么都不填就用它
    h.patch(
        &format!("/api/v1/speech/voices/{id}"),
        json!({ "is_default": true }),
    )
    .await;
    let response = h.preview(json!({ "text": "你好" })).await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.text());
    assert_eq!(response.header("x-tts-engine"), Some("index2"));

    // 两个都填了
    let response = h
        .preview(json!({ "text": "你好", "voice_id": id, "description": "男声" }))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);

    let response = h
        .preview(json!({ "text": "你好", "voice_id": "00000000-0000-0000-0000-000000000000" }))
        .await;
    assert_eq!(response.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn preview_rejects_bad_input_and_reports_engine_outages(pool: PgPool) {
    let h = Harness::with_backend(
        pool,
        FakeBackend::new()
            .with_unavailable(EngineId::Index2)
            .failing_on("出错"),
    )
    .await;

    let response = h
        .preview(json!({ "text": "  ", "description": "男声" }))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let response = h
        .preview(json!({ "text": "好".repeat(301), "description": "男声" }))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    let response = h
        .preview(json!({
            "text": "你好", "description": "男声",
            "style": { "emotion_strength": -0.1 },
        }))
        .await;
    assert_eq!(response.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(h.tts.calls(), 0);

    // 引擎没开：503，告诉用户去看 Mac 上的服务
    let cloned = h.cloned("面试官", "index2").await;
    let id = id_of(&cloned);
    assert_eq!(h.upload(id, tone_wav(5_000)).await.status, StatusCode::OK);
    let response = h.preview(json!({ "text": "你好", "voice_id": id })).await;
    assert_eq!(response.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(code_of(&response), "tts_engine_unavailable");

    // 合成过程中出错：502
    let response = h
        .preview(json!({ "text": "这句话会出错", "description": "男声" }))
        .await;
    assert_eq!(response.status, StatusCode::BAD_GATEWAY);
    assert_eq!(code_of(&response), "tts_error");
}

#[sqlx::test(migrator = "interview_platform::db::MIGRATOR")]
async fn speech_api_needs_login(pool: PgPool) {
    let ctx = testing::test_context(pool);
    let deps = Deps {
        ctx,
        synth: Arc::new(Synthesizer::new(Arc::new(FakeBackend::new()))),
        ffmpeg: Ffmpeg::from_env(),
        default_engine: EngineId::DEFAULT,
    };
    let (router, _api) = interview_tts::http::router(deps).split_for_parts();
    for request in [
        TestRequest::get("/api/v1/speech/engines"),
        TestRequest::get("/api/v1/speech/voices"),
        TestRequest::post("/api/v1/speech/preview").json(&json!({ "text": "你好" })),
        TestRequest::put("/api/v1/speech/voices/00000000-0000-0000-0000-000000000000/reference")
            .bytes(OCTET_STREAM, tone_wav(3_000)),
    ] {
        let response = request.send(&router).await;
        assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    }
}
