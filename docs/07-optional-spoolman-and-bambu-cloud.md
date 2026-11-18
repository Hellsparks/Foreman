# Foreman — optional Spoolman, and Bambu Cloud with multiple accounts

Written 18 Nov 2026, after the first release. Two features, planned together because both change what the compatibility engine is allowed to assume.

---

## A. Filament tracking becomes optional

Spoolman is currently load-bearing: the guard, the compatibility engine, reservations, deduction, inventory, project colour slots and the exports all assume spools exist. Plenty of people just want a fleet dashboard.

### The setting
Settings → Filament: **Full** (built-in Spoolman, today's behaviour) · **External** (one-time import, as the wizard does) · **Off**.

Serving the Spoolman API on 7912 is a *separate* switch, defaulting to on in Full and off in Off — because Moonraker and other clients may point at it, and silently dropping that is worse than the feature being off.

### What "Off" means, concretely
- **Navigation:** Filament section hidden. Printer page loses the Spools tab. Tools loses the HueForge and swatch cards that need TD values; the Orca profile export stays but exports only what the files know.
- **Compatibility engine:** drops the verdicts that need spools — Colour, Material, No spool. Keeps Ready, Busy, Plate, Too big, Nozzle, Offline, Not supported, and the profile match levels. This must be a property of the verdict set, not an `if` sprinkled through the UI.
- **Guard:** off. No abrasive-vs-nozzle check either, since that reads the filament's flag — unless the printer is marked hardened and the *file* says CF/GF, which still works from file metadata alone.
- **Reservations and deduction:** off. Jobs still record the grams the file says were used, so history and statistics keep working; nothing is subtracted from anything.
- **Projects:** colour slots become plain named slots ("PRIMARY — PLA White") taken from the slice, with no spool picker. Everything else — copies, plates, fan-out, progress — is unchanged.
- **Attention:** no runout-to-load-spool, no low-spool items. Runout still raises "paused, filament runout" from the printer itself.
- **Library and Send:** the file's own material and colour still show; the spool hint from the Orca preset is ignored.

### Switching
Turning it off releases open reservations, keeps all records, and warns that projects lose their spool assignments. Turning it back on restores the data (nothing is deleted) but not the assignments. Both directions need a confirm that says what will happen.

### Done means
A fresh install with tracking Off never mentions a spool, and Send, queue, projects and jobs all work end to end.

---

## B. Bambu Cloud, with multiple accounts

### Why this is two tiers, not one
Monitoring and controlling are different problems:

- **Monitoring** — sign in, list devices, live status over cloud MQTT, print history — is plain HTTPS and MQTT. Other projects do it without any Bambu binary. **Foreman can do this natively in Rust.**
- **Sending a print** — uploading the 3MF to Bambu's storage and dispatching it — is the part the closed network plugin owns. Current firmware also refuses control commands from unauthorized software. The open reimplementations (Open Bamboo Networking, and the Conservancy's clean-room effort) exist but are moving targets, and the wrappers around the proprietary library drag a C++ dependency into a Rust codebase.

So: build tier 1 now, natively. Leave tier 2 behind a flag, as a helper process, and only if it still looks worth it after tier 1 ships. Foreman can already print to Bambu over LAN when the printer is in Developer Mode; cloud dispatch is a convenience, not the only path.

### Multiple accounts
This is the headline requirement, so the model starts there rather than being retrofitted.

- A `bambu_account` record: label, region (global or China — different endpoints), the stored token, its expiry, and the sign-in state.
- A printer belongs to **one** account. Accounts are independent: tokens, refresh, failures and Attention items are per account.
- The device list is per account, so adding a printer means picking the account first, then a device from that account's list.
- Nothing is global. If three accounts are signed in and one token dies, the other two keep working and only that one raises an Attention item.

### Sign-in
Bambu requires a second factor: a six-digit code by email, or TOTP from an authenticator app, depending on the account. Some accounts (SSO sign-ups) need a password set in the Bambu app first, and there's a token-paste fallback for accounts that can't do password login at all.

So the sign-in flow is: email + password → the server tells us which second factor → prompt for the code → store the token. Plus "paste a token" as an alternative on the same screen. Tokens are long-lived but not permanent: store the expiry, warn ahead of it, and raise an Attention item when a re-sign-in is needed rather than silently going offline.

Credentials are stored encrypted with the same key as SSH passwords, and the password itself is never kept — only the token.

### Cloud as a connection path
This fits the existing tiered-path model from Phase 2.4: a Bambu printer gets LAN first, cloud last. Status comes from whichever path is live, and the UI shows "via cloud" the same way it shows "via Tailscale". When only cloud is available, the printer is monitor-only until tier 2 exists, and the UI says so instead of failing on click.

### Done means
Two different Bambu accounts signed in at once, each listing its own printers, both showing live status over cloud with the printers off the LAN — and one account's token expiring doesn't disturb the other.

---

## Session order

1. **Filament tracking Off** — one session. Self-contained, and it simplifies the compatibility engine before the cloud work touches it.
2. **Bambu accounts + sign-in** — accounts model, 2FA flow, token storage, Settings → Integrations → Bambu Cloud.
3. **Cloud transport** — device list, cloud MQTT status, cloud as a connection path, print history import.
4. *(Later, optional)* **Cloud dispatch** — only after a fresh look at whether the open plugin is stable enough to depend on.
