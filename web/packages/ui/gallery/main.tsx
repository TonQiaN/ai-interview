import "../src/styles/index.css";
import "./gallery.css";
import {
  Briefcase,
  Camera,
  FileText,
  LayoutDashboard,
  Mic,
  Moon,
  Plus,
  Search,
  Send,
  Settings2,
  Sun,
  Users,
  Video,
  Wifi,
} from "lucide-react";
import { StrictMode, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  Avatar,
  allConsented,
  applyTheme,
  Badge,
  Banner,
  Brand,
  Button,
  CaptionCard,
  Card,
  CardBody,
  CardFooter,
  CardHeader,
  Checkbox,
  Chip,
  ConsentList,
  CopyField,
  DeviceCheckCard,
  Dialog,
  Dropzone,
  EmptyState,
  Field,
  Hero,
  Input,
  InterviewerPresence,
  KeyValue,
  LevelMeter,
  Nav,
  NavBadge,
  NavSection,
  navItemClass,
  PageHeader,
  Painting,
  type PaintingName,
  type PresenceState,
  Progress,
  paintings,
  QuestionProgress,
  Radio,
  RecordingIndicator,
  Ring,
  readTheme,
  ScoreBar,
  Segmented,
  Select,
  SilenceRing,
  Skeleton,
  Slider,
  Stage,
  Stat,
  StatusPage,
  Steps,
  Switch,
  Table,
  Tabs,
  Textarea,
  type Theme,
  Timer,
  ToastProvider,
  useToast,
  VideoFrame,
} from "../src";

const pigments = [
  ["普鲁士蓝", "--oil-pigment-prussian"],
  ["群青", "--oil-pigment-ultramarine"],
  ["天蓝", "--oil-pigment-cerulean"],
  ["翠绿", "--oil-pigment-viridian"],
  ["土黄", "--oil-pigment-ochre"],
  ["那不勒斯黄", "--oil-pigment-naples"],
  ["赭石", "--oil-pigment-sienna"],
  ["朱红", "--oil-pigment-vermilion"],
  ["茜素红", "--oil-pigment-alizarin"],
  ["生褐", "--oil-pigment-umber"],
] as const;

const surfaces = [
  ["亚麻（页面）", "--oil-linen"],
  ["画布（卡片）", "--oil-canvas"],
  ["铅白（输入）", "--oil-raised"],
  ["凹槽", "--oil-sunken"],
  ["墨", "--oil-ink"],
  ["次墨", "--oil-ink-2"],
  ["淡墨", "--oil-ink-3"],
  ["线", "--oil-line-strong"],
] as const;

const consentItems = [
  {
    id: "record",
    label: "我同意本次面试全程录音录像",
    detail: "录像和录音只用于本次招聘的评估，由招聘方保存，到期删除。",
  },
  {
    id: "ai",
    label: "我知道面试官是 AI，结果由招聘人员复核",
    detail: "AI 负责提问和整理记录，是否进入下一轮由招聘人员决定。",
  },
  { id: "alone", label: "我会独立作答，不使用他人或工具代答" },
];

const people = [
  {
    name: "林知远",
    job: "后端工程师（Rust）",
    state: "已完成",
    tone: "success",
    score: 86,
    when: "今天 10:20",
  },
  {
    name: "苏晚晴",
    job: "产品经理",
    state: "进行中",
    tone: "info",
    score: null,
    when: "今天 14:05",
  },
  {
    name: "陈墨",
    job: "数据分析师",
    state: "待开始",
    tone: "neutral",
    score: null,
    when: "链接 3 天后过期",
  },
  {
    name: "周予安",
    job: "前端工程师",
    state: "已过期",
    tone: "warning",
    score: null,
    when: "昨天",
  },
  {
    name: "顾清和",
    job: "测试工程师",
    state: "不推荐",
    tone: "danger",
    score: 41,
    when: "11 月 11 日",
  },
] as const;

function Section({
  id,
  title,
  note,
  children,
}: {
  id: string;
  title: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="g-section" id={id}>
      <div className="g-section__head">
        <h2 className="oil-title-sm">
          <span className="oil-brushed">{title}</span>
        </h2>
        {note && <p className="oil-muted">{note}</p>}
      </div>
      {children}
    </section>
  );
}

function ToastDemo() {
  const toast = useToast();
  return (
    <Button variant="secondary" onClick={() => toast("面试链接已复制")}>
      弹一条提示
    </Button>
  );
}

function Gallery() {
  const [theme, setTheme] = useState<Theme>(() => readTheme());
  const [tab, setTab] = useState("all");
  const [effort, setEffort] = useState<"low" | "high" | "max">("high");
  const [minutes, setMinutes] = useState(30);
  const [silence, setSilence] = useState(5);
  const [thinking, setThinking] = useState(true);
  const [dialog, setDialog] = useState(false);
  const [consent, setConsent] = useState<Set<string>>(new Set(["record"]));
  const [presence, setPresence] = useState<PresenceState>("speaking");
  const [levels, setLevels] = useState<number[]>(() => Array.from({ length: 24 }, () => 0.2));

  useEffect(() => applyTheme(theme), [theme]);
  useEffect(() => {
    const t = window.setInterval(() => {
      setLevels((prev) =>
        prev.map((_, i) => Math.max(0, Math.sin(Date.now() / 300 + i * 0.6) * 0.5 + 0.45)),
      );
    }, 120);
    return () => window.clearInterval(t);
  }, []);

  return (
    <div className="g-page">
      <Hero painting="dusk" className="g-hero">
        <div className="oil-eyebrow">AI 面试 · 设计系统</div>
        <h1 className="oil-title-xl" style={{ margin: "10px 0 14px" }}>
          画室
        </h1>
        <p className="oil-muted" style={{ fontSize: "1rem", maxWidth: "36ch" }}>
          亚麻画布做底，生褐做墨，普鲁士蓝是主色，朱红和土黄点睛。HR 后台和候选人面试页共用这一套。
        </p>
        <div className="oil-row" style={{ marginTop: 22 }}>
          <Button
            variant="gold"
            icon={theme === "dark" ? <Sun /> : <Moon />}
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
          >
            {theme === "dark" ? "换到画布（浅色）" : "换到夜画（深色）"}
          </Button>
        </div>
      </Hero>

      <Section
        id="color"
        title="颜料"
        note="装饰直接用颜料色；文字和状态用语义令牌，两套主题里各有取值。"
      >
        <div className="g-swatches">
          {pigments.map(([name, v]) => (
            <div className="g-swatch" key={v}>
              <span className="g-swatch__dab" style={{ background: `var(${v})` }} />
              <strong>{name}</strong>
              <code>{v.replace("--oil-pigment-", "")}</code>
            </div>
          ))}
        </div>
        <div className="g-chips">
          {surfaces.map(([name, v]) => (
            <div className="g-chipcolor" key={v}>
              <span style={{ background: `var(${v})` }} />
              {name}
            </div>
          ))}
        </div>
      </Section>

      <Section
        id="type"
        title="字体"
        note="标题用自托管、按字切片的宋体；正文用系统字体，不走外网。"
      >
        <Card>
          <CardBody className="oil-stack">
            <div className="oil-display oil-title-xl">第一场面试，从一幅画开始</div>
            <div className="oil-display oil-title-lg">候选人画像 Candidate Profile 2026</div>
            <div className="oil-display oil-title-md">面试设置与考察重点</div>
            <div className="oil-display oil-title-sm">带时间戳的转写记录</div>
            <p style={{ maxWidth: "62ch" }}>
              正文：这是招聘流程里的第一场面试。AI
              面试官按岗位要求和候选人简历准备题目，逐题提问，候选人口头作答，全程录音录像，结束后生成分析报告供招聘人员复核。
            </p>
            <p className="oil-muted">次要文字：链接 7 天内有效，候选人不需要登录。</p>
            <p className="oil-mono oil-faint">00:12:36 → 00:13:02 · deepseek-flash · effort=high</p>
          </CardBody>
        </Card>
      </Section>

      <Section id="buttons" title="按钮" note="一块厚涂的颜料：有笔触起伏和受光面。">
        <Card>
          <CardBody className="oil-stack">
            <div className="oil-row oil-row--wrap">
              <Button icon={<Plus />}>新建面试</Button>
              <Button variant="accent" icon={<Send />}>
                发送链接
              </Button>
              <Button variant="gold">生成题目</Button>
              <Button variant="secondary">取消</Button>
              <Button variant="ghost">查看详情</Button>
              <Button variant="danger">作废链接</Button>
              <Button disabled>不可用</Button>
              <Button loading>保存中</Button>
            </div>
            <div className="oil-row oil-row--wrap">
              <Button size="sm">小按钮</Button>
              <Button size="sm" variant="secondary">
                小按钮
              </Button>
              <Button size="lg" variant="accent">
                开始面试
              </Button>
              <Button variant="secondary" iconOnly aria-label="设置">
                <Settings2 />
              </Button>
              <ToastDemo />
              <Button variant="secondary" onClick={() => setDialog(true)}>
                打开对话框
              </Button>
            </div>
          </CardBody>
        </Card>
      </Section>

      <Section id="form" title="表单">
        <div className="g-two">
          <Card>
            <CardHeader title="面试设置" actions={<Badge tone="info">草稿</Badge>} />
            <CardBody className="oil-stack">
              <Field label="岗位" required>
                {({ id }) => (
                  <Select id={id} defaultValue="rust">
                    <option value="rust">后端工程师（Rust）</option>
                    <option value="pm">产品经理</option>
                  </Select>
                )}
              </Field>
              <Field
                label="预期时长"
                hint="题目数量和追问深度会按时长安排；答得很差时可以提前结束。"
              >
                {({ id }) => (
                  <Slider
                    id={id}
                    value={minutes}
                    min={10}
                    max={60}
                    step={5}
                    onChange={setMinutes}
                    format={(v) => `${v} 分钟`}
                  />
                )}
              </Field>
              <Field label="静音判定" hint="候选人停下来这么久，算这一题答完。">
                {({ id }) => (
                  <Slider
                    id={id}
                    value={silence}
                    min={2}
                    max={15}
                    onChange={setSilence}
                    format={(v) => `${v} 秒`}
                  />
                )}
              </Field>
              <div className="oil-row oil-row--between oil-row--wrap">
                <Switch
                  label="开启思考模式"
                  checked={thinking}
                  onChange={(e) => setThinking(e.target.checked)}
                />
                <Segmented
                  aria-label="思考强度"
                  value={effort}
                  onChange={setEffort}
                  options={[
                    { value: "low", label: "低" },
                    { value: "high", label: "高" },
                    { value: "max", label: "最高" },
                  ]}
                />
              </div>
              <Field label="额外考察重点" hint="会注入给面试官，例如：一定要出一道逻辑推理题。">
                {({ id }) => (
                  <Textarea
                    id={id}
                    defaultValue="重点核实简历里「日均千万级请求」项目的真实参与程度。"
                  />
                )}
              </Field>
            </CardBody>
            <CardFooter>
              <Button variant="secondary">存为模板</Button>
              <Button>保存设置</Button>
            </CardFooter>
          </Card>
          <div className="oil-stack">
            <Card>
              <CardBody className="oil-stack">
                <Field label="搜索候选人">
                  {({ id }) => (
                    <div className="oil-input-group">
                      <Search aria-hidden="true" />
                      <Input id={id} placeholder="姓名、手机号或岗位" />
                    </div>
                  )}
                </Field>
                <Field label="手机号" error="手机号格式不对">
                  {({ id, describedBy }) => (
                    <Input
                      id={id}
                      defaultValue="1380000"
                      aria-invalid="true"
                      aria-describedby={describedBy}
                    />
                  )}
                </Field>
                <div
                  className="oil-row oil-row--wrap"
                  style={{ "--oil-gap": "20px" } as React.CSSProperties}
                >
                  <Checkbox label="克隆音色" defaultChecked />
                  <Checkbox label="录制视频" defaultChecked />
                  <Radio label="IndexTTS 2" name="tts" defaultChecked />
                  <Radio label="BreezeTTS2" name="tts" />
                </div>
                <Dropzone
                  onFiles={() => {}}
                  accept=".pdf"
                  title="上传简历"
                  hint="PDF，由大模型直接解析成候选人画像"
                />
                <Field label="面试链接">
                  <CopyField value="https://interview.example.com/i/7Qm2xK9vTz4bLp8w" />
                </Field>
              </CardBody>
            </Card>
          </div>
        </div>
      </Section>

      <Section id="data" title="数据">
        <div className="g-stats">
          {[
            ["本周面试", 38, "场", "比上周多 6 场"],
            ["完成率", 82, "%", "31 场完成"],
            ["平均时长", 24, "分钟", "预期 30 分钟"],
            ["待复核", 7, "份", "最早的等了 2 天"],
          ].map(([label, value, unit, delta]) => (
            <Card key={label as string}>
              <CardBody>
                <Stat label={label} value={value} unit={unit} delta={delta} />
              </CardBody>
            </Card>
          ))}
        </div>
        <Card style={{ marginTop: 16 }}>
          <div style={{ padding: "4px 20px 0" }}>
            <Tabs
              aria-label="面试状态"
              value={tab}
              onChange={setTab}
              items={[
                { value: "all", label: "全部", count: 38 },
                { value: "todo", label: "待开始", count: 5 },
                { value: "done", label: "已完成", count: 31 },
                { value: "review", label: "待复核", count: 7 },
              ]}
            />
          </div>
          <Table>
            <thead>
              <tr>
                <th>候选人</th>
                <th>岗位</th>
                <th>状态</th>
                <th>综合分</th>
                <th>时间</th>
              </tr>
            </thead>
            <tbody>
              {people.map((p) => (
                <tr key={p.name}>
                  <td>
                    <div className="oil-row">
                      <Avatar name={p.name} />
                      <strong>{p.name}</strong>
                    </div>
                  </td>
                  <td>{p.job}</td>
                  <td>
                    <Badge tone={p.tone}>{p.state}</Badge>
                  </td>
                  <td className="oil-num">{p.score ?? <span className="oil-faint">—</span>}</td>
                  <td className="oil-muted">{p.when}</td>
                </tr>
              ))}
            </tbody>
          </Table>
        </Card>
        <div className="g-two" style={{ marginTop: 16 }}>
          <Card>
            <CardHeader
              title="各维度评分"
              actions={
                <Ring value={0.86} size={56} thickness={6} color="var(--oil-success)">
                  86
                </Ring>
              }
            />
            <CardBody className="oil-stack">
              <ScoreBar label="专业能力" value={88} />
              <ScoreBar label="项目经历真实性" value={74} />
              <ScoreBar label="沟通表达" value={91} />
              <ScoreBar label="逻辑推理" value={62} />
              <ScoreBar label="岗位匹配" value={43} />
            </CardBody>
          </Card>
          <Card>
            <CardHeader title="候选人画像" />
            <CardBody className="oil-stack">
              <div className="oil-row">
                <Avatar name="林知远" size="xl" />
                <div>
                  <div className="oil-display oil-title-sm">林知远</div>
                  <div className="oil-muted">6 年后端经验 · 杭州</div>
                </div>
              </div>
              <KeyValue
                items={[
                  ["应聘岗位", "后端工程师（Rust）"],
                  ["联系方式", "13800000001 · lin@example.com"],
                  [
                    "技能",
                    <span
                      className="oil-row oil-row--wrap"
                      style={{ "--oil-gap": "6px" } as React.CSSProperties}
                      key="s"
                    >
                      <Chip>Rust</Chip>
                      <Chip>Tokio</Chip>
                      <Chip>PostgreSQL</Chip>
                      <Chip>分布式</Chip>
                    </span>,
                  ],
                  ["简历解析", <Progress value={0.7} tone="gold" label="简历解析进度" key="p" />],
                ]}
              />
              <Banner tone="warning" title="有一处需要核实">
                简历写「主导」支付网关重构，面试中描述的职责偏执行。
              </Banner>
            </CardBody>
          </Card>
        </div>
        <div className="g-two" style={{ marginTop: 16 }}>
          <div className="oil-stack">
            <Banner title="题目已生成">10 道题的语音已合成完，可以发送链接。</Banner>
            <Banner tone="success" title="面试完成">
              录像和转写已保存，报告生成中。
            </Banner>
            <Banner tone="danger" title="语音合成失败">
              TTS 服务没有响应，稍后重试。
            </Banner>
            <Card>
              <CardBody className="oil-stack">
                <Steps steps={["选候选人和岗位", "面试设置", "生成题目", "发送链接"]} current={2} />
                <Progress value={0.62} />
                <div className="oil-row">
                  <Skeleton width={120} />
                  <Skeleton width={220} />
                  <Skeleton width={60} />
                </div>
              </CardBody>
            </Card>
          </div>
          <Card>
            <EmptyState
              title="还没有候选人"
              description="上传一份简历，或者从 RAAS 导入，就可以开始安排面试。"
              actions={<Button icon={<Plus />}>添加候选人</Button>}
            />
          </Card>
        </div>
      </Section>

      <Section
        id="paintings"
        title="插画"
        note="都是程序画出来的：先铺色块，再沿着边缘一笔一笔铺笔触，最后按颜料厚度打侧光。静态图片，页面上不跑滤镜。"
      >
        <div className="g-paintings">
          {(Object.keys(paintings) as PaintingName[]).map((name) => (
            <figure key={name}>
              <Painting
                name={name}
                ratio={name === "dusk" ? "5 / 2" : name === "palette" ? "1 / 2" : "4 / 3"}
              />
              <figcaption className="oil-mono oil-faint">{name}</figcaption>
            </figure>
          ))}
        </div>
      </Section>

      <Section
        id="shell"
        title="后台骨架"
        note="侧栏的底是调色盘上搅开的颜料，当前项后面是一笔土黄。"
      >
        <div className="g-shell">
          <aside className="oil-sidebar" style={{ position: "relative", height: "auto" }}>
            <Brand mark="面" name="AI 面试" sub="招聘工作台" />
            <Nav>
              <a className={navItemClass} href="#shell">
                <LayoutDashboard /> 总览
              </a>
              <a className={navItemClass} href="#shell" aria-current="page">
                <Video /> 面试 <NavBadge>7</NavBadge>
              </a>
              <a className={navItemClass} href="#shell">
                <Users /> 候选人
              </a>
              <a className={navItemClass} href="#shell">
                <Briefcase /> 岗位
              </a>
              <NavSection>配置</NavSection>
              <a className={navItemClass} href="#shell">
                <Settings2 /> 面试模板
              </a>
              <a className={navItemClass} href="#shell">
                <FileText /> 报告
              </a>
            </Nav>
          </aside>
          <div style={{ padding: 28, minWidth: 0 }}>
            <PageHeader
              crumbs={
                <>
                  <a href="#shell">面试</a> / 新建
                </>
              }
              title="新建面试"
              description="选一位候选人和一个岗位，调好设置，生成题目后把链接发给候选人。"
              actions={
                <>
                  <Button variant="secondary">存为草稿</Button>
                  <Button variant="accent" icon={<Send />}>
                    生成并发送
                  </Button>
                </>
              }
            />
            <Card>
              <CardBody>
                <Steps steps={["选候选人和岗位", "面试设置", "生成题目", "发送链接"]} current={1} />
              </CardBody>
            </Card>
          </div>
        </div>
      </Section>

      <Section
        id="interview"
        title="候选人面试页"
        note="手机优先。面试进行中是深色舞台，动效只动 transform 和 opacity。"
      >
        <div className="g-phones">
          <div className="g-phone">
            <Stage
              header={<Steps compact steps={["须知", "设备", "同意", "开始"]} current={1} />}
              footer={
                <Button size="lg" block variant="accent">
                  下一步
                </Button>
              }
            >
              <div>
                <h1 className="oil-title-md">检查设备</h1>
                <p className="oil-muted" style={{ marginTop: 6 }}>
                  面试需要摄像头和麦克风。
                </p>
              </div>
              <DeviceCheckCard
                state="passed"
                icon={<Camera />}
                title="摄像头"
                description="画面正常"
              >
                <VideoFrame
                  weight="thin"
                  mirror
                  placeholder="这里是你的画面"
                  topLeft={<RecordingIndicator>预览</RecordingIndicator>}
                />
              </DeviceCheckCard>
              <DeviceCheckCard
                state="checking"
                icon={<Mic />}
                title="麦克风"
                description="请说一句话试试"
              >
                <LevelMeter levels={levels} />
              </DeviceCheckCard>
              <DeviceCheckCard
                state="failed"
                icon={<Wifi />}
                title="网络"
                description="连接不稳定，换个网络再试"
              >
                <Button variant="secondary" size="sm">
                  重新检测
                </Button>
              </DeviceCheckCard>
            </Stage>
          </div>

          <div className="g-phone">
            <Stage
              header={<Steps compact steps={["须知", "设备", "同意", "开始"]} current={2} />}
              footer={
                <Button
                  size="lg"
                  block
                  variant="accent"
                  disabled={!allConsented(consentItems, consent)}
                >
                  同意并开始面试
                </Button>
              }
            >
              <div>
                <h1 className="oil-title-md">开始之前</h1>
                <p className="oil-muted" style={{ marginTop: 6 }}>
                  后端工程师（Rust）· 约 30 分钟 · 10 道题
                </p>
              </div>
              <ConsentList items={consentItems} checked={consent} onChange={setConsent} />
              <Banner>面试中请保持正对镜头，周围安静。答完一题停顿几秒，会自动进入下一题。</Banner>
            </Stage>
          </div>

          <div className="g-phone">
            <Stage
              night
              header={
                <>
                  <RecordingIndicator />
                  <div className="oil-grow" />
                  <Timer seconds={756} />
                </>
              }
              footer={
                <Button block variant="secondary" size="lg">
                  我答完了，下一题
                </Button>
              }
            >
              <QuestionProgress total={10} current={3} />
              <InterviewerPresence state={presence} />
              <CaptionCard meta="第 4 题 / 共 10 题" aside="建议 2 分钟">
                请讲一个你主导过的性能优化：当时的瓶颈在哪，你是怎么定位的？
              </CaptionCard>
              <div className="oil-row" style={{ justifyContent: "center" }}>
                <SilenceRing remaining={3.2} total={5} />
                <LevelMeter levels={levels.slice(0, 14)} />
              </div>
              <Segmented
                aria-label="面试官状态（演示）"
                value={presence}
                onChange={setPresence}
                options={[
                  { value: "speaking", label: "提问" },
                  { value: "listening", label: "听" },
                  { value: "thinking", label: "思考" },
                ]}
              />
            </Stage>
          </div>

          <div className="g-phone">
            <Stage>
              <StatusPage
                painting="meadow"
                title="面试完成，谢谢你"
                description="录像已经保存。招聘人员看过之后会联系你，一般在 3 个工作日内。"
                actions={
                  <Button variant="secondary" size="lg" block>
                    关闭页面
                  </Button>
                }
              />
            </Stage>
          </div>

          <div className="g-phone">
            <Stage>
              <StatusPage
                painting="nocturne"
                title="链接已过期"
                description="这条面试链接已经失效。请联系发给你链接的招聘人员，重新获取。"
              />
            </Stage>
          </div>
        </div>
      </Section>

      <Dialog
        open={dialog}
        onClose={() => setDialog(false)}
        title="作废这条面试链接？"
        footer={
          <>
            <Button variant="secondary" onClick={() => setDialog(false)}>
              先不作废
            </Button>
            <Button variant="danger" onClick={() => setDialog(false)}>
              作废链接
            </Button>
          </>
        }
      >
        <p>作废之后候选人打开链接会看到「链接已失效」。已经录下的内容会保留。</p>
      </Dialog>
    </div>
  );
}

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <ToastProvider>
        <Gallery />
      </ToastProvider>
    </StrictMode>,
  );
}
