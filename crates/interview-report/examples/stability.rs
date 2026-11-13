//! 量一量报告的结论稳不稳：同一份面试记录分析多遍，看每道题的评分、总分和建议差多少。
//! 会真实调用 DeepSeek，8 份样例各跑 5 遍约 5 元。
//!
//! ```bash
//! # 全部样例各分析 5 遍
//! cargo run -p interview-report --example stability
//! # 只跑两份，各 3 遍，每题只评一次（用来对比「评两次」有没有用）
//! cargo run -p interview-report --example stability -- backend-strong pm-mediocre --runs 3 --judgements 1
//! # 每一遍的结果另存一份，方便事后再算
//! cargo run -p interview-report --example stability -- --out /tmp/stability
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use futures_util::future::join_all;
use interview_llm::{DeepSeekClient, Effort, Pricing, Thinking};
use interview_report::record::InterviewRecord;
use interview_report::report::{InterviewReport, Usage};
use interview_report::{AnalyzeOptions, analyze};
use serde::Serialize;
use tokio::sync::Semaphore;

#[derive(Parser)]
#[command(about = "同一份面试记录分析多遍，看结论差多少")]
struct Cli {
    /// 只跑这几份样例（文件名去掉 .json），不填就跑目录里的全部
    only: Vec<String>,
    /// 每份记录分析几遍
    #[arg(long, default_value_t = 5)]
    runs: usize,
    #[arg(long, value_enum, default_value = "high")]
    thinking: ThinkingArg,
    /// 每道题评几次（见 `AnalyzeOptions::question_judgements`）。不填用默认值
    #[arg(long)]
    judgements: Option<u32>,
    #[arg(long, default_value = "crates/interview-report/samples")]
    samples: PathBuf,
    /// 同时进行的分析数。开得太大接口会吃不消，输出被中途截断
    #[arg(long, default_value_t = 3)]
    parallel: usize,
    /// 每一遍的结果存到这个目录（每份记录一个文件）。不填就不存
    #[arg(long)]
    out: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum ThinkingArg {
    Off,
    Low,
    High,
    Max,
}

impl ThinkingArg {
    fn thinking(self) -> Thinking {
        match self {
            Self::Off => Thinking::off(),
            Self::Low => Thinking::on(Effort::Low),
            Self::High => Thinking::on(Effort::High),
            Self::Max => Thinking::on(Effort::Max),
        }
    }
}

/// 一遍分析里和稳定性有关的结论。
#[derive(Serialize)]
struct Run {
    /// 每道题的评分，没评的题不在里面。
    ratings: BTreeMap<String, f32>,
    /// 每道题每一次评的分数（一道题评不止一次时才有）。
    all_ratings: BTreeMap<String, Vec<f32>>,
    /// 每道题的权重。
    weights: BTreeMap<String, f32>,
    score: Option<u8>,
    score_range: Option<(u8, u8)>,
    decision: String,
    integrity: Option<String>,
    credibility: Option<String>,
    match_level: Option<String>,
    seconds: f64,
    usage: Usage,
}

impl Run {
    fn from_report(record: &InterviewRecord, report: &InterviewReport) -> Self {
        Self {
            all_ratings: report
                .questions
                .iter()
                .filter_map(|q| {
                    Some((q.question_id.clone(), q.judgement.as_ref()?.ratings.clone()))
                })
                .collect(),
            weights: record
                .questions
                .iter()
                .map(|q| (q.id.clone(), q.weight.unwrap_or(1.0)))
                .collect(),
            ratings: report
                .questions
                .iter()
                .filter_map(|q| Some((q.question_id.clone(), q.judgement.as_ref()?.rating)))
                .collect(),
            score: report.overall_score,
            score_range: report.score_range.map(|range| (range.low, range.high)),
            decision: label(&report.recommendation.decision),
            integrity: report.integrity.as_ref().map(|i| label(&i.risk_level)),
            credibility: report
                .resume_verification
                .as_ref()
                .map(|r| label(&r.credibility)),
            match_level: report
                .requirement_match
                .as_ref()
                .map(|m| label(&m.match_level)),
            seconds: millis_to_seconds(report.meta.elapsed_ms),
            usage: report.meta.usage,
        }
    }
}

/// 枚举在 JSON 里的写法，比如 `advance`。
fn label<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn millis_to_seconds(ms: u64) -> f64 {
    std::time::Duration::from_millis(ms).as_secs_f64()
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.runs == 0 {
        bail!("--runs 至少是 1");
    }
    let model = DeepSeekClient::from_env().context("需要设置环境变量 DEEPSEEK_API_KEY")?;
    let records = load_records(&cli.samples, &cli.only)?;
    let mut options = AnalyzeOptions {
        thinking: Some(cli.thinking.thinking()),
        ..Default::default()
    };
    if let Some(judgements) = cli.judgements {
        options.question_judgements = judgements;
    }

    let limit = Arc::new(Semaphore::new(cli.parallel.max(1)));
    let jobs = records.iter().flat_map(|(id, record)| {
        (0..cli.runs).map({
            let (model, options, limit) = (&model, &options, &limit);
            move |_| async move {
                let _permit = limit.acquire().await;
                match analyze(record, model, options).await {
                    Ok(report) => Some(Run::from_report(record, &report)),
                    Err(error) => {
                        // 一遍失败不该让别的白跑，记下来接着算
                        eprintln!("{id} 有一遍分析失败：{error}");
                        None
                    }
                }
            }
        })
    });
    let mut results = join_all(jobs).await.into_iter();

    let mut total = Usage::default();
    let mut summary = Summary::default();
    let mut analyses = 0;
    for (id, _) in &records {
        let runs: Vec<Run> = results.by_ref().take(cli.runs).flatten().collect();
        if let Some(out) = &cli.out {
            std::fs::create_dir_all(out)?;
            std::fs::write(
                out.join(format!("{id}.json")),
                serde_json::to_string_pretty(&runs)?,
            )?;
        }
        if runs.is_empty() {
            println!("\n== {id}：每一遍都失败了");
            continue;
        }
        print_record(id, &runs);
        summary.add(&runs);
        analyses += runs.len();
        for run in &runs {
            add_usage(&mut total, &run.usage);
        }
    }
    summary.print(cli.runs);
    println!("\n{}", usage_line(&total, analyses));
    Ok(())
}

fn load_records(dir: &Path, only: &[String]) -> Result<Vec<(String, InterviewRecord)>> {
    let mut records = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("读不到目录 {}", dir.display()))?
    {
        let path = entry?.path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if !only.is_empty() && !only.iter().any(|wanted| wanted == id) {
            continue;
        }
        let text = std::fs::read_to_string(&path)?;
        let record = serde_json::from_str(&text)
            .with_context(|| format!("{} 不是面试记录", path.display()))?;
        records.push((id.to_owned(), record));
    }
    if records.is_empty() {
        bail!("{} 里没有找到要跑的样例", dir.display());
    }
    records.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(records)
}

fn print_record(id: &str, runs: &[Run]) {
    println!("\n== {id}");
    let questions: Vec<&String> = runs
        .iter()
        .flat_map(|run| run.ratings.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    for question in questions {
        let ratings: Vec<String> = runs
            .iter()
            .map(|run| {
                run.ratings
                    .get(question)
                    .map_or("-".to_owned(), ToString::to_string)
            })
            .collect();
        println!("  {question:<8} {}", ratings.join(" "));
    }
    let column = |pick: &dyn Fn(&Run) -> String| -> String {
        runs.iter().map(pick).collect::<Vec<_>>().join(" ")
    };
    let or_dash = |value: &Option<String>| value.clone().unwrap_or_else(|| "-".to_owned());
    println!(
        "  总分     {}",
        column(&|run| run.score.map_or("-".to_owned(), |s| s.to_string()))
    );
    println!(
        "  区间     {}",
        column(&|run| run
            .score_range
            .map_or("-".to_owned(), |(low, high)| format!("{low}~{high}")))
    );
    println!("  建议     {}", column(&|run| run.decision.clone()));
    println!("  诚信     {}", column(&|run| or_dash(&run.integrity)));
    println!("  简历     {}", column(&|run| or_dash(&run.credibility)));
    println!("  匹配     {}", column(&|run| or_dash(&run.match_level)));
}

/// 所有记录合起来的统计。
#[derive(Default)]
struct Summary {
    records: usize,
    /// 每道题几遍评分的最大值减最小值。
    rating_spreads: Vec<f32>,
    /// 每份记录几遍总分的最大值减最小值。
    score_spreads: Vec<u8>,
    /// 每份记录总分的标准差。
    score_deviations: Vec<f64>,
    /// 几遍的总分是不是都落在每一遍给出的区间里：（落在里面的次数，一共比了几次）。
    inside_range: (usize, usize),
    same_decision: usize,
    same_integrity: usize,
    same_credibility: usize,
    same_match: usize,
    seconds: Vec<f64>,
}

impl Summary {
    fn add(&mut self, runs: &[Run]) {
        self.records += 1;
        let questions: std::collections::BTreeSet<&String> =
            runs.iter().flat_map(|run| run.ratings.keys()).collect();
        for question in questions {
            let ratings: Vec<f32> = runs
                .iter()
                .filter_map(|run| run.ratings.get(question).copied())
                .collect();
            let highest = ratings.iter().copied().fold(f32::MIN, f32::max);
            let lowest = ratings.iter().copied().fold(f32::MAX, f32::min);
            self.rating_spreads.push(highest - lowest);
        }
        let scores: Vec<u8> = runs.iter().filter_map(|run| run.score).collect();
        if !scores.is_empty() {
            self.score_spreads.push(spread(&scores));
            self.score_deviations.push(deviation(&scores));
        }
        for run in runs {
            let Some((low, high)) = run.score_range else {
                continue;
            };
            for score in &scores {
                self.inside_range.1 += 1;
                if (low..=high).contains(score) {
                    self.inside_range.0 += 1;
                }
            }
        }
        let all_same = |pick: &dyn Fn(&Run) -> Option<&str>| {
            runs.windows(2).all(|pair| pick(&pair[0]) == pick(&pair[1]))
        };
        self.same_decision += usize::from(all_same(&|run| Some(run.decision.as_str())));
        self.same_integrity += usize::from(all_same(&|run| run.integrity.as_deref()));
        self.same_credibility += usize::from(all_same(&|run| run.credibility.as_deref()));
        self.same_match += usize::from(all_same(&|run| run.match_level.as_deref()));
        self.seconds.extend(runs.iter().map(|run| run.seconds));
    }

    fn print(&self, runs: usize) {
        println!(
            "\n== 合计：{} 份记录，各 {runs} 遍（失败的不算）",
            self.records
        );
        let cells = self.rating_spreads.len();
        let count = |wanted: &dyn Fn(f32) -> bool| {
            self.rating_spreads
                .iter()
                .filter(|spread| wanted(**spread))
                .count()
        };
        println!(
            "每题评分：{cells} 道题里，几遍完全一样的 {} 道，差半分的 {} 道，差 1 分的 {} 道，差更多的 {} 道",
            count(&|s| s < 0.25),
            count(&|s| (0.25..0.75).contains(&s)),
            count(&|s| (0.75..1.25).contains(&s)),
            count(&|s| s >= 1.25),
        );
        println!(
            "总分：几遍之间最多差 {} 分，平均差 {:.1} 分，标准差平均 {:.2}",
            self.score_spreads.iter().max().copied().unwrap_or(0),
            mean(
                &self
                    .score_spreads
                    .iter()
                    .map(|s| f64::from(*s))
                    .collect::<Vec<_>>()
            ),
            mean(&self.score_deviations),
        );
        if self.inside_range.1 > 0 {
            println!(
                "总分区间：任取两遍，一遍的总分落在另一遍给的区间里的比例 {}/{}",
                self.inside_range.0, self.inside_range.1
            );
        }
        println!(
            "几遍结论完全一致的记录：建议 {}/{n}，诚信风险 {}/{n}，简历可信度 {}/{n}，岗位匹配 {}/{n}",
            self.same_decision,
            self.same_integrity,
            self.same_credibility,
            self.same_match,
            n = self.records,
        );
        println!("每遍用时：平均 {:.0} 秒", mean(&self.seconds));
    }
}

fn spread(values: &[u8]) -> u8 {
    let max = values.iter().max().copied().unwrap_or(0);
    let min = values.iter().min().copied().unwrap_or(0);
    max - min
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / f64::from(u32::try_from(values.len()).unwrap_or(u32::MAX))
}

fn deviation(values: &[u8]) -> f64 {
    let values: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
    let center = mean(&values);
    let squares: Vec<f64> = values.iter().map(|v| (v - center).powi(2)).collect();
    mean(&squares).sqrt()
}

fn add_usage(total: &mut Usage, usage: &Usage) {
    total.calls += usage.calls;
    total.prompt_tokens += usage.prompt_tokens;
    total.completion_tokens += usage.completion_tokens;
    total.reasoning_tokens += usage.reasoning_tokens;
    total.cache_hit_tokens += usage.cache_hit_tokens;
    total.cache_miss_tokens += usage.cache_miss_tokens;
}

fn usage_line(usage: &Usage, analyses: usize) -> String {
    let llm_usage = interview_llm::Usage {
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        reasoning_tokens: usage.reasoning_tokens,
        cache_hit_tokens: usage.cache_hit_tokens,
        cache_miss_tokens: usage.cache_miss_tokens,
    };
    let cost = Pricing::FLASH_OFF_PEAK.cost_cny(&llm_usage);
    let per = cost / f64::from(u32::try_from(analyses.max(1)).unwrap_or(u32::MAX));
    format!(
        "调用 {} 次，输入 {} token（命中缓存 {}），输出 {} token（思考 {}），费用约 {cost:.2} 元，每遍 {per:.3} 元（高峰时段翻倍）",
        usage.calls,
        usage.prompt_tokens,
        usage.cache_hit_tokens,
        usage.completion_tokens,
        usage.reasoning_tokens,
    )
}
