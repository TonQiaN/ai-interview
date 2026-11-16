//! 几场面试同时用一个 worker 时，现场要说的话要多等多久。
//!
//! 模型一次只能合成一段。这个测试让几场「面试」各自隔一会儿要一句现场的话，
//! 同时让几路「面试前的准备」不停地预先合成题目，量每句现场的话比没人抢的时候多等了多久，
//! 也量预先合成被拖慢了多少。只能在装了模型的 Mac 上跑，所以标了 `#[ignore]`：
//!
//! ```bash
//! export AI_INTERVIEW_TTS_WORKER_URLS=http://127.0.0.1:7801
//! export AI_INTERVIEW_TTS_LIVE_REFERENCE=参考录音.wav
//! cargo test -p interview-tts --test worker_load -- --ignored --nocapture
//! ```
//!
//! 可选：
//!
//! - `AI_INTERVIEW_TTS_LOAD_ENGINE`：测哪个引擎，默认是默认引擎。测 breeze2 还要
//!   `AI_INTERVIEW_TTS_LIVE_TRANSCRIPT` 给逐字稿。
//! - `AI_INTERVIEW_TTS_LOAD_SCENARIOS`：跑哪几种情况，`几场面试+几路预先合成`，逗号隔开。
//!   默认 `1+0,2+0,4+0,1+4,2+4,4+4`。后端的任务默认 4 个一起做，所以预先合成最多 4 路。
//! - `AI_INTERVIEW_TTS_LOAD_SECONDS`：每种情况跑多久，默认 90 秒。
//! - `AI_INTERVIEW_TTS_LOAD_GAP_SECONDS`：一场面试里，上一句合成完到要下一句隔多久，默认 15 秒。
//!   真实面试里候选人一段回答二三十秒起，15 秒比真实的密，量出来的等待偏多不偏少。
//! - `AI_INTERVIEW_TTS_LIVE_DIFFUSION_STEPS`：现场的话走几步，和服务进程读的是同一个变量。
//!
//! 参考录音只从环境变量给的位置读，不进仓库；合成出来的音频不落盘，打印的只有时间。

use std::sync::Arc;
use std::time::{Duration, Instant};

use interview_tts::{
    BatchItem, EngineId, Language, ReferenceAudio, SynthesisRequest, Synthesizer, TtsBackendKind,
    TtsConfig, Voice,
};

const REFERENCE_ENV_VAR: &str = "AI_INTERVIEW_TTS_LIVE_REFERENCE";
const TRANSCRIPT_ENV_VAR: &str = "AI_INTERVIEW_TTS_LIVE_TRANSCRIPT";
const ENGINE_ENV_VAR: &str = "AI_INTERVIEW_TTS_LOAD_ENGINE";
const SCENARIOS_ENV_VAR: &str = "AI_INTERVIEW_TTS_LOAD_SCENARIOS";
const SECONDS_ENV_VAR: &str = "AI_INTERVIEW_TTS_LOAD_SECONDS";
const GAP_ENV_VAR: &str = "AI_INTERVIEW_TTS_LOAD_GAP_SECONDS";

const DEFAULT_SCENARIOS: &str = "1+0,2+0,4+0,1+4,2+4,4+4";

/// 一次调用最多等多久。排在好几条后面也就几十秒，超过这个数就是卡住了。
const CALL_TIMEOUT: Duration = Duration::from_secs(300);

/// 实时对话里现场说的话：追问和过渡，十几到二十几个字。
const LIVE_LINES: [&str; 6] = [
    "这个方案上线以后，效果怎么样？",
    "当时为什么没有选另一种做法？",
    "你在这个项目里具体负责哪一部分？",
    "遇到分歧的时候，你是怎么和同事沟通的？",
    "如果再做一次，你会改哪些地方？",
    "好的，那我们换一个话题。",
];

/// 面试前预先合成的题目：三四十个字到五六十个字。
const PREPARED_LINES: [&str; 4] = [
    "请先做一个简单的自我介绍，重点讲一讲你最近负责的项目，以及你在里面承担的角色。",
    "请讲一个你排查线上故障的例子：问题是怎么发现的，你是怎么一步步定位到原因的，最后怎么修的。",
    "如果让你设计一个每天要处理一千万条消息的任务队列，你会怎么保证消息不丢、不重复处理？",
    "你过去和产品、测试同事配合的时候，遇到过需求反复变化的情况吗？你是怎么处理的？",
];

struct Setup {
    engine: EngineId,
    reference: ReferenceAudio,
    transcript: Option<String>,
    scenarios: Vec<(usize, usize)>,
    run_for: Duration,
    gap: Duration,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn env_seconds(name: &str, default: u64) -> Duration {
    let seconds = match env(name) {
        Some(value) => value
            .parse()
            .unwrap_or_else(|_| panic!("{name} 要是秒数（整数），现在是 {value:?}")),
        None => default,
    };
    Duration::from_secs(seconds)
}

fn parse_scenarios(list: &str) -> Vec<(usize, usize)> {
    list.split(',')
        .map(|scenario| {
            let parsed = scenario.trim().split_once('+').and_then(|(live, prepared)| {
                Some((live.trim().parse().ok()?, prepared.trim().parse().ok()?))
            });
            match parsed {
                Some((interviews, prepared)) if interviews > 0 => (interviews, prepared),
                _ => panic!(
                    "{SCENARIOS_ENV_VAR} 里的 {scenario:?} 不对：要写成 几场面试+几路预先合成，比如 2+4，面试至少一场"
                ),
            }
        })
        .collect()
}

impl Setup {
    fn from_env() -> Self {
        let engine = match env(ENGINE_ENV_VAR) {
            Some(name) => name
                .parse()
                .unwrap_or_else(|error| panic!("{ENGINE_ENV_VAR}：{error}")),
            None => EngineId::default(),
        };
        let path = env(REFERENCE_ENV_VAR)
            .unwrap_or_else(|| panic!("要用 {REFERENCE_ENV_VAR} 给出参考录音（WAV）的路径"));
        // 报错里不带路径以外的东西：录音本身是真人的声音
        let wav = std::fs::read(&path).unwrap_or_else(|error| panic!("读不了 {path}：{error}"));
        let reference = ReferenceAudio::from_wav(wav)
            .unwrap_or_else(|error| panic!("{path} 不能当参考录音：{error}"));
        let transcript = env(TRANSCRIPT_ENV_VAR);
        assert!(
            transcript.is_some() || engine != EngineId::Breeze2,
            "BreezeTTS2 克隆要逐字稿：用 {TRANSCRIPT_ENV_VAR} 给出"
        );
        let scenarios = parse_scenarios(
            &env(SCENARIOS_ENV_VAR).unwrap_or_else(|| DEFAULT_SCENARIOS.to_owned()),
        );
        Self {
            engine,
            reference,
            transcript,
            scenarios,
            run_for: env_seconds(SECONDS_ENV_VAR, 90),
            gap: env_seconds(GAP_ENV_VAR, 15),
        }
    }

    fn request(&self, text: &str) -> SynthesisRequest {
        SynthesisRequest::new(
            self.engine,
            text,
            Language::Zh,
            Voice::Clone {
                reference: self.reference.clone(),
                transcript: self.transcript.clone(),
            },
        )
    }
}

fn seconds(duration: Duration) -> f64 {
    duration.as_secs_f64()
}

/// 排好序的一组数里取第 `percent` 百分位（就近取，不插值）。空的返回 0。
fn percentile(sorted: &[Duration], percent: usize) -> Duration {
    match sorted.len() {
        0 => Duration::ZERO,
        len => sorted[((len - 1) * percent + 50) / 100],
    }
}

fn summary(mut values: Vec<Duration>) -> String {
    values.sort();
    format!(
        "中位 {:.1} 秒，九成在 {:.1} 秒以内，最长 {:.1} 秒",
        seconds(percentile(&values, 50)),
        seconds(percentile(&values, 90)),
        seconds(values.last().copied().unwrap_or_default()),
    )
}

/// 合成现场的一句话，返回用了多久。
async fn speak_live(synthesizer: &Synthesizer, request: &SynthesisRequest) -> Duration {
    let started = Instant::now();
    match tokio::time::timeout(CALL_TIMEOUT, synthesizer.synthesize_live(request)).await {
        Ok(Ok(_)) => started.elapsed(),
        Ok(Err(error)) => panic!("现场的话没合成出来：{error}"),
        Err(_) => panic!("现场的话等了 {} 秒还没回来", CALL_TIMEOUT.as_secs()),
    }
}

/// 预先合成一道题，返回用了多久。走批量合成，和面试前的准备是同一条路。
async fn prepare_one(synthesizer: &Synthesizer, request: SynthesisRequest) -> Duration {
    let started = Instant::now();
    let item = BatchItem {
        key: "题目".to_owned(),
        request,
    };
    match tokio::time::timeout(CALL_TIMEOUT, synthesizer.synthesize_batch(vec![item])).await {
        Ok(outcomes) => {
            if let Some(Err(error)) = outcomes.into_iter().next().map(|outcome| outcome.result) {
                panic!("预先合成没成功：{error}");
            }
            started.elapsed()
        }
        Err(_) => panic!("预先合成等了 {} 秒还没回来", CALL_TIMEOUT.as_secs()),
    }
}

/// 一场面试：隔一会儿要一句现场的话，到点为止。返回每句（第几句话，用了多久）。
async fn interview(
    synthesizer: Arc<Synthesizer>,
    requests: Arc<Vec<SynthesisRequest>>,
    first_after: Duration,
    gap: Duration,
    deadline: Instant,
    offset: usize,
) -> Vec<(usize, Duration)> {
    let mut spoken = Vec::new();
    tokio::time::sleep(first_after).await;
    let mut next = offset;
    while Instant::now() < deadline {
        let line = next % requests.len();
        spoken.push((line, speak_live(&synthesizer, &requests[line]).await));
        next += 1;
        tokio::time::sleep(gap).await;
    }
    spoken
}

/// 一路预先合成：一道接一道，到点为止。返回每道用了多久。
async fn preparation(
    synthesizer: Arc<Synthesizer>,
    requests: Arc<Vec<SynthesisRequest>>,
    deadline: Instant,
    offset: usize,
) -> Vec<Duration> {
    let mut done = Vec::new();
    let mut next = offset;
    while Instant::now() < deadline {
        let request = requests[next % requests.len()].clone();
        done.push(prepare_one(&synthesizer, request).await);
        next += 1;
    }
    done
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "要连 Mac 上的真 worker：设 AI_INTERVIEW_TTS_WORKER_URLS 和 AI_INTERVIEW_TTS_LIVE_REFERENCE"]
async fn live_lines_wait_behind_other_interviews_and_preparation() {
    let setup = Setup::from_env();
    let mut config = TtsConfig::from_env().expect("语音合成的环境变量不对");
    config.backend = TtsBackendKind::Worker;
    let backend = config.build().expect("建不了 worker 后端");
    // 不接缓存：每一句都要真的合成
    let synthesizer =
        Arc::new(Synthesizer::new(backend).with_live_diffusion_steps(config.live_diffusion_steps));
    let live: Arc<Vec<_>> = Arc::new(LIVE_LINES.iter().map(|text| setup.request(text)).collect());
    let prepared: Arc<Vec<_>> = Arc::new(
        PREPARED_LINES
            .iter()
            .map(|text| setup.request(text))
            .collect(),
    );

    println!(
        "引擎 {}，现场的话{}，每种情况跑 {} 秒，一场面试每隔 {} 秒要一句",
        setup.engine,
        match config.live_diffusion_steps {
            Some(steps) => format!("走 {steps} 步"),
            None => "用引擎默认的步数".to_owned(),
        },
        setup.run_for.as_secs(),
        setup.gap.as_secs(),
    );

    // 没人抢的时候每句要多久。第一遍可能带着加载和预热，所以量两遍，取短的
    println!("\n没人抢的时候：");
    let mut alone = Vec::with_capacity(live.len());
    for (text, request) in LIVE_LINES.iter().zip(live.iter()) {
        let first = speak_live(&synthesizer, request).await;
        let second = speak_live(&synthesizer, request).await;
        let best = first.min(second);
        println!(
            "  现场 {} 个字：{:.2} 秒",
            text.chars().count(),
            seconds(best)
        );
        alone.push(best);
    }
    let mut prepared_alone = Vec::with_capacity(prepared.len());
    for (text, request) in PREPARED_LINES.iter().zip(prepared.iter()) {
        let took = prepare_one(&synthesizer, request.clone()).await;
        println!(
            "  题目 {} 个字：{:.2} 秒",
            text.chars().count(),
            seconds(took)
        );
        prepared_alone.push(took);
    }
    let prepared_alone_mean = prepared_alone.iter().sum::<Duration>() / prepared_alone.len() as u32;

    for &(interviews, streams) in &setup.scenarios {
        println!("\n{interviews} 场面试，{streams} 路预先合成：");
        let deadline = Instant::now() + setup.run_for;
        let live_tasks: Vec<_> = (0..interviews)
            .map(|index| {
                // 几场面试错开开始，不要一上来就撞在一起
                let first_after = setup.gap * index as u32 / interviews as u32;
                tokio::spawn(interview(
                    Arc::clone(&synthesizer),
                    Arc::clone(&live),
                    first_after,
                    setup.gap,
                    deadline,
                    index,
                ))
            })
            .collect();
        let prepare_tasks: Vec<_> = (0..streams)
            .map(|index| {
                tokio::spawn(preparation(
                    Arc::clone(&synthesizer),
                    Arc::clone(&prepared),
                    deadline,
                    index,
                ))
            })
            .collect();

        let mut took = Vec::new();
        let mut waited = Vec::new();
        for task in live_tasks {
            for (line, elapsed) in task.await.expect("一场面试中途出错了") {
                took.push(elapsed);
                waited.push(elapsed.saturating_sub(alone[line]));
            }
        }
        let mut prepared_took = Vec::new();
        for task in prepare_tasks {
            prepared_took.extend(task.await.expect("一路预先合成中途出错了"));
        }

        println!("  现场的话 {} 句", took.len());
        println!("    从要到拿到：{}", summary(took));
        println!("    比没人抢时多等：{}", summary(waited));
        if !prepared_took.is_empty() {
            let count = prepared_took.len();
            println!(
                "  预先合成 {count} 道，每道从要到拿到：{}（没人抢时平均 {:.1} 秒）",
                summary(prepared_took),
                seconds(prepared_alone_mean),
            );
            println!(
                "    合起来每分钟 {:.1} 道",
                count as f64 * 60.0 / seconds(setup.run_for)
            );
        }
    }
}

#[test]
fn scenarios_are_read_as_interviews_plus_preparation_streams() {
    assert_eq!(
        parse_scenarios(DEFAULT_SCENARIOS),
        vec![(1, 0), (2, 0), (4, 0), (1, 4), (2, 4), (4, 4)]
    );
    assert_eq!(parse_scenarios(" 3 + 1 "), vec![(3, 1)]);
}

#[test]
fn percentiles_pick_the_nearest_value() {
    let values: Vec<_> = (1..=10).map(Duration::from_secs).collect();
    assert_eq!(percentile(&values, 50), Duration::from_secs(6));
    assert_eq!(percentile(&values, 90), Duration::from_secs(9));
    assert_eq!(percentile(&[], 90), Duration::ZERO);
}
