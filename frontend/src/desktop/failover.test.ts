import { describe, expect, it } from "vitest";
import { Failover } from "./failover";
import { addressKind, sameAddress, withPort } from "./addresses";

const ID = "srv-1";

/** Fake network: address → server id answering there (null = down). */
function net(initial: Record<string, string | null>) {
  const answers = { ...initial };
  return {
    answers,
    check: async (a: string) => answers[a] ?? null,
  };
}

function setup(answers: Record<string, string | null>, addresses = ["lan:9090", "ts:9090"]) {
  let t = 0;
  const n = net(answers);
  const switches: string[] = [];
  const f = new Failover(ID, addresses, n.check, (a) => switches.push(a), {
    failAfter: 3,
    backAfterMs: 60_000,
    now: () => t,
  });
  return { f, n, switches, advance: (ms: number) => (t += ms) };
}

describe("Failover", () => {
  it("connects to the first address that answers with the right id", async () => {
    const { f, switches } = setup({ "lan:9090": null, "ts:9090": ID });
    await f.tick();
    expect(switches).toEqual(["ts:9090"]);
    expect(f.snapshot.active).toBe(1);
    expect(f.snapshot.status).toBe("ok");
  });

  it("refuses an address that answers with a different server id", async () => {
    const { f, switches } = setup({ "lan:9090": "someone-else", "ts:9090": ID });
    await f.tick();
    expect(switches).toEqual(["ts:9090"]);
  });

  it("switches only after 3 failed checks, then to the next address", async () => {
    const { f, n, switches } = setup({ "lan:9090": ID, "ts:9090": ID });
    await f.tick();
    expect(switches).toEqual(["lan:9090"]);
    n.answers["lan:9090"] = null;
    await f.tick();
    await f.tick();
    expect(f.snapshot.active).toBe(0);
    await f.tick();
    expect(switches).toEqual(["lan:9090", "ts:9090"]);
    expect(f.snapshot.active).toBe(1);
  });

  it("moves back to the primary after 60 s healthy", async () => {
    const { f, n, switches, advance } = setup({ "lan:9090": null, "ts:9090": ID });
    await f.tick();
    n.answers["lan:9090"] = ID;
    await f.tick();
    advance(30_000);
    await f.tick();
    expect(f.snapshot.active).toBe(1);
    advance(30_000);
    await f.tick();
    expect(f.snapshot.active).toBe(0);
    expect(switches).toEqual(["ts:9090", "lan:9090"]);
  });

  it("a primary blip resets the 60 s window", async () => {
    const { f, n, advance } = setup({ "lan:9090": null, "ts:9090": ID });
    await f.tick();
    n.answers["lan:9090"] = ID;
    await f.tick();
    advance(50_000);
    n.answers["lan:9090"] = null;
    await f.tick();
    n.answers["lan:9090"] = ID;
    advance(20_000);
    await f.tick();
    expect(f.snapshot.active).toBe(1);
  });

  it("is lost only when every address is down, and recovers", async () => {
    const { f, n, switches } = setup({ "lan:9090": ID, "ts:9090": null });
    await f.tick();
    n.answers["lan:9090"] = null;
    await f.tick();
    await f.tick();
    await f.tick();
    expect(f.snapshot.status).toBe("lost");
    expect(f.snapshot.perAddress).toEqual(["down", "down"]);
    n.answers["ts:9090"] = ID;
    await f.tick();
    expect(f.snapshot.status).toBe("ok");
    expect(switches).toEqual(["lan:9090", "ts:9090"]);
  });
});

describe("addresses", () => {
  it("adds the default port", () => {
    expect(withPort("10.0.0.20")).toBe("10.0.0.20:9090");
    expect(withPort("pi.local:8080")).toBe("pi.local:8080");
    expect(withPort("https://pi.ts.net")).toBe("https://pi.ts.net:9090");
  });

  it("compares addresses loosely", () => {
    expect(sameAddress("Pi.local", "pi.local:9090")).toBe(true);
    expect(sameAddress("10.0.0.20", "10.0.0.21")).toBe(false);
  });

  it("labels routes", () => {
    expect(addressKind("10.0.0.20:9090")).toBe("LAN IP");
    expect(addressKind("pi.local:9090")).toBe("LAN name");
    expect(addressKind("100.101.5.7:9090")).toBe("Tailscale IP");
    expect(addressKind("pi.tail1234.ts.net:9090")).toBe("Tailscale");
  });
});
