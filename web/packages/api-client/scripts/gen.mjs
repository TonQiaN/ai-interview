// 由后端的 OpenAPI 文档生成 TypeScript 类型。
//
//   pnpm gen:api                      让 cargo 跑一次后端的 openapi 子命令（不用数据库）
//   OPENAPI_JSON=path pnpm gen:api    用现成的文档（CI 里先生成好再传进来）
//
// 生成的文件在 src/generated/ 下，不进 Git。改了后端接口之后重新跑一遍。
import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import openapiTS, { astToString } from "openapi-typescript";

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, "../../../..");
const outDir = resolve(here, "../src/generated");

function loadSpec() {
  const provided = process.env.OPENAPI_JSON;
  if (provided) {
    return readFileSync(resolve(process.cwd(), provided), "utf8");
  }
  return execFileSync("cargo", ["run", "--quiet", "-p", "interview-server", "--", "openapi"], {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    stdio: ["ignore", "pipe", "inherit"],
  });
}

const specText = loadSpec();
const spec = JSON.parse(specText);
const ast = await openapiTS(spec, { alphabetize: true });
const banner = "// 这个文件是生成的，不要手改。重新生成：pnpm gen:api\n\n";

mkdirSync(outDir, { recursive: true });
writeFileSync(resolve(outDir, "openapi.json"), `${JSON.stringify(spec, null, 2)}\n`);
writeFileSync(resolve(outDir, "schema.d.ts"), banner + astToString(ast));
console.log(
  `已生成接口类型：${Object.keys(spec.paths ?? {}).length} 个路径，${Object.keys(spec.components?.schemas ?? {}).length} 个类型`,
);
