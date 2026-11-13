//! 用真实的 DeepSeek 出一份面试计划，看题目质量、耗时和费用。
//!
//! ```bash
//! cargo run -p interview-engine --example plan -- crates/interview-engine/fixtures/backend_engineer.json
//! cargo run -p interview-engine --example plan -- <输入.json> <计划输出.json>
//! ```
//!
//! 读环境变量 `DEEPSEEK_API_KEY`，会产生少量费用。输入是一份 `PlanInput` 的 JSON。

use std::time::Instant;

use anyhow::{Context, Result};
use interview_engine::{PlanInput, plan_interview};
use interview_llm::{DeepSeekClient, Pricing};

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .context("用法：plan <输入.json> [计划输出.json]")?;
    let output = args.next();
    let input: PlanInput = serde_json::from_str(
        &std::fs::read_to_string(&path).with_context(|| format!("读不到 {path}"))?,
    )
    .with_context(|| format!("{path} 不是合法的 PlanInput"))?;

    let model = DeepSeekClient::from_env()?;
    let budget = input.config.time_budget();
    println!(
        "预期 {} 分钟，{} 道题，回答时间共 {} 秒",
        input.config.target_duration_minutes, budget.question_count, budget.answer_sec
    );

    let started = Instant::now();
    let plan = plan_interview(&model, &input).await?;
    let elapsed = started.elapsed();

    println!("\n== 分析 ==");
    println!("岗位：{}", plan.brief.role_summary);
    println!("候选人：{}", plan.brief.candidate_summary);
    for point in &plan.brief.verify_points {
        println!("要核实：{point}");
    }
    for item in &plan.brief.competencies {
        println!("维度：{}（{}）{}", item.name, item.weight, item.description);
    }

    println!("\n== 面试官要说的话 ==");
    for item in plan.utterances() {
        println!("[{}] {}", item.key, item.text);
        if let Some(question) = plan.question(&item.key) {
            println!(
                "      {:?} / {} / {:?} / required={} / {} 秒",
                question.category,
                question.competency,
                question.origin,
                question.required,
                question.expected_answer_sec
            );
            println!("      意图：{}", question.intent);
        }
        if let Some((_, follow_up)) = plan.follow_up(&item.key) {
            println!("      什么时候问：{}", follow_up.ask_when);
        }
    }

    println!("\n== 审题改掉的 ==");
    for note in &plan.generation.review_notes {
        println!("{note}");
    }

    let usage = plan.generation.usage;
    println!("\n== 开销 ==");
    println!(
        "耗时 {:.1} 秒，调用 {} 次",
        elapsed.as_secs_f64(),
        plan.generation.llm_calls
    );
    println!(
        "输入 {} token（缓存命中 {}），输出 {} token（其中思考 {}）",
        usage.prompt_tokens,
        usage.cache_hit_tokens,
        usage.completion_tokens,
        usage.reasoning_tokens
    );
    println!(
        "费用：空闲时段 {:.4} 元，高峰时段 {:.4} 元",
        Pricing::FLASH_OFF_PEAK.cost_cny(&usage),
        Pricing::FLASH_PEAK.cost_cny(&usage)
    );
    println!("按计划走完约 {} 分钟", plan.expected_duration_sec / 60);

    if let Some(output) = output {
        std::fs::write(&output, serde_json::to_string_pretty(&plan)?)?;
        println!("计划已写到 {output}");
    }
    Ok(())
}
