# HR 管理后台

招聘同事用的后台：安排面试、管理候选人和岗位、调面试设置和声音、回看面试、看分析报告。界面用 `@ai-interview/ui` 的油画风格设计系统。

## 运行

```bash
pnpm -C web install
scripts/dev.sh                                        # 先起后端（127.0.0.1:8080），终端里会打印本机的管理员账号
pnpm -C web --filter @ai-interview/admin dev          # http://localhost:5173/admin/，/api 转给本机后端
pnpm -C web --filter @ai-interview/admin dev:demo     # 不用后端，用演示数据
pnpm -C web --filter @ai-interview/admin test         # 单元测试
pnpm -C web --filter @ai-interview/admin build        # 产物在 web/dist/admin，由 interview-server 挂在 /admin/
```

后台默认请求真实后端。所有请求都走 `@ai-interview/api-client` 的客户端（`src/api/http.ts` 里的 `api`），路径、参数和返回的类型由后端的 OpenAPI 文档生成，后端改了接口这里编译就会报错。

页面顶上的标题用 `src/components/PageHeader.tsx` 里的 `PageHeader`（不是设计系统里的那个），它顺带把浏览器标签页的标题换成这一页的名字。没有标题栏的页面放一个 `<PageTitle title="…" />`。

登录用到一半失效了（会话过期、密码改过），请求会返回 401「请先登录」。这时不回登录页，`src/api/session.ts` 记下来，`ReLoginDialog` 在当前页面上让人再输一次密码，页面上填了一半的内容不丢。本来就没登录的照常去登录页。

带表单的弹窗（改岗位、开账号、改题目等）在填了还没保存的时候，按 Esc、点到弹窗外面不关，只提示一句内容还没保存，要点「取消」或右上角的关闭才关。做法是给设计系统的 `Dialog` 传 `unsaved`，用 `src/lib/form.ts` 的 `sameForm` 比现在的内容和打开时的内容。新加带表单的弹窗时也接上。只显示一次的密码（开账号、重设密码以后）用 `persistent`，必须点「好了」才关。

整页的表单（新建和编辑模板、安排面试）填了还没保存时，离开之前先问一句：在后台里换页面（侧栏、链接、浏览器后退）弹出来问，默认留下；刷新、关标签页时浏览器弹它自带的提示。做法是 `src/components/LeaveGuard.tsx` 的 `useLeaveGuard(有没有改过)`，把它给的 `dialog` 渲染出来，自己要跳走时（点取消、保存成功以后）先调 `allow()`。它靠路由的拦截功能，所以 `main.tsx` 用的是 `createBrowserRouter`（页面还是写在 `App` 的 `<Routes>` 里）；测试里渲染这两页要用 `src/test/router.tsx` 的 `TestRouter`，放在 `MemoryRouter` 里会报错。

## 演示数据

`dev:demo` 和 `build:demo`（也就是 `--mode demo`，或者设 `VITE_API_MOCK=1`）会把请求交给浏览器里的一套假后端（`src/api/mock/`），右上角显示「演示数据」。这时邮箱和密码随便填都能进（密码填 `wrong` 可以看登录失败的样子），所有改动只存在内存里，刷新就没了。正常构建的产物里不带假后端。

演示数据里的面试、逐字稿和报告来自分析报告线的合成样例（`crates/interview-report/samples/`），人名、手机号、邮箱都是编的。假后端返回的结构和真实接口一样，测试（`src/api/mock/server.test.ts`）也是对着它跑的。

演示数据里没有语音模型，试听和参考录音播放的是一段提示音，页面顶上会有一句说明。真实后端没连上 Mac 上的模型时也是这样（接口会返回 `simulated: true`）。

试听接口（`POST /speech/preview`）能临时换引擎、语言和语气，逐字稿却只认音色上存着的那份。所以修改克隆音色时，现在选的引擎要用逐字稿（BreezeTTS2）、填的又和存着的不一样，点「试听」会先单独把逐字稿存到音色上再合成，页面上有一句说明；逐字稿还没填时「试听」点不了。

不用 Mac 也能在真实后端上把「声音」页走一遍：`python3 tts/worker/worker.py --engines index2,index25,breeze2 --fake --port 7800` 起一个不加载模型的 worker，后端带上 `AI_INTERVIEW_TTS_BACKEND=worker AI_INTERVIEW_TTS_WORKER_URLS=http://127.0.0.1:7800` 启动，三个引擎就都显示可用。合成出来的还是提示音，但走的是和真模型一样的路。

假后端里没有真的录像文件，回看只能按时间轴放逐字稿。分段录像的连续播放（`src/lib/clock.ts`）要连真后端才看得到。

## 目录

- `src/api/`：接口类型（`types.ts`）、请求（`http.ts`、`queries.ts`）、登录用到一半失效时怎么办（`session.ts`）、假后端（`mock/`）。
- `src/pages/`：一个页面一个文件。
- `src/components/`：页面之间共用的部件，比如面试设置表单（`ConfigForm`）、回看播放器（`ReplayPlayer`）、报告（`ReportView`）。
- `src/lib/`：枚举到中文文案的对照表、时间和时长的格式化、按设置估算题目数量（`budget.ts`，算法和后端一致）、回看的时钟（`clock.ts`）。
- `src/admin.css`：后台自己的排版。颜色、间距、字号只用设计系统的 `--oil-*` 变量。

## 约定

- 接口返回的每个枚举值都要在 `src/lib/format.ts` 里有中文文案。`src/api/mock/samples.test.ts` 会拿报告线的样例检查，样例里出现没有文案的值测试就失败。
- 录像不完整（候选人没传完，后端把收到的部分合并出来）或者有一段没保存下来时，原因在录像的 `error` 字段里，后端已经写成给 HR 看的话。回看页用 `src/lib/recordings.ts` 的 `recordingNotes` 原样显示在播放器下面，不要自己再编一套说法。
- 候选人的上行网速慢时，面试页会有意把录像的画面降下来（`interview_started` 事件里 `video_reduced` 为 true）。回看页在播放器下面说明画面糊不是摄像头的问题（`reducedVideoNote`），过程记录里那一条「面试开始」后面也写着降到了多少。
- 报告里的判断下面跟着原话，点一下跳到回看的那个时间。新加的报告内容也照这个做，简历里的原文没有时间，不做成可点的。
- 页面的脚本跑起来之前，`index.html` 里先有一句「正在打开招聘工作台…」；脚本取不到、一上来就报错或者十几秒还没好，`<head>` 里那一小段不经过打包的脚本把它换成说明和「刷新页面」（`src/boot.test.ts` 照着 `index.html` 跑一遍）。这一段要排在页面的脚本前面，用老写法（`var`、`function`），不要引用打包后的东西。
- 某一页画到一半抛了错（多半是后端返回了页面没料到的内容），只有这一页换成一句说明，侧栏和别的页面照常用：`src/components/PageError.tsx` 的 `PageErrorBoundary` 包在外框的 `<Outlet />` 外面，换了地址自己复位，「再试一次」先扔掉没人在用的查询结果再重画。外框自己出错时是路由上的 `AppCrashed`（`main.tsx` 和测试用的 `TestRouter` 都挂了）。新加的页面不用管这件事；不要在页面里自己接住渲染错误再显示空白。
- 面试结果可以导出成表格（`src/components/ExportInterviews.tsx`，「面试」列表和岗位页都有）：`fetchAllInterviews` 把符合筛选条件的面试一页一页取完，不只是眼前这一页；`src/lib/export.ts` 的 `csv` 开头带 BOM（没有的话 Excel 里中文是乱码），以 `=`、`+`、`-`、`@` 开头的格子前面加单引号，不让 Excel 当公式算。再加别的导出也用这两个函数，不要自己拼逗号。页面测试里用 `src/test/download.ts` 的 `catchDownload` 接住存下来的文件。
- 报告里「候选人向公司提的问题」（`report.candidate_questions`）有三种情况，别混：数组里有东西就一条条列出来；空数组是候选人没有提问；没有这个字段（`null`）是这一步没成功或者是旧报告，不能写成「没有提问」。面试官是 AI，它当场答了的那些要写明不是公司的正式答复；「说会转告」和「没有回应」的算等 HR 回复的。这一栏不影响评分。
- 报告可以打印或存成 PDF（`src/lib/print.ts` 和 `admin.css` 末尾的打印样式）：打印前把收起来的内容展开、换成浅色主题。报告里新加了要点开才看得到的内容，要么用 `<details>`，要么读 `PrintingContext`，不然印不出来。候选人对比也能打印，两处共用 `src/lib/print.ts` 里的 `usePrinting`；只在屏幕上有用的东西（选人的那一块、跳到别处的链接）加 `adm-no-print`，只印在纸上的抬头用 `adm-print-only`。表格在纸上不能横向滚动，新加的宽表格要在打印样式里让长的那一列换行。
- 面试概览里的「归档」和「彻底删除」是两回事：归档（`DELETE /interviews/{id}`）只是不在列表里显示，数据还在；彻底删除（`POST /interviews/{id}/purge`）把录像、题目、对话和报告真的删掉，只有管理员看得到，要输候选人的名字才能点。文案别把两个说混。候选人详情页也一样：归档是 `DELETE /candidates/{id}`，彻底删除（`usePurgeCandidate`）先把他名下的面试一场场彻底删除（归档的也算），再调 `POST /candidates/{id}/purge`，中间哪一场删不掉就停下来。岗位只有归档（`DELETE /positions/{id}`），没有彻底删除。归档的候选人和岗位后端不让再改、不让再安排面试，详情页上这些按钮要跟着收起来；列表上勾「带上已归档的」（地址栏 `?archived=1`）才看得到它们。归档的详情页上有「恢复」（`POST /candidates/{id}/restore`、`POST /positions/{id}/restore`），点了就回到列表里，不用确认。 归档了的面试在面试列表勾上「带上已归档的」能找到（标着「已归档」），打开只能看：不能再改设置、改题目、生成链接。归档的面试详情页上也有「恢复」（`POST /interviews/{id}/restore`），恢复以后没过期、没撤销的链接又能用。
- 候选人对比（`/positions/:id/comparison`，`pages/PositionComparison.tsx`）：对比的正文里候选人是代号 A、B、C，页面用 `src/lib/comparison.ts` 的 `namesInText` 换成姓名。按建议的档次分组，组里不显示名次；`close_to` 标成「分不出先后」；诚信迹象只说「要人工复核」；`caveats` 直接摆出来。这几条是报告线实测后提的要求（issue #238），改展示时别丢。
- 出题不会去读候选人之前的面试，只看这一场的设置。所以从一场面试接着安排时，要带的东西都放进设置里：`/interviews/new?from=<面试 id>`（「再安排一场」）把人、岗位、设置和声音原样带过去；再加 `&round=next`（「安排下一轮」）按那一场的报告把设置改成深入面试，改法在 `src/lib/nextRound.ts`（报告建议的方向进重点考察、建议的问题进必须问的题、结论记进「之前已经了解到的情况」、流程说明换掉）。报告里新加了对下一轮有用的内容，也在这里接进去。面试详情最上面的「下一步」提示在决定记成「进入下一轮」后会接着提示安排下一轮，已经有更晚安排的一场时改成说那一场的状态。
- 面试详情「题目」页签里改措辞、去掉一道题或一句追问、加一道自己的题，都是把整份计划 `PUT /interviews/{id}/plan` 回去：后端只给文字变了的和新增的句子合成语音，去掉的句子的语音跟着清掉，预计时长重新算，至少要留一道主问题。加题不调模型（`src/lib/plan.ts` 的 `addQuestion`）：排在主问题最后、备用题前面，标成必问题，重新出题时不会保留。调先后、「时间紧也要问」、主问题和备用题互换在一个窗口里调好再一起存（`ArrangeQuestionsDialog`）：每存一次面试都要回到「准备中」走一遍，不要做成点一下存一次；只在主问题之间挪，备用题要排在所有主问题后面（改成备用的排到备用题最前面，改成主问题的排到主问题最后），备用题不能是必问的。面试中时间不够时没标必问（`required`）的题会被跳过，越靠后越容易轮不上，题目清单里每道主问题都写着是哪一种。每句话的合成状态只在面试「准备中」时定时取，准备结束的那一下要再取一次
- 完整的面试链接后端只在生成时返回一次，页面上拿到以后要让 HR 当场复制，不能指望之后再取。单场在面试详情的「链接」里生成；一个岗位下题目准备好、还没发链接的，岗位页可以一起生成（`BatchInvitesDialog`）：一场一场来，没成的不影响后面的，生成完窗口只能点底下的按钮关，还有没复制过的会先提醒。两处给候选人的那段话是同一份，在 `src/lib/invite.ts`，改措辞只改那里。
- 页面上「有几场」这样的数字用 `useInterviewCounts()`（`GET /api/v1/interviews/counts`：各个状态、各个决定各有几场，等做决定的、报告失败的、链接过期的各有几场，不含已归档的），不要翻列表自己数：列表一页最多 100 场，面试多了会数少。总览的「等你处理」列的是最近 100 场里的，数字比列出来的多时底下说明还有几场没列；`todoCount` 和 `todoOf` 是同一套条件，改一个要改另一个。
- 链接到期以后后端不改面试的状态，还是 `invited`。列表上显示面试到哪一步用 `InterviewStatusBadge`（`src/components/common.tsx`），它拿列表项的 `invite_expires_at` 看，过期的标成「链接已过期」；要自己判断就用 `src/lib/format.ts` 的 `inviteExpired`。不要直接拿 `status` 去写「已发链接」。深入面试用 `StageBadge` 标出来，第一轮初筛不标。演示数据里的到期时间是假后端按链接现算的，和后端一样取没撤销的链接里最晚的那个。
- 一批面试一起做的事都放在岗位页「这个岗位的面试」上面：准备失败的「一起重试」、草稿「一起准备」（`PrepareAll`）、准备好的「一起生成链接」、链接过期候选人还没面的「一起换新链接」（和生成链接是同一个 `BatchInvitesDialog`，带 `renew`）。做法一样：按状态各取一页（最多 100 场）来数，动手时用 `src/lib/schedule.ts` 的 `scheduleAll` 一场一场来，没成的不影响后面的，最后说明是谁、为什么，全部做完再刷新一次列表。
- 诚信相关的内容写成「值得核对的迹象」，带上别的可能解释，不写成结论。
- 「使用说明」页（`/help`，`src/pages/Help.tsx`）把一场面试从头到尾讲给第一次用后台的人。上面写的都和后端实际做的核对过：改了面试怎么流转、链接、续面、报告怎么读，或者改了页面上按钮和页签的名字，回来看这一页要不要跟着改。页面上的链接指向的页面都要存在（`Help.test.tsx` 会一个个打开）。
- 页面的标题层级照 `packages/ui/README.md` 的约定走：页面顶上一级，卡片标题二级（`CardHeader` 自己就是），卡片里面再分小节用三级。改了页面结构以后在真实后端上用 axe-core 把每一页查一遍，浅色、深色、电脑和手机宽度都应该是 0 个问题（2026-11-16 查过）。
