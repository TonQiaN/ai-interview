//! 让模型扮演候选人，走一场实时对话模式的面试，看面试官接话的质量、每一轮的延迟和费用。
//!
//! ```bash
//! cargo run -p interview-engine --example converse -- <输入.json> <计划.json> <人设> [思考强度]
//! ```
//!
//! - 输入是 `PlanInput` 的 JSON。计划文件不存在时会先出题并写到这个路径。
//! - 人设和 `simulate` 一样：`strong`、`vague`、`faker`、`lost`、`quitter`，另外多两个：`curious`
//!   （会反问、要求重复、最后提问）和 `offtopic`（说的话和题目完全无关，像是收到了别的声音）。
//! - 思考强度：不写就用输入里的 `live_thinking`；`off`、`low`、`high`、`max` 会覆盖它。
//!
//! 读环境变量 `DEEPSEEK_API_KEY`，会产生少量费用。

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use interview_engine::live::{open, respond};
use interview_engine::{
    CandidateTurn, InterviewPlan, LiveAction, LiveEvent, PlanInput, plan_interview,
};
use interview_llm::{
    ChatModel, ChatRequest, DeepSeekClient, Effort, Message, Pricing, Thinking, Usage,
};

/// 估算念一段话要多久：中文大约每秒四个半字。
fn speak_ms(text: &str) -> u64 {
    text.chars().count() as u64 * 1000 * 2 / 9
}

fn persona(name: &str) -> Result<&'static str> {
    Ok(match name {
        "strong" => {
            "你确实做过简历上写的每一件事，而且是主要负责人。回答具体，有背景、有你自己的做法、有数字、有踩过的坑。表达有条理，每次回答 100 到 200 字。"
        }
        "vague" => {
            "你做过简历上的事情，但只是参与者，很多细节不清楚。回答喜欢说套话和大道理，比如「我们团队协作得很好」「我学到了很多」，很少给具体的细节和数字。被追问时会多说一点点，但还是不具体。每次回答 40 到 100 字。"
        }
        "faker" => {
            "你的简历注了水：写的成果其实是同事做的，你只是旁观过。第一次被问到时你会照着简历的说法复述，显得很流利；一旦被追问具体细节、数字的来源、你本人做了什么，你就开始含糊、绕开、前后不太一致。每次回答 60 到 140 字。"
        }
        "lost" => {
            "你完全不适合这个岗位，题目基本听不懂。回答很短，经常说「这个我不太清楚」「没怎么接触过」，或者答非所问。每次回答 10 到 40 字。"
        }
        "quitter" => {
            "前两个问题你正常回答，每次 80 到 150 字。从第三个问题开始，你觉得这个岗位不适合自己，直接说不想继续面试了，想结束。"
        }
        "curious" => {
            "你做过简历上的事情，回答还算具体，每次 60 到 150 字。你比较爱互动：有时会先反问面试官一句（比如「您是指技术方案还是项目管理方面？」），有一次会说没听清请对方再说一遍，有一次会问这个岗位的团队多大。面试官最后问你有没有问题时，你会问两个问题：薪资范围和什么时候有结果。"
        }
        "offtopic" => {
            "不管面试官问什么，你都像没听见一样，自顾自念一段和面试完全无关的内容：有时是英语语法讲解（比如现在完成时和一般过去时的区别），有时是一段菜谱，有时是天气预报。每次 30 到 80 字。你从不回应面试官的问题，从不提到面试、岗位和自己的经历，也从不说不想面试或者要结束。"
        }
        other => bail!("不认识的人设：{other}"),
    })
}

async fn candidate_reply(
    model: &dyn ChatModel,
    input: &PlanInput,
    persona: &str,
    history: &[(String, String)],
    asked: &str,
) -> Result<(String, Usage)> {
    let system = format!(
        "你在扮演一位正在参加语音面试的候选人，用来测试面试系统。\n\n你的简历：\n{}\n\n你的人设：{}\n\n要求：只输出你口头回答的话，像真人说话一样，不要加括号、动作描写或解释。不要替面试官说话。",
        input.candidate.resume_text, persona
    );
    let mut messages = vec![Message::system(system)];
    for (question, answer) in history {
        messages.push(Message::user(question.clone()));
        messages.push(Message::assistant(answer.clone()));
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
    let (input_path, plan_path, persona_name, thinking) = match args.as_slice() {
        [input, plan, persona] => (input, plan, persona, None),
        [input, plan, persona, thinking] => (input, plan, persona, Some(thinking.as_str())),
        _ => bail!("用法：converse <输入.json> <计划.json> <人设> [off|low|high|max]"),
    };
    let persona = persona(persona_name)?;
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

    let (mut state, first) = open(&plan, config)?;
    let mut elapsed_ms = 0u64;
    let mut asked = Vec::new();
    for item in &first.utterances {
        println!("\n面试官 [{}] {}", item.key, item.text);
        elapsed_ms += speak_ms(&item.text);
        asked.push(item.text.clone());
    }

    let mut history: Vec<(String, String)> = Vec::new();
    let mut live_usage = Usage::default();
    let mut candidate_usage = Usage::default();
    let mut decisions = Vec::new();
    let mut first_generated = Vec::new();
    let mut first_prepared = Vec::new();
    let mut totals = Vec::new();
    let mut done_spoken = Vec::new();
    let mut done_moved = Vec::new();
    let mut saved_spoken = Vec::new();
    let mut saved_moved = Vec::new();
    let mut counts = [0u32; 4];
    let mut turns = 0u32;

    loop {
        let question = asked.join("\n");
        let (answer, usage) = candidate_reply(&model, &input, persona, &history, &question).await?;
        candidate_usage += usage;
        println!("候选人 {answer}");
        let answer_ms = speak_ms(&answer);
        elapsed_ms += answer_ms;
        history.push((question, answer.clone()));

        let turn = CandidateTurn {
            text: answer,
            elapsed_ms,
            answer_ms,
        };
        let started = Instant::now();
        let mut done_at = None;
        let mut sink = |event: LiveEvent| {
            let at = started.elapsed().as_millis();
            match event {
                LiveEvent::Action { action } => println!("  [{at:>5} 毫秒] 决定：{action:?}"),
                LiveEvent::Speak { key, text } => {
                    println!("  [{at:>5} 毫秒] 现场说 [{key}] {text}")
                }
                LiveEvent::Play { utterance } => {
                    println!(
                        "  [{at:>5} 毫秒] 播放 [{}] {}",
                        utterance.key, utterance.text
                    );
                }
                LiveEvent::Done {
                    listen, finished, ..
                } => {
                    done_at = Some(at as u64);
                    match (listen, finished) {
                        (Some(listen), _) => {
                            println!(
                                "  [{at:>5} 毫秒] 话都给出来了，接着听 [{}]",
                                listen.utterance_key
                            )
                        }
                        (None, finished) => {
                            println!("  [{at:>5} 毫秒] 话都给出来了，面试结束 {finished:?}")
                        }
                    }
                }
            }
        };
        let outcome = respond(&model, &plan, config, &state, &turn, &mut sink).await?;
        turns += 1;
        live_usage += outcome.usage;
        decisions.push(outcome.timing.decision_ms);
        totals.push(outcome.timing.total_ms);
        if let Some(done_at) = done_at {
            let saved = outcome.timing.total_ms.saturating_sub(done_at);
            match outcome.action {
                LiveAction::Probe | LiveAction::Reply => {
                    done_spoken.push(done_at);
                    saved_spoken.push(saved);
                }
                LiveAction::Next | LiveAction::End | LiveAction::Repeat => {
                    done_moved.push(done_at);
                    saved_moved.push(saved);
                }
                LiveAction::Wait => {}
            }
        }
        if let Some(first) = outcome.timing.first_speech_ms {
            match outcome.action {
                LiveAction::Probe | LiveAction::Reply => first_generated.push(first),
                LiveAction::Next | LiveAction::End | LiveAction::Repeat => {
                    first_prepared.push(first)
                }
                // 这里每一轮都是说完了才问的，不会出现接着听
                LiveAction::Wait => {}
            }
        }
        counts[match outcome.action {
            LiveAction::Probe => 0,
            LiveAction::Reply => 1,
            LiveAction::Next | LiveAction::Repeat => 2,
            LiveAction::End => 3,
            LiveAction::Wait => continue,
        }] += 1;
        if let Some(note) = &outcome.note {
            println!(
                "  记录：{:?} / {} {}",
                note.quality,
                note.summary,
                if note.flags.is_empty() {
                    String::new()
                } else {
                    format!("/ 留意：{}", note.flags.join("、"))
                }
            );
        }
        asked.clear();
        for item in &outcome.said {
            elapsed_ms += speak_ms(&item.text);
            asked.push(item.text.clone());
        }
        state = outcome.state;
        if let Some(reason) = outcome.finished {
            println!("\n== 面试结束：{reason:?} ==");
            break;
        }
        if turns > 80 {
            bail!("超过 80 轮还没结束，停下来检查");
        }
    }

    println!(
        "模拟时长约 {} 分 {} 秒（预期 {} 分钟），答了 {} 道主问题，跳过 {:?}",
        elapsed_ms / 60_000,
        elapsed_ms % 60_000 / 1000,
        config.target_duration_minutes,
        state.session.answered_questions(&plan),
        state.session.skipped
    );
    println!(
        "一共 {turns} 轮：追问 {}，回应 {}，下一题 {}，结束 {}",
        counts[0], counts[1], counts[2], counts[3]
    );
    summarize("定下动作", &mut decisions);
    summarize("现场生成的第一句（追问、回应）", &mut first_generated);
    summarize("提前合成的第一句（下一题、结束）", &mut first_prepared);
    summarize("整轮模型输出完", &mut totals);
    summarize("追问、回应：知道之后接着听", &mut done_spoken);
    summarize("追问、回应：比等模型写完早了", &mut saved_spoken);
    summarize("下一题、结束：知道之后听不听", &mut done_moved);
    summarize("下一题、结束：比等模型写完早了", &mut saved_moved);
    println!(
        "面试官用量：输入 {} token（命中缓存 {}），输出 {} token；空闲时段 {:.4} 元，高峰时段 {:.4} 元",
        live_usage.prompt_tokens,
        live_usage.cache_hit_tokens,
        live_usage.completion_tokens,
        Pricing::FLASH_OFF_PEAK.cost_cny(&live_usage),
        Pricing::FLASH_PEAK.cost_cny(&live_usage)
    );
    println!(
        "扮演候选人的开销（不算在面试成本里）：空闲时段 {:.4} 元",
        Pricing::FLASH_OFF_PEAK.cost_cny(&candidate_usage)
    );
    Ok(())
}
