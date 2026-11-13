import { Banner, Card, CardBody, CardHeader, KeyValue, PageHeader } from "@ai-interview/ui";
import { Link } from "react-router";
import { isMock } from "../api/http";
import { useConfigDefaults, useSystemStatus } from "../api/queries";
import { configSummary } from "../components/ConfigForm";
import { QueryView } from "../components/common";
import { SystemStatusList } from "../components/SystemStatusList";

/** 系统：各个依赖是不是就绪，和面试设置的默认值、可调范围。 */
export function SettingsPage() {
  const system = useSystemStatus();
  const defaults = useConfigDefaults();

  return (
    <>
      <PageHeader title="系统" description="面试要用到的几样东西是不是都在正常工作。" />
      {isMock && (
        <Banner tone="info" title="现在看到的是演示数据">
          页面连的是浏览器里的假后端，数据都是编的，刷新页面会恢复原样。接上真后端后这条提示会消失。
        </Banner>
      )}
      <div className="adm-split adm-split--even adm-section">
        <Card>
          <CardHeader title="运行状态" />
          <CardBody>
            <QueryView query={system}>
              {({ components }) => <SystemStatusList components={components} />}
            </QueryView>
          </CardBody>
        </Card>
        <Card>
          <CardHeader
            title="面试设置的默认值"
            actions={
              <Link to="/templates" className="adm-link">
                管理模板
              </Link>
            }
          />
          <CardBody>
            <QueryView query={defaults}>
              {({ config, limits }) => (
                <>
                  <p className="adm-report-intro">
                    没有选模板时，新安排的面试用这一套：{configSummary(config)}。
                  </p>
                  <KeyValue
                    items={[
                      [
                        "预期时长",
                        `${limits.target_duration_minutes.min} 到 ${limits.target_duration_minutes.max} 分钟`,
                      ],
                      [
                        "题目数",
                        `${limits.question_count.min} 到 ${limits.question_count.max} 道，或者按时长自动安排`,
                      ],
                      [
                        "停顿多久算答完",
                        `${limits.silence_sec.suggested.join("、")} 秒，也可以在 ${limits.silence_sec.min} 到 ${limits.silence_sec.max} 秒之间自己定`,
                      ],
                      ["一题最多答多久", `${limits.max_sec.min} 到 ${limits.max_sec.max} 秒`],
                      ["每题最多追问", `${limits.follow_ups_per_question.max} 次`],
                      ["大模型", "DeepSeek Flash，思考可以开关，强度分三档"],
                    ]}
                  />
                </>
              )}
            </QueryView>
          </CardBody>
        </Card>
      </div>
    </>
  );
}
