import { useState, useCallback } from "react";

export interface ToastItem {
  id: number;
  text: string;
  onUndo?: () => void;
}

interface ToastStackProps {
  toasts: ToastItem[];
  onDismiss: (id: number) => void;
}

/** Fixed-position toast stack, bottom-right. Renders nothing when empty. */
export function ToastStack({ toasts, onDismiss }: ToastStackProps) {
  if (toasts.length === 0) return null;
  return (
    <div role="status" aria-live="polite" className="ui-toast-stack">
      {toasts.map((t) => (
        <div key={t.id} className="ui-toast">
          <span className="ui-toast__text">{t.text}</span>
          {t.onUndo && (
            <button
              type="button"
              className="ui-toast__undo"
              onClick={() => {
                t.onUndo?.();
                onDismiss(t.id);
              }}
            >
              Undo
            </button>
          )}
          <button
            type="button"
            className="ui-toast__dismiss"
            aria-label="Dismiss notification"
            onClick={() => onDismiss(t.id)}
          >
            ✕
          </button>
        </div>
      ))}
    </div>
  );
}

let _nextId = 0;

/** Manages a list of toasts; pair with <ToastStack>. */
export function useToasts() {
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const addToast = useCallback((text: string, onUndo?: () => void) => {
    const id = ++_nextId;
    setToasts((prev) => [...prev, { id, text, onUndo }].slice(-3));
  }, []);

  const removeToast = useCallback((id: number) => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, []);

  return { toasts, addToast, removeToast };
}
