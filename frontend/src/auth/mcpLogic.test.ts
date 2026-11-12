import { describe, expect, it } from "vitest";
import type { McpTool } from "../api/types/McpTool";
import { clientSnippets, effectiveTools, mcpUrl, preferredBase } from "./mcpLogic";

const tools: McpTool[] = [
  { name: "get_fleet_status", title: "", description: "", class: "read" },
  { name: "create_vendor", title: "", description: "", class: "write" },
  { name: "pause_print", title: "", description: "", class: "destructive" },
];

describe("mcpLogic", () => {
  it("builds the URL", () => {
    expect(mcpUrl("http://10.0.0.10:9090/", "/mcp")).toBe("http://10.0.0.10:9090/mcp");
    expect(mcpUrl("http://h:1", "mcp")).toBe("http://h:1/mcp");
  });

  it("prefers a LAN address over loopback", () => {
    const lan = ["http://127.0.0.1:9090", "http://10.0.0.10:9090"];
    expect(preferredBase("http://127.0.0.1:9090", lan)).toBe("http://10.0.0.10:9090");
    expect(preferredBase("http://localhost:9090", lan)).toBe("http://10.0.0.10:9090");
    expect(preferredBase("http://10.0.0.10:9090", lan)).toBe("http://10.0.0.10:9090");
    expect(preferredBase("http://127.0.0.1:9090", [])).toBe("http://127.0.0.1:9090");
  });

  it("defaults the allow list to read-only tools", () => {
    expect(effectiveTools({ tools: null, allow_destructive: false, rate_per_minute: 60 }, tools)).toEqual([
      "get_fleet_status",
    ]);
    expect(
      effectiveTools({ tools: ["pause_print"], allow_destructive: true, rate_per_minute: 60 }, tools),
    ).toEqual(["pause_print"]);
  });

  it("makes a valid client config", () => {
    const s = clientSnippets("http://h:9090/mcp");
    const cfg = JSON.parse(s.json);
    expect(cfg.mcpServers.foreman).toEqual({
      type: "http",
      url: "http://h:9090/mcp",
      headers: { Authorization: "Bearer fmk_…" },
    });
    expect(s.cli).toContain("--transport http foreman http://h:9090/mcp");
  });
});
