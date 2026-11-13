//! 本地联调和端到端测试用的小工具：造一场准备好的面试，或者看一场面试现在的状态。
//!
//! ```text
//! cargo run -p interview-room --example demo -- seed [--silence-sec 2] [--min-sec 1] [--max-sec 60] [--tone-ms 800] [--realtime 1]
//! cargo run -p interview-room --example demo -- state <面试的 ID>
//! ```
//!
//! 和服务进程读同一套环境变量（`DATABASE_URL`、`AI_INTERVIEW_DATA_DIR`、`AI_INTERVIEW_PUBLIC_URL`），
//! 所以要和服务进程用一样的环境来跑。结果是一行 JSON，打在标准输出上。
//!
//! `seed` 造出来的岗位、候选人、题目都是编的，面试官的「语音」是合成的提示音，不含任何真人的声音。
//! 它直接写库，只用在自己机器的开发库上，不要对着有真实数据的库跑。

use std::f32::consts::TAU;

use anyhow::{Context as _, bail};
use chrono::{DateTime, Duration, Utc};
use interview_engine::InterviewConfig;
use interview_engine::plan::{
    FollowUp, GenerationInfo, InterviewBrief, InterviewPlan, PLAN_SCHEMA_VERSION, Phrases,
    PlannedQuestion, QuestionCategory, QuestionOrigin, ScoringGuide, keys,
};
use interview_platform::AppContext;
use interview_platform::auth::{generate_token, hash_token};
use interview_platform::config::PlatformConfig;
use serde_json::{Value, json};
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = PlatformConfig::from_env().context("配置不对，对照 .env.example 检查环境变量")?;
    let ctx = AppContext::connect(config).await?;
    let output = match args.first().map(String::as_str) {
        Some("seed") => seed(&ctx, &SeedOptions::parse(&args[1..])?).await?,
        Some("state") => {
            let id = args.get(1).context("用法：demo state <面试的 ID>")?;
            state(&ctx, id.parse().context("面试的 ID 不是 UUID")?).await?
        }
        _ => bail!("用法：demo seed [选项] | demo state <面试的 ID>"),
    };
    println!("{output}");
    Ok(())
}

struct SeedOptions {
    silence_sec: u32,
    min_sec: u32,
    max_sec: u32,
    tone_ms: u32,
    /// 配成实时对话。服务进程接了大模型、语音识别和语音合成时才真的按实时对话进行
    realtime: bool,
}

impl SeedOptions {
    fn parse(args: &[String]) -> anyhow::Result<Self> {
        let mut options = Self {
            silence_sec: 2,
            min_sec: 1,
            max_sec: 60,
            tone_ms: 800,
            realtime: false,
        };
        let mut rest = args.iter();
        while let Some(name) = rest.next() {
            let value: u32 = rest
                .next()
                .with_context(|| format!("{name} 后面要跟一个数"))?
                .parse()
                .with_context(|| format!("{name} 后面要跟一个数"))?;
            match name.as_str() {
                "--silence-sec" => options.silence_sec = value,
                "--min-sec" => options.min_sec = value,
                "--max-sec" => options.max_sec = value,
                "--tone-ms" => options.tone_ms = value,
                "--realtime" => options.realtime = value != 0,
                _ => bail!("不认识的选项 {name}"),
            }
        }
        Ok(options)
    }
}

/// 一份小的面试计划：三道题，第一道带一个追问。内容都是编的。
fn sample_plan(config: &InterviewConfig) -> InterviewPlan {
    let question = |number: usize, text: &str| PlannedQuestion {
        key: keys::question(number),
        text: text.to_owned(),
        category: QuestionCategory::Behavioral,
        competency: "沟通表达".to_owned(),
        intent: "看表达是否清楚".to_owned(),
        origin: QuestionOrigin::Generated,
        required: true,
        // 不能超过单题最长时间，不然引擎不认这份计划
        expected_answer_sec: config.answer.max_sec.min(60),
        reserve: false,
        scoring: ScoringGuide::default(),
        follow_ups: Vec::new(),
    };
    let mut first = question(1, "请先简单介绍一下你自己，以及最近在做的工作。");
    first.follow_ups.push(FollowUp {
        key: keys::follow_up("q1", 1),
        text: "能再具体说说你自己做了什么吗？".to_owned(),
        ask_when: "回答里只讲了团队，没讲本人的贡献".to_owned(),
    });
    let mut plan = InterviewPlan {
        schema_version: PLAN_SCHEMA_VERSION,
        brief: InterviewBrief::default(),
        phrases: Phrases {
            opening: "你好，我是这次面试的 AI 面试官。接下来我会问你几个问题，答完停一会儿就会进入下一题。"
                .to_owned(),
            acks: vec!["好的，谢谢。".to_owned(), "明白了。".to_owned()],
            nudge: "我这边没有听到声音，你可以再说一遍吗？".to_owned(),
            repeat: "好的，我再说一遍。".to_owned(),
            candidate_questions: Some("我的问题问完了，你有什么想问的吗？".to_owned()),
            closing: "今天的面试到这里，谢谢你的时间。".to_owned(),
            closing_early: "今天先到这里，谢谢你的时间。".to_owned(),
        },
        questions: vec![
            first,
            question(2, "讲一个你解决过的、印象最深的线上问题。"),
            question(3, "你为什么想来做这个岗位？"),
        ],
        expected_duration_sec: 0,
        generation: GenerationInfo::default(),
    };
    plan.recompute_expected_duration(config);
    plan
}

/// 一小段提示音（16 kHz、单声道、16 位的 WAV），当作面试官这句话的语音。
fn tone_wav(duration_ms: u32, frequency: f32) -> Vec<u8> {
    const RATE: u32 = 16_000;
    let samples = RATE * duration_ms / 1000;
    let fade = (RATE / 50).max(1);
    let mut pcm = Vec::with_capacity(samples as usize * 2);
    for i in 0..samples {
        // 开头和结尾各有 20 毫秒的淡入淡出，免得有爆音
        let edge = i.min(samples - 1 - i).min(fade) as f32 / fade as f32;
        let value = 0.25 * edge * (TAU * frequency * i as f32 / RATE as f32).sin();
        pcm.extend_from_slice(&((value * f32::from(i16::MAX)) as i16).to_le_bytes());
    }
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&RATE.to_le_bytes());
    wav.extend_from_slice(&(RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(&pcm);
    wav
}

async fn insert_invite(
    ctx: &AppContext,
    interview_id: Uuid,
    expires_at: DateTime<Utc>,
    revoked: bool,
) -> anyhow::Result<String> {
    let token = generate_token();
    sqlx::query(
        "INSERT INTO interview_invites (id, interview_id, token_hash, expires_at, revoked_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(interview_core::new_id())
    .bind(interview_id)
    .bind(hash_token(&token))
    .bind(expires_at)
    .bind(revoked.then(Utc::now))
    .execute(&ctx.db)
    .await
    .context("写面试链接失败")?;
    Ok(token)
}

/// 造一场题目和语音都备好、已经发出链接的面试。另外附一条过期的和一条撤销了的链接。
async fn seed(ctx: &AppContext, options: &SeedOptions) -> anyhow::Result<Value> {
    let config = json!({
        "mode": if options.realtime { "conversational" } else { "scripted" },
        "answer": {
            "silence_sec": options.silence_sec,
            "min_sec": options.min_sec,
            "max_sec": options.max_sec,
        }
    });
    let parsed: InterviewConfig = serde_json::from_value(config.clone()).context("面试配置不对")?;
    let plan = sample_plan(&parsed);
    let issues = plan.validate(&parsed);
    if !issues.is_empty() {
        bail!("这组选项配不出能用的面试计划：{issues:?}");
    }

    let (position_id,): (Uuid,) =
        sqlx::query_as("INSERT INTO positions (title) VALUES ('后端工程师（Rust）') RETURNING id")
            .fetch_one(&ctx.db)
            .await
            .context("建岗位失败")?;
    let (candidate_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO candidates (name, email, phone)
         VALUES ('测试候选人', 'candidate@example.com', '13800000001') RETURNING id",
    )
    .fetch_one(&ctx.db)
    .await
    .context("建候选人失败")?;
    let (interview_id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO interviews (candidate_id, position_id, status, config, plan)
         VALUES ($1, $2, 'invited', $3, $4) RETURNING id",
    )
    .bind(candidate_id)
    .bind(position_id)
    .bind(&config)
    .bind(serde_json::to_value(&plan).context("计划序列化失败")?)
    .fetch_one(&ctx.db)
    .await
    .context("建面试失败")?;

    for (seq, utterance) in plan.utterances().into_iter().enumerate() {
        let kind = serde_json::to_value(utterance.kind).context("类别序列化失败")?;
        let blob_key = interview_core::blob_key::utterance_audio(interview_id, &utterance.key);
        let frequency = 330.0 + 40.0 * (seq % 8) as f32;
        ctx.blobs
            .put(&blob_key, tone_wav(options.tone_ms, frequency).into())
            .await
            .context("写语音失败")?;
        sqlx::query(
            "INSERT INTO interview_utterances
                    (interview_id, key, seq, kind, text, synth_status,
                     audio_blob_key, audio_content_type, audio_duration_ms)
             VALUES ($1, $2, $3, $4, $5, 'done', $6, 'audio/wav', $7)",
        )
        .bind(interview_id)
        .bind(&utterance.key)
        .bind(i32::try_from(seq).context("句子太多")?)
        .bind(kind.as_str().context("类别应该是字符串")?)
        .bind(&utterance.text)
        .bind(blob_key)
        .bind(i32::try_from(options.tone_ms).context("提示音太长")?)
        .execute(&ctx.db)
        .await
        .context("写句子失败")?;
    }

    let now = Utc::now();
    let token = insert_invite(ctx, interview_id, now + Duration::days(7), false).await?;
    let expired = insert_invite(ctx, interview_id, now - Duration::hours(1), false).await?;
    let revoked = insert_invite(ctx, interview_id, now + Duration::days(7), true).await?;
    Ok(json!({
        "interview_id": interview_id,
        "url": ctx.config.invite_url(&token),
        "token": token,
        "expired_token": expired,
        "revoked_token": revoked,
    }))
}

/// 一场面试现在的样子：状态、对话记录、过程事件、录像。
async fn state(ctx: &AppContext, interview_id: Uuid) -> anyhow::Result<Value> {
    let interview: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT status, end_reason FROM interviews WHERE id = $1")
            .bind(interview_id)
            .fetch_optional(&ctx.db)
            .await?;
    let (status, end_reason) = interview.context("没有这场面试")?;

    type TurnRow = (
        i32,
        String,
        Option<String>,
        i64,
        Option<i64>,
        Option<String>,
        bool,
        Option<String>,
    );
    // 只说有没有文字，不把文字打出来：对着真的库用时，那是候选人说的话
    let turns: Vec<TurnRow> = sqlx::query_as(
        "SELECT seq, speaker, utterance_key, start_ms, end_ms, end_reason, text <> '', stt_engine
           FROM interview_turns WHERE interview_id = $1 ORDER BY seq",
    )
    .bind(interview_id)
    .fetch_all(&ctx.db)
    .await?;

    let events: Vec<(String, String, Option<i64>, Value)> = sqlx::query_as(
        "SELECT kind, source, at_ms, payload
           FROM interview_events WHERE interview_id = $1 ORDER BY created_at, id",
    )
    .bind(interview_id)
    .fetch_all(&ctx.db)
    .await?;

    type RecordingRow = (
        Uuid,
        String,
        i32,
        String,
        String,
        i64,
        Option<i64>,
        Option<i32>,
        Option<i64>,
        Option<String>,
    );
    let recordings: Vec<RecordingRow> = sqlx::query_as(
        "SELECT id, track, segment, content_type, status, start_ms, duration_ms,
                total_chunks, size_bytes, blob_key
           FROM interview_recordings WHERE interview_id = $1 ORDER BY segment, track",
    )
    .bind(interview_id)
    .fetch_all(&ctx.db)
    .await?;
    let mut recording_list = Vec::with_capacity(recordings.len());
    for row in recordings {
        let chunks: Vec<(i32,)> = sqlx::query_as(
            "SELECT seq FROM interview_recording_chunks WHERE recording_id = $1 ORDER BY seq",
        )
        .bind(row.0)
        .fetch_all(&ctx.db)
        .await?;
        // 合并好的文件在磁盘上的位置，方便拿 ffprobe 看
        let file = row
            .9
            .as_ref()
            .map(|key| ctx.config.data_dir.join("blobs").join(key));
        recording_list.push(json!({
            "track": row.1,
            "segment": row.2,
            "content_type": row.3,
            "status": row.4,
            "start_ms": row.5,
            "duration_ms": row.6,
            "total_chunks": row.7,
            "size_bytes": row.8,
            "chunks": chunks.into_iter().map(|(seq,)| seq).collect::<Vec<_>>(),
            "file": file,
        }));
    }

    Ok(json!({
        "status": status,
        "end_reason": end_reason,
        "turns": turns
            .into_iter()
            .map(|(seq, speaker, utterance_key, start_ms, end_ms, end_reason, has_text, stt_engine)| json!({
                "seq": seq,
                "speaker": speaker,
                "utterance_key": utterance_key,
                "start_ms": start_ms,
                "end_ms": end_ms,
                "end_reason": end_reason,
                "has_text": has_text,
                "stt_engine": stt_engine,
            }))
            .collect::<Vec<_>>(),
        "events": events
            .into_iter()
            .map(|(kind, source, at_ms, payload)| json!({
                "kind": kind,
                "source": source,
                "at_ms": at_ms,
                "payload": payload,
            }))
            .collect::<Vec<_>>(),
        "recordings": recording_list,
    }))
}
