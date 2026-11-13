# interview-engine

面试引擎的基础模式：提前出好题，逐题问，候选人逐题答。纯逻辑，不碰数据库、网络和音频；模型通过 `interview-llm` 的 `ChatModel` 传进来。

## 四块东西

| 模块 | 做什么 | 存在哪 |
| --- | --- | --- |
| `config` | 一场面试的全部设置 `InterviewConfig`，`validate()` 检查，`time_budget()` 把时长换成题数和每题的答题时间 | `interviews.config` |
| `planner` | `plan_interview(model, &PlanInput)` 出一份 `InterviewPlan`：面试官要说的每一句话、每道题的考察意图和评分要点、备用追问 | `interviews.plan` |
| `session` | `start` 给出第一步，`advance` 根据候选人刚答完的一段给出下一步 | 包在 `RoomState` 里 |
| `room` | 把一步拆成一次播一句的节拍，给面试间直接用 | `interviews.runtime_state` |

所有对外的类型都能序列化，也都带 `utoipa::ToSchema`。

## 面试配置里有什么

时长（默认 20 分钟）、轮次定位（默认是初筛，后面还有人工面试）、语言、候选人资历、面试官的名字和风格、公司介绍、HR 的考察要求（例如「一定要考察逻辑思维」）、必问题（原样问）、不能问的话题、想考察的能力维度、要确认的基本条件（到岗时间、能否出差等）、前面环节留下的备注、是否核实简历、固定题数、是否留时间让候选人提问、答题的静音秒数和最长时间、追问开关和次数、提前结束的条件、出题和面试中各自的思考模式。

`ConfigLimits::current()` 给出各项的取值范围，后台表单可以直接用。

## 出题

三次模型调用：

1. **简报**：读岗位和简历，写出岗位核心要求、候选人和岗位对得上和对不上的地方、简历里需要核实的点、要考察的维度。
2. **出题**：按简报和 HR 的要求出题。题数和每题时间由 Rust 按时长算好告诉模型，模型不做算术。硬性要求（题数、必问题一条不少、长度）由代码检查，不满足就带着原因让模型重出。
3. **复核**：换一个视角挑毛病（一题问多件事、书面腔、敏感话题、没落实 HR 的要求），有问题直接改。

除了正式题目，还会多出几道备用题，只有候选人答得快、时间有富余时才问。

候选人的姓名不发给模型：模型写 `{candidate_name}`，Rust 再替换。简历正文会发给模型，调用方应先去掉手机号、邮箱这类出题用不上的信息。

`InterviewPlan::utterances()` 是要提前合成语音的全部句子，键是 `opening`、`q1`、`q1.f1`、`ack.1`、`nudge`、`repeat`、`candidate_questions`、`closing`、`closing.early`。

## 逐题问答

`start(plan, config)` 返回第一步：念开场白和第一题，然后听回答。之后每次候选人答完一段，调 `advance(model, plan, config, state, input)` 拿下一步。

一步（`Step`）里有：

- `seq`：第几步。同一步重复上报（断网重试）会原样拿到同一个下一步。
- `utterances`：这一步面试官依次要说的话。
- `listen`：说完之后是否听回答，静音多少秒算答完、最长多久。`listen.ack` 是候选人答完时前端立刻播的一句过渡语，用来盖住转写和模型判断的那一两秒。
- `finished`：面试是否结束，为什么结束。
- `progress`：第几题、共几题、用了多久。

`advance` 做的事：

- 有转写文字时让模型判断这段回答（强、够用、空泛、很差、没内容），以及候选人是不是在要求重复题目或者想结束。这一步限时 12 秒，超时或出错就当没有判断，面试照常往下走。
- 回答空泛或者有疑点，并且时间够，就问一道提前写好的追问。
- 没听到声音：提醒一次并重问；还是没有就下一题；连续两题没声音就结束。
- 时间不够时跳过非必答的题；超出计划时长太多就收尾。
- 最近几道题没有一道说清楚、并且大部分答不上来，就提前结束（可以关）。只是回答空泛不会提前结束。
- 候选人明确说不想继续，就结束。

没有转写文字时（比如语音识别还没接上），引擎按顺序把题问完，不追问也不提前结束。

## 面试间怎么接（一次一句）

面试间的协议是一次播一句，`room` 模块把上面的一步拆成一拍一拍，面试间只要存状态、调转写、调这三个函数：

- `room::open(plan, config)`：第一拍（开场白）。
- `room::report(plan, state, &BeatReport)`：前端报告一拍播完了，或者候选人答完了一段。纯函数。
  - `Reported::Next`：把这一拍发给前端。
  - `Reported::Answered`：候选人答完了。`ack` 有值就马上发给前端播（过渡语），同时在后台转写并调 `settle`；没有值（没开口）就直接调 `settle`，把它给的那一拍发给前端。
  - `Reported::Waiting`：过渡语播完了，判断还没回来。等 `settle` 完成再调一次 `report`。后台任务丢了（比如进程重启）也是这个结果，重新调 `settle` 即可。
- `room::settle(model, plan, config, state, transcript)`：判断这段回答并决定下一步。重复调用是安全的。

每一拍带递增的 `seq`，上报时原样带回：上报当前这一拍就往下走，重复上报上一拍结果不变，其他序号报错（接口返回 409）。同一句话会出现在不同的拍里（没听到声音时重问、过渡语复用），所以幂等按 `seq` 判断。`RoomStep` 序列化出来就是 `{"action":"say","seq":…,"utterance":…,"expects_answer":…,"answer":…,"progress":…,"preload_keys":[…]}` 或 `{"action":"finish","reason":…}`，和 `docs/architecture/api.md` 里的面试间协议对应（`audio_url` 这些由面试间补上）。`RoomState` 整个存进 `interviews.runtime_state`，`state.last` 是断点续面时该重播的那一拍。

## 用真模型试

```bash
cargo run -p interview-engine --example plan -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json
cargo run -p interview-engine --example simulate -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json lost
```

`simulate` 让模型扮演候选人，人设有 `strong`（经验扎实）、`vague`（说话空泛）、`faker`（简历注水）、`lost`（大多答不上来）、`quitter`（中途不想面了）。`fixtures/` 里的岗位和简历都是编的。

2026-11-13 用 deepseek-flash 实测：

| 项目 | 结果 |
| --- | --- |
| 出一份 20 分钟的题（7 道正式题加 3 道备用题） | 约 100 秒，3 次调用，空闲时段约 0.07 元，高峰时段约 0.15 元 |
| 出一份 30 分钟的题（11 道正式题加 4 道备用题） | 约 100 秒，约 0.08 到 0.16 元 |
| 判断一段回答 | 中位 1.1 到 1.3 秒，最慢 1.9 秒 |
| 一场面试里全部回答判断的费用 | 不到 0.02 元 |
| `lost` 人设 | 第 4 题后提前结束 |
| `quitter` 人设 | 说不想面了之后立即结束 |
| `vague` 人设 | 问完全部正式题，中间触发了追问 |
| `strong` 人设 | 答得快，备用题也问完了 |
