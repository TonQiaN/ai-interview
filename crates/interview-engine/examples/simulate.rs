//! 让模型扮演候选人，把一场逐题面试从头走到尾，看状态机和回答判断的表现、延迟和费用。
//!
//! ```bash
//! cargo run -p interview-engine --example simulate -- <输入.json> <计划.json> <人设>
//! ```
//!
//! - 输入是 `PlanInput` 的 JSON。计划文件不存在时会先出题并写到这个路径。
//! - 人设：`strong`（真做过、讲得出细节）、`vague`（空话套话）、`faker`（简历注水，一追问就露怯）、
//!   `lost`（基本答不上来）、`quitter`（答两题就不想面了）。
//!
//! 读环境变量 `DEEPSEEK_API_KEY`，会产生少量费用。

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use interview_engine::{
    AnswerEnd, InterviewPlan, PlanInput, StepInput, Utterance, advance, plan_interview, start,
};
use interview_llm::{ChatModel, ChatRequest, DeepSeekClient, Message, Pricing, Thinking, Usage};

/// 估算念一段话要多久：中文大约每秒四个半字。
fn speak_ms(text: &str) -> u64 {
    text.chars().count() as u64 * 1000 * 2 / 9
}

fn persona(name: &str) -> Result<&'static str> {
    Ok(match name {
        "strong" => {
            "你确实做过简历上写的每一件事，而且是主要负责人。回答具体，有背景、有你自己的做法、有数字、有踩过的坑。表达有条理，每次回答 150 到 300 字。"
        }
        "vague" => {
            "你做过简历上的事情，但只是参与者，很多细节不清楚。回答喜欢说套话和大道理，比如「我们团队协作得很好」「我学到了很多」，很少给具体的细节和数字。每次回答 60 到 120 字。"
        }
        "faker" => {
            "你的简历注了水：写的成果其实是同事做的，你只是旁观过。第一次被问到时你会照着简历的说法复述，显得很流利；一旦被追问具体细节、数字的来源、你本人做了什么，你就开始含糊、绕开、前后不太一致。每次回答 80 到 160 字。"
        }
        "lost" => {
            "你完全不适合这个岗位，题目基本听不懂。回答很短，经常说「这个我不太清楚」「没怎么接触过」，或者答非所问。每次回答 10 到 40 字。"
        }
        "quitter" => {
            "前两个问题你正常回答，每次 80 到 150 字。从第三个问题开始，你觉得这个岗位不适合自己，直接说不想继续面试了，想结束。"
        }
        other => bail!("不认识的人设：{other}"),
    })
}

async fn candidate_reply(
    model: &dyn ChatModel,
    input: &PlanInput,
    persona: &str,
    history: &[(String, String)],
    asked: &[Utterance],
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
    let said: Vec<&str> = asked.iter().map(|item| item.text.as_str()).collect();
    messages.push(Message::user(said.join("\n")));
    let response = model
        .chat(
            ChatRequest::new(messages)
                .with_thinking(Thinking::off())
                .with_max_tokens(600),
        )
        .await?;
    Ok((response.content.trim().to_owned(), response.usage))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input_path, plan_path, persona_name] = args.as_slice() else {
        bail!("用法：simulate <输入.json> <计划.json> <人设>");
    };
    let persona = persona(persona_name)?;
    let input: PlanInput = serde_json::from_str(&std::fs::read_to_string(input_path)?)
        .with_context(|| format!("{input_path} 不是合法的 PlanInput"))?;
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

    let (mut state, mut step) = start(&plan, config)?;
    let mut elapsed_ms = 0u64;
    let mut history: Vec<(String, String)> = Vec::new();
    let mut assess_usage = Usage::default();
    let mut candidate_usage = Usage::default();
    let mut latencies = Vec::new();

    loop {
        for item in &step.utterances {
            println!("\n面试官 [{}] {}", item.key, item.text);
            elapsed_ms += speak_ms(&item.text);
        }
        if let Some(reason) = step.finished {
            println!("\n== 面试结束：{reason:?} ==");
            break;
        }
        let listen = step.listen.clone().context("这一步既不听回答也没有结束")?;

        let (answer, usage) =
            candidate_reply(&model, &input, persona, &history, &step.utterances).await?;
        candidate_usage += usage;
        println!("候选人 {answer}");
        let answer_ms = speak_ms(&answer);
        elapsed_ms += answer_ms + u64::from(listen.silence_sec) * 1000;
        let asked: Vec<&str> = step
            .utterances
            .iter()
            .map(|item| item.text.as_str())
            .collect();
        history.push((asked.join("\n"), answer.clone()));

        let report = StepInput {
            seq: step.seq,
            elapsed_ms,
            end_reason: AnswerEnd::Silence,
            answer_ms,
            transcript: Some(answer),
        };
        let started = Instant::now();
        let result = advance(&model, &plan, config, &state, &report).await?;
        let latency = started.elapsed();
        if let Some(note) = &result.note {
            latencies.push(latency.as_millis() as u64);
            println!(
                "  判断（{} 毫秒）：{:?} / {:?} / 追问 {:?} / {} {}",
                latency.as_millis(),
                note.quality,
                note.intent,
                note.follow_up,
                note.summary,
                if note.flags.is_empty() {
                    String::new()
                } else {
                    format!("/ 留意：{}", note.flags.join("、"))
                }
            );
        }
        assess_usage += result.usage;
        state = result.state;
        step = result.step;
    }

    println!(
        "模拟时长约 {} 分 {} 秒（预期 {} 分钟），答了 {} 道主问题，跳过 {:?}",
        elapsed_ms / 60_000,
        elapsed_ms % 60_000 / 1000,
        config.target_duration_minutes,
        state.answered_questions(&plan),
        state.skipped
    );
    if !latencies.is_empty() {
        latencies.sort_unstable();
        println!(
            "回答判断 {} 次：中位 {} 毫秒，最慢 {} 毫秒；输入 {} token，输出 {} token，空闲时段 {:.4} 元",
            latencies.len(),
            latencies[latencies.len() / 2],
            latencies[latencies.len() - 1],
            assess_usage.prompt_tokens,
            assess_usage.completion_tokens,
            Pricing::FLASH_OFF_PEAK.cost_cny(&assess_usage)
        );
    }
    println!(
        "扮演候选人的开销（不算在面试成本里）：空闲时段 {:.4} 元",
        Pricing::FLASH_OFF_PEAK.cost_cny(&candidate_usage)
    );
    Ok(())
}
