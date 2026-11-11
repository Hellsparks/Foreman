import { useSignedUrl } from "../hooks/useSignedUrl";

/** Library thumbnail (signed in the desktop app); dashed box when there is none. */
export default function Thumb({ url, big, alt = "" }: { url: string | null | undefined; big?: boolean; alt?: string }) {
  const src = useSignedUrl(url ?? null);
  const cls = `prj-thumb${big ? " prj-thumb--big" : ""}`;
  if (!url || !src) return <span className={`${cls} prj-thumb--empty`} aria-hidden={!alt}>no image</span>;
  return <img className={cls} src={src} alt={alt} loading="lazy" />;
}

/** Colour dot: `RRGGBB` or none. */
export function Dot({ hex, big }: { hex: string | null | undefined; big?: boolean }) {
  const h = hex?.replace(/^#/, "");
  return (
    <span
      aria-hidden="true"
      className={`prj-dot${big ? " prj-dot--big" : ""}${h ? "" : " prj-dot--none"}`}
      style={h ? { background: `#${h}` } : undefined}
    />
  );
}
