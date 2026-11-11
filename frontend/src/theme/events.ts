/** Fired on window after a user theme is applied, removed, or its fonts load. */
export const THEME_APPLIED_EVENT = "foreman:theme-applied";

export function announceThemeApplied(): void {
  window.dispatchEvent(new Event(THEME_APPLIED_EVENT));
}
