//! 用真人面试校准报告的工具。不调用模型，不联网。步骤见这个 crate 的 `CALIBRATION.md`。
//!
//! ```bash
//! # 给评审生成评分表（一场面试一位评审一个 HTML 文件，浏览器打开就能填）
//! cargo run -p interview-report --example calibrate -- form 报告目录/*.json --annotator hr-1 --out 评分表目录
//! # 生成记录实际情况的文件，组织的人照着填每一场安排了什么
//! cargo run -p interview-report --example calibrate -- truth 报告目录/*.json --out sessions.json
//! # 对照
//! cargo run -p interview-report --example calibrate -- stats --reports 报告目录 --annotations 收回的评分表目录 --truth sessions.json
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use interview_report::InterviewReport;
use interview_report::calibration::{
    Annotation, Calibration, DisagreementKind, Rate, RatingAgreement, SessionTruth, behaviour_name,
    calibrate, decision_name, form_html, parse_report,
};

#[derive(Parser)]
#[command(about = "用真人面试校准报告：生成评分表，把收回的评分表和报告对照")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 按报告里的题目生成空的评分表。评分表里只有题目和能力的名称，没有报告的结论
    Form {
        /// 报告的 JSON 文件。接口 `GET /api/v1/interviews/{id}/report` 返回的整个对象也认
        #[arg(required = true)]
        reports: Vec<PathBuf>,
        /// 评审的代号，比如 hr-1。可以给几个，每位评审一份
        #[arg(long, required = true)]
        annotator: Vec<String>,
        /// 评分表存到哪个目录
        #[arg(long)]
        out: PathBuf,
    },
    /// 生成记录实际情况的文件：每场面试一条，behaviours 先是空的。文件已经有的话只补上还没有的面试
    Truth {
        #[arg(required = true)]
        reports: Vec<PathBuf>,
        #[arg(long)]
        out: PathBuf,
    },
    /// 把报告、评分表和实际情况对起来，打印结果
    Stats {
        /// 放报告的目录，里面是一份份 JSON
        #[arg(long)]
        reports: PathBuf,
        /// 放评分表的目录，里面是评审保存出来的 JSON
        #[arg(long)]
        annotations: PathBuf,
        /// 记录实际情况的文件。不给就不算漏报和误报
        #[arg(long)]
        truth: Option<PathBuf>,
        /// 结果另存成 JSON。里面只有数字、面试 id 和评审的代号
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Form {
            reports,
            annotator,
            out,
        } => form(&reports, &annotator, &out),
        Command::Truth { reports, out } => truth(&reports, &out),
        Command::Stats {
            reports,
            annotations,
            truth,
            out,
        } => stats(&reports, &annotations, truth.as_deref(), out.as_deref()),
    }
}

fn read_report(path: &Path) -> Result<InterviewReport> {
    let text = fs::read_to_string(path).with_context(|| format!("读不了 {}", path.display()))?;
    parse_report(&text).with_context(|| format!("{} 不是一份报告", path.display()))
}

fn form(reports: &[PathBuf], annotators: &[String], out: &Path) -> Result<()> {
    for annotator in annotators {
        let plain = annotator
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if annotator.is_empty() || !plain {
            bail!("评审的代号只能用字母、数字、- 和 _，比如 hr-1。不要用真名：{annotator}");
        }
    }
    fs::create_dir_all(out)?;
    for path in reports {
        let report = read_report(path)?;
        for annotator in annotators {
            let blank = Annotation::blank(&report, annotator);
            let file = out.join(format!("{}.{annotator}.html", report.interview_id));
            fs::write(&file, form_html(&blank))?;
            println!("{}（{} 道题）", file.display(), blank.questions.len());
        }
    }
    Ok(())
}

fn truth(reports: &[PathBuf], out: &Path) -> Result<()> {
    let mut sessions: Vec<SessionTruth> = if out.exists() {
        serde_json::from_str(&fs::read_to_string(out)?)
            .with_context(|| format!("{} 已经有了，但读不出来", out.display()))?
    } else {
        Vec::new()
    };
    let mut added = 0;
    for path in reports {
        let report = read_report(path)?;
        if sessions
            .iter()
            .any(|s| s.interview_id == report.interview_id)
        {
            continue;
        }
        sessions.push(SessionTruth {
            interview_id: report.interview_id,
            behaviours: Vec::new(),
            note: String::new(),
        });
        added += 1;
    }
    fs::write(out, serde_json::to_string_pretty(&sessions)?)?;
    println!(
        "{}：一共 {} 场，这次加了 {added} 场。behaviours 空着表示正常作答；安排了什么照 CALIBRATION.md 里的写法填。",
        out.display(),
        sessions.len()
    );
    Ok(())
}

/// 读一个目录里所有的 JSON。读不出来的跳过，返回跳过了哪些。
fn read_dir<T>(dir: &Path, parse: impl Fn(&str) -> Option<T>) -> Result<(Vec<T>, Vec<String>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("读不了目录 {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    let mut items = Vec::new();
    let mut skipped = Vec::new();
    for path in paths {
        match fs::read_to_string(&path).ok().and_then(|text| parse(&text)) {
            Some(item) => items.push(item),
            None => skipped.push(path.display().to_string()),
        }
    }
    Ok((items, skipped))
}

fn stats(
    reports: &Path,
    annotations: &Path,
    truth: Option<&Path>,
    out: Option<&Path>,
) -> Result<()> {
    let (reports, skipped_reports) = read_dir(reports, |text| parse_report(text).ok())?;
    let (annotations, skipped_annotations) = read_dir(annotations, |text| {
        serde_json::from_str::<Annotation>(text).ok()
    })?;
    let truths: Vec<SessionTruth> = match truth {
        Some(path) => serde_json::from_str(&fs::read_to_string(path)?)
            .with_context(|| format!("{} 读不出来", path.display()))?,
        None => Vec::new(),
    };
    for path in skipped_reports {
        println!("跳过（不是报告）：{path}");
    }
    for path in skipped_annotations {
        println!("跳过（不是评分表）：{path}");
    }

    let calibration = calibrate(&reports, &annotations, &truths);
    print(&calibration);
    if let Some(out) = out {
        fs::write(out, serde_json::to_string_pretty(&calibration)?)?;
        println!("\n结果存到了 {}", out.display());
    }
    Ok(())
}

fn rate(rate: Rate) -> String {
    match (rate.ratio(), rate.interval()) {
        (Some(ratio), Some((low, high))) => format!(
            "{}/{}（{:.0}%，区间 {:.0}% 到 {:.0}%）",
            rate.hits,
            rate.total,
            ratio * 100.0,
            low * 100.0,
            high * 100.0
        ),
        _ => "没有样本".to_owned(),
    }
}

fn number(value: Option<f64>) -> String {
    value.map_or_else(|| "算不出".to_owned(), |v| format!("{v:.2}"))
}

/// 一个带方向的差：正数用 `more` 说，负数用 `less` 说。
fn direction(value: Option<f64>, more: &str, less: &str, unit: &str) -> String {
    match value {
        None => "算不出偏向".to_owned(),
        Some(v) if v.abs() < 0.005 => "没有偏向".to_owned(),
        Some(v) if v > 0.0 => format!("{more} {v:.2} {unit}"),
        Some(v) => format!("{less} {:.2} {unit}", -v),
    }
}

/// 打印两组评分的对比。`between` 是评审和评审比，没有谁高谁低的说法。
fn ratings(name: &str, agreement: &RatingAgreement, between: bool) {
    if agreement.pairs == 0 {
        println!("{name}：没有可比的评分");
        return;
    }
    let bias = if between {
        String::new()
    } else {
        format!(
            "，{}",
            direction(agreement.bias, "报告平均高", "报告平均低", "分")
        )
    };
    println!(
        "{name}：{} 对，平均差 {} 分{bias}，相关系数 {}",
        agreement.pairs,
        number(agreement.mean_abs_diff),
        number(agreement.correlation),
    );
    println!("  差在半分以内 {}", rate(agreement.within_half));
    println!("  差在一分以内 {}", rate(agreement.within_one));
}

fn print(c: &Calibration) {
    println!(
        "有评分表的面试 {} 场，评分表 {} 份（独立评分的 {} 份）；有实际情况记录的面试 {} 场",
        c.interviews, c.annotations, c.blind_annotations, c.sessions_with_truth
    );
    for warning in &c.warnings {
        println!("注意：{warning}");
    }

    println!("\n【建议和评审的结论】");
    println!("完全一样 {}", rate(c.decision.exact));
    println!("一样或只差一档 {}", rate(c.decision.adjacent));
    println!(
        "报告偏宽 {} 对，偏严 {} 对",
        c.decision.report_more_lenient, c.decision.report_stricter
    );
    for cell in &c.decision.matrix {
        println!(
            "  评审：{}　报告：{}　{} 对",
            decision_name(cell.human),
            decision_name(cell.report),
            cell.count
        );
    }

    println!("\n【评分】");
    ratings("逐题评分", &c.question_ratings, false);
    ratings("能力评分", &c.competency_ratings, false);

    println!("\n【评审之间】");
    if c.between_annotators.interviews == 0 {
        println!("没有哪一场面试有两位评审");
    } else {
        println!("{} 场面试有两位以上的评审", c.between_annotators.interviews);
        println!("结论完全一样 {}", rate(c.between_annotators.decision_exact));
        println!(
            "结论一样或只差一档 {}",
            rate(c.between_annotators.decision_adjacent)
        );
        ratings("逐题评分", &c.between_annotators.question_ratings, true);
    }
    for annotator in &c.annotators {
        println!(
            "  {}：{} 场，和报告结论一样 {}，{}，逐题分{}",
            annotator.annotator,
            annotator.interviews,
            rate(annotator.decision_exact),
            direction(annotator.decision_bias, "报告比他宽", "报告比他严", "档"),
            direction(annotator.rating_bias, "报告比他高", "报告比他低", "分"),
        );
    }

    println!("\n【诚信迹象】（「报了」指报告的诚信风险是中或高）");
    println!("安排了作弊，报告报了 {}", rate(c.integrity.staged_flagged));
    println!(
        "安排了作弊，报了而且种类对得上 {}",
        rate(c.integrity.staged_kind_matched)
    );
    println!(
        "写明了在哪几道题上，证据落在这几道题上 {}",
        rate(c.integrity.staged_located)
    );
    for kind in &c.integrity.by_kind {
        println!(
            "  {}：报了 {}，种类对得上 {}",
            behaviour_name(kind.kind),
            rate(kind.flagged),
            rate(kind.kind_matched)
        );
    }
    println!(
        "正常作答，报告却报了（误报） {}",
        rate(c.integrity.clean_flagged)
    );
    println!(
        "只有网络、设备、无关打扰，报告却报了（误报） {}",
        rate(c.integrity.disturbed_flagged)
    );
    println!(
        "和评审自己看出来的一致 {}；评审觉得有、报告没报 {} 对；报告报了、评审没觉得 {} 对",
        rate(c.integrity.same_as_human),
        c.integrity.human_only,
        c.integrity.report_only
    );

    println!("\n【简历核验】");
    println!("可信度和评审给的一样 {}", rate(c.resume.same_as_human));
    println!(
        "安排了简历夸大，报告起了疑心 {}",
        rate(c.resume.inflated_caught)
    );
    println!(
        "简历是实话，报告却起了疑心（误报） {}",
        rate(c.resume.honest_doubted)
    );

    println!("\n【评审对着报告核对】");
    println!("{} 份评分表做了核对", c.review.reviews);
    println!("抽查的证据对得上 {}", rate(c.review.evidence_correct));
    println!(
        "指出报告说错 {} 处，漏掉 {} 处",
        c.review.wrong_findings, c.review.missed
    );
    println!(
        "有用程度平均 {}（满分 5）；看完会改自己结论的 {}",
        number(c.review.usefulness_mean),
        rate(c.review.would_change_decision)
    );
    println!("评审看一场平均花 {} 分钟", number(c.review.minutes_mean));

    println!("\n【要逐个看的不一致】{} 处", c.disagreements.len());
    for item in &c.disagreements {
        let kind = match item.kind {
            DisagreementKind::IntegrityMissed => "作弊没报",
            DisagreementKind::IntegrityFalseAlarm => "诚信误报",
            DisagreementKind::DecisionFar => "结论差两档以上",
            DisagreementKind::ResumeMissed => "简历夸大没认出",
            DisagreementKind::ResumeFalseAlarm => "简历误报",
            DisagreementKind::DecisionAdjacent => "结论差一档",
            DisagreementKind::QuestionRating => "一道题差 1.5 分以上",
        };
        let who = item
            .annotator
            .as_deref()
            .map(|a| format!("　评审 {a}"))
            .unwrap_or_default();
        let question = item
            .question_id
            .as_deref()
            .map(|q| format!("　题目 {q}"))
            .unwrap_or_default();
        println!(
            "  {kind}　面试 {}{who}{question}　报告：{}　对照：{}",
            item.interview_id, item.report, item.expected
        );
    }
}
