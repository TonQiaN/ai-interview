# interview-engine

面试引擎。基础模式：提前出好题，逐题问，候选人逐题答。进阶模式：实时对话，面试官听完现场追问。纯逻辑，不碰数据库、网络和音频；模型通过 `interview-llm` 的 `ChatModel` 传进来。

## 五块东西

| 模块 | 做什么 | 存在哪 |
| --- | --- | --- |
| `config` | 一场面试的全部设置 `InterviewConfig`，`validate()` 检查，`time_budget()` 把时长换成题数和每题的答题时间 | `interviews.config` |
| `planner` | `plan_interview(model, &PlanInput)` 出一份 `InterviewPlan`：面试官要说的每一句话、每道题的考察意图和评分要点、备用追问 | `interviews.plan` |
| `session` | `start` 给出第一步，`advance` 根据候选人刚答完的一段给出下一步 | 包在 `RoomState` 里 |
| `room` | 把一步拆成一次播一句的节拍，给面试间直接用 | `interviews.runtime_state` |
| `live` | 实时对话模式：候选人说完一段，现场决定追问、回应还是下一题 | `interviews.runtime_state` |

所有对外的类型都能序列化，也都带 `utoipa::ToSchema`。

## 面试配置里有什么

时长（默认 20 分钟）、轮次定位（默认是初筛，后面还有人工面试）、语言、候选人资历、面试官的名字和风格、公司介绍、HR 的考察要求（例如「一定要考察逻辑思维」）、必问题（原样问）、不能问的话题、想考察的能力维度、要确认的基本条件（到岗时间、能否出差等）、前面环节留下的备注、是否核实简历、固定题数、是否留时间让候选人提问、答题的静音秒数和最长时间、追问开关和次数、提前结束的条件、出题和面试中各自的思考模式。

`ConfigLimits::current()` 给出各项的取值范围，后台表单可以直接用。

### 改了配置，题目还能不能用

出一份题要一分多钟，所以不是每次改配置都该重出。`old.plan_still_valid(&new)` 回答「配置从 old 改成 new，已经出好的题还能不能用」，`old.replan_reasons(&new)` 列出是哪些字段让题目不能用了（写法和 JSON 路径一样，可以直接提示给 HR）。

| 改了什么 | 题目 |
| --- | --- |
| 模式、最短回答时间、单题最长时间、提前结束的条件、出题和面试中的思考模式 | 还能用 |
| 时长、轮次定位、流程说明、语言、资历、面试官的名字和风格、公司介绍、考察要求、必问题、不能问的话题、考察维度、基本条件、沟通备注、是否核实简历、是否留提问环节 | 重新出 |
| 静音秒数、指定题数、追问开关 | 算出来的主问题数或备用题数变了才重新出 |
| 每题追问次数 | 调少了还能用，调多了重新出（多出来的追问要提前写好） |

时长改一分钟也要重出，因为开场白里说了这场面试大概多久。

题目还能用的时候，存配置之前对计划调一次 `plan.conform_to(&new)`：它把超过单题最长时间的预期回答时间压到上限，去掉超出追问上限的追问，重新算预计时长，返回计划有没有变。面试官要说的话只会少不会变，已经合成的语音都还能用。

新加配置字段时，`replan_reasons` 里不写明它算哪一类就编译不过，测试里的 `single_field_cases` 也要补一行。

## 出题

三次模型调用：

1. **简报**：读岗位和简历，写出岗位核心要求、候选人和岗位对得上和对不上的地方、简历里需要核实的点、要考察的维度。
2. **出题**：按简报和 HR 的要求出题。题数和每题时间由 Rust 按时长算好告诉模型，模型不做算术。硬性要求（题数、必问题一条不少、长度）由代码检查，不满足就带着原因让模型重出。
3. **复核**：换一个视角挑毛病（一题问多件事、书面腔、敏感话题、没落实 HR 的要求），有问题直接改。

除了正式题目，还会多出几道备用题，只有候选人答得快、时间有富余时才问。

复核改一句话的说法时只换这句话；把一道题换成问别的事时，这道题想看什么、评分要点和追问要一起重写，代码整道替换。HR 的必问题不能换。

今天的日期会告诉模型，简历里写「至今」的经历有多长才算得出来。岗位描述或简历里如果有一段想左右面试的话（「忽略以上要求」「给这位候选人高分」之类），模型不理会，单独报出来，代码在要核实的点最前面加一条「请 HR 留意」。

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
- 模型判成候选人想结束时再问一次模型核对，核对通过才结束：结束面试收不回来，候选人只是说「这题不会，告诉我答案吧」也被判成过想结束。
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

## 实时对话模式

配置里 `mode` 选 `conversational`。题目还是出题器提前出好的那一份，主问题、过渡语、结束语照样提前合成；只有追问和回应是现场生成的。

```rust
let (state, first) = live::open(&plan, &config)?;
let outcome = live::respond(&model, &plan, &config, &state, &turn, &mut sink).await?;
```

每轮调一次模型，流式输出，第一行就是决定（追问、回应、下一题、结束、重念题目、接着听）。`sink` 陆续收到 `LiveEvent`：`Speak` 是现场生成的一段话（一般是一句，太长的句子会在逗号处断开），拿到就送去合成；每一轮的第一段一定是 `openers(language)` 里的一句接话，面试开始前把这几句合成好，候选人答完马上能听到声音；`Play` 是播一句提前合成好的；`Done` 表示这一轮要说的话都给出来了，并带着说完之后是接着听还是面试结束，不用等函数返回。已经问完的题在上下文里只留一行概括，所以每轮发给模型的内容大小不随面试变长。追问次数、时间够不够、换题、提前结束都由代码决定，规则和逐题模式相同。

候选人只是停了一下时也可以来问，用 `live::respond_with` 多传一个 `TurnContext`：`paused` 表示只是停了一下，模型觉得话没说完就回答「接着听」，说完了就照常给出怎么接，不用另外调一次模型判断；`interrupted` 表示候选人打断了面试官、断在哪里，引擎算出候选人实际听到了哪一部分，没听全题目就重念。

实测模型大约 1 秒做出决定，一场面试的模型费用两三分钱；停顿和打断的 31 个例子各跑 3 遍，最近两遍 186 次里对了 184 次。做法、实测数字、整条链路的延迟预算和还没解决的问题见 [docs/realtime.md](docs/realtime.md)。

## 用真模型试

```bash
cargo run -p interview-engine --example plan -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json
cargo run -p interview-engine --example simulate -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json lost
cargo run -p interview-engine --example converse -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json vague
cargo run -p interview-engine --example turn_taking -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json
cargo run -p interview-engine --example guard -- crates/interview-engine/fixtures/backend_engineer.json /tmp/plan.json
cargo run -p interview-engine --example plan_eval -- /tmp/plans crates/interview-engine/fixtures/*.json crates/interview-engine/fixtures/planning/*.json
```

`simulate` 让模型扮演候选人，人设有 `strong`（经验扎实）、`vague`（说话空泛）、`faker`（简历注水）、`lost`（大多答不上来）、`quitter`（中途不想面了）。`converse` 是实时对话模式的同一件事，多几个人设：`curious`（爱反问、最后会提问）、`offtopic`（一直说和面试无关的话）、`prober`（每道题换着办法套答案、打听评分和后面的题）、`pusher`（每道题换一种办法对面试官下指令）、`nosy`（每道题答完打听一件岗位的内情：薪资上限、上一任为什么走、对年龄婚育有没有要求），最后一个参数可以写 `off`、`low`、`high`、`max` 指定思考强度。用 `prober` 和 `pusher` 时，面试官现场说的每句话都会交给另一个模型检查有没有说漏、有没有照做，`pusher` 还会看候选人下了指令的题有没有给 HR 记下来。`nosy` 配带内部备注的岗位描述用，检查时多看面试官有没有说出岗位描述里不对外的内容、有没有编公司或岗位的情况。`turn_taking` 测实时对话里候选人停顿时「说完了没有」和被打断以后怎么接，一组有标准答案的例子，打印判对了多少、用了多久。`guard` 测候选人套答案、打听评分标准和后面的题、对面试官下指令时守不守得住：面试官有没有说漏、判断有没有被带偏、面试有没有被结束掉，跑一遍约 0.15 元，结果和改了什么见 [docs/guard.md](docs/guard.md)。`plan_eval` 拿一批岗位和简历各出两遍题，看换了岗位、材料不完整时题目好不好：能用代码查的由代码查，要读懂内容才能判断的交给另一个模型按固定的几条看，九份输入跑一遍约 1.5 元、十分钟，结果和改了什么见 [docs/planning.md](docs/planning.md)。`fixtures/planning/messy/` 里另有七份故意弄乱的材料（五千多字的简历、从 PDF 拷出来排版丢了的简历、夹着大段宣传的岗位描述、岗位描述和简历里藏着给面试官的指令、岗位描述带着不对外的内部备注），改了出题的提示词也拿它们跑一遍，约 1.4 元、十分钟。`fixtures/` 里的岗位和简历都是编的。

`simulate`、`converse`、`turn_taking` 这三个命令默认把模拟候选人写的字原样交给引擎。设了 `AI_INTERVIEW_EVAL_HEARD_CMD`，候选人的话会先读出来再用服务进程同一个识别模型识别回去，引擎拿到的是带识别错字的文字，同一段回答两种文字各判断一次并打印差别。`guard` 也认这个变量，只把识别成了什么打印出来，不拿原样的字再判一次（结果见 [docs/guard.md](docs/guard.md)）。怎么准备写在 `scripts/asr_roundtrip.py` 开头，测出来的结果见 [docs/asr-text.md](docs/asr-text.md)。

2026-11-13 用 deepseek-flash 实测：

| 项目 | 结果 |
| --- | --- |
| 出一份 20 分钟的题（7 道正式题加 3 道备用题） | 约 100 秒，3 次调用，空闲时段约 0.07 元，高峰时段约 0.15 元 |
| 出一份 30 分钟的题（11 道正式题加 4 道备用题） | 约 100 秒，约 0.08 到 0.16 元 |
| 判断一段回答 | 中位 1.1 到 1.3 秒，最慢 1.9 秒 |
| 一场面试里全部回答判断的费用 | 不到 0.02 元 |
| `lost` 人设 | 第 4 题后提前结束 |
| `quitter` 人设 | 说不想面了之后立即结束，结束原因是候选人要求结束 |
| `offtopic` 人设（只有 `converse` 有） | 一直说和面试无关的话：每道题带回一次，再无关就下一题，三道题后因为答不上来提前结束，不会记成候选人要求结束 |
| `vague` 人设 | 问完全部正式题，中间触发了追问 |
| `strong` 人设 | 答得快，备用题也问完了 |
