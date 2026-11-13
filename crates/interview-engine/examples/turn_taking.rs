//! 用真模型测实时对话里两件事判断得准不准、要多久：
//!
//! 1. 候选人停了一下，这段话是说完了还是没说完（没说完应该选 `wait`）。
//! 2. 面试官的话被候选人打断以后怎么接（没听全题目应该重念，听够了就照常往下）。
//!
//! ```bash
//! cargo run -p interview-engine --example turn_taking -- <输入.json> <计划.json> [思考强度] [每个例子跑几遍]
//! ```
//!
//! - 输入是 `PlanInput` 的 JSON。计划文件不存在时会先出题并写到这个路径。
//! - 思考强度：不写就用输入里的 `live_thinking`；`off`、`low`、`high`、`max` 会覆盖它。
//! - 每个例子默认跑 2 遍。
//!
//! 例子都落在第二道题上（第一道通常是自我介绍）。候选人的完整回答由模型按简历现写，没说完的版本
//! 是把它从中间截断得到的，另外有一批不依赖题目的固定说法。
//!
//! 读环境变量 `DEEPSEEK_API_KEY`，会产生少量费用（默认设置下约 0.03 元）。

use std::path::Path;

use anyhow::{Context, Result, bail};
use interview_engine::live::{open, respond, respond_with};
use interview_engine::{
    CandidateTurn, Interruption, InterviewConfig, InterviewPlan, LiveAction, LiveEvent, LiveState,
    PlanInput, TurnContext, plan_interview,
};
use interview_llm::testing::ScriptedModel;
use interview_llm::{
    ChatModel, ChatRequest, DeepSeekClient, Effort, Message, Pricing, Thinking, Usage,
};

/// 念一个字大约要多久，和引擎里估算打断位置用的一样。
const MS_PER_CHAR: u64 = 220;

struct Case {
    group: &'static str,
    label: String,
    state: LiveState,
    turn: CandidateTurn,
    context: TurnContext,
    expected: &'static [LiveAction],
}

fn said(text: impl Into<String>, elapsed_ms: u64) -> CandidateTurn {
    CandidateTurn {
        text: text.into(),
        elapsed_ms,
        answer_ms: 20_000,
    }
}

/// 候选人只是停了一下。
const PAUSED: TurnContext = TurnContext {
    paused: true,
    interrupted: None,
};

/// 候选人是打断面试官开的口。
fn cut_in(key: &str, played_ms: u64, total_ms: Option<u64>) -> TurnContext {
    TurnContext {
        paused: false,
        interrupted: Some(Interruption {
            utterance_key: key.to_owned(),
            played_ms,
            total_ms,
        }),
    }
}

/// 一段话按句子切开，标点留在句子后面。
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        current.push(c);
        if matches!(c, '。' | '！' | '？' | '!' | '?') {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// 一句话说到一半停住：截在中间，并且保证停的地方前后都是字，不会正好停在一个分句的末尾。
fn half(sentence: &str) -> String {
    let chars: Vec<char> = sentence.chars().collect();
    let mut end = chars.len() / 2;
    while end > 1
        && !(chars[end - 1].is_alphanumeric()
            && chars.get(end).is_some_and(|c| c.is_alphanumeric())
            && chars.get(end + 1).is_some_and(|c| c.is_alphanumeric()))
    {
        end -= 1;
    }
    chars[..end].iter().collect()
}

/// 让模型扮演候选人回答 `asked`。`before` 是这之前的一问一答。
async fn candidate_answer(
    model: &dyn ChatModel,
    input: &PlanInput,
    before: Option<(&str, &str)>,
    asked: &str,
) -> Result<(String, Usage)> {
    let system = format!(
        "你在扮演一位正在参加语音面试的候选人，用来测试面试系统。\n\n你的简历：\n{}\n\n你确实做过简历上写的事情。回答具体，有你自己的做法和数字，分四到六句话说完，一共 120 到 200 字。\n\n要求：只输出你口头回答的话，像真人说话一样，不要加括号、动作描写或解释。",
        input.candidate.resume_text
    );
    let mut messages = vec![Message::system(system)];
    if let Some((question, answer)) = before {
        messages.push(Message::user(question.to_owned()));
        messages.push(Message::assistant(answer.to_owned()));
    }
    messages.push(Message::user(asked.to_owned()));
    let response = model
        .chat(
            ChatRequest::new(messages)
                .with_thinking(Thinking::off())
                .with_max_tokens(600),
        )
        .await?;
    Ok((response.content.trim().to_owned(), response.usage))
}

/// 用写好的模型输出把状态往前推一步，不花钱。
async fn advance(
    plan: &InterviewPlan,
    config: &InterviewConfig,
    state: &LiveState,
    reply: &str,
    answer: &str,
    elapsed_ms: u64,
) -> Result<LiveState> {
    let scripted = ScriptedModel::new().reply(reply);
    let turn = said(answer, elapsed_ms);
    let outcome = respond(&scripted, plan, config, state, &turn, &mut |_| {}).await?;
    Ok(outcome.state)
}

fn percentile(sorted: &[u64], percent: usize) -> u64 {
    sorted[(sorted.len() * percent / 100).min(sorted.len() - 1)]
}

fn summarize(label: &str, values: &mut [u64]) {
    if values.is_empty() {
        return;
    }
    values.sort_unstable();
    println!(
        "{label}：{} 次，中位 {} 毫秒，90% 在 {} 毫秒以内，最慢 {} 毫秒",
        values.len(),
        percentile(values, 50),
        percentile(values, 90),
        values[values.len() - 1]
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input_path, plan_path, thinking, runs) = match args.as_slice() {
        [input, plan] => (input, plan, None, 2),
        [input, plan, thinking] => (input, plan, Some(thinking.as_str()), 2),
        [input, plan, thinking, runs] => (
            input,
            plan,
            Some(thinking.as_str()),
            runs.parse::<u32>().context("每个例子跑几遍要写数字")?,
        ),
        _ => bail!("用法：turn_taking <输入.json> <计划.json> [off|low|high|max] [每个例子跑几遍]"),
    };
    let mut input: PlanInput = serde_json::from_str(&std::fs::read_to_string(input_path)?)
        .with_context(|| format!("{input_path} 不是合法的 PlanInput"))?;
    match thinking {
        None => {}
        Some("off") => input.config.live_thinking = Thinking::off(),
        Some("low") => input.config.live_thinking = Thinking::on(Effort::Low),
        Some("high") => input.config.live_thinking = Thinking::on(Effort::High),
        Some("max") => input.config.live_thinking = Thinking::on(Effort::Max),
        Some(other) => bail!("不认识的思考强度：{other}"),
    }
    let config = &input.config;
    let model = DeepSeekClient::from_env()?;

    let plan: InterviewPlan = if Path::new(plan_path).exists() {
        serde_json::from_str(&std::fs::read_to_string(plan_path)?)?
    } else {
        println!("没有现成的计划，先出题……");
        let plan = plan_interview(&model, &input).await?;
        std::fs::write(plan_path, serde_json::to_string_pretty(&plan)?)?;
        plan
    };
    if plan.questions.len() < 2 {
        bail!("计划里至少要有两道题");
    }

    // 三个起点：刚开场（开场白 + 第一题）、刚换到第二题（过渡语 + 第二题）、第二题追问了一句
    let (at_opening, _) = open(&plan, config)?;
    let at_second = advance(
        &plan,
        config,
        &at_opening,
        "next adequate\n做了自我介绍。",
        "我做了五年后端开发，最近三年主要负责交易和库存这一块。",
        60_000,
    )
    .await?;
    let question = &plan.questions[1];
    let transition = at_second.last_said.first().cloned().unwrap_or_default();
    println!("第二题 [{}] {}\n", question.key, question.text);

    let (answer, answer_usage) = candidate_answer(&model, &input, None, &question.text).await?;
    println!("候选人的完整回答：{answer}\n");
    let parts = sentences(&answer);
    if parts.len() < 3 {
        bail!("模型给的回答不到三句话，再跑一次");
    }
    let probe = "明白。这里面你本人具体负责的是哪一部分？";
    let probed = advance(
        &plan,
        config,
        &at_second,
        &format!("probe adequate\n{probe}"),
        &answer,
        150_000,
    )
    .await?;
    let probe_key = probed.last_said.first().cloned().unwrap_or_default();
    let (probe_answer, probe_usage) =
        candidate_answer(&model, &input, Some((&question.text, &answer)), probe).await?;
    // 再追问到不能追问为止：这时候选人说没听清，不能因为不让追问了就换题
    let mut probed_out = probed.clone();
    while config.follow_ups.enabled
        && probed_out.probes_asked < config.follow_ups.max_per_question * 2
    {
        probed_out = advance(
            &plan,
            config,
            &probed_out,
            "probe adequate\n好的。那当时一共用了多少台服务器？",
            "这块主要是我负责的。",
            170_000,
        )
        .await?;
    }
    let question_ms = question.text.chars().count() as u64 * MS_PER_CHAR;
    let probe_ms = probe.chars().count() as u64 * MS_PER_CHAR;

    const WAIT: &[LiveAction] = &[LiveAction::Wait];
    const SPOKE: &[LiveAction] = &[LiveAction::Probe, LiveAction::Next];
    const REPEAT: &[LiveAction] = &[LiveAction::Repeat];
    let mut cases = Vec::new();

    // 一、没说完，应该接着听
    let unfinished: Vec<(String, String)> = vec![
        (
            "第二句说到一半".into(),
            format!("{}{}", parts[0], half(&parts[1])),
        ),
        (
            "第三句说到一半".into(),
            format!("{}{}{}", parts[0], parts[1], half(&parts[2])),
        ),
        (
            "第二句说到一半，识别还给加了句号".into(),
            format!("{}{}。", parts[0], half(&parts[1])),
        ),
        ("第一句就说到一半".into(), half(&parts[0])),
        ("第一句后面跟着「然后」".into(), format!("{}然后", parts[0])),
        (
            "两句后面跟着「比如说」".into(),
            format!("{}{}比如说", parts[0], parts[1]),
        ),
        ("只开了个头".into(), "嗯，这个问题的话".into()),
        ("在想怎么说".into(), "我想一下啊，就是".into()),
        (
            "说了三点只讲到第二点".into(),
            "这个主要有三个方面，第一个是先把需求理清楚，第二个是".into(),
        ),
        (
            "停在「主要负责的是」".into(),
            "我之前在上一家公司主要负责的是".into(),
        ),
        (
            "停在「因为」".into(),
            "这个项目我是从头跟到尾的，因为".into(),
        ),
        (
            "卡在一个词上".into(),
            "当时遇到的最大的困难其实是，就是我们那个".into(),
        ),
    ];
    for (label, text) in unfinished {
        cases.push(Case {
            group: "没说完",
            label,
            state: at_second.clone(),
            turn: said(text, 100_000),
            context: PAUSED,
            expected: WAIT,
        });
    }

    // 二、说完了，不应该再等
    let finished: Vec<(String, String, &'static [LiveAction])> = vec![
        ("完整回答".into(), answer.clone(), SPOKE),
        (
            "只答了前两句，但句子是完整的".into(),
            format!("{}{}", parts[0], parts[1]),
            SPOKE,
        ),
        ("不知道".into(), "这个我不太清楚。".into(), SPOKE),
        ("没做过，没有标点".into(), "这块没怎么接触过".into(), SPOKE),
        (
            "很空泛但说完了".into(),
            "这块主要是我们组长在弄，我就是配合一下，具体的不太了解。".into(),
            SPOKE,
        ),
        ("收尾的话".into(), "嗯，大概就是这样。".into(), SPOKE),
        // 回答一句再把问题带回来，模型有时记成回应、有时记成追问，说出来的话是一样的
        (
            "反问".into(),
            "您是指技术方案还是项目管理方面？".into(),
            &[LiveAction::Reply, LiveAction::Probe],
        ),
        (
            "没听清".into(),
            "不好意思，没听清，能再说一遍吗？".into(),
            REPEAT,
        ),
        (
            "不想面了".into(),
            "我觉得这个岗位不太适合我，不想继续面了。".into(),
            &[LiveAction::End],
        ),
    ];
    for (label, text, expected) in finished {
        cases.push(Case {
            group: "说完了",
            label,
            state: at_second.clone(),
            turn: said(text, 100_000),
            context: PAUSED,
            expected,
        });
    }

    cases.push(Case {
        group: "说完了",
        label: "追问到上限以后，候选人说没听清追问".into(),
        state: probed_out,
        turn: said("抱歉，这个我没听清，您能再说一遍吗？", 180_000),
        context: PAUSED,
        expected: &[LiveAction::Reply],
    });

    // 三、面试官的话被打断
    cases.push(Case {
        group: "被打断",
        label: "题目播到三成，候选人说没听清".into(),
        state: at_second.clone(),
        turn: said("不好意思，刚才没听清。", 70_000),
        context: cut_in(&question.key, question_ms * 3 / 10, Some(question_ms)),
        expected: REPEAT,
    });
    cases.push(Case {
        group: "被打断",
        label: "开场白没播完，候选人问能不能听到".into(),
        state: at_opening.clone(),
        turn: said("喂，能听到我说话吗？", 5_000),
        context: cut_in("opening", 3_000, None),
        expected: &[LiveAction::Repeat, LiveAction::Reply],
    });
    cases.push(Case {
        group: "被打断",
        label: "题目播到九成，候选人直接开始回答".into(),
        state: at_second.clone(),
        turn: said(answer.clone(), 110_000),
        context: cut_in(&question.key, question_ms * 9 / 10, Some(question_ms)),
        expected: SPOKE,
    });
    cases.push(Case {
        group: "被打断",
        label: "刚说过渡语，候选人还在补充上一题".into(),
        state: at_second.clone(),
        turn: said(
            "对了，刚才那个我再补充一点，我们组一共六个人，我是负责后端的。",
            75_000,
        ),
        context: cut_in(&transition, 300, None),
        expected: REPEAT,
    });
    cases.push(Case {
        group: "被打断",
        label: "题目播到三成，候选人请面试官说慢一点".into(),
        state: at_second.clone(),
        turn: said("不好意思，您能说慢一点吗？", 70_000),
        context: cut_in(&question.key, question_ms * 3 / 10, Some(question_ms)),
        expected: &[LiveAction::Repeat, LiveAction::Reply],
    });
    cases.push(Case {
        group: "被打断",
        label: "追问说到一半，候选人说没听清".into(),
        state: probed.clone(),
        turn: said("没听清，您刚才问的是什么？", 160_000),
        context: cut_in(&probe_key, probe_ms * 4 / 10, None),
        expected: &[LiveAction::Probe, LiveAction::Reply],
    });
    cases.push(Case {
        group: "被打断",
        label: "追问快说完了，候选人抢着回答".into(),
        state: probed.clone(),
        turn: said(probe_answer, 190_000),
        context: cut_in(&probe_key, probe_ms * 8 / 10, None),
        expected: SPOKE,
    });

    let mut usage = Usage::default();
    let mut wait_ms = Vec::new();
    let mut other_ms = Vec::new();
    let mut first_speech_ms = Vec::new();
    let mut right = [0u32; 3];
    let mut total = [0u32; 3];
    let mut group = "";
    for case in &cases {
        if case.group != group {
            group = case.group;
            println!("\n== {group} ==");
        }
        let slot = match case.group {
            "没说完" => 0,
            "说完了" => 1,
            _ => 2,
        };
        let mut got = Vec::new();
        let mut spoken = String::new();
        for _ in 0..runs {
            let mut heard = String::new();
            let mut sink = |event: LiveEvent| match event {
                LiveEvent::Speak { text, .. } => heard.push_str(&text),
                LiveEvent::Play { utterance } => {
                    heard.push_str(&format!("[{}]", utterance.key));
                }
                LiveEvent::Action { .. } | LiveEvent::Done { .. } => {}
            };
            let outcome = respond_with(
                &model,
                &plan,
                config,
                &case.state,
                &case.turn,
                &case.context,
                &mut sink,
            )
            .await?;
            usage += outcome.usage;
            if outcome.action == LiveAction::Wait {
                wait_ms.push(outcome.timing.decision_ms);
            } else {
                other_ms.push(outcome.timing.decision_ms);
            }
            if let Some(first) = outcome.timing.first_speech_ms {
                first_speech_ms.push(first);
            }
            total[slot] += 1;
            if case.expected.contains(&outcome.action) {
                right[slot] += 1;
            }
            got.push(outcome.action);
            spoken = heard;
        }
        let ok = got.iter().all(|action| case.expected.contains(action));
        println!(
            "{} {}：{:?}（应该是 {:?}）",
            if ok { "对" } else { "错" },
            case.label,
            got,
            case.expected
        );
        println!("    候选人：{}", case.turn.text);
        if !spoken.is_empty() {
            println!("    面试官：{spoken}");
        }
    }

    println!();
    for (index, name) in ["没说完", "说完了", "被打断"].iter().enumerate() {
        println!("{name}：{} 次里对了 {} 次", total[index], right[index]);
    }
    summarize("决定接着听", &mut wait_ms);
    summarize("决定别的动作", &mut other_ms);
    summarize("要说的第一句出来", &mut first_speech_ms);
    println!(
        "用量：输入 {} token（命中缓存 {}），输出 {} token；空闲时段 {:.4} 元，高峰时段 {:.4} 元",
        usage.prompt_tokens,
        usage.cache_hit_tokens,
        usage.completion_tokens,
        Pricing::FLASH_OFF_PEAK.cost_cny(&usage),
        Pricing::FLASH_PEAK.cost_cny(&usage)
    );
    let mut acting = answer_usage;
    acting += probe_usage;
    println!(
        "扮演候选人的开销：空闲时段 {:.4} 元",
        Pricing::FLASH_OFF_PEAK.cost_cny(&acting)
    );
    Ok(())
}
