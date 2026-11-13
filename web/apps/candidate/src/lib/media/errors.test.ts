import { describe, expect, it } from "vitest";
import { classifyMediaError } from "./errors";

describe("classifyMediaError", () => {
  it.each([
    ["NotAllowedError", "permission_denied"],
    ["PermissionDeniedError", "permission_denied"],
    ["NotFoundError", "device_not_found"],
    ["NotReadableError", "device_in_use"],
    ["SecurityError", "insecure_context"],
    ["OverconstrainedError", "constraints_unsatisfied"],
    ["AbortError", "aborted"],
    ["NotSupportedError", "unsupported"],
  ])("%s → %s", (name, kind) => {
    expect(classifyMediaError({ name })).toBe(kind);
  });

  it("认不出来的都算 unknown", () => {
    expect(classifyMediaError(new Error("boom"))).toBe("unknown");
    expect(classifyMediaError(null)).toBe("unknown");
    expect(classifyMediaError("NotAllowedError")).toBe("unknown");
  });
});
