import { buttonClass, Card, CardBody, CardHeader } from "@ai-interview/ui";
import type { ReactNode } from "react";
import { Link } from "react-router";
import { useMe } from "../api/queries";
import { PageHeader } from "../components/PageHeader";

// 这一页上写的都要和后端实际做的对得上。改了后端的行为（面试状态怎么流转、链接、续面、报告）
// 或者改了页面上按钮和页签的名字，回来看一眼这里要不要跟着改。

interface Step {
  title: string;
  body: ReactNode;
  /** 这一步在哪一页做 */
  to?: string;
  action?: string;
}

const STEPS: Step[] = [
  {
    title: "准备候选人",
    body: (
      <>
        在「候选人」页上传简历，PDF、Word、图片、纯文本都可以。系统读完会整理出姓名、联系方式和经历，姓名和联系方式读错了可以在候选人页里「改基本信息」。面试官出题时读的是简历原文，所以没有简历也能面，只是题目不会结合他的经历。
      </>
    ),
    to: "/candidates",
    action: "去候选人页",
  },
  {
    title: "准备岗位",
    body: (
      <>
        在「岗位」页新建岗位。手头有现成的岗位描述，贴进去点「自动整理」就行。岗位职责和任职要求 AI
        面试官都看得到，也可能说给候选人听，不想让候选人知道的内容（比如内部的薪资底线）不要写在这两栏。部署时接了
        RAAS 的话，岗位页和候选人页上会多出「从 RAAS 导入」。
      </>
    ),
    to: "/positions",
    action: "去岗位页",
  },
  {
    title: "安排面试",
    body: (
      <>
        选候选人和岗位（可以一次选几位），定好面试设置，点「生成题目和语音」。出题和合成语音一般要一两分钟，这段时间可以去做别的，回来在面试详情页看进度。常用的设置可以在「面试设置」页存成模板，下次直接选。
      </>
    ),
    to: "/interviews/new",
    action: "安排面试",
  },
  {
    title: "看一眼题目，把链接发给候选人",
    body: (
      <>
        准备好以后，面试详情页的「题目」页签列出面试官要说的每一句：可以改措辞、去掉不想问的题、加自己的题，改的那一句会单独重新合成；时间不够时哪些题可以跳过、题目的先后、哪些题留作备用也能调。没问题就到「链接」页签生成链接，把「给候选人的话」整段复制，用微信、邮件或短信发给候选人。完整的链接只显示这一次，离开页面以后只能换一个新的。
      </>
    ),
    to: "/interviews",
    action: "去面试列表",
  },
  {
    title: "看回看和报告，记下你的决定",
    body: (
      <>
        候选人面完以后报告自己开始生成，一般一两分钟。「回看」页签里有录像和逐字稿，点哪句话就跳到哪里；「报告」页签是
        AI
        的初筛意见。看完在详情页右边记下决定：进入下一轮、待定或者不推进。记成「进入下一轮」以后，页面会给出「安排下一轮」，设置按报告的建议改好，你只需要确认。
      </>
    ),
    to: "/interviews",
    action: "去面试列表",
  },
];

interface Topic {
  title: string;
  body: ReactNode;
}

const SETTINGS: Topic[] = [
  {
    title: "面试方式",
    body: "「逐题问答」是题目和语音都提前准备好，一题一题问，候选人答完再问下一题。「实时对话」题目照样提前出好，但面试官听完回答会现场决定追问还是问下一题，候选人也可以插话。实时对话需要大模型、语音合成、语音识别三样都在正常工作，缺一样这场面试就按逐题问答进行。",
  },
  {
    title: "预期时长和题目数量",
    body: "预期时长决定出多少题、每题聊多深。默认按时长自动安排题目数量，也可以自己指定。",
  },
  {
    title: "考察要求",
    body: "「重点考察」写想多了解的方向；「必须问的题」面试官会照你写的问，时间紧的时候先保证问到这些；「要确认的基本条件」比如能不能出差、到岗时间；「不要问的话题」面试官会避开。",
  },
  {
    title: "面试官的声音",
    body: "在「声音」页添加和试听音色，安排面试时选用哪一个。不选就用默认音色。",
  },
  {
    title: "模型怎么想",
    body: "可以分别给出题和面试进行中打开思考、调强度。强度越高想得越细，也越慢；面试进行中调高会让候选人等得更久。",
  },
];

const CANDIDATE_SIDE: string[] = [
  "打开链接就能面，不用下载软件，也不用注册登录。电脑和手机都可以，要用最新版的 Chrome、Edge 或 Safari。",
  "不能在微信、钉钉这类聊天软件里直接点开，那里面录不了音和像。页面会提示候选人复制链接到浏览器里打开，「给候选人的话」里也写了。",
  "候选人先看到面试说明和需要同意的事项（面试会录像），同意以后检查摄像头、麦克风和扬声器，然后点「开始面试」。",
  "面试官用语音提问，候选人对着镜头回答，全程录像。设置里开着「最后请候选人提问」的话，问完以后候选人可以向公司提问，面试官记下来，报告里单独有一栏。",
  "中途刷新了页面、断了网或者换了设备，重新打开同一条链接可以接着面。",
  "页面超过 15 分钟没有任何动静（这个时间部署时可以调），这场面试就按中断结束。",
];

const REPORT: Topic[] = [
  {
    title: "结论和综合分",
    body: "最上面是建议（强烈建议推进、建议推进、拿不准、不建议推进）和综合分。分数后面括号里的范围，是同一场面试重新生成报告时分数大多会落在的区间：两位候选人只差几分，不说明谁更好。",
  },
  {
    title: "每条判断都有出处",
    body: "判断下面列着候选人的原话或者简历里的句子。点原话前面的时间，就跳到回看里那一刻，可以自己听一遍。",
  },
  {
    title: "诚信观察是迹象，不是结论",
    body: "这一栏列的是值得你去回看里核对的地方，比如画面里没有人、回答像在念稿。每一条都写了别的可能解释，核对之前不要直接当成作弊。",
  },
  {
    title: "局限",
    body: "报告最后写着这份报告哪些地方把握不大，比如语音识别可能有错字、有的题没问到、录像不完整。",
  },
  {
    title: "报告是参考，决定由你做",
    body: "报告可以重新生成，也可以打印或存成 PDF。同一个岗位有两位以上候选人出了报告，岗位页上会出现「对比候选人」；面试列表可以「导出表格」。",
  },
];

function faq(isAdmin: boolean): Topic[] {
  return [
    {
      title: "链接过期了，候选人还没面",
      body: "打开这场面试的「链接」页签，换一个新链接再发给他。一个岗位有好几位都过期了，在岗位页「这个岗位的面试」上面可以一起换。发了新链接，旧的那条就打不开了。",
    },
    {
      title: "候选人说打不开，或者没有画面、没有声音",
      body: "先确认他不是在微信、钉钉里直接点开的；再确认浏览器弹出来问能不能用摄像头和麦克风时点了允许；还不行就换最新版的 Chrome 或 Edge 再试。",
    },
    {
      title: "候选人面到一半断了",
      body: "让他重新打开同一条链接接着面。要是隔了太久，这场面试已经按中断结束，就不能再发链接了：在面试详情页点「再安排一场」，人、岗位、设置和声音都沿用。",
    },
    {
      title: "准备失败了",
      body: (
        <>
          面试详情页会说明是题目没出来还是语音没合成出来，点「重试」就行，已经做好的部分不会重做。语音引擎或者大模型用不了的原因在
          <Link to="/settings">「系统」页</Link>里。
        </>
      ),
    },
    {
      title: "链接已经发出去了，还想改题目或设置",
      body: "先在「链接」页签把链接作废，面试回到待发送，就可以改了。改完再生成一个新链接发给候选人。",
    },
    {
      title: "选了实时对话，面的时候却是一题一题问",
      body: (
        <>
          候选人点「开始面试」的那一刻，大模型、语音合成、语音识别有一样用不了，这场面试就按逐题问答进行。
          <Link to="/settings">「系统」页</Link>里能看到哪一样不正常。
        </>
      ),
    },
    {
      title: "不想在列表里再看到某场面试或某位候选人",
      body: "用「归档」。归档只是从列表里藏起来，录像和报告都还在，随时可以恢复。「彻底删除」会把录像、逐字稿和报告一起删掉，不能恢复，只有管理员能做。",
    },
    {
      title: "改密码，给同事开账号",
      body: isAdmin ? (
        <>
          点左下角自己的名字进<Link to="/account">「我的账号」</Link>改密码。你是管理员，可以在
          <Link to="/accounts">「账号」页</Link>
          给同事开账号、停用离职同事的账号、帮忘了密码的人重设密码。
        </>
      ) : (
        <>
          点左下角自己的名字进<Link to="/account">「我的账号」</Link>
          改密码。开账号、停用账号、重设别人的密码要找管理员。
        </>
      ),
    },
  ];
}

function Topics({ items }: { items: Topic[] }) {
  return (
    <dl className="adm-help__topics">
      {items.map((item) => (
        <div key={item.title}>
          <dt>{item.title}</dt>
          <dd>{item.body}</dd>
        </div>
      ))}
    </dl>
  );
}

/** 使用说明：把一场面试从头到尾讲一遍，给第一次用后台的人看。 */
export function HelpPage() {
  const isAdmin = useMe().data?.role === "admin";
  return (
    <>
      <PageHeader
        title="使用说明"
        description="从准备候选人到看报告，一场 AI 面试怎么走完；遇到常见的情况怎么办。"
      />
      <div className="oil-stack adm-help">
        <Card>
          <CardHeader title="一场面试怎么走" />
          <CardBody>
            <ol className="adm-list" aria-label="一场面试怎么走">
              {STEPS.map((step, index) => (
                <li key={step.title} className="adm-item adm-item--wrap adm-help__step">
                  <span className="adm-step__dot" aria-hidden="true">
                    {index + 1}
                  </span>
                  <div className="adm-item__main">
                    <h3 className="oil-title-sm">{step.title}</h3>
                    <p>{step.body}</p>
                  </div>
                  {step.to && (
                    <Link
                      to={step.to}
                      className={buttonClass({ size: "sm", variant: "secondary" })}
                    >
                      {step.action}
                    </Link>
                  )}
                </li>
              ))}
            </ol>
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="面试设置里常改的几项" />
          <CardBody>
            <Topics items={SETTINGS} />
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="候选人那边是什么样" />
          <CardBody>
            <ul className="adm-bullets adm-help__text">
              {CANDIDATE_SIDE.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="报告怎么读" />
          <CardBody>
            <Topics items={REPORT} />
          </CardBody>
        </Card>

        <Card>
          <CardHeader title="常见情况" />
          <CardBody>
            <Topics items={faq(isAdmin)} />
          </CardBody>
        </Card>
      </div>
    </>
  );
}
