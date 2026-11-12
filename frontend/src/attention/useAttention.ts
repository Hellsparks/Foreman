/** Open Attention items, live (§3): one list for the bell and the Attention page. */
import { useCallback, useEffect, useState } from "react";
import { api, type AttentionItem } from "../api/client";
import { useEventStream } from "../hooks/useEventStream";

const TOPICS = ["attention.added", "attention.updated", "attention.cleared"];

export function useAttention() {
  const [items, setItems] = useState<AttentionItem[]>([]);
  const [loaded, setLoaded] = useState(false);
  const reload = useCallback(async () => {
    try {
      setItems(await api.listAttention());
    } catch (e) {
      console.error("Failed to load attention:", e);
    } finally {
      setLoaded(true);
    }
  }, []);
  useEffect(() => {
    void reload();
  }, [reload]);
  const onEvent = useCallback(() => void reload(), [reload]);
  useEventStream(onEvent, TOPICS);
  const remove = useCallback((id: number) => setItems((prev) => prev.filter((i) => i.id !== id)), []);
  return { items, loaded, reload, remove };
}
