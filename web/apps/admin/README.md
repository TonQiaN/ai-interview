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

## 演示数据

`dev:demo` 和 `build:demo`（也就是 `--mode demo`，或者设 `VITE_API_MOCK=1`）会把请求交给浏览器里的一套假后端（`src/api/mock/`），右上角显示「演示数据」。这时邮箱和密码随便填都能进（密码填 `wrong` 可以看登录失败的样子），所有改动只存在内存里，刷新就没了。正常构建的产物里不带假后端。

演示数据里的面试、逐字稿和报告来自分析报告线的合成样例（`crates/interview-report/samples/`），人名、手机号、邮箱都是编的。假后端返回的结构和真实接口一样，测试（`src/api/mock/server.test.ts`）也是对着它跑的。

演示数据里没有语音模型，试听和参考录音播放的是一段提示音，页面顶上会有一句说明。真实后端没连上 Mac 上的模型时也是这样（接口会返回 `simulated: true`）。

假后端里没有真的录像文件，回看只能按时间轴放逐字稿。分段录像的连续播放（`src/lib/clock.ts`）要连真后端才看得到。

## 目录

- `src/api/`：接口类型（`types.ts`）、请求（`http.ts`、`queries.ts`）、假后端（`mock/`）。
- `src/pages/`：一个页面一个文件。
- `src/components/`：页面之间共用的部件，比如面试设置表单（`ConfigForm`）、回看播放器（`ReplayPlayer`）、报告（`ReportView`）。
- `src/lib/`：枚举到中文文案的对照表、时间和时长的格式化、按设置估算题目数量（`budget.ts`，算法和后端一致）、回看的时钟（`clock.ts`）。
- `src/admin.css`：后台自己的排版。颜色、间距、字号只用设计系统的 `--oil-*` 变量。

## 约定

- 接口返回的每个枚举值都要在 `src/lib/format.ts` 里有中文文案。`src/api/mock/samples.test.ts` 会拿报告线的样例检查，样例里出现没有文案的值测试就失败。
- 报告里的判断下面跟着原话，点一下跳到回看的那个时间。新加的报告内容也照这个做，简历里的原文没有时间，不做成可点的。
- 诚信相关的内容写成「值得核对的迹象」，带上别的可能解释，不写成结论。
