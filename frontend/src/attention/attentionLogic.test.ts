import { describe, expect, it } from "vitest";
import { ATTENTION_PREFIX } from "../api/maintenance";
import { relTime } from "../ui/relTime";
import { attentionTone, attentionWord, groupsOf, isService, serviceTask } from "./attentionLogic";

describe("attentionLogic", () => {
  it("names and tones", () => {
    expect(attentionWord("print_finished")).toBe("Clear plate");
    expect(attentionWord(`${ATTENTION_PREFIX}7`)).toBe("Service");
    expect(attentionTone("error")).toBe("bad");
    expect(attentionTone("offline")).toBe("off");
    expect(isService(`${ATTENTION_PREFIX}7`)).toBe(true);
    expect(serviceTask(`${ATTENTION_PREFIX}7`)).toBe("7");
  });
  it("groups for the filter", () => {
    expect(groupsOf(["error", "print_finished", "error", `${ATTENTION_PREFIX}1`])).toEqual([
      { group: "Error", n: 2 },
      { group: "Clear plate", n: 1 },
      { group: "Service", n: 1 },
    ]);
  });
});

describe("relTime", () => {
  const now = Date.parse("2026-11-12T12:00:00Z");
  it("is relative for recent times", () => {
    expect(relTime("2026-11-12T11:59:50Z", now)).toBe("just now");
    expect(relTime("2026-11-12T11:56:00Z", now)).toBe("4 min ago");
    expect(relTime("2026-11-12T09:00:00Z", now)).toBe("3 h ago");
    expect(relTime("2026-11-10T12:00:00Z", now)).toBe("2 d ago");
    expect(relTime(null, now)).toBe("");
    expect(relTime("nope", now)).toBe("");
  });
});
