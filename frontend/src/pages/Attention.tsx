/**
 * Attention page (§3, doc 06): every open item, filtered by type. The bell
 * panel's "View all" opens it. History of cleared items needs a history
 * endpoint (backlog in doc 05).
 */
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import AttentionList from "../attention/AttentionList";
import { attentionGroup, groupsOf } from "../attention/attentionLogic";
import { useAttention } from "../attention/useAttention";

export default function Attention() {
  const { items, loaded, remove } = useAttention();
  const [group, setGroup] = useState<string | null>(null);
  const [names, setNames] = useState<Record<number, string>>({});

  useEffect(() => {
    api
      .listPrinters()
      .then((ps) => setNames(Object.fromEntries(ps.map((p) => [p.id, p.name]))))
      .catch(() => {});
  }, []);

  const groups = useMemo(() => groupsOf(items.map((i) => i.kind)), [items]);
  const active = group != null && groups.some((g) => g.group === group) ? group : null;
  const shown = active == null ? items : items.filter((i) => attentionGroup(i.kind) === active);

  return (
    <div className="attn-page">
      <div className="page-header" style={{ marginBottom: 0 }}>
        <h1 className="page-header__title phone-dup-title">Attention</h1>
      </div>
      {groups.length > 1 && (
        <div className="attn-page__filters" role="group" aria-label="Filter by type">
          <button type="button" className="attn-chip" aria-pressed={active == null} onClick={() => setGroup(null)}>
            All · {items.length}
          </button>
          {groups.map((g) => (
            <button key={g.group} type="button" className="attn-chip" aria-pressed={active === g.group} onClick={() => setGroup(g.group)}>
              {g.group} · {g.n}
            </button>
          ))}
        </div>
      )}
      {loaded ? <AttentionList items={shown} printerNames={names} onResolved={remove} /> : <p className="attn-empty">Loading…</p>}
    </div>
  );
}
