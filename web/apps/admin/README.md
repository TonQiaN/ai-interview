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
- 报告里的判断下面跟着原话，点一下跳到回看的那个时间。新加的报告内容也照这个做，简历里的原文没有时间，不做成可点的。
- 报告可以打印或存成 PDF（`src/lib/print.ts` 和 `admin.css` 末尾的打印样式）：打印前把收起来的内容展开、换成浅色主题。报告里新加了要点开才看得到的内容，要么用 `<details>`，要么读 `PrintingContext`，不然印不出来。
- 面试概览里的「归档」和「彻底删除」是两回事：归档（`DELETE /interviews/{id}`）只是不在列表里显示，数据还在；彻底删除（`POST /interviews/{id}/purge`）把录像、题目、对话和报告真的删掉，只有管理员看得到，要输候选人的名字才能点。文案别把两个说混。候选人详情页也一样：归档是 `DELETE /candidates/{id}`，彻底删除（`usePurgeCandidate`）先把他名下的面试一场场彻底删除（归档的也算），再调 `POST /candidates/{id}/purge`，中间哪一场删不掉就停下来。岗位只有归档（`DELETE /positions/{id}`），没有彻底删除。归档的候选人和岗位后端不让再改、不让再安排面试，详情页上这些按钮要跟着收起来；列表上勾「带上已归档的」（地址栏 `?archived=1`）才看得到它们。归档的详情页上有「恢复」（`POST /candidates/{id}/restore`、`POST /positions/{id}/restore`），点了就回到列表里，不用确认。 归档了的面试在面试列表勾上「带上已归档的」能找到（标着「已归档」），打开只能看：不能再改设置、改题目、生成链接。归档的面试详情页上也有「恢复」（`POST /interviews/{id}/restore`），恢复以后没过期、没撤销的链接又能用。
- 候选人对比（`/positions/:id/comparison`，`pages/PositionComparison.tsx`）：对比的正文里候选人是代号 A、B、C，页面用 `src/lib/comparison.ts` 的 `namesInText` 换成姓名。按建议的档次分组，组里不显示名次；`close_to` 标成「分不出先后」；诚信迹象只说「要人工复核」；`caveats` 直接摆出来。这几条是报告线实测后提的要求（issue #238），改展示时别丢。
- 诚信相关的内容写成「值得核对的迹象」，带上别的可能解释，不写成结论。
