import type { McpPolicy } from "../api/types/McpPolicy";
import type { McpTool } from "../api/types/McpTool";

/** `http://host:port` + `/mcp`, without a doubled slash. */
export function mcpUrl(base: string, path: string): string {
  return base.replace(/\/+$/, "") + (path.startsWith("/") ? path : `/${path}`);
}

/** The page's own origin, unless it is loopback and the server reports another address
 *  (a client on another machine can't use 127.0.0.1). */
export function preferredBase(here: string, addresses: string[]): string {
  const loopback = /^https?:\/\/(localhost|127\.|\[::1\])/.test(here);
  if (!loopback) return here;
  return addresses.find((a) => !/^https?:\/\/(localhost|127\.|\[::1\])/.test(a)) ?? here;
}

/** Tool names ticked on the allow list (`null` = every read-only tool). */
export function effectiveTools(p: McpPolicy, tools: McpTool[]): string[] {
  if (p.tools == null) return tools.filter((t) => t.class === "read").map((t) => t.name);
  return p.tools;
}

/** Copyable client set-up: a `claude mcp add` line and an `mcpServers` JSON block. */
export function clientSnippets(url: string): { cli: string; json: string } {
  const key = "fmk_…";
  const cli = `claude mcp add --transport http foreman ${url} --header "Authorization: Bearer ${key}"`;
  const json = JSON.stringify(
    { mcpServers: { foreman: { type: "http", url, headers: { Authorization: `Bearer ${key}` } } } },
    null,
    2,
  );
  return { cli, json };
}
